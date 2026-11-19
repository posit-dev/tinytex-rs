//! The compile contract and driver: [`CompileRequest`] in, [`Compiled`] or
//! [`CompileError`] out (design section 5).
//!
//! The request is plain serializable data: the library takes it directly and
//! the CLI parses its flags into it. The engine and every auxiliary tool run
//! with the `.tex` file's directory as their working directory, as Quarto 1
//! does (`latex.ts:63-64` at `v1.12.3`).

// `CompileError::Latex` carries the failed run's streams and log path inline,
// as the design contract specifies (section 5), so every `Result` of this
// module is large. A compile runs a handful of processes; boxing would only
// change the public error shape.
#![allow(clippy::result_large_err)]

use std::io;
use std::path::{Path, PathBuf};

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};

use crate::ErrorCode;
use crate::config::Config;
use crate::detect::{self, Distribution};
use crate::exec::{Exec, Observer};
use crate::log::{AccessibilityWarnings, Hint, InstallFailure, SearchTerm};
use crate::tlmgr::TlmgrError;

mod place;
mod run;

pub use place::{AUX_EXTENSIONS, aux_files};
pub(crate) use place::{move_file, tweak_aux};

/// The default total engine runs, first included (Quarto 1's `(minRuns || 1)`).
pub(crate) const DEFAULT_MIN_RUNS: u32 = 1;
/// The default maximum of engine runs (Quarto 1's `(maxRuns || 10)`).
pub(crate) const DEFAULT_MAX_RUNS: u32 = 10;

/// The TeX engine.
///
/// `Latexmk` is Quarto 1's meaning (`pdf-engine: latexmk`, `latexmk` as the
/// per-pass engine); [`Driver::Latexmk`] is R's, a different setting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Engine {
    #[default]
    /// `pdflatex`, the default.
    PdfLatex,
    /// `xelatex`.
    XeLatex,
    /// `lualatex`.
    LuaLatex,
    /// `latexmk` as the per-pass engine, or as the whole driver (see the type docs).
    Latexmk,
    /// `tectonic`, which runs its own rerun loop.
    Tectonic,
}

impl Engine {
    /// The program name.
    pub fn program(self) -> &'static str {
        match self {
            Engine::PdfLatex => "pdflatex",
            Engine::XeLatex => "xelatex",
            Engine::LuaLatex => "lualatex",
            Engine::Latexmk => "latexmk",
            Engine::Tectonic => "tectonic",
        }
    }
}

/// Which bibliography tool runs. `Skip` is citeproc; `Infer` is R's rule
/// (`.bcf` present means `biber`, else `bibtex` when the `.aux` asks for it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Bib {
    #[default]
    /// `biber` when a `.bcf` file exists, else `bibtex` when the `.aux` file asks for it.
    Infer,
    /// Always run `bibtex`.
    Bibtex,
    /// Always run `biber`.
    Biber,
    /// Run no bibliography tool (the host resolves citations itself).
    Skip,
}

/// Who runs the engine repeatedly. `Builtin` is this crate's loop (R's
/// `emulation = TRUE`); `Latexmk` runs the real `latexmk` once (R's
/// `emulation = FALSE`), with no recovery and no auto-install, and the
/// request must leave run counts, `bib` and `index` at their defaults.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Driver {
    #[default]
    /// This crate's own rerun loop.
    Builtin,
    Latexmk,
}

/// Package recovery after a failed engine run: `FirstRun` (Quarto 1,
/// `pdf.ts:567-572`) or `EveryRun` (R, `latex.R:199-227` at `7818777`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Recover {
    #[default]
    /// Install missing packages after a failed first run only.
    FirstRun,
    /// Install missing packages after a failure of any run.
    EveryRun,
}

/// A non-zero exit from the bibliography tool: `Error` (R) fails the compile
/// with [`CompileError::Bibliography`]; `Warn` (Quarto 1) records
/// [`Diagnostics::bibliography_failed`] and continues.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OnBibFailure {
    #[default]
    /// Fail the compile.
    Error,
    /// Record the failure in diagnostics and continue.
    Warn,
}

/// Which runs end with the cleanup of the auxiliary files the run created.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Clean {
    /// Leave every auxiliary file in place.
    Never,
    /// Clean up only after a successful compile.
    OnSuccess,
    #[default]
    /// Clean up after every compile, successful or not.
    Always,
}

/// When the engine log is exempt from the cleanup `Clean` selects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KeepLog {
    /// Keep the log only when the compile failed.
    OnFailure,
    #[default]
    /// Keep the log when the compile failed or produced warnings.
    OnWarnings,
    Always,
    Never,
}

/// The kind of output file. DVI is additive later.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Output {
    #[default]
    /// A PDF file.
    Pdf,
}

/// The index tool: `command` with `args`, then the `.idx` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEngine {
    /// The program to run.
    pub command: String,
    /// Arguments placed before the `.idx` file name.
    pub args: Vec<String>,
}

/// Host overrides for tool locations. Resolution order for every tool: this
/// struct, then the distribution's `bin_dir`, then `PATH`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPaths {
    /// The engine program.
    pub engine: Option<PathBuf>,
    /// `bibtex`.
    pub bibtex: Option<PathBuf>,
    /// `biber`.
    pub biber: Option<PathBuf>,
    /// `makeindex`.
    pub makeindex: Option<PathBuf>,
    /// `latexmk`.
    pub latexmk: Option<PathBuf>,
    /// `tectonic`.
    pub tectonic: Option<PathBuf>,
}

/// The whole compile contract, as plain serializable data.
///
/// `Default` is written by hand: the crate's defaults, which are R's where R
/// and Quarto 1 differ (design section 5). A stored 0 for either run count
/// reads as the default (1 and 10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct CompileRequest {
    /// The TeX engine.
    pub engine: Engine,
    /// Who runs the engine repeatedly.
    pub driver: Driver,
    /// Extra engine arguments, placed after the crate's own.
    pub engine_args: Vec<String>,
    /// The bibliography tool.
    pub bib: Bib,
    /// `None` is `makeindex` with no options. The index pass runs whenever
    /// the first run left an `.idx` file; there is no way to skip it.
    pub index: Option<IndexEngine>,
    /// Total engine runs, first included.
    pub min_runs: u32,
    /// Upper bound on total engine runs.
    pub max_runs: u32,
    /// `-output-directory`, relative to the `.tex` directory.
    pub output_dir: Option<PathBuf>,
    /// Name of the output file, when it is not `<stem>.pdf`.
    pub pdf_file: Option<PathBuf>,
    /// `TEXINPUTS` and `BSTINPUTS`.
    pub input_paths: Vec<PathBuf>,
    /// Tool location overrides.
    pub tools: ToolPaths,
    /// Install missing LaTeX packages with `tlmgr` and retry.
    pub auto_install: bool,
    /// When package recovery runs.
    pub recover: Recover,
    /// What a failing bibliography tool does.
    pub on_bib_failure: OnBibFailure,
    /// When auxiliary files are removed.
    pub clean: Clean,
    /// When the log survives the cleanup.
    pub keep_log: KeepLog,
    /// The kind of output file.
    pub output: Output,
}

impl Default for CompileRequest {
    fn default() -> Self {
        Self {
            engine: Engine::PdfLatex,
            driver: Driver::Builtin,
            engine_args: Vec::new(),
            bib: Bib::Infer,
            index: None,
            min_runs: DEFAULT_MIN_RUNS,
            max_runs: DEFAULT_MAX_RUNS,
            output_dir: None,
            pdf_file: None,
            input_paths: Vec::new(),
            tools: ToolPaths::default(),
            auto_install: false,
            recover: Recover::FirstRun,
            on_bib_failure: OnBibFailure::Error,
            clean: Clean::Always,
            keep_log: KeepLog::OnWarnings,
            output: Output::Pdf,
        }
    }
}

impl CompileRequest {
    /// The default request.
    pub fn new() -> Self {
        Self::default()
    }
    /// Sets [`CompileRequest::engine`].
    pub fn engine(mut self, e: Engine) -> Self {
        self.engine = e;
        self
    }
    /// Sets [`CompileRequest::driver`].
    pub fn driver(mut self, d: Driver) -> Self {
        self.driver = d;
        self
    }
    /// Sets [`CompileRequest::engine_args`].
    pub fn engine_args(mut self, a: Vec<String>) -> Self {
        self.engine_args = a;
        self
    }
    /// Sets [`CompileRequest::bib`].
    pub fn bib(mut self, b: Bib) -> Self {
        self.bib = b;
        self
    }
    /// Sets [`CompileRequest::index`].
    pub fn index(mut self, i: IndexEngine) -> Self {
        self.index = Some(i);
        self
    }
    /// Total engine runs, first included.
    pub fn runs(mut self, min: u32, max: u32) -> Self {
        self.min_runs = min;
        self.max_runs = max;
        self
    }
    /// R's meaning of `min_times` and `max_times`: reruns after a first run,
    /// so `runs(min + 1, max + 1)`.
    pub fn reruns(self, min: u32, max: u32) -> Self {
        self.runs(min.saturating_add(1), max.saturating_add(1))
    }
    /// Sets [`CompileRequest::output_dir`].
    pub fn output_dir(mut self, d: PathBuf) -> Self {
        self.output_dir = Some(d);
        self
    }
    /// Sets [`CompileRequest::pdf_file`].
    pub fn pdf_file(mut self, p: PathBuf) -> Self {
        self.pdf_file = Some(p);
        self
    }
    /// Sets [`CompileRequest::input_paths`].
    pub fn input_paths(mut self, d: Vec<PathBuf>) -> Self {
        self.input_paths = d;
        self
    }
    /// Sets [`CompileRequest::tools`].
    pub fn tools(mut self, t: ToolPaths) -> Self {
        self.tools = t;
        self
    }
    /// Sets [`CompileRequest::auto_install`].
    pub fn auto_install(mut self, on: bool) -> Self {
        self.auto_install = on;
        self
    }
    /// Sets [`CompileRequest::recover`].
    pub fn recover(mut self, r: Recover) -> Self {
        self.recover = r;
        self
    }
    /// Sets [`CompileRequest::on_bib_failure`].
    pub fn on_bib_failure(mut self, f: OnBibFailure) -> Self {
        self.on_bib_failure = f;
        self
    }
    /// Sets [`CompileRequest::clean`].
    pub fn clean(mut self, c: Clean) -> Self {
        self.clean = c;
        self
    }
    /// Sets [`CompileRequest::keep_log`].
    pub fn keep_log(mut self, k: KeepLog) -> Self {
        self.keep_log = k;
        self
    }
    /// Sets [`CompileRequest::output`].
    pub fn output(mut self, o: Output) -> Self {
        self.output = o;
        self
    }

    /// `(min, max)` with a stored 0 read as the default, and `min` capped at
    /// `max`.
    pub(crate) fn run_bounds(&self) -> (u32, u32) {
        let min = if self.min_runs == 0 {
            DEFAULT_MIN_RUNS
        } else {
            self.min_runs
        };
        let max = if self.max_runs == 0 {
            DEFAULT_MAX_RUNS
        } else {
            self.max_runs
        };
        (min.min(max), max)
    }
}

/// Non-fatal facts about a successful compile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Diagnostics {
    /// [`crate::log::latex_warnings`] of the last run.
    pub warnings: Vec<String>,
    pub accessibility: AccessibilityWarnings,
    /// Hyphenation packages the log asked for and the crate did not install
    /// (auto-install off or failed, or the install did not clear the report).
    pub missing_hyphenation: Vec<String>,
    /// The run bound stopped a rerun the document still wanted.
    pub max_runs_reached: bool,
    /// With [`OnBibFailure::Warn`]: the tool and a log excerpt.
    pub bibliography_failed: Option<String>,
    /// Non-fatal facts with no other channel (a failed `fmtutil`, an ignored
    /// option).
    pub notices: Vec<String>,
}

/// A successful compile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Compiled {
    /// The output file; named `pdf` while [`Output`] has one variant.
    pub pdf: PathBuf,
    pub output_kind: Output,
    /// `Some` only if the file exists after the compile.
    pub log: Option<PathBuf>,
    /// Total engine runs, equal to the highest `Event::RunningEngine` number.
    pub runs: u32,
    pub installed_packages: Vec<String>,
    pub diagnostics: Diagnostics,
}

/// Why a compile failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CompileError {
    #[error("no TeX installation found")]
    NoTexInstallation,
    #[error("could not find `{tool}`")]
    EngineNotFound { tool: String },
    #[error("{}", .summary.as_deref().unwrap_or("the TeX engine failed"))]
    Latex {
        summary: Option<String>,
        hints: Vec<Hint>,
        log: Option<PathBuf>,
        /// [`crate::log::missing_packages`] of the failed run's log.
        missing: Vec<SearchTerm>,
        stdout: String,
        stderr: String,
        exit_code: Option<i32>,
    },
    #[error("package installation failed ({reason:?}): {source}")]
    PackageInstall {
        reason: InstallFailure,
        source: TlmgrError,
    },
    #[error("the TeX engine exited successfully but wrote no output file")]
    NoOutput,
    #[error("invalid request: {message}")]
    InvalidRequest { message: String },
    #[error("the bibliography tool `{engine}` failed: {log_excerpt}")]
    Bibliography {
        engine: String,
        log_excerpt: String,
        log: Option<PathBuf>,
    },
    #[error("the index tool failed: {log_excerpt}")]
    Index {
        log_excerpt: String,
        log: Option<PathBuf>,
    },
    #[error("the compile was cancelled")]
    Cancelled,
    #[error("could not run a command: {0}")]
    Io(#[from] io::Error),
}

impl CompileError {
    /// The stable code of this error.
    pub fn code(&self) -> ErrorCode {
        ErrorCode::new(match self {
            CompileError::NoTexInstallation => "compile.no_tex_installation",
            CompileError::EngineNotFound { .. } => "compile.engine_not_found",
            CompileError::Latex { .. } => "compile.latex",
            CompileError::PackageInstall { .. } => "compile.package_install",
            CompileError::NoOutput => "compile.no_output",
            CompileError::InvalidRequest { .. } => "compile.invalid_request",
            CompileError::Bibliography { .. } => "compile.bibliography",
            CompileError::Index { .. } => "compile.index",
            CompileError::Cancelled => "compile.cancelled",
            CompileError::Io(_) => "compile.io",
        })
    }
}

impl Serialize for CompileError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("CompileError", 3)?;
        s.serialize_field("code", &self.code())?;
        s.serialize_field("message", &self.to_string())?;
        match self {
            CompileError::Latex {
                summary,
                hints,
                log,
                missing,
                stdout,
                stderr,
                exit_code,
            } => s.serialize_field(
                "details",
                &serde_json_free::Latex {
                    summary,
                    hints,
                    log,
                    missing,
                    stdout,
                    stderr,
                    exit_code,
                },
            )?,
            _ => s.skip_field("details")?,
        }
        s.end()
    }
}

/// Borrowed shapes for [`CompileError`]'s `Serialize`.
mod serde_json_free {
    use std::path::PathBuf;

    use serde::Serialize;

    use crate::log::{Hint, SearchTerm};

    #[derive(Serialize)]
    pub(super) struct Latex<'a> {
        pub summary: &'a Option<String>,
        pub hints: &'a [Hint],
        pub log: &'a Option<PathBuf>,
        pub missing: &'a [SearchTerm],
        pub stdout: &'a str,
        pub stderr: &'a str,
        pub exit_code: &'a Option<i32>,
    }
}

/// Compiles `.tex` files with one distribution (or the tools on `PATH`).
pub struct Compiler {
    dist: Option<Distribution>,
    cfg: Config,
    windows: bool,
}

impl Compiler {
    /// `None` takes every tool from `PATH` and cannot install packages.
    pub fn new(dist: Option<Distribution>, cfg: Config) -> Self {
        Self {
            dist,
            cfg,
            windows: cfg!(windows),
        }
    }

    /// [`detect::find`], then [`Compiler::new`].
    pub fn auto(cfg: Config) -> Self {
        let dist = detect::find(&cfg);
        Self::new(dist, cfg)
    }

    /// Behave as on Windows (or not) whatever the host is, so a test on one
    /// platform runs the other's branch.
    #[doc(hidden)]
    pub fn for_windows(mut self, windows: bool) -> Self {
        self.windows = windows;
        self
    }

    /// Compiles `tex` as `req` asks.
    pub fn compile(
        &self,
        tex: &Path,
        req: &CompileRequest,
        x: &dyn Exec,
        o: &dyn Observer,
    ) -> Result<Compiled, CompileError> {
        run::compile(self.dist.as_ref(), &self.cfg, self.windows, tex, req, x, o)
    }
}
