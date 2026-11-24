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

## The `tinytex` command

With the `cli` feature the crate builds a `tinytex` program, a thin layer over
the library for scripts and for programs in other languages. It needs no
knowledge of any other tool: it compiles a LaTeX file to PDF and runs the
`tlmgr` package manager of a TeX Live or TinyTeX installation.

```text
cargo install tinytex --features cli
tinytex detect                          # where is TeX?
tinytex compile paper.tex               # prints the path of paper.pdf
tinytex compile --json paper.tex        # the result as JSON on stdout
tinytex compile --pdf-engine xelatex --min 2 --max 11 paper.tex
tinytex log paper.log                   # the errors and warnings in a log
tinytex tlmgr install booktabs
```

Every flag of `compile` sets exactly one field of `CompileRequest` or
`Config`; `--print-request` prints the pair the flags produce and exits, and
`--request <file|->` reads a request from JSON. The directory of the `.tex`
file is the working directory; there is no flag to change it. Without
`--auto-install` or `--no-auto-install`, packages are installed when the
distribution is writable.

Standard output carries the result: the output path, or with `--json` an
object `{"schema": 1, ...}`. Standard error carries progress (`--events json`
prints one JSON object per line, with a `type` tag) and diagnostics. A JSON
error is `{"schema": 1, "code": ..., "message": ...}`. Readers ignore unknown
fields and unknown `type` values; removing or retyping a field bumps `schema`.

| Exit code | Meaning |
|---|---|
| 0 | success |
| 1 | LaTeX failure (also a package install, bibliography, index, no-output or `tlmgr` failure) |
| 2 | usage error or invalid request |
| 3 | environment problem: no TeX installation, a tool not found, an I/O error |
| 4 | cancelled, or the `--timeout` deadline passed |
| 5 | the installed TeX Live is older than the remote repository |

The `code` values are `no_tex_installation`, `engine_not_found`, `latex_error`,
`package_install`, `texlive_too_old`, `bibliography`, `index`, `no_output`,
`invalid_request`, `cancelled`, `io` and `tlmgr`. The JSON shape is
unstable until the first tagged release that another project pins.

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
