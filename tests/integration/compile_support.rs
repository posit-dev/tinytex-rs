//! Shared helpers for the compile tests. Not a test module.
#![allow(clippy::result_large_err)] // `CompileError` is large by design (see `compile`).

use std::path::PathBuf;
use std::sync::Arc;

use tempfile::TempDir;
use tinytex::test_support::{FakeTex, Reply};
use tinytex::{CompileError, CompileRequest, Compiled, Compiler, Config, NullObserver, Observer};

use super::tlmgr_model::{Model, dist, fake};

/// A temporary directory holding `doc.tex`.
pub struct Doc {
    pub dir: TempDir,
    pub tex: PathBuf,
}

pub fn doc() -> Doc {
    let dir = tempfile::tempdir().unwrap();
    let tex = dir.path().join("doc.tex");
    std::fs::write(
        &tex,
        "\\documentclass{article}\\begin{document}x\\end{document}",
    )
    .unwrap();
    Doc { dir, tex }
}

impl Doc {
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
    pub fn exists(&self, name: &str) -> bool {
        self.path(name).exists()
    }
    pub fn write(&self, name: &str, text: &str) {
        std::fs::write(self.path(name), text).unwrap();
    }
}

pub const CLEAN_LOG: &str = "This is pdfTeX\nOutput written on doc.pdf (1 page).\n";
pub const RERUN_LOG: &str = "This is pdfTeX\nLaTeX Warning: Label(s) may have changed. Rerun to get cross-references right.\n";

/// A successful engine run: writes `doc.log` and `doc.pdf`.
pub fn ok(log: &str) -> Reply {
    Reply::ok()
        .creates("doc.log", log)
        .creates("doc.pdf", "%PDF-1.5")
}

/// A successful run that also writes more files.
pub fn ok_with(log: &str, files: &[(&str, &str)]) -> Reply {
    let mut r = ok(log);
    for (n, c) in files {
        r = r.creates(*n, *c);
    }
    r
}

/// A failed run: status 1, a log, no PDF.
pub fn fails(log: &str) -> Reply {
    Reply::code(1).creates("doc.log", log)
}

pub fn missing_sty(name: &str) -> String {
    format!("! LaTeX Error: File `{name}.sty' not found.\n\nType X to quit.\n")
}

/// `tlmgr search --file --global /name.sty` answers package `pkg`.
pub fn search_answer(x: &FakeTex, file: &str, pkg: &str) {
    x.reply(
        "tlmgr",
        &["search", "--file", "--global", &format!("/{file}")],
        Reply::ok().stdout(format!("{pkg}:\n\ttexmf-dist/tex/latex/{pkg}/{file}\n")),
    );
}

/// A `FakeTex` over `model` (for `tlmgr info` and `tlmgr install`).
pub fn tex(model: &Arc<Model>) -> FakeTex {
    fake(model)
}

pub fn compiler(name: &str) -> Compiler {
    Compiler::new(Some(dist(name)), Config::default())
}

pub fn run(
    name: &str,
    d: &Doc,
    req: &CompileRequest,
    x: &FakeTex,
) -> Result<Compiled, CompileError> {
    compiler(name).compile(&d.tex, req, x, &NullObserver)
}

pub fn run_observed(
    name: &str,
    d: &Doc,
    req: &CompileRequest,
    x: &FakeTex,
    o: &dyn Observer,
) -> Result<Compiled, CompileError> {
    compiler(name).compile(&d.tex, req, x, o)
}

/// An observer that records every event.
#[derive(Default)]
pub struct Recorder(pub std::sync::Mutex<Vec<tinytex::Event>>);

impl Observer for Recorder {
    fn event(&self, e: &tinytex::Event) {
        self.0.lock().unwrap().push(e.clone());
    }
}

impl Recorder {
    pub fn events(&self) -> Vec<tinytex::Event> {
        self.0.lock().unwrap().clone()
    }
}

/// Engine command lines only.
pub fn engine_lines(x: &FakeTex) -> Vec<String> {
    x.lines()
        .into_iter()
        .filter(|l| l.starts_with("pdflatex"))
        .collect()
}
