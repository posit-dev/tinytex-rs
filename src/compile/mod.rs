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

mod run;

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
    PdfLatex,
    XeLatex,
    LuaLatex,
    Latexmk,
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
    Infer,
    Bibtex,
    Biber,
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
    FirstRun,
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
    Error,
    Warn,
}

/// Which auxiliary files the cleanup removes (plan 5.4b gives it behavior).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Clean {
    Never,
    OnSuccess,
    #[default]
    Always,
}

/// When the engine log is kept (plan 5.4b gives it behavior).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KeepLog {
    OnFailure,
    #[default]
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
    Pdf,
}

/// The index tool: `command` with `args`, then the `.idx` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEngine {
    pub command: String,
    pub args: Vec<String>,
}

/// Host overrides for tool locations. Resolution order for every tool: this
/// struct, then the distribution's `bin_dir`, then `PATH`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPaths {
    pub engine: Option<PathBuf>,
    pub bibtex: Option<PathBuf>,
    pub biber: Option<PathBuf>,
    pub makeindex: Option<PathBuf>,
    pub latexmk: Option<PathBuf>,
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
    pub engine: Engine,
    pub driver: Driver,
    pub engine_args: Vec<String>,
    pub bib: Bib,
    /// `None` is `makeindex` with no options. The index pass runs whenever
    /// the first run left an `.idx` file; there is no way to skip it.
    pub index: Option<IndexEngine>,
    /// Total engine runs, first included.
    pub min_runs: u32,
    pub max_runs: u32,
    /// `-output-directory`, relative to the `.tex` directory.
    pub output_dir: Option<PathBuf>,
    pub pdf_file: Option<PathBuf>,
    /// `TEXINPUTS` and `BSTINPUTS`.
    pub input_paths: Vec<PathBuf>,
    pub tools: ToolPaths,
    pub auto_install: bool,
    pub recover: Recover,
    pub on_bib_failure: OnBibFailure,
    pub clean: Clean,
    pub keep_log: KeepLog,
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
    pub fn engine(mut self, e: Engine) -> Self {
        self.engine = e;
        self
    }
    pub fn driver(mut self, d: Driver) -> Self {
        self.driver = d;
        self
    }
    pub fn engine_args(mut self, a: Vec<String>) -> Self {
        self.engine_args = a;
        self
    }
    pub fn bib(mut self, b: Bib) -> Self {
        self.bib = b;
        self
    }
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
    pub fn output_dir(mut self, d: PathBuf) -> Self {
        self.output_dir = Some(d);
        self
    }
    pub fn pdf_file(mut self, p: PathBuf) -> Self {
        self.pdf_file = Some(p);
        self
    }
    pub fn input_paths(mut self, d: Vec<PathBuf>) -> Self {
        self.input_paths = d;
        self
    }
    pub fn tools(mut self, t: ToolPaths) -> Self {
        self.tools = t;
        self
    }
    pub fn auto_install(mut self, on: bool) -> Self {
        self.auto_install = on;
        self
    }
    pub fn recover(mut self, r: Recover) -> Self {
        self.recover = r;
        self
    }
    pub fn on_bib_failure(mut self, f: OnBibFailure) -> Self {
        self.on_bib_failure = f;
        self
    }
    pub fn clean(mut self, c: Clean) -> Self {
        self.clean = c;
        self
    }
    pub fn keep_log(mut self, k: KeepLog) -> Self {
        self.keep_log = k;
        self
    }
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
}

impl Compiler {
    /// `None` takes every tool from `PATH` and cannot install packages.
    pub fn new(dist: Option<Distribution>, cfg: Config) -> Self {
        Self { dist, cfg }
    }

    /// [`detect::find`], then [`Compiler::new`].
    pub fn auto(cfg: Config) -> Self {
        let dist = detect::find(&cfg);
        Self::new(dist, cfg)
    }

    /// Compiles `tex` as `req` asks.
    pub fn compile(
        &self,
        tex: &Path,
        req: &CompileRequest,
        x: &dyn Exec,
        o: &dyn Observer,
    ) -> Result<Compiled, CompileError> {
        run::compile(self.dist.as_ref(), &self.cfg, tex, req, x, o)
    }
}
