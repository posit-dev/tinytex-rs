//! Injected execution and progress: [`Exec`], [`Observer`], [`Event`].

use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::log::SearchTerm;

/// One subprocess to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// Overrides applied on top of the inherited environment.
    pub env: Vec<(OsString, OsString)>,
    pub cwd: Option<PathBuf>,
    pub changes_installation: bool,
}

/// Runs subprocesses; blocking.
pub trait Exec: Send + Sync {
    fn run(&self, cmd: &CommandSpec) -> io::Result<Output>;
    fn interrupted(&self) -> bool {
        false
    }
}

/// Receives progress events.
pub trait Observer: Send + Sync {
    fn event(&self, _e: &Event) {}
}

/// An [`Observer`] that ignores everything.
pub struct NullObserver;
impl Observer for NullObserver {}

/// Progress events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Event {
    Searching {
        term: SearchTerm,
    },
    InstallingPackage {
        name: String,
        index: usize,
        of: usize,
    },
    UpdatingTlmgr,
    UpdatingPackages,
    RebuildingFormats,
    RunningEngine {
        run: u32,
    },
    MakingIndex {
        command: String,
    },
    RunningBibEngine {
        engine: String,
    },
    InstallingHyphenation {
        file: String,
    },
    PackagesInstalled {
        names: Vec<String>,
    },
    Downloading {
        url: String,
        received: u64,
        total: Option<u64>,
    },
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::Searching { term } => write!(f, "finding package for {term}"),
            Event::InstallingPackage { name, index, of } => {
                write!(f, "installing package {name} ({index} of {of})")
            }
            Event::UpdatingTlmgr => f.write_str("updating tlmgr"),
            Event::UpdatingPackages => f.write_str("updating installed packages"),
            Event::RebuildingFormats => f.write_str("rebuilding formats"),
            Event::RunningEngine { run } => write!(f, "running the TeX engine (run {run})"),
            Event::MakingIndex { command } => write!(f, "making the index with {command}"),
            Event::RunningBibEngine { engine } => write!(f, "running {engine}"),
            Event::InstallingHyphenation { file } => {
                write!(f, "installing hyphenation file {file}")
            }
            Event::PackagesInstalled { names } => {
                write!(f, "installed packages: {}", names.join(", "))
            }
            Event::Downloading {
                url,
                received,
                total,
            } => match total {
                Some(t) => write!(f, "downloading {url} ({received} of {t} bytes)"),
                None => write!(f, "downloading {url} ({received} bytes)"),
            },
        }
    }
}

/// The default [`Exec`]: runs the child with [`std::process::Command`].
///
/// With [`StdExec::with_deadline`] the whole instance has one time budget,
/// measured from construction. Each child gets only the time that remains,
/// and is killed when it runs out. On Windows a kill stops the child process
/// only, not its descendants.
pub struct StdExec {
    deadline: Option<Instant>,
}

const POLL: Duration = Duration::from_millis(10);

impl StdExec {
    /// No deadline.
    pub fn new() -> Self {
        Self { deadline: None }
    }

    /// A whole-command budget, starting now.
    pub fn with_deadline(budget: Duration) -> Self {
        Self {
            deadline: Some(Instant::now() + budget),
        }
    }

    fn expired(&self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
    }
}

impl Default for StdExec {
    fn default() -> Self {
        Self::new()
    }
}

fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "the command's time budget ran out")
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    rx
}

impl Exec for StdExec {
    fn run(&self, spec: &CommandSpec) -> io::Result<Output> {
        if self.expired() {
            return Err(timed_out());
        }
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        if let Some(cwd) = &spec.cwd {
            cmd.current_dir(cwd);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let out_rx = drain(child.stdout.take());
        let err_rx = drain(child.stderr.take());
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if self.expired() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(timed_out());
            }
            std::thread::sleep(POLL);
        };
        // A grandchild may still hold a pipe open; the budget still applies.
        let collect = |rx: &mpsc::Receiver<Vec<u8>>| -> io::Result<Vec<u8>> {
            loop {
                match rx.recv_timeout(POLL) {
                    Ok(b) => return Ok(b),
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(Vec::new()),
                    Err(mpsc::RecvTimeoutError::Timeout) if self.expired() => {
                        return Err(timed_out());
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        };
        let stdout = collect(&out_rx)?;
        let stderr = collect(&err_rx)?;
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }

    fn interrupted(&self) -> bool {
        self.expired()
    }
}

impl CommandSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
            changes_installation: false,
        }
    }
}
