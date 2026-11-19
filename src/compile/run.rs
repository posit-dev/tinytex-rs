//! The compile loop: a port of the R package's algorithm merged with Quarto
//! 1's extra layer, each difference decided in design section 3.1.
//!
//! One [`Session`] owns the state of one compile: the run counters, the
//! packages installed so far and the diagnostics. Every `Exec::run` result
//! goes through [`Session::exec`], which applies the cancellation rule: after
//! an `Err`, a non-zero exit or a command that changes the installation,
//! `interrupted()` is tested first and wins.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Output;

use crate::config::Config;
use crate::detect::Distribution;
use crate::exec::{CommandSpec, Event, Exec, Observer};
use crate::log::{self, InstallFailure};
use crate::tlmgr::{InstallOutcome, Tlmgr, TlmgrError};

use super::{
    Bib, CompileError, CompileRequest, Compiled, Diagnostics, Driver, Engine, OnBibFailure, Recover,
};

pub(super) fn compile(
    dist: Option<&Distribution>,
    cfg: &Config,
    tex: &Path,
    req: &CompileRequest,
    x: &dyn Exec,
    o: &dyn Observer,
) -> Result<Compiled, CompileError> {
    let mut s = Session::new(dist, cfg, tex, req, x, o)?;
    match req.driver {
        Driver::Builtin => s.builtin(),
        Driver::Latexmk => s.latexmk(),
    }
}

/// The result of one engine run.
struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    log: String,
    success: bool,
}

/// What an install attempt did.
enum Attempt {
    Ran(InstallOutcome),
    /// Nothing to install, or these packages were the last attempt.
    Skipped,
    Failed(TlmgrError),
}

struct Session<'a> {
    dist: Option<&'a Distribution>,
    req: &'a CompileRequest,
    x: &'a dyn Exec,
    o: &'a dyn Observer,
    tlmgr: Option<Tlmgr<'a>>,
    /// The `.tex` file's directory: the working directory of every command.
    dir: PathBuf,
    file_name: OsString,
    stem: String,
    out_dir: PathBuf,
    min: u32,
    max: u32,
    /// Every engine run, failed ones included.
    runs: u32,
    /// Successful engine runs: what `max_runs` bounds.
    counted: u32,
    installed: Vec<String>,
    last_attempt: Vec<String>,
    diag: Diagnostics,
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn synthetic(args: &[&str], message: String) -> TlmgrError {
    TlmgrError::Command {
        program: "tlmgr".to_owned(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        code: None,
        stderr: message,
    }
}

/// `\citation`, `\bibdata` and `\bibstyle` all present (`containsBiblioData`,
/// `pdf.ts:589-593` at `v1.12.3`; R `require_bibtex`).
fn aux_wants_bibtex(aux: &str) -> bool {
    let has = |marker: &str| aux.lines().any(|l| l.starts_with(marker));
    has("\\citation{") && has("\\bibdata{") && has("\\bibstyle{")
}

/// The last lines of a tool log, for an error message.
fn excerpt(text: &str) -> String {
    let lines: Vec<&str> = text.trim().lines().collect();
    let from = lines.len().saturating_sub(40);
    lines[from..].join("\n")
}

impl<'a> Session<'a> {
    fn new(
        dist: Option<&'a Distribution>,
        cfg: &'a Config,
        tex: &Path,
        req: &'a CompileRequest,
        x: &'a dyn Exec,
        o: &'a dyn Observer,
    ) -> Result<Self, CompileError> {
        let dir = match tex.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let file_name = tex
            .file_name()
            .ok_or_else(|| CompileError::InvalidRequest {
                message: format!("`{}` is not a file name", tex.display()),
            })?
            .to_owned();
        let stem = tex
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let out_dir = match &req.output_dir {
            Some(d) => dir.join(d),
            None => dir.clone(),
        };
        let (min, max) = req.run_bounds();
        let tlmgr = match (dist, req.auto_install, req.driver) {
            (Some(d), true, Driver::Builtin) => Some(Tlmgr::new(d, cfg, x, o)),
            _ => None,
        };
        Ok(Self {
            dist,
            req,
            x,
            o,
            tlmgr,
            dir,
            file_name,
            stem,
            out_dir,
            min,
            max,
            runs: 0,
            counted: 0,
            installed: Vec::new(),
            last_attempt: Vec::new(),
            diag: Diagnostics::default(),
        })
    }

    // ---------- paths and commands ----------

    fn out_file(&self, ext: &str) -> PathBuf {
        self.out_dir.join(format!("{}.{ext}", self.stem))
    }

    /// An auxiliary file's path as the tools see it: relative to the `.tex`
    /// directory, which is their working directory.
    fn rel(&self, name: &str) -> PathBuf {
        match &self.req.output_dir {
            Some(d) => d.join(name),
            None => PathBuf::from(name),
        }
    }

    fn read(&self, path: &Path) -> Option<String> {
        std::fs::read(path).ok().map(|b| text(&b))
    }

    fn tool(&self, name: &str, override_path: &Option<PathBuf>) -> CommandSpec {
        let mut spec = match self.dist {
            Some(d) => d.command(name),
            None => CommandSpec::new(name),
        };
        if let Some(p) = override_path {
            spec.program = p.clone();
        }
        spec.cwd = Some(self.dir.clone());
        spec
    }

    fn engine_program(&self) -> Result<&'static str, CompileError> {
        match self.req.engine {
            Engine::PdfLatex | Engine::XeLatex | Engine::LuaLatex => Ok(self.req.engine.program()),
            // Plan 5.4b adds the `tectonic` and `latexmk` per-pass engines.
            other => Err(CompileError::InvalidRequest {
                message: format!(
                    "the `{}` engine is not supported by the built-in loop",
                    other.program()
                ),
            }),
        }
    }

    /// Runs one command under the cancellation rule.
    fn exec(
        &self,
        spec: &CommandSpec,
        tool: &str,
        is_engine: bool,
    ) -> Result<Output, CompileError> {
        let result = self.x.run(spec);
        let failed = !matches!(&result, Ok(out) if out.status.success());
        if (failed || spec.changes_installation) && self.x.interrupted() {
            return Err(CompileError::Cancelled);
        }
        result.map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                if is_engine && self.dist.is_none() {
                    CompileError::NoTexInstallation
                } else {
                    CompileError::EngineNotFound {
                        tool: tool.to_owned(),
                    }
                }
            } else {
                CompileError::Io(e)
            }
        })
    }

    fn emit(&self, e: Event) {
        self.o.event(&e);
    }

    fn prepare_output_dir(&self) -> Result<(), CompileError> {
        if self.req.output_dir.is_some() {
            std::fs::create_dir_all(&self.out_dir)?;
        }
        Ok(())
    }

    // ---------- the engine ----------

    fn engine_once(&mut self) -> Result<Run, CompileError> {
        let program = self.engine_program()?;
        self.runs += 1;
        self.emit(Event::RunningEngine { run: self.runs });
        let mut spec = self.tool(program, &self.req.tools.engine);
        spec.args.push("-halt-on-error".into());
        spec.args.push("-interaction=batchmode".into());
        if let Some(d) = &self.req.output_dir {
            let mut arg = OsString::from("-output-directory=");
            arg.push(d);
            spec.args.push(arg);
        }
        spec.args
            .extend(self.req.engine_args.iter().map(Into::into));
        spec.args.push(self.file_name.clone());
        let out = self.exec(&spec, program, true)?;
        Ok(self.finish_run(&out))
    }

    fn finish_run(&self, out: &Output) -> Run {
        Run {
            code: out.status.code(),
            stdout: text(&out.stdout),
            stderr: text(&out.stderr),
            log: self.read(&self.out_file("log")).unwrap_or_default(),
            success: out.status.success() && self.out_file("pdf").exists(),
        }
    }

    /// The error of a failed run: `NoOutput` when the engine exited cleanly
    /// and nothing in the log explains the missing file.
    fn failure(&self, run: &Run) -> CompileError {
        let missing = log::missing_packages(&run.log, self.missfont().as_deref());
        let error = log::latex_error(&run.log, Some(&run.stderr));
        if run.code == Some(0) && error.is_none() && missing.is_empty() {
            return CompileError::NoOutput;
        }
        let log_path = self.out_file("log");
        let (summary, hints) = match error {
            Some(e) => (Some(e.summary), e.hints),
            None => (None, Vec::new()),
        };
        CompileError::Latex {
            summary,
            hints,
            log: log_path.exists().then_some(log_path),
            missing,
            stdout: run.stdout.clone(),
            stderr: run.stderr.clone(),
            exit_code: run.code,
        }
    }

    fn missfont(&self) -> Option<String> {
        self.read(&self.out_dir.join("missfont.log"))
    }

    /// One engine run with recovery. A failed run is retried after a package
    /// install: after the first run always, and after later runs only with
    /// [`Recover::EveryRun`]. A failed run that is retried does not count
    /// toward `max_runs`; only a successful one does.
    fn engine_run(&mut self, first: bool) -> Result<Run, CompileError> {
        loop {
            let run = self.engine_once()?;
            if run.success {
                self.counted += 1;
                return Ok(run);
            }
            if first || self.req.recover == Recover::EveryRun {
                self.recover(&run)?;
            } else {
                return Err(self.failure(&run));
            }
        }
    }

    // ---------- packages ----------

    fn can_install(&self) -> bool {
        self.tlmgr.is_some()
    }

    /// Installs, at most once per distinct set (Quarto 1's `lastPkgs`). Only
    /// cancellation is an error.
    fn attempt(&mut self, pkgs: &[String]) -> Result<Attempt, CompileError> {
        let Some(tlmgr) = &self.tlmgr else {
            return Ok(Attempt::Skipped);
        };
        if pkgs.iter().all(|p| self.last_attempt.contains(p)) {
            return Ok(Attempt::Skipped);
        }
        self.last_attempt = pkgs.to_vec();
        let result = tlmgr.install(pkgs);
        if matches!(result, Err(TlmgrError::Interrupted)) || self.x.interrupted() {
            return Err(CompileError::Cancelled);
        }
        match result {
            Err(e) => Ok(Attempt::Failed(e)),
            Ok(out) => {
                for p in &out.installed {
                    if !self.installed.contains(p) {
                        self.installed.push(p.clone());
                    }
                }
                self.diag
                    .notices
                    .extend(out.warnings.iter().map(|w| w.message.clone()));
                Ok(Attempt::Ran(out))
            }
        }
    }

    /// The packages a log asks for, through `tlmgr search`.
    fn packages_for(
        &self,
        text: &str,
        missfont: Option<&str>,
    ) -> Result<Vec<String>, CompileError> {
        let Some(tlmgr) = &self.tlmgr else {
            return Ok(Vec::new());
        };
        tlmgr
            .packages_for_log(text, missfont)
            .map_err(|_| CompileError::Cancelled)
    }

    /// Installs what a failed run asks for, so the caller can run again.
    fn recover(&mut self, run: &Run) -> Result<(), CompileError> {
        if !self.can_install() {
            return Err(self.failure(run));
        }
        let missfont = self.missfont();
        if log::missing_packages(&run.log, missfont.as_deref()).is_empty() {
            return Err(self.failure(run));
        }
        let pkgs = self.packages_for(&run.log, missfont.as_deref())?;
        if pkgs.is_empty() {
            return Err(CompileError::PackageInstall {
                reason: InstallFailure::NoMatch,
                source: synthetic(
                    &["search", "--file", "--global"],
                    "no package contains the files the log asks for".to_owned(),
                ),
            });
        }
        match self.attempt(&pkgs)? {
            Attempt::Ran(_) => Ok(()),
            Attempt::Skipped => Err(CompileError::PackageInstall {
                reason: InstallFailure::InstallFailed,
                source: synthetic(
                    &["install"],
                    format!(
                        "the engine still asks for {} after an install attempt",
                        pkgs.join(", ")
                    ),
                ),
            }),
            Attempt::Failed(e) => Err(CompileError::PackageInstall {
                reason: match e {
                    TlmgrError::TexLiveTooOld { .. } => InstallFailure::TexLiveTooOld,
                    _ => InstallFailure::InstallFailed,
                },
                source: e,
            }),
        }
    }

    // ---------- the loop ----------

    fn builtin(&mut self) -> Result<Compiled, CompileError> {
        self.engine_program()?;
        self.prepare_output_dir()?;
        let mut run = self.initial()?;

        let mut index_created = false;
        if self.out_file("idx").exists() {
            // bookdown#1274: the index needs a fresh `.idx`, so run once first.
            if self.counted < self.max {
                run = self.engine_run(false)?;
            } else {
                self.diag.max_runs_reached = true;
            }
            self.make_index()?;
            index_created = true;
        }
        let bib_created = self.bibliography()?;

        let mut pending = index_created || bib_created || log::needs_rerun(&run.log);
        while pending || self.counted < self.min {
            if self.counted >= self.max {
                self.diag.max_runs_reached = true;
                break;
            }
            run = self.engine_run(false)?;
            pending = log::needs_rerun(&run.log);
        }

        self.diag.warnings = log::latex_warnings(&run.log);
        self.diag.accessibility = log::accessibility(&run.log);
        Ok(self.compiled())
    }

    /// The first run, repeated while installs of hyphenation files and
    /// modules ask for it.
    fn initial(&mut self) -> Result<Run, CompileError> {
        loop {
            let run = self.engine_run(true)?;
            let hyphenation = log::missing_hyphenation(&run.log);
            let mut wanted = hyphenation.clone();
            for module in log::missing_modules(&run.log) {
                if !wanted.contains(&module) {
                    wanted.push(module);
                }
            }
            let mut repeat = false;
            if !wanted.is_empty() && self.can_install() && self.counted < self.max {
                for file in &hyphenation {
                    self.emit(Event::InstallingHyphenation { file: file.clone() });
                }
                if let Attempt::Ran(out) = self.attempt(&wanted)? {
                    repeat = !out.installed.is_empty() || !out.failed.is_empty();
                }
            }
            if repeat {
                continue;
            }
            self.diag.missing_hyphenation = hyphenation;
            return Ok(run);
        }
    }

    // ---------- index ----------

    fn make_index(&mut self) -> Result<(), CompileError> {
        let (command, args) = match &self.req.index {
            Some(i) => (i.command.clone(), i.args.clone()),
            None => ("makeindex".to_owned(), Vec::new()),
        };
        self.emit(Event::MakingIndex {
            command: command.clone(),
        });
        let override_path = if command == "makeindex" {
            self.req.tools.makeindex.clone()
        } else {
            None
        };
        let mut spec = self.tool(&command, &override_path);
        spec.args.extend(args.iter().map(Into::into));
        spec.args
            .push(self.rel(&format!("{}.idx", self.stem)).into_os_string());
        let out = self.exec(&spec, &command, false)?;
        let ilg = self.read(&self.out_file("ilg")).unwrap_or_default();
        let log_path = self.out_file("log");
        let log = log_path.exists().then_some(log_path);
        if !out.status.success() {
            let log_excerpt = log::index_error(&ilg)
                .or_else(|| (!ilg.trim().is_empty()).then(|| excerpt(&ilg)))
                .unwrap_or_else(|| text(&out.stderr));
            return Err(CompileError::Index { log_excerpt, log });
        }
        if let Some(log_excerpt) = log::index_error(&ilg) {
            return Err(CompileError::Index { log_excerpt, log });
        }
        Ok(())
    }

    // ---------- bibliography ----------

    fn bib_choice(&self) -> Option<&'static str> {
        let bcf = self.out_file("bcf").exists();
        let bibtex = || {
            self.read(&self.out_file("aux"))
                .is_some_and(|aux| aux_wants_bibtex(&aux))
        };
        match self.req.bib {
            Bib::Skip => None,
            Bib::Biber => bcf.then_some("biber"),
            Bib::Bibtex => bibtex().then_some("bibtex"),
            _ if bcf => Some("biber"),
            _ => bibtex().then_some("bibtex"),
        }
    }

    /// Runs the bibliography tool; whether it ran.
    fn bibliography(&mut self) -> Result<bool, CompileError> {
        loop {
            let Some(tool) = self.bib_choice() else {
                return Ok(false);
            };
            self.emit(Event::RunningBibEngine {
                engine: tool.to_owned(),
            });
            let override_path = if tool == "biber" {
                self.req.tools.biber.clone()
            } else {
                self.req.tools.bibtex.clone()
            };
            let mut spec = self.tool(tool, &override_path);
            spec.args.push(self.rel(&self.stem).into_os_string());
            let out = self.exec(&spec, tool, false)?;
            if out.status.success() {
                return Ok(true);
            }

            let blg = self.read(&self.out_file("blg")).unwrap_or_default();
            let log_path = self.out_file("log");
            let log = log_path.exists().then_some(log_path);
            let log_excerpt = if blg.trim().is_empty() {
                let stderr = text(&out.stderr);
                if stderr.trim().is_empty() {
                    text(&out.stdout)
                } else {
                    stderr
                }
            } else {
                excerpt(&blg)
            };
            let failure = CompileError::Bibliography {
                engine: tool.to_owned(),
                log_excerpt: log_excerpt.clone(),
                log,
            };

            // An ` open style file ` line asks for a `.bst` package; both
            // `OnBibFailure` settings install it, and a style that cannot be
            // installed is an error under either.
            if blg.contains(" open style file ") && self.can_install() {
                let pkgs = self.packages_for(&blg, None)?;
                if !pkgs.is_empty()
                    && let Attempt::Ran(_) = self.attempt(&pkgs)?
                {
                    continue;
                }
                return Err(failure);
            }
            return match self.req.on_bib_failure {
                OnBibFailure::Error => Err(failure),
                _ => {
                    self.diag.bibliography_failed = Some(format!("{tool}: {log_excerpt}"));
                    Ok(true)
                }
            };
        }
    }

    // ---------- latexmk ----------

    fn latexmk(&mut self) -> Result<Compiled, CompileError> {
        let req = self.req;
        let engine = match req.engine {
            Engine::PdfLatex | Engine::XeLatex | Engine::LuaLatex => req.engine.program(),
            other => {
                return Err(CompileError::InvalidRequest {
                    message: format!(
                        "`Driver::Latexmk` needs a TeX engine, not `{}`",
                        other.program()
                    ),
                });
            }
        };
        let defaults = (super::DEFAULT_MIN_RUNS, super::DEFAULT_MAX_RUNS);
        let mut conflicts = Vec::new();
        if (self.min, self.max) != defaults {
            conflicts.push("run counts");
        }
        if req.bib != Bib::Infer {
            conflicts.push("a bibliography tool");
        }
        if req.index.is_some() {
            conflicts.push("an index tool");
        }
        if !conflicts.is_empty() {
            return Err(CompileError::InvalidRequest {
                message: format!(
                    "`Driver::Latexmk` runs one delegated build and cannot also set {}",
                    conflicts.join(", ")
                ),
            });
        }
        if req.auto_install {
            self.diag.notices.push(
                "auto_install is ignored with Driver::Latexmk: latexmk cannot install packages"
                    .to_owned(),
            );
        }
        self.prepare_output_dir()?;

        self.runs = 1;
        self.emit(Event::RunningEngine { run: 1 });
        let mut spec = self.tool("latexmk", &req.tools.latexmk);
        spec.args.push("-latexoption=-halt-on-error".into());
        spec.args.push("-interaction=batchmode".into());
        spec.args.push("-pdf".into());
        spec.args.push(format!("-pdflatex={engine}").into());
        if let Some(d) = &req.output_dir {
            let mut arg = OsString::from("-outdir=");
            arg.push(d);
            spec.args.push(arg);
        }
        spec.args.extend(req.engine_args.iter().map(Into::into));
        spec.args.push(self.file_name.clone());
        let out = self.exec(&spec, "latexmk", true)?;
        let run = self.finish_run(&out);
        if !run.success {
            return Err(self.failure(&run));
        }
        self.counted = 1;
        self.diag.warnings = log::latex_warnings(&run.log);
        self.diag.accessibility = log::accessibility(&run.log);
        Ok(self.compiled())
    }

    fn compiled(&mut self) -> Compiled {
        let log = self.out_file("log");
        Compiled {
            pdf: self.out_file("pdf"),
            output_kind: self.req.output,
            log: log.exists().then_some(log),
            runs: self.runs,
            installed_packages: std::mem::take(&mut self.installed),
            diagnostics: std::mem::take(&mut self.diag),
        }
    }
}
