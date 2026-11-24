//! Flag parity: each flag sets exactly the field it is documented to set, so
//! the printed `{request, config}` equals the pair built through the library's
//! setters, and a JSON request round-trips through `--request`.
#![cfg(feature = "cli")]

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use tinytex::test_support::FakeTex;
use tinytex::{
    Bib, Clean, CompileRequest, Config, Driver, Engine, IndexEngine, KeepLog, OnBibFailure, Output,
    Prefer, Recover, TlmgrUpdate, ToolPaths,
};

use super::cli_support::{parse_pair, printed, run, run_with};

type Pair = (CompileRequest, Config);

fn req(f: impl FnOnce(CompileRequest) -> CompileRequest) -> Pair {
    (f(CompileRequest::new()), Config::default())
}

fn cfg(f: impl FnOnce(&mut Config)) -> Pair {
    let mut c = Config::default();
    f(&mut c);
    (CompileRequest::new(), c)
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| (*x).to_owned()).collect()
}

/// `(flags, expected pair)`; every flag of the parity table appears here.
fn table() -> Vec<(Vec<&'static str>, Pair)> {
    vec![
        (vec![], (CompileRequest::new(), Config::default())),
        (
            vec!["--pdf-engine", "xelatex"],
            req(|r| r.engine(Engine::XeLatex)),
        ),
        (
            vec!["--pdf-engine", "latexmk"],
            req(|r| r.engine(Engine::Latexmk)),
        ),
        (
            vec![
                "--pdf-engine-opt=-shell-escape",
                "--pdf-engine-opt",
                "-8bit",
            ],
            req(|r| r.engine_args(s(&["-shell-escape", "-8bit"]))),
        ),
        (vec!["--bib-engine", "bibtex"], req(|r| r.bib(Bib::Bibtex))),
        (vec!["--bib-engine", "biber"], req(|r| r.bib(Bib::Biber))),
        (vec!["--bib-engine", "infer"], req(|r| r.bib(Bib::Infer))),
        (vec!["--bib-engine", "skip"], req(|r| r.bib(Bib::Skip))),
        (vec!["--bib-engine", "natbib"], req(|r| r.bib(Bib::Bibtex))),
        (vec!["--bib-engine", "biblatex"], req(|r| r.bib(Bib::Biber))),
        (
            vec![
                "--index-engine",
                "mendex",
                "--index-engine-opt",
                "-s",
                "--index-engine-opt=x.ist",
            ],
            req(|r| {
                r.index(IndexEngine {
                    command: "mendex".into(),
                    args: s(&["-s", "x.ist"]),
                })
            }),
        ),
        (
            vec!["--index-engine-opt", "-q"],
            req(|r| {
                r.index(IndexEngine {
                    command: "makeindex".into(),
                    args: s(&["-q"]),
                })
            }),
        ),
        (vec!["--min", "2", "--max", "11"], req(|r| r.runs(2, 11))),
        // R's `min_times = 1, max_times = 10`.
        (vec!["--min", "2", "--max", "11"], req(|r| r.reruns(1, 10))),
        (vec!["--max", "4"], req(|r| r.runs(1, 4))),
        (
            vec!["--driver", "latexmk"],
            req(|r| r.driver(Driver::Latexmk)),
        ),
        (
            vec!["--driver", "builtin"],
            req(|r| r.driver(Driver::Builtin)),
        ),
        (
            vec!["--recover", "first-run"],
            req(|r| r.recover(Recover::FirstRun)),
        ),
        (
            vec!["--recover", "every-run"],
            req(|r| r.recover(Recover::EveryRun)),
        ),
        (
            vec!["--on-bib-failure", "error"],
            req(|r| r.on_bib_failure(OnBibFailure::Error)),
        ),
        (
            vec!["--on-bib-failure", "warn"],
            req(|r| r.on_bib_failure(OnBibFailure::Warn)),
        ),
        (
            vec!["--keep-log", "on-failure"],
            req(|r| r.keep_log(KeepLog::OnFailure)),
        ),
        (
            vec!["--keep-log", "on-warnings"],
            req(|r| r.keep_log(KeepLog::OnWarnings)),
        ),
        (
            vec!["--keep-log", "always"],
            req(|r| r.keep_log(KeepLog::Always)),
        ),
        (
            vec!["--keep-log", "never"],
            req(|r| r.keep_log(KeepLog::Never)),
        ),
        (vec!["--output", "pdf"], req(|r| r.output(Output::Pdf))),
        (
            vec!["--output-dir", "out/dir"],
            req(|r| r.output_dir(PathBuf::from("out/dir"))),
        ),
        (
            vec!["--pdf-file", "final.pdf"],
            req(|r| r.pdf_file(PathBuf::from("final.pdf"))),
        ),
        (
            vec!["--input-path", "a", "--input-path", "b"],
            req(|r| r.input_paths(vec!["a".into(), "b".into()])),
        ),
        (vec!["--clean", "never"], req(|r| r.clean(Clean::Never))),
        (
            vec!["--clean", "on-success"],
            req(|r| r.clean(Clean::OnSuccess)),
        ),
        (vec!["--clean", "always"], req(|r| r.clean(Clean::Always))),
        (vec!["--no-clean"], req(|r| r.clean(Clean::Never))),
        (vec!["--auto-install"], req(|r| r.auto_install(true))),
        (vec!["--no-auto-install"], req(|r| r.auto_install(false))),
        (
            vec!["--tool", "bibtex=/x/bibtex", "--tool", "engine=/x/e"],
            req(|r| {
                r.tools(ToolPaths {
                    engine: Some("/x/e".into()),
                    bibtex: Some("/x/bibtex".into()),
                    ..ToolPaths::default()
                })
            }),
        ),
        (
            vec!["--tool", "biber=/b", "--tool", "makeindex=/m"],
            req(|r| {
                r.tools(ToolPaths {
                    biber: Some("/b".into()),
                    makeindex: Some("/m".into()),
                    ..ToolPaths::default()
                })
            }),
        ),
        (
            vec!["--tool", "latexmk=/l", "--tool", "tectonic=/t"],
            req(|r| {
                r.tools(ToolPaths {
                    latexmk: Some("/l".into()),
                    tectonic: Some("/t".into()),
                    ..ToolPaths::default()
                })
            }),
        ),
        (
            vec!["--tlmgr-opt=--repository=x", "--tlmgr-opt", "--usermode"],
            cfg(|c| c.tlmgr_opts = s(&["--repository=x", "--usermode"])),
        ),
        (
            vec!["--tlmgr-update", "never"],
            cfg(|c| c.tlmgr_update = TlmgrUpdate::Never),
        ),
        (
            vec!["--tlmgr-update", "auto"],
            cfg(|c| c.tlmgr_update = TlmgrUpdate::Auto),
        ),
        (
            vec!["--prefer", "system"],
            cfg(|c| c.prefer = Prefer::System),
        ),
        (
            vec!["--prefer", "tinytex"],
            cfg(|c| c.prefer = Prefer::TinyTeX),
        ),
        (
            vec!["--install-dir", "/opt/tt"],
            cfg(|c| c.install_dir = Some("/opt/tt".into())),
        ),
        (
            vec!["--repository", "https://example.org/tex"],
            cfg(|c| c.repository = Some("https://example.org/tex".into())),
        ),
        (
            vec!["--tlmgr-path", "/opt/tlmgr"],
            cfg(|c| c.tlmgr_path = Some("/opt/tlmgr".into())),
        ),
        (vec!["--usermode"], cfg(|c| c.usermode = true)),
        (vec!["--delete-tlpdb"], cfg(|c| c.delete_tlpdb = true)),
        (vec!["--verbose"], cfg(|c| c.verbose = true)),
    ]
}

#[test]
fn every_flag_sets_the_field_the_library_setter_sets() {
    for (flags, expected) in table() {
        assert_eq!(printed(&flags), expected, "flags: {flags:?}");
    }
}

#[test]
fn the_flag_table_exercises_every_enum_value_of_the_new_flags() {
    // A guard on the table itself: the new flags of plan 5.6 each appear.
    let all: String = table()
        .iter()
        .flat_map(|(f, _)| f.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");
    for needle in [
        "--driver",
        "--recover",
        "--on-bib-failure",
        "--clean",
        "--no-clean",
        "--keep-log",
        "--output",
        "--tlmgr-update",
        "--prefer",
        "--install-dir",
        "--repository",
    ] {
        assert!(all.contains(needle), "{needle} is not in the parity table");
    }
}

#[test]
fn a_later_flag_wins_where_two_flags_set_one_field() {
    assert!(
        printed(&["--no-auto-install", "--auto-install"])
            .0
            .auto_install
    );
    assert!(
        !printed(&["--auto-install", "--no-auto-install"])
            .0
            .auto_install
    );
}

#[test]
fn a_json_request_round_trips_through_request() {
    let original = CompileRequest::new()
        .engine(Engine::LuaLatex)
        .driver(Driver::Builtin)
        .engine_args(s(&["-shell-escape"]))
        .bib(Bib::Biber)
        .index(IndexEngine {
            command: "makeindex".into(),
            args: s(&["-q"]),
        })
        .runs(2, 7)
        .output_dir(PathBuf::from("build"))
        .pdf_file(PathBuf::from("out.pdf"))
        .input_paths(vec!["styles".into()])
        .auto_install(true)
        .recover(Recover::EveryRun)
        .on_bib_failure(OnBibFailure::Warn)
        .clean(Clean::OnSuccess)
        .keep_log(KeepLog::Always)
        .output(Output::Pdf);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("request.json");
    std::fs::write(&file, serde_json::to_string(&original).unwrap()).unwrap();
    let ran = run(
        None,
        &[
            "compile",
            "--print-request",
            "--request",
            file.to_str().unwrap(),
            "doc.tex",
        ],
    );
    assert_eq!(ran.code, 0, "{}", ran.err);
    assert_eq!(parse_pair(&ran.out), (original, Config::default()));
}

#[test]
fn the_printed_pair_is_accepted_back_as_a_request() {
    let first = run(
        None,
        &[
            "compile",
            "--print-request",
            "--pdf-engine",
            "xelatex",
            "--min",
            "3",
            "--prefer",
            "system",
            "doc.tex",
        ],
    );
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pair.json");
    std::fs::write(&file, &first.out).unwrap();
    let second = run(
        None,
        &[
            "compile",
            "--print-request",
            "--request",
            file.to_str().unwrap(),
            "doc.tex",
        ],
    );
    assert_eq!(parse_pair(&second.out), parse_pair(&first.out));
}

#[test]
fn request_dash_reads_standard_input() {
    let x = Arc::new(FakeTex::new());
    let asked = Arc::new(std::sync::Mutex::new(None));
    let mut h = super::cli_support::host(&x, None, false, &asked);
    h.stdin = Arc::new(|| Ok(r#"{"engine":"xe_latex","max_runs":5}"#.to_owned()));
    let args: Vec<OsString> = ["compile", "--print-request", "--request", "-", "doc.tex"]
        .iter()
        .map(OsString::from)
        .collect();
    let ran = super::cli_support::run_on(&h, &x, &asked, args);
    assert_eq!(ran.code, 0, "{}", ran.err);
    let (request, _) = parse_pair(&ran.out);
    assert_eq!(request.engine, Engine::XeLatex);
    assert_eq!(request.max_runs, 5);
}

#[test]
fn flags_override_a_request_field_and_leave_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("request.json");
    std::fs::write(&file, r#"{"engine":"xe_latex","bib":"biber","max_runs":6}"#).unwrap();
    let ran = run(
        None,
        &[
            "compile",
            "--print-request",
            "--request",
            file.to_str().unwrap(),
            "--pdf-engine",
            "lualatex",
            "doc.tex",
        ],
    );
    let (request, _) = parse_pair(&ran.out);
    assert_eq!(request.engine, Engine::LuaLatex);
    assert_eq!(request.bib, Bib::Biber);
    assert_eq!(request.max_runs, 6);
}

#[test]
fn a_request_file_that_is_not_json_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("request.json");
    std::fs::write(&file, "not json").unwrap();
    let ran = run(
        None,
        &["compile", "--request", file.to_str().unwrap(), "doc.tex"],
    );
    assert_eq!(ran.code, 2, "{}", ran.err);
    assert!(ran.err.contains("request"), "{}", ran.err);
}

#[test]
fn the_environment_feeds_the_config_and_a_flag_overrides_it() {
    let x = Arc::new(FakeTex::new());
    let asked = Arc::new(std::sync::Mutex::new(None));
    let mut h = super::cli_support::host(&x, None, false, &asked);
    h.env = Arc::new(|name| match name {
        "TINYTEX_DIR" => Some("/from/env".into()),
        "TINYTEX_PREVENT_INSTALL" => Some("true".into()),
        _ => None,
    });
    let run_it = |extra: &[&str]| {
        let mut args: Vec<OsString> = vec!["compile".into(), "--print-request".into()];
        args.extend(extra.iter().map(OsString::from));
        args.push("doc.tex".into());
        let ran = super::cli_support::run_on(&h, &x, &asked, args);
        parse_pair(&ran.out).1
    };
    let c = run_it(&[]);
    assert_eq!(c.install_dir, Some("/from/env".into()));
    assert!(c.prevent_install);
    let c = run_it(&["--install-dir", "/from/flag"]);
    assert_eq!(c.install_dir, Some("/from/flag".into()));
}

#[test]
fn defaults_are_the_crates_with_auto_install_from_writability() {
    // No flags at all: the crate's defaults; only `auto_install` comes from
    // the distribution.
    for (name, writable, auto) in [
        (None, true, false),
        (Some("p-none"), false, false),
        (Some("p-writable"), true, true),
    ] {
        let x = Arc::new(FakeTex::new());
        let args = ["compile", "--print-request", "doc.tex"]
            .iter()
            .map(OsString::from)
            .collect();
        let ran = run_with(&x, name, writable, args);
        let (request, config) = parse_pair(&ran.out);
        assert_eq!(request, CompileRequest::new().auto_install(auto));
        assert_eq!(config, Config::default());
    }
}

#[test]
fn min_and_max_count_total_runs_not_reruns() {
    let (request, _) = printed(&["--min", "2", "--max", "11"]);
    assert_eq!((request.min_runs, request.max_runs), (2, 11));
    assert_eq!(request, CompileRequest::new().reruns(1, 10));
}

#[test]
fn a_print_request_run_runs_nothing() {
    let x = Arc::new(FakeTex::new());
    let args = ["compile", "--print-request", "doc.tex"]
        .iter()
        .map(OsString::from)
        .collect();
    let ran = run_with(&x, Some("p-nothing"), true, args);
    assert_eq!(ran.code, 0);
    assert!(ran.x.calls().is_empty());
}

#[test]
fn bib_engine_rejects_unknown_values_with_a_usage_error() {
    let ran = run(None, &["compile", "--bib-engine", "pandoc", "doc.tex"]);
    assert_eq!(ran.code, 2);
    let ran = run(None, &["compile", "--tool", "nope=/x", "doc.tex"]);
    assert_eq!(ran.code, 2);
    let ran = run(None, &["compile", "--tool", "bibtex", "doc.tex"]);
    assert_eq!(ran.code, 2);
}
