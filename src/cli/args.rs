//! The `clap` definition: every flag names one field of `CompileRequest` or
//! `Config`, or chooses how the command reads input or prints output.

use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::{
    Bib, Clean, Driver, Engine, KeepLog, OnBibFailure, Output, Prefer, Recover, TlmgrUpdate,
};

/// `tinytex`: compile `.tex` files to PDF and manage TinyTeX.
#[derive(Debug, Parser)]
#[command(name = "tinytex", version, about, propagate_version = true)]
pub struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub command: Command,
}

/// Flags every subcommand accepts, before or after the subcommand name.
#[derive(Debug, Args)]
pub struct Global {
    /// Print the result as JSON on stdout (`{"schema": 1, ...}`).
    #[arg(long, global = true)]
    pub json: bool,
    /// How progress events appear on stderr.
    #[arg(long, global = true, value_enum, default_value = "text")]
    pub events: EventsFormat,
    /// A whole-command deadline in seconds; a run that exceeds it exits 4.
    #[arg(long, global = true, value_name = "SECONDS", value_parser = parse_timeout)]
    pub timeout: Option<Duration>,
    /// Where TinyTeX is (or is to be) installed (`TINYTEX_DIR`).
    #[arg(long, global = true, value_name = "DIR")]
    pub install_dir: Option<PathBuf>,
    /// The CTAN repository `tlmgr` uses.
    #[arg(long, global = true, value_name = "URL")]
    pub repository: Option<String>,
    /// An extra option for every `tlmgr` call (repeatable).
    #[arg(long, global = true, value_name = "ARG", allow_hyphen_values = true, action = clap::ArgAction::Append)]
    pub tlmgr_opt: Vec<String>,
    /// When `tlmgr` updates itself and the packages.
    #[arg(long, global = true, value_enum)]
    pub tlmgr_update: Option<TlmgrUpdateArg>,
    /// Which distribution to look for first.
    #[arg(long, global = true, value_enum)]
    pub prefer: Option<PreferArg>,
    /// The `tlmgr` program, when it is not next to the engines.
    #[arg(long, global = true, value_name = "PATH")]
    pub tlmgr_path: Option<PathBuf>,
    /// Run `tlmgr` in user mode.
    #[arg(long, global = true)]
    pub usermode: bool,
    /// Delete the `texlive.tlpdb.<md5>` copies after an update.
    #[arg(long, global = true)]
    pub delete_tlpdb: bool,
    /// Report more detail.
    #[arg(long, global = true)]
    pub verbose: bool,
}

fn parse_timeout(text: &str) -> Result<Duration, String> {
    let seconds: f64 = text
        .parse()
        .map_err(|_| format!("`{text}` is not a number"))?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err("the timeout must be a positive number of seconds".into());
    }
    Duration::try_from_secs_f64(seconds).map_err(|e| e.to_string())
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Compile a `.tex` file to PDF.
    Compile(Box<CompileArgs>),
    /// Find the TeX distribution.
    Detect,
    /// Run a `tlmgr` operation.
    Tlmgr {
        #[command(subcommand)]
        op: TlmgrOp,
    },
    /// Print the diagnostics found in an engine log.
    Log {
        /// The `.log` file.
        file: PathBuf,
    },
}

#[derive(Debug, Args)]
pub struct CompileArgs {
    /// The `.tex` file; the engine runs in its directory.
    pub file: PathBuf,
    /// Read the whole request from this JSON file (`-` is standard input).
    /// Flags given as well override its fields.
    #[arg(long, value_name = "FILE")]
    pub request: Option<String>,
    /// Print the `{request, config}` the flags produce, then exit.
    #[arg(long)]
    pub print_request: bool,
    /// The TeX engine.
    #[arg(long, value_enum)]
    pub pdf_engine: Option<EngineArg>,
    /// An extra engine argument (repeatable).
    #[arg(long, value_name = "ARG", allow_hyphen_values = true, action = clap::ArgAction::Append)]
    pub pdf_engine_opt: Vec<String>,
    /// The bibliography tool (`natbib` is `bibtex`, `biblatex` is `biber`).
    #[arg(long, value_enum)]
    pub bib_engine: Option<BibArg>,
    /// The index program (default `makeindex`).
    #[arg(long, value_name = "CMD")]
    pub index_engine: Option<String>,
    /// An index program argument (repeatable).
    #[arg(long, value_name = "ARG", allow_hyphen_values = true, action = clap::ArgAction::Append)]
    pub index_engine_opt: Vec<String>,
    /// Total engine runs at least, first included (R's `min_times` plus one).
    #[arg(long, value_name = "N")]
    pub min: Option<u32>,
    /// Total engine runs at most, first included (R's `max_times` plus one).
    #[arg(long, value_name = "N")]
    pub max: Option<u32>,
    /// Who runs the engine repeatedly.
    #[arg(long, value_enum)]
    pub driver: Option<DriverArg>,
    /// When missing packages are installed after a failed run.
    #[arg(long, value_enum)]
    pub recover: Option<RecoverArg>,
    /// What a failing bibliography tool does.
    #[arg(long, value_enum)]
    pub on_bib_failure: Option<OnBibFailureArg>,
    /// When the engine log survives the cleanup.
    #[arg(long, value_enum)]
    pub keep_log: Option<KeepLogArg>,
    /// The kind of output file.
    #[arg(long, value_enum)]
    pub output: Option<OutputArg>,
    /// `-output-directory`, relative to the `.tex` directory.
    #[arg(long, value_name = "DIR")]
    pub output_dir: Option<PathBuf>,
    /// The name of the output file, when it is not `<stem>.pdf`.
    #[arg(long, value_name = "PATH")]
    pub pdf_file: Option<PathBuf>,
    /// A directory for `TEXINPUTS` and `BSTINPUTS` (repeatable).
    #[arg(long, value_name = "DIR", action = clap::ArgAction::Append)]
    pub input_path: Vec<PathBuf>,
    /// When auxiliary files are removed.
    #[arg(long, value_enum, overrides_with = "no_clean")]
    pub clean: Option<CleanArg>,
    /// Same as `--clean never`.
    #[arg(long, overrides_with = "clean")]
    pub no_clean: bool,
    /// Install missing LaTeX packages with `tlmgr` and retry.
    #[arg(long, overrides_with = "no_auto_install")]
    pub auto_install: bool,
    /// Never install packages. With neither flag, packages are installed when
    /// the distribution is writable.
    #[arg(long, overrides_with = "auto_install")]
    pub no_auto_install: bool,
    /// Where a tool is: `engine`, `bibtex`, `biber`, `makeindex`, `latexmk` or
    /// `tectonic`, as `name=path` (repeatable).
    #[arg(long, value_name = "NAME=PATH", value_parser = parse_tool, action = clap::ArgAction::Append)]
    pub tool: Vec<(String, PathBuf)>,
}

fn parse_tool(text: &str) -> Result<(String, PathBuf), String> {
    let (name, path) = text
        .split_once('=')
        .ok_or_else(|| format!("`{text}` is not of the form name=path"))?;
    if ![
        "engine",
        "bibtex",
        "biber",
        "makeindex",
        "latexmk",
        "tectonic",
    ]
    .contains(&name)
    {
        return Err(format!("unknown tool `{name}`"));
    }
    if path.is_empty() {
        return Err("the path is empty".into());
    }
    Ok((name.to_owned(), PathBuf::from(path)))
}

#[derive(Debug, Subcommand)]
pub enum TlmgrOp {
    /// The TeX Live year and the `tlmgr` date.
    Version,
    /// The packages that contain a file.
    Search {
        /// A file name, or a regular expression for one.
        term: String,
    },
    /// Fields of packages, one row per package.
    Info {
        /// Package names.
        pkgs: Vec<String>,
        /// Comma-separated `tlmgr info --data` fields.
        #[arg(long, default_value = "name,localrev")]
        fields: String,
        /// Only packages that are installed.
        #[arg(long)]
        only_installed: bool,
    },
    /// Install packages (those already present are left alone).
    Install { pkgs: Vec<String> },
    /// Update `tlmgr` and the installed packages.
    Update {
        /// Update `tlmgr` itself only.
        #[arg(long = "self")]
        self_only: bool,
    },
    /// Remove packages.
    Remove { pkgs: Vec<String> },
    /// Print the repository, or set it.
    Repository { url: Option<String> },
    /// Add the TeX Live binaries to the system `PATH`, or remove them.
    Path {
        #[arg(value_enum)]
        action: PathArg,
    },
    /// Rebuild the formats.
    Fmtutil,
    /// Run `tlmgr` with the arguments after `--`.
    Run {
        /// The command changes the installation (so it takes the lock).
        #[arg(long)]
        changes_installation: bool,
        #[arg(last = true)]
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PathArg {
    Add,
    Remove,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum EventsFormat {
    Text,
    Json,
}

macro_rules! mirror {
    ($(#[$m:meta])* $name:ident => $target:ty { $($(#[$vm:meta])* $var:ident => $t:expr),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, ValueEnum)]
        pub enum $name { $($(#[$vm])* $var),+ }
        impl From<$name> for $target {
            fn from(v: $name) -> Self { match v { $($name::$var => $t),+ } }
        }
    };
}

mirror!(EngineArg => Engine {
    Pdflatex => Engine::PdfLatex,
    Xelatex => Engine::XeLatex,
    Lualatex => Engine::LuaLatex,
    Latexmk => Engine::Latexmk,
    Tectonic => Engine::Tectonic,
});
mirror!(BibArg => Bib {
    Infer => Bib::Infer,
    #[value(alias = "natbib")]
    Bibtex => Bib::Bibtex,
    #[value(alias = "biblatex")]
    Biber => Bib::Biber,
    Skip => Bib::Skip,
});
mirror!(DriverArg => Driver { Builtin => Driver::Builtin, Latexmk => Driver::Latexmk });
mirror!(RecoverArg => Recover { FirstRun => Recover::FirstRun, EveryRun => Recover::EveryRun });
mirror!(OnBibFailureArg => OnBibFailure { Error => OnBibFailure::Error, Warn => OnBibFailure::Warn });
mirror!(KeepLogArg => KeepLog {
    OnFailure => KeepLog::OnFailure,
    OnWarnings => KeepLog::OnWarnings,
    Always => KeepLog::Always,
    Never => KeepLog::Never,
});
mirror!(OutputArg => Output { Pdf => Output::Pdf });
mirror!(CleanArg => Clean {
    Never => Clean::Never,
    OnSuccess => Clean::OnSuccess,
    Always => Clean::Always,
});
mirror!(TlmgrUpdateArg => TlmgrUpdate { Auto => TlmgrUpdate::Auto, Never => TlmgrUpdate::Never });
mirror!(PreferArg => Prefer {
    #[value(name = "tinytex")]
    TinyTeX => Prefer::TinyTeX,
    System => Prefer::System,
});
