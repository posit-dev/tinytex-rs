# tinytex

A blocking Rust driver for TeX Live and TinyTeX: compile a `.tex` file to PDF,
parse the log, and manage `tlmgr` packages.

**Status:** bootstrap. The crate exposes no compile API yet; the design is in
[`docs/design.md`](docs/design.md).

## Dependencies

The crate has no async runtime. Normal dependencies are `serde` (with
`derive`), `thiserror` and `regex`. The `cli` feature adds `clap` and
`serde_json` for the `tinytex` binary. Test-only dependencies are `serde_json`
and `tempfile`. The crate has no HTTP or archive dependency, and none on any
Quarto crate.

## Conventions

Public request, config, event and result types are `Serialize` and
`Deserialize`; error types are `Serialize` and carry a stable string code.

## License

MIT. See [`LICENSE`](LICENSE) and [`NOTICE.md`](NOTICE.md).
