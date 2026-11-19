# tinytex

A blocking Rust driver for TeX Live and TinyTeX. It compiles a `.tex` file to
PDF, reruns the engine until cross-references settle, runs BibTeX, Biber and
index tools, parses the log into diagnostics, installs missing LaTeX packages
with `tlmgr`, and finds a distribution on the machine. It has no async runtime
and no HTTP dependency, and it knows nothing about any document format.

**Status:** `0.x` pre-release. The API can change between tags.

## Example

`Compiler::compile` takes the source, a `CompileRequest`, an `Exec` that runs
subprocesses, and an `Observer` that receives progress events. The example
below uses a mock `Exec`, so it runs without TeX. Use `StdExec` to run the real
tools. The same code ships as `examples/compile_with_mock.rs`.

```rust
use std::io;
use std::process::Output;
use tinytex::{CommandSpec, CompileRequest, Compiler, Config, Exec, NullObserver};

struct MockTex;

impl Exec for MockTex {
    fn run(&self, cmd: &CommandSpec) -> io::Result<Output> {
        // Write the PDF and the log where the engine would.
        let dir = cmd.cwd.clone().unwrap_or_default();
        let src = cmd.args.last().map(std::path::PathBuf::from).unwrap_or_default();
        let stem = src.file_stem().unwrap_or_default();
        std::fs::write(dir.join(stem).with_extension("pdf"), b"%PDF-1.5")?;
        std::fs::write(dir.join(stem).with_extension("log"), b"")?;
        Ok(Output {
            status: Default::default(),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
}

let dir = tempfile::tempdir()?;
let tex = dir.path().join("hello.tex");
std::fs::write(&tex, r"\documentclass{article}\begin{document}Hi\end{document}")?;

let compiler = Compiler::new(None, Config::default());
let compiled = compiler.compile(&tex, &CompileRequest::new(), &MockTex, &NullObserver)?;
assert!(compiled.pdf.exists());
# Ok::<(), Box<dyn std::error::Error>>(())
```

Pass `Compiler::auto(Config::from_env())` to locate TinyTeX or TeX Live
instead of taking the tools from `PATH`.

## Modules

| Module | Purpose |
|---|---|
| `compile` | `Compiler`, `CompileRequest` and the rerun loop; engines `pdflatex`, `xelatex`, `lualatex`, `tectonic` and `latexmk`; BibTeX, Biber and index tools; cleanup and output placement |
| `log` | Parse engine logs into missing-package search terms and diagnostics |
| `tlmgr` | Search for and install packages, update `tlmgr`, rebuild formats |
| `detect` | Find a TinyTeX or TeX Live installation and build commands for it |
| `config` | `Config`, read from the environment in one place |
| `exec` | The `Exec` and `Observer` traits and the `StdExec` implementation |

## Dependencies

Normal dependencies are `serde` (with `derive`), `thiserror` and `regex`. The
`cli` feature adds `clap` and `serde_json` for the `tinytex` binary. Test-only
dependencies are `serde_json` and `tempfile`. The crate depends on no Quarto
crate. The minimum supported Rust version is 1.94 (`rust-version` in
`Cargo.toml`).

## Conventions

Public request, config, event and result types are `Serialize` and
`Deserialize`. Error types are `Serialize` and carry a stable string
`ErrorCode`, so a host in another language can switch on it. The design is in
`docs/design.md` in the repository.

## License

MIT. See the `LICENSE` and `NOTICE.md` files in the repository.
