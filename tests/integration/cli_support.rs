//! Shared helpers for the CLI tests. Not a test module.
#![cfg(feature = "cli")]

use std::ffi::OsString;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tinytex::cli::{Host, Io};
use tinytex::test_support::FakeTex;
use tinytex::{CompileRequest, Config, Distribution, Exec};

use super::tlmgr_model::dist;

/// A `Write` that appends to a shared buffer.
#[derive(Clone, Default)]
pub struct Buf(pub Arc<Mutex<Vec<u8>>>);

impl Write for Buf {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Buf {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// One CLI run: its exit code, both streams, the mocked `Exec`, and the
/// deadline the CLI asked the `Exec` factory for.
pub struct Ran {
    pub code: i32,
    pub out: String,
    pub err: String,
    pub x: Arc<FakeTex>,
    pub deadline: Option<Option<Duration>>,
}

impl Ran {
    /// Stdout as one JSON value.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.out)
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {:?}", self.out))
    }
}

/// A host with a TinyTeX-shaped distribution named `name` (or none), a
/// writability answer, an empty environment, and `x` as the only `Exec`.
pub fn host(
    x: &Arc<FakeTex>,
    name: Option<&str>,
    writable: bool,
    asked: &Arc<Mutex<Option<Option<Duration>>>>,
) -> Host {
    let found: Option<Distribution> = name.map(dist);
    let mut h = Host::system();
    h.env = Arc::new(|_| None);
    h.detect = Arc::new(move |_| found.clone());
    h.writable = Arc::new(move |_| writable);
    let x = Arc::clone(x);
    let asked = Arc::clone(asked);
    h.exec = Arc::new(move |deadline| {
        *asked.lock().unwrap() = Some(deadline);
        Arc::clone(&x) as Arc<dyn Exec>
    });
    h.stdin = Arc::new(|| Ok(String::new()));
    h
}

/// Runs the CLI with `args` (without the program name) against a fresh
/// `FakeTex` and the distribution `name`.
pub fn run(name: Option<&str>, args: &[&str]) -> Ran {
    let x = Arc::new(FakeTex::new());
    run_with(&x, name, true, args.iter().map(OsString::from).collect())
}

pub fn run_with(x: &Arc<FakeTex>, name: Option<&str>, writable: bool, args: Vec<OsString>) -> Ran {
    let asked = Arc::new(Mutex::new(None));
    let h = host(x, name, writable, &asked);
    run_on(&h, x, &asked, args)
}

pub fn run_on(
    h: &Host,
    x: &Arc<FakeTex>,
    asked: &Arc<Mutex<Option<Option<Duration>>>>,
    args: Vec<OsString>,
) -> Ran {
    let (out, err) = (Buf::default(), Buf::default());
    let io = Io::new(out.clone(), err.clone());
    let mut argv = vec![OsString::from("tinytex")];
    argv.extend(args);
    let code = tinytex::cli::run(argv, h, &io);
    let deadline = *asked.lock().unwrap();
    Ran {
        code,
        out: out.text(),
        err: err.text(),
        x: Arc::clone(x),
        deadline,
    }
}

/// `compile --print-request <args> doc.tex` against no distribution, parsed
/// back into the printed pair.
pub fn printed(args: &[&str]) -> (CompileRequest, Config) {
    let mut all = vec!["compile", "--print-request"];
    all.extend_from_slice(args);
    all.push("doc.tex");
    let ran = run(None, &all);
    assert_eq!(ran.code, 0, "stderr: {}", ran.err);
    parse_pair(&ran.out)
}

pub fn parse_pair(text: &str) -> (CompileRequest, Config) {
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    (
        serde_json::from_value(v["request"].clone()).unwrap(),
        serde_json::from_value(v["config"].clone()).unwrap(),
    )
}
