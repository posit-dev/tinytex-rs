//! The fail-closed gate for tests that install or remove packages. Not a test
//! module (see `sandbox_gate` for its tests).
//!
//! Such a test may run only inside a disposable copy of a TeX tree built by
//! `scripts/tex-sandbox.sh`. [`gate`] decides that from the environment
//! (`TINYTEX_DIR` names the copy, `TINYTEX_SANDBOX_NONCE` is the nonce the
//! script printed) and returns a [`Sandbox`] only when every check passes:
//!
//! 1. the root is set, exists, and is not (or inside) a default install
//!    directory (`~/.TinyTeX`, `~/Library/TinyTeX`), compared canonically;
//! 2. it lies strictly inside the temporary directory or `$RUNNER_TEMP`;
//! 3. `DISPOSABLE_TINYTEX` holds the root's canonical path and the nonce, and
//!    `COMPLETE` exists;
//! 4. the system directories of the child `PATH` hold no `tlmgr`, `kpsewhich`,
//!    or `pdflatex`;
//! 5. the distribution the code under test will use, resolved with the child's
//!    `HOME` and `PATH` (not the test process's), is rooted there;
//! 6. the real paths of `tlmgr`, the engine `bin` directory, and `kpsewhich`
//!    are under the root (`tlmgr` derives its tree from its own location);
//! 7. `kpsewhich -var-value=TEXMFROOT`, run in the child environment, prints
//!    the root.
//!
//! Children run through [`SandboxExec`]: a cleared environment with only
//! `PATH` (the copy's `bin` plus system directories), `HOME` (a fresh
//! directory next to the copy), `TMPDIR`, and `LANG`. The test process's
//! `TEXMF*` variables are not checked, because children never inherit them.
//!
//! On refusal the test is skipped with a message, or fails when
//! `TINYTEX_RS_REQUIRE_SANDBOX=1` (CI sets it once its copy step exists). The
//! gate deliberately ignores `TINYTEX_RS_REQUIRE_TEX` and `CI`: neither says
//! that the tree is disposable.

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Output;

use tinytex::detect::{HostEnv, find_with};
use tinytex::{CommandSpec, Config, Distribution, Exec, StdExec};

/// The tools whose presence on the child's system `PATH` would let a lookup
/// fall through to another installation.
const TEX_TOOLS: [&str; 3] = ["tlmgr", "kpsewhich", "pdflatex"];

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// This platform has no sandbox support (the script is POSIX `sh`).
    Unsupported,
    RootUnset,
    RootUnreadable(PathBuf),
    DefaultInstallDir(PathBuf),
    NotUnderTemp(PathBuf),
    SentinelMissing,
    SentinelPath {
        expected: PathBuf,
        found: String,
    },
    NonceMismatch,
    Incomplete,
    NoDistribution,
    EscapesRoot {
        what: &'static str,
        real: PathBuf,
    },
    SystemPathHasTex(PathBuf),
    KpsewhichRoot {
        got: String,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Unsupported => f.write_str("no sandbox support on this platform"),
            Refusal::RootUnset => f.write_str("TINYTEX_DIR is not set"),
            Refusal::RootUnreadable(p) => {
                write!(f, "TINYTEX_DIR {} cannot be resolved", p.display())
            }
            Refusal::DefaultInstallDir(p) => write!(
                f,
                "TINYTEX_DIR is, or lies inside, the default install directory {}",
                p.display()
            ),
            Refusal::NotUnderTemp(p) => {
                write!(
                    f,
                    "{} is not inside the temporary directory or $RUNNER_TEMP",
                    p.display()
                )
            }
            Refusal::SentinelMissing => f.write_str("DISPOSABLE_TINYTEX is missing"),
            Refusal::SentinelPath { expected, found } => write!(
                f,
                "DISPOSABLE_TINYTEX names {found:?}, not {} (a copy of a sandbox?)",
                expected.display()
            ),
            Refusal::NonceMismatch => {
                f.write_str("TINYTEX_SANDBOX_NONCE is unset or differs from DISPOSABLE_TINYTEX")
            }
            Refusal::Incomplete => {
                f.write_str("the COMPLETE marker is missing (a half-built copy)")
            }
            Refusal::NoDistribution => f.write_str("no bin/*/tlmgr under TINYTEX_DIR"),
            Refusal::EscapesRoot { what, real } => {
                write!(
                    f,
                    "the real path of {what} is {}, outside the sandbox",
                    real.display()
                )
            }
            Refusal::SystemPathHasTex(p) => {
                write!(f, "{} on the system PATH is a TeX tool", p.display())
            }
            Refusal::KpsewhichRoot { got } => {
                write!(
                    f,
                    "kpsewhich -var-value=TEXMFROOT printed {got:?}, not the sandbox root"
                )
            }
        }
    }
}

/// Everything the gate reads from outside, so a test can supply it.
#[derive(Debug, Clone)]
pub struct Inputs {
    /// `TINYTEX_DIR`.
    pub root: Option<PathBuf>,
    /// `TINYTEX_SANDBOX_NONCE`.
    pub nonce: Option<String>,
    /// The allowed bases: the temporary directory and `$RUNNER_TEMP`.
    pub temp_dirs: Vec<PathBuf>,
    /// The default install directories a sandbox must not be.
    pub default_dirs: Vec<PathBuf>,
    /// System directories of the child `PATH`.
    pub system_path: Vec<PathBuf>,
}

impl Inputs {
    pub fn from_env() -> Self {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        let home = var("HOME").map(PathBuf::from);
        let mut temp_dirs = vec![std::env::temp_dir()];
        temp_dirs.extend(var("RUNNER_TEMP").map(PathBuf::from));
        Self {
            root: var("TINYTEX_DIR").map(PathBuf::from),
            nonce: var("TINYTEX_SANDBOX_NONCE").map(|v| v.to_string_lossy().into_owned()),
            temp_dirs,
            default_dirs: home
                .into_iter()
                .flat_map(|h| [h.join(".TinyTeX"), h.join("Library").join("TinyTeX")])
                .collect(),
            system_path: ["/usr/bin", "/bin", "/usr/sbin", "/sbin"]
                .iter()
                .map(PathBuf::from)
                .collect(),
        }
    }
}

/// Runs children in a cleared environment: every command is wrapped in
/// `env -i` with the allowlist, and `PATH`, `HOME`, `TMPDIR`, `LANG` and any
/// `TEXMF*` the spec carries are ignored in favor of the sandbox's own.
pub struct SandboxExec {
    inner: StdExec,
    path: OsString,
    home: PathBuf,
}

impl Exec for SandboxExec {
    fn run(&self, spec: &CommandSpec) -> io::Result<Output> {
        let mut args: Vec<OsString> = vec!["-i".into()];
        let mut set = |k: &str, v: &std::ffi::OsStr| {
            let mut kv = OsString::from(format!("{k}="));
            kv.push(v);
            args.push(kv);
        };
        set("PATH", &self.path);
        set("HOME", self.home.as_os_str());
        set("TMPDIR", self.home.join("tmp").as_os_str());
        set("LANG", "C".as_ref());
        for (k, v) in &spec.env {
            let key = k.to_string_lossy();
            if matches!(key.as_ref(), "PATH" | "HOME" | "TMPDIR" | "LANG")
                || key.starts_with("TEXMF")
            {
                continue;
            }
            set(&key, v);
        }
        args.push(spec.program.clone().into_os_string());
        args.extend(spec.args.iter().cloned());
        let mut wrapped = CommandSpec::new("/usr/bin/env");
        wrapped.args = args;
        wrapped.cwd = spec.cwd.clone();
        wrapped.changes_installation = spec.changes_installation;
        self.inner.run(&wrapped)
    }
}

/// A disposable tree that passed every check.
pub struct Sandbox {
    /// The canonical root.
    pub root: PathBuf,
    /// The fresh `HOME` the children get.
    pub home: PathBuf,
    /// The distribution resolved in the child environment.
    pub dist: Distribution,
    exec: SandboxExec,
}

impl Sandbox {
    /// The `Exec` to give the code under test.
    pub fn exec(&self) -> &SandboxExec {
        &self.exec
    }

    /// The child's `PATH` (the copy's `bin` plus system directories).
    pub fn exec_path(&self) -> OsString {
        self.exec.path.clone()
    }

    /// Runs one of the distribution's programs in the child environment.
    pub fn run(&self, program: &str, args: &[&str]) -> io::Result<Output> {
        let mut spec = self.dist.command(program);
        spec.args.extend(args.iter().map(OsString::from));
        self.exec.run(&spec)
    }
}

fn canon(p: &Path) -> Result<PathBuf, Refusal> {
    p.canonicalize()
        .map_err(|_| Refusal::RootUnreadable(p.to_path_buf()))
}

/// The gate's decision for `i`.
pub fn check(i: &Inputs) -> Result<Sandbox, Refusal> {
    if !cfg!(unix) {
        return Err(Refusal::Unsupported);
    }
    let root = canon(i.root.as_deref().ok_or(Refusal::RootUnset)?)?;

    for d in &i.default_dirs {
        if let Ok(d) = d.canonicalize()
            && root.starts_with(&d)
        {
            return Err(Refusal::DefaultInstallDir(d));
        }
    }
    let under_temp = i
        .temp_dirs
        .iter()
        .filter_map(|t| t.canonicalize().ok())
        .any(|t| root != t && root.starts_with(&t));
    if !under_temp {
        return Err(Refusal::NotUnderTemp(root));
    }

    let sentinel = std::fs::read_to_string(root.join("DISPOSABLE_TINYTEX"))
        .map_err(|_| Refusal::SentinelMissing)?;
    let mut lines = sentinel.lines();
    let named = lines.next().unwrap_or("");
    if Path::new(named) != root {
        return Err(Refusal::SentinelPath {
            expected: root,
            found: named.to_owned(),
        });
    }
    match (&i.nonce, lines.next()) {
        (Some(want), Some(have)) if want == have => {}
        _ => return Err(Refusal::NonceMismatch),
    }
    if !root.join("COMPLETE").exists() {
        return Err(Refusal::Incomplete);
    }

    // The child environment: the copy's bin directories, then system ones.
    let home = PathBuf::from(format!("{}.home", root.display()));
    std::fs::create_dir_all(home.join("tmp")).map_err(|_| Refusal::RootUnreadable(home.clone()))?;
    for dir in &i.system_path {
        if TEX_TOOLS.iter().any(|t| dir.join(t).exists()) {
            return Err(Refusal::SystemPathHasTex(dir.clone()));
        }
    }
    let mut bins: Vec<PathBuf> = std::fs::read_dir(root.join("bin"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    bins.sort();
    let path = std::env::join_paths(bins.iter().chain(&i.system_path))
        .map_err(|_| Refusal::NoDistribution)?;

    // The distribution the code under test will use, resolved the way it
    // would be in the child: its HOME and its PATH, not this process's.
    let mut cfg = Config::default();
    cfg.install_dir = Some(root.clone());
    let mut host = HostEnv::current();
    host.home = Some(home.clone());
    let dist = find_with(&cfg, &host, Some(&path), &[]).ok_or(Refusal::NoDistribution)?;
    if dist
        .root
        .as_deref()
        .and_then(|r| r.canonicalize().ok())
        .as_deref()
        != Some(root.as_path())
    {
        return Err(Refusal::NoDistribution);
    }
    let inside = |what: &'static str, p: &Path| -> Result<(), Refusal> {
        let real = p.canonicalize().map_err(|_| Refusal::EscapesRoot {
            what,
            real: p.to_path_buf(),
        })?;
        if real.starts_with(&root) {
            Ok(())
        } else {
            Err(Refusal::EscapesRoot { what, real })
        }
    };
    inside("tlmgr", &dist.tlmgr)?;
    inside("the engine bin directory", &dist.bin_dir)?;
    inside("kpsewhich", &dist.bin_dir.join("kpsewhich"))?;

    let sandbox = Sandbox {
        root: root.clone(),
        home: home.clone(),
        dist,
        exec: SandboxExec {
            inner: StdExec::new(),
            path,
            home,
        },
    };
    let out = sandbox
        .run("kpsewhich", &["-var-value=TEXMFROOT"])
        .map_err(|e| Refusal::KpsewhichRoot {
            got: format!("(failed to run: {e})"),
        })?;
    let got = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let same = out.status.success() && Path::new(&got).canonicalize().is_ok_and(|g| g == root);
    if !same {
        return Err(Refusal::KpsewhichRoot { got });
    }
    Ok(sandbox)
}

/// A [`Sandbox`], or `None` after printing why the test skips; panics instead
/// when `TINYTEX_RS_REQUIRE_SANDBOX=1`.
pub fn gate(test: &str) -> Option<Sandbox> {
    match check(&Inputs::from_env()) {
        Ok(s) => Some(s),
        Err(r) => {
            assert!(
                std::env::var("TINYTEX_RS_REQUIRE_SANDBOX").as_deref() != Ok("1"),
                "{test}: TINYTEX_RS_REQUIRE_SANDBOX=1 but the sandbox gate refused: {r}"
            );
            eprintln!(
                "SKIPPED {test}: no disposable TeX tree ({r}); build one with scripts/tex-sandbox.sh \
                 (TINYTEX_RS_REQUIRE_SANDBOX=1 fails instead)"
            );
            None
        }
    }
}
