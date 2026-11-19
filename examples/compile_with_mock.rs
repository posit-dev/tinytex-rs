//! Compiles a `.tex` file through a mocked `Exec`, so the example runs without
//! a TeX installation. Swap `MockTex` for `StdExec` to run the real engine.

use std::io;
use std::path::PathBuf;
use std::process::Output;

use tinytex::{CommandSpec, CompileRequest, Compiler, Config, Exec, NullObserver};

/// Pretends to be the engine: "compiles" by writing a PDF and a log next to
/// the source, in the working directory the crate hands it.
struct MockTex;

#[cfg(unix)]
fn ok_status() -> std::process::ExitStatus {
    std::os::unix::process::ExitStatusExt::from_raw(0)
}
#[cfg(windows)]
fn ok_status() -> std::process::ExitStatus {
    std::os::windows::process::ExitStatusExt::from_raw(0)
}

impl Exec for MockTex {
    fn run(&self, cmd: &CommandSpec) -> io::Result<Output> {
        let dir: PathBuf = cmd.cwd.clone().unwrap_or_default();
        // The last argument names the source file.
        let src = cmd.args.last().map(PathBuf::from).unwrap_or_default();
        let stem = src.file_stem().unwrap_or_default();
        std::fs::write(dir.join(stem).with_extension("pdf"), b"%PDF-1.5 mock")?;
        std::fs::write(dir.join(stem).with_extension("log"), b"This is MockTeX\n")?;
        Ok(Output {
            status: ok_status(),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
}

fn main() {
    let dir = tempfile::tempdir().expect("temp dir");
    let tex = dir.path().join("hello.tex");
    std::fs::write(
        &tex,
        r"\documentclass{article}\begin{document}Hi\end{document}",
    )
    .expect("write source");

    // `None` takes the engine from PATH; `Compiler::auto` would find TinyTeX.
    let compiler = Compiler::new(None, Config::default());
    let compiled = compiler
        .compile(&tex, &CompileRequest::new(), &MockTex, &NullObserver)
        .expect("compile");
    assert!(compiled.pdf.exists());
    println!("wrote {}", compiled.pdf.display());
}
