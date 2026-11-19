//! [`Config`]: the one place the environment is read.

use std::ffi::OsString;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// When `tlmgr` itself and the installed packages are updated.
///
/// `Auto` updates lazily, at most once per process and distribution. `Never`
/// skips the on-failure and the pre-install update (R's
/// `tinytex.tlmgr_update = FALSE`, extended to both).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TlmgrUpdate {
    #[default]
    /// Update lazily.
    Auto,
    /// Never update.
    Never,
}

/// Which distribution [`crate::detect::find`] looks for first.
///
/// Quarto 1's `latex-tinytex: false` is `System`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Prefer {
    #[default]
    /// Look for TinyTeX first.
    TinyTeX,
    /// Use the TeX Live on `PATH`.
    System,
}

/// Every field has a command-line flag, or is environment-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Config {
    /// Where TinyTeX is (or is to be) installed.
    pub install_dir: Option<PathBuf>,
    /// The CTAN repository `tlmgr` uses.
    pub repository: Option<String>,
    /// Extra options for every `tlmgr` call.
    pub tlmgr_opts: Vec<String>,
    /// Never install packages.
    pub prevent_install: bool,
    /// When `tlmgr` updates itself and the packages.
    pub tlmgr_update: TlmgrUpdate,
    /// Which distribution to prefer.
    pub prefer: Prefer,
    /// The `tlmgr` program, when not found next to the engines.
    pub tlmgr_path: Option<PathBuf>,
    /// Run `tlmgr` in user mode.
    pub usermode: bool,
    /// Delete the `texlive.tlpdb.<md5>` copies under `tlpkg` after an update.
    pub delete_tlpdb: bool,
    /// Report more detail through the observer.
    pub verbose: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            install_dir: None,
            repository: None,
            tlmgr_opts: Vec::new(),
            prevent_install: false,
            tlmgr_update: TlmgrUpdate::Auto,
            prefer: Prefer::TinyTeX,
            tlmgr_path: None,
            usermode: false,
            delete_tlpdb: false,
            verbose: false,
        }
    }
}

impl Config {
    /// Reads `TINYTEX_DIR` and `TINYTEX_PREVENT_INSTALL` from the process environment.
    pub fn from_env() -> Self {
        Self::from_env_with(|name| std::env::var_os(name))
    }

    /// [`Config::from_env`] over an injected lookup.
    ///
    /// This is the only place the crate reads the environment. An empty
    /// `TINYTEX_DIR` is unset; `TINYTEX_PREVENT_INSTALL` is true when it
    /// equals `true`, ignoring case (R's `tolower(...) == 'true'`).
    pub fn from_env_with(get: impl Fn(&str) -> Option<OsString>) -> Self {
        Self {
            install_dir: get("TINYTEX_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            prevent_install: get("TINYTEX_PREVENT_INSTALL")
                .is_some_and(|v| v.to_string_lossy().eq_ignore_ascii_case("true")),
            ..Self::default()
        }
    }
}
