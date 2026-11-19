//! Helpers for the crate's own tests. Not part of the public API.

use std::process::ExitStatus;

/// An `ExitStatus` with the given exit code.
#[cfg(unix)]
pub fn exit_status(code: i32) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(code << 8)
}

/// An `ExitStatus` with the given exit code.
#[cfg(windows)]
pub fn exit_status(code: i32) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(code as u32)
}

// ---------------------------------------------------------------------------
// FakeTex: a mocked `Exec` for every mocked-`Exec` test in the crate.
// ---------------------------------------------------------------------------

use std::io;
use std::path::PathBuf;
use std::process::Output;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::exec::{CommandSpec, Exec};

/// What a [`FakeTex`] answers for one command.
#[derive(Debug, Clone)]
pub struct Reply {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    files: Vec<(PathBuf, Vec<u8>)>,
    error: Option<io::ErrorKind>,
    interrupt: bool,
    delay: Duration,
}

impl Reply {
    /// Exit status 0, no output.
    pub fn ok() -> Self {
        Self::code(0)
    }

    /// The given exit status, no output.
    pub fn code(code: i32) -> Self {
        Self {
            code,
            stdout: Vec::new(),
            stderr: Vec::new(),
            files: Vec::new(),
            error: None,
            interrupt: false,
            delay: Duration::ZERO,
        }
    }

    pub fn stdout(mut self, text: impl Into<String>) -> Self {
        self.stdout = text.into().into_bytes();
        self
    }

    pub fn stderr(mut self, text: impl Into<String>) -> Self {
        self.stderr = text.into().into_bytes();
        self
    }

    /// Creates `name` in the command's `cwd` (relative names only) before it
    /// returns, as the engine does with `.log`, `.aux` and the PDF.
    pub fn creates(mut self, name: impl Into<PathBuf>, content: impl Into<Vec<u8>>) -> Self {
        self.files.push((name.into(), content.into()));
        self
    }

    /// Makes `run` return `Err` of this kind, as a killed or missing child does.
    pub fn fails(mut self, kind: io::ErrorKind) -> Self {
        self.error = Some(kind);
        self
    }

    /// Flips `interrupted()` to true once this command has returned.
    pub fn interrupts(mut self) -> Self {
        self.interrupt = true;
        self
    }

    /// Sleeps before returning, so a test can hold a lock across threads.
    pub fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

type Handler = Arc<dyn Fn(&CommandSpec) -> Option<Reply> + Send + Sync>;

struct Rule {
    program: String,
    args: Vec<String>,
    replies: Vec<Reply>,
    used: usize,
}

#[derive(Default)]
struct State {
    calls: Vec<CommandSpec>,
    rules: Vec<Rule>,
}

/// A mocked [`Exec`]: records every [`CommandSpec`] and answers from a script.
///
/// Rules match on the program's file name (without `.bat`, `.cmd` or `.exe`)
/// and an argument prefix; the first matching rule wins, and a rule with
/// several replies uses them in order, repeating the last. A command no rule
/// matches goes to the optional handler, then answers with status 0 and no
/// output.
#[derive(Default)]
pub struct FakeTex {
    state: Mutex<State>,
    handler: Mutex<Option<Handler>>,
    interrupted: AtomicBool,
}

fn program_name(spec: &CommandSpec) -> String {
    let name = spec
        .program
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    for ext in [".bat", ".cmd", ".exe"] {
        if let Some(stem) = name.strip_suffix(ext) {
            return stem.to_owned();
        }
    }
    name
}

fn arg_strings(spec: &CommandSpec) -> Vec<String> {
    spec.args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

impl FakeTex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers commands for `program` whose arguments start with `args` with
    /// `replies`, in order, repeating the last.
    pub fn on(&self, program: &str, args: &[&str], replies: Vec<Reply>) -> &Self {
        assert!(!replies.is_empty(), "a rule needs at least one reply");
        self.state.lock().unwrap().rules.push(Rule {
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            replies,
            used: 0,
        });
        self
    }

    /// [`FakeTex::on`] with a single reply.
    pub fn reply(&self, program: &str, args: &[&str], reply: Reply) -> &Self {
        self.on(program, args, vec![reply])
    }

    /// A fallback for commands no rule matches; returning `None` falls through
    /// to the default reply. It runs outside every `FakeTex` lock, so it may
    /// panic or block.
    pub fn with_handler(
        self,
        handler: impl Fn(&CommandSpec) -> Option<Reply> + Send + Sync + 'static,
    ) -> Self {
        *self.handler.lock().unwrap() = Some(Arc::new(handler));
        self
    }

    pub fn set_interrupted(&self, value: bool) {
        self.interrupted.store(value, Ordering::SeqCst);
    }

    /// Every command run so far, in order.
    pub fn calls(&self) -> Vec<CommandSpec> {
        self.state.lock().unwrap().calls.clone()
    }

    /// Every command as `program arg arg`, for readable assertions.
    pub fn lines(&self) -> Vec<String> {
        self.calls()
            .iter()
            .map(|c| {
                let mut line = program_name(c);
                for a in arg_strings(c) {
                    line.push(' ');
                    line.push_str(&a);
                }
                line
            })
            .collect()
    }

    /// How many recorded command lines start with `prefix`.
    pub fn count(&self, prefix: &str) -> usize {
        self.lines()
            .iter()
            .filter(|l| l.starts_with(prefix))
            .count()
    }

    fn answer(&self, spec: &CommandSpec) -> Reply {
        let name = program_name(spec);
        let args = arg_strings(spec);
        {
            let mut state = self.state.lock().unwrap();
            state.calls.push(spec.clone());
            for rule in &mut state.rules {
                if rule.program == name && args.starts_with(&rule.args) {
                    let i = rule.used.min(rule.replies.len() - 1);
                    rule.used += 1;
                    return rule.replies[i].clone();
                }
            }
        }
        // Clone the handler out so it runs outside every lock: it may block
        // or panic without poisoning the script.
        let handler = self.handler.lock().unwrap().clone();
        let reply = handler.and_then(|h| h(spec));
        reply.unwrap_or_else(Reply::ok)
    }
}

impl Exec for FakeTex {
    fn run(&self, spec: &CommandSpec) -> io::Result<Output> {
        let reply = self.answer(spec);
        if !reply.delay.is_zero() {
            std::thread::sleep(reply.delay);
        }
        for (name, content) in &reply.files {
            let base = spec.cwd.clone().unwrap_or_default();
            std::fs::write(base.join(name), content)?;
        }
        if reply.interrupt {
            self.set_interrupted(true);
        }
        match reply.error {
            Some(kind) => Err(io::Error::new(kind, "FakeTex: scripted failure")),
            None => Ok(Output {
                status: exit_status(reply.code),
                stdout: reply.stdout,
                stderr: reply.stderr,
            }),
        }
    }

    fn interrupted(&self) -> bool {
        self.interrupted.load(Ordering::SeqCst)
    }
}
