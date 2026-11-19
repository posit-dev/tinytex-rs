//! Distribution detection and command construction.
//!
//! [`Config::from_env`] is the one place *configuration* is read from the
//! environment. Platform facts that detection needs (`HOME`, `APPDATA`,
//! `ProgramData`, `PATH`) are gathered by [`HostEnv::current`], and every
//! function that uses them has an injectable twin (`*_for`, `*_with`,
//! `command_for`) so the tests run the Windows and macOS forms on any host.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::config::{Config, Prefer};
use crate::exec::{CommandSpec, Exec};

static RTOOLS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(^|[\\/])rtools[0-9]*([\\/]|$)").expect("valid regex"));
static YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^TeX Live.* version (\d+)").expect("valid regex"));
static REVISION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^tlmgr revision \d+ \(([0-9-]+) ").expect("valid regex"));
static TINYTEX_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\.?tinytex$").expect("valid regex"));
static TINYTEX_CNF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\W\.?TinyTeX\W").expect("valid regex"));
static PLAIN_ASCII: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[!-~]+$").expect("valid regex"));

/// What kind of TeX Live install this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Kind {
    TinyTeX,
    /// Any other `tlmgr`-based install.
    TexLive,
}

/// A located distribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Distribution {
    /// Unknown for some system installs.
    pub root: Option<PathBuf>,
    pub bin_dir: PathBuf,
    pub tlmgr: PathBuf,
    pub kind: Kind,
}

/// The parsed `tlmgr --version` output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlVersion {
    pub year: u32,
    /// The `tlmgr revision` date; empty when the output has none.
    pub date: String,
}

/// Parses `tlmgr --version` output. `None` when there is no TeX Live year.
///
/// [`Distribution::tex_live_year`] and `Tlmgr::version` (plan 5.3b) share this
/// parser so the two never disagree.
#[doc(hidden)]
pub fn parse_tl_version(output: &str) -> Option<TlVersion> {
    let year = YEAR.captures(output)?[1].parse().ok()?;
    let date = REVISION
        .captures(output)
        .map(|c| c[1].to_owned())
        .unwrap_or_default();
    Some(TlVersion { year, date })
}

/// Removes Rtools directories from a `;`-separated Windows `PATH`.
///
/// `tlmgr` may pick up `xz`, `zstd` or `gzip` from `PATH`, and the Rtools
/// copies make it extremely slow (R `fb7a799`, after the pinned `7818777`).
pub fn filter_rtools(path: &str) -> String {
    path.split(';')
        .filter(|entry| !RTOOLS.is_match(entry))
        .collect::<Vec<_>>()
        .join(";")
}

/// The operating system family that decides default locations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Linux,
    MacOs,
    Windows,
}

impl HostOs {
    fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

/// Platform facts that detection reads.
#[derive(Debug, Clone)]
pub struct HostEnv {
    pub os: HostOs,
    pub home: Option<PathBuf>,
    pub appdata: Option<String>,
    pub program_data: Option<String>,
}

impl HostEnv {
    /// The facts of the running process.
    pub fn current() -> Self {
        Self {
            os: HostOs::current(),
            home: std::env::home_dir(),
            appdata: std::env::var("APPDATA").ok().filter(|v| !v.is_empty()),
            program_data: std::env::var("ProgramData").ok().filter(|v| !v.is_empty()),
        }
    }
}

/// Where TinyTeX installs by default (`Config::install_dir` wins).
pub fn default_install_dir(cfg: &Config) -> Option<PathBuf> {
    default_install_dir_for(cfg, &HostEnv::current())
}

/// [`default_install_dir`] over injected host facts.
///
/// Windows follows Quarto 1: `%APPDATA%\TinyTeX`, unless `APPDATA` is unset or
/// holds anything but printable ASCII without spaces (TinyTeX does not work
/// from such a path), then `%ProgramData%\TinyTeX`.
pub fn default_install_dir_for(cfg: &Config, host: &HostEnv) -> Option<PathBuf> {
    if let Some(dir) = &cfg.install_dir {
        return Some(dir.clone());
    }
    match host.os {
        HostOs::Linux => host.home.as_ref().map(|h| h.join(".TinyTeX")),
        HostOs::MacOs => host
            .home
            .as_ref()
            .map(|h| h.join("Library").join("TinyTeX")),
        HostOs::Windows => {
            let base = match &host.appdata {
                Some(a) if PLAIN_ASCII.is_match(a) => Some(a),
                _ => host.program_data.as_ref(),
            };
            base.map(|b| Path::new(b).join("TinyTeX"))
        }
    }
}

fn tlmgr_file_name() -> &'static str {
    if cfg!(windows) { "tlmgr.bat" } else { "tlmgr" }
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// The first `<dir>/bin/<arch>` that holds an executable `tlmgr`.
fn find_in_install_dir(dir: &Path) -> Option<Distribution> {
    let mut bins: Vec<PathBuf> = fs::read_dir(dir.join("bin"))
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    bins.sort();
    bins.into_iter()
        .find(|b| is_executable(&b.join(tlmgr_file_name())))
        .map(Distribution::from_bin_dir)
}

/// Looks for TinyTeX in its install directory. Filesystem only: it runs on
/// every render, so it spawns nothing and reads no `PATH`.
pub fn find_tinytex(cfg: &Config) -> Option<Distribution> {
    find_tinytex_for(cfg, &HostEnv::current())
}

fn find_tinytex_for(cfg: &Config, host: &HostEnv) -> Option<Distribution> {
    find_in_install_dir(&default_install_dir_for(cfg, host)?)
}

fn find_on_path(path: Option<&OsStr>, extra: &[PathBuf]) -> Option<Distribution> {
    let from_path = path.into_iter().flat_map(std::env::split_paths);
    from_path
        .chain(extra.iter().cloned())
        .find(|d| is_executable(&d.join(tlmgr_file_name())))
        .map(Distribution::from_bin_dir)
}

/// Finds a distribution following `cfg.prefer`, adding `tlmgr` on `PATH` and
/// (macOS) `/Library/TeX/texbin`.
///
/// `Prefer::TinyTeX` takes the TinyTeX directory, then `PATH`. `Prefer::System`
/// takes `PATH` first and falls back to the TinyTeX directory. (Quarto 1 with
/// `latex-tinytex: false` runs bare commands from `PATH` only; the fallback is
/// this crate's addition, so a machine with just TinyTeX still compiles.)
/// `Config::tlmgr_path`, when set, wins over both.
pub fn find(cfg: &Config) -> Option<Distribution> {
    let host = HostEnv::current();
    let path = std::env::var_os("PATH");
    let extra: &[PathBuf] = if host.os == HostOs::MacOs {
        &[PathBuf::from("/Library/TeX/texbin")]
    } else {
        &[]
    };
    find_with(cfg, &host, path.as_deref(), extra)
}

/// [`find`] over injected host facts, `PATH` and extra directories.
pub fn find_with(
    cfg: &Config,
    host: &HostEnv,
    path: Option<&OsStr>,
    extra: &[PathBuf],
) -> Option<Distribution> {
    if let Some(t) = cfg.tlmgr_path.as_deref().filter(|t| is_executable(t))
        && let Some(bin) = t.parent()
    {
        return Some(Distribution::from_bin_dir(bin.to_path_buf()));
    }
    match cfg.prefer {
        Prefer::TinyTeX => find_tinytex_for(cfg, host).or_else(|| find_on_path(path, extra)),
        Prefer::System => find_on_path(path, extra).or_else(|| find_tinytex_for(cfg, host)),
    }
}

impl Distribution {
    /// A distribution whose programs live in `bin_dir`.
    ///
    /// The root is `bin_dir/../..` when `bin_dir`'s parent is named `bin`
    /// (`<root>/bin/<arch>`), and unknown otherwise (a macOS `texbin`).
    pub fn from_bin_dir(bin_dir: PathBuf) -> Self {
        let root = bin_dir
            .parent()
            .filter(|p| p.file_name() == Some(OsStr::new("bin")))
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let tlmgr = bin_dir.join(tlmgr_file_name());
        let mut d = Self {
            root,
            bin_dir,
            tlmgr,
            kind: Kind::TexLive,
        };
        if d.is_tinytex() {
            d.kind = Kind::TinyTeX;
        }
        d
    }

    /// TinyTeX by directory name (`TinyTeX`, `.TinyTeX`), a `.tinytex` marker
    /// file, or a `texmf-dist/web2c/fmtutil.cnf` that mentions TinyTeX.
    pub fn is_tinytex(&self) -> bool {
        let Some(root) = &self.root else {
            return false;
        };
        if root
            .file_name()
            .is_some_and(|n| TINYTEX_NAME.is_match(&n.to_string_lossy()))
        {
            return true;
        }
        if root.join(".tinytex").exists() {
            return true;
        }
        fs::read_to_string(root.join("texmf-dist").join("web2c").join("fmtutil.cnf"))
            .is_ok_and(|t| TINYTEX_CNF.is_match(&t))
    }

    /// R's `tlmgr_writable()`: TinyTeX, and the `tlmgr` script passes a real
    /// write-access probe (opening it for writing, not a metadata check).
    pub fn is_writable(&self) -> bool {
        self.is_writable_with(&|p| fs::OpenOptions::new().write(true).open(p).is_ok())
    }

    /// [`Distribution::is_writable`] with the probe injected.
    pub fn is_writable_with(&self, probe: &dyn Fn(&Path) -> bool) -> bool {
        self.is_tinytex() && probe(&self.tlmgr)
    }

    /// The `version` file of a TinyTeX install, such as `v2026.01`.
    pub fn installed_version(&self) -> Option<String> {
        let text = fs::read_to_string(self.root.as_ref()?.join("version")).ok()?;
        let v = text.trim();
        (!v.is_empty()).then(|| v.to_owned())
    }

    /// The TeX Live year, from `tlmgr --version` run through `x`.
    pub fn tex_live_year(&self, x: &dyn Exec) -> Option<u32> {
        let mut spec = self.command("tlmgr");
        spec.args.push("--version".into());
        let out = x.run(&spec).ok()?;
        if !out.status.success() {
            return None;
        }
        parse_tl_version(&String::from_utf8_lossy(&out.stdout)).map(|v| v.year)
    }

    /// A [`CommandSpec`] for one of the distribution's programs.
    ///
    /// `PATH` gets `bin_dir` in front for every program. `TEXLIVE_PREFER_OWN=1`
    /// is set for `tlmgr`, `fmtutil` and `updmap` only. On Windows `fmtutil`
    /// and `updmap` are called by short name with `--sys` (and `--all`)
    /// already in `args`, `tlmgr` is `tlmgr.bat`, and Rtools leaves the `PATH`
    /// of those three. The spec names no temporary file.
    pub fn command(&self, program: &str) -> CommandSpec {
        let path = std::env::var_os("PATH");
        self.command_for(program, cfg!(windows), path.as_deref())
    }

    /// [`Distribution::command`] for either platform family over a given `PATH`.
    pub fn command_for(
        &self,
        program: &str,
        windows: bool,
        inherited_path: Option<&OsStr>,
    ) -> CommandSpec {
        let own_tool = matches!(program, "tlmgr" | "fmtutil" | "updmap");
        let sep = if windows { ";" } else { ":" };

        let inherited: Option<OsString> = inherited_path.filter(|p| !p.is_empty()).map(|p| {
            if windows && own_tool {
                OsString::from(filter_rtools(&p.to_string_lossy()))
            } else {
                p.to_owned()
            }
        });
        let mut path = self.bin_dir.clone().into_os_string();
        if let Some(rest) = inherited.filter(|r| !r.is_empty()) {
            path.push(sep);
            path.push(rest);
        }

        let mut spec = CommandSpec::new(self.bin_dir.join(program));
        spec.env.push(("PATH".into(), path));
        if own_tool {
            spec.env.push(("TEXLIVE_PREFER_OWN".into(), "1".into()));
        }
        if windows {
            match program {
                "tlmgr" => spec.program = self.bin_dir.join("tlmgr.bat"),
                "fmtutil" => {
                    spec.program = PathBuf::from("fmtutil");
                    spec.args = vec!["--sys".into(), "--all".into()];
                    spec.changes_installation = true;
                }
                "updmap" => {
                    spec.program = PathBuf::from("updmap");
                    spec.args = vec!["--sys".into()];
                    spec.changes_installation = true;
                }
                _ => {}
            }
        }
        spec
    }
}
