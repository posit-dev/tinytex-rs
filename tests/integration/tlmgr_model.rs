//! A small model of a TeX Live installation for mocked `tlmgr` tests: which
//! packages are installed, which can be installed, and how each `tlmgr
//! install` call behaves. Not a test module.

use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tinytex::test_support::{FakeTex, Reply};
use tinytex::{Distribution, Kind};

pub struct Model {
    pub installed: Mutex<BTreeSet<String>>,
    pub installable: BTreeSet<String>,
    /// One `(exit status, whether it installs)` per `tlmgr install` call;
    /// `(0, true)` once the script runs out.
    pub script: Mutex<VecDeque<(i32, bool)>>,
}

pub fn model(installed: &[&str], installable: &[&str], script: &[(i32, bool)]) -> Arc<Model> {
    Arc::new(Model {
        installed: Mutex::new(installed.iter().map(|s| (*s).to_owned()).collect()),
        installable: installable.iter().map(|s| (*s).to_owned()).collect(),
        script: Mutex::new(script.iter().copied().collect()),
    })
}

/// A `FakeTex` answering `tlmgr info --list --only-installed --data name ..`
/// and `tlmgr install ..` from `model`; every other command answers 0.
pub fn fake(model: &Arc<Model>) -> FakeTex {
    let model = Arc::clone(model);
    FakeTex::new().with_handler(move |spec| {
        let name = spec.program.file_name()?.to_string_lossy().into_owned();
        if name != "tlmgr" && name != "tlmgr.bat" {
            return None;
        }
        let args: Vec<String> = spec
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let at = |word: &str| args.iter().position(|a| a == word);
        if let Some(i) = at("info") {
            let pkgs = &args[args.iter().position(|a| a == "name")? + 1..];
            let _ = i;
            let installed = model.installed.lock().unwrap();
            let mut out = String::new();
            for p in pkgs {
                if installed.contains(p) {
                    out.push_str(p);
                    out.push('\n');
                }
            }
            return Some(Reply::ok().stdout(out));
        }
        if let Some(i) = at("install") {
            let (code, installs) = model
                .script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or((0, true));
            if installs {
                let mut installed = model.installed.lock().unwrap();
                for p in &args[i + 1..] {
                    if model.installable.contains(p) {
                        installed.insert(p.clone());
                    }
                }
            }
            return Some(Reply::code(code));
        }
        None
    })
}

/// A TinyTeX-shaped distribution whose root is unique to `name`, so tests in
/// one process never share a registry entry by accident.
pub fn dist(name: &str) -> Distribution {
    let root = PathBuf::from("/nonexistent/tinytex-rs-tests").join(name);
    let bin_dir = root.join("bin").join("x86_64-linux");
    Distribution {
        tlmgr: bin_dir.join("tlmgr"),
        bin_dir,
        root: Some(root),
        kind: Kind::TinyTeX,
    }
}

/// The recorded lines without the read-only `tlmgr info` checks.
pub fn without_info(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .filter(|l| !l.starts_with("tlmgr info"))
        .collect()
}

/// `Config` and `UpdateOpts` are `#[non_exhaustive]`: a test outside the crate
/// builds one from its default and sets fields.
pub fn cfg_with(f: impl FnOnce(&mut tinytex::Config)) -> tinytex::Config {
    let mut c = tinytex::Config::default();
    f(&mut c);
    c
}

pub fn opts_with(f: impl FnOnce(&mut tinytex::UpdateOpts)) -> tinytex::UpdateOpts {
    let mut o = tinytex::UpdateOpts::default();
    f(&mut o);
    o
}
