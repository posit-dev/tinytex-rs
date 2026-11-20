//! The JSON contract: `schema`, the event `type` tag, path handling, and the
//! shapes of `compile`, `detect`, `tlmgr` and `log` output.
#![cfg(feature = "cli")]

use std::ffi::OsString;
use std::sync::Arc;

use serde::Deserialize;
use tinytex::detect::TlVersion;
use tinytex::test_support::{FakeTex, Reply};
use tinytex::{Compiled, Distribution, InstallOutcome, UpdateOutcome};

use super::cli_support::{run, run_with};
use super::compile_support::{CLEAN_LOG, RERUN_LOG, doc, ok};

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

#[test]
fn compile_json_is_the_compiled_type_with_a_schema_field() {
    let d = doc();
    let x = Arc::new(FakeTex::new());
    x.on("pdflatex", &[], vec![ok(RERUN_LOG), ok(CLEAN_LOG)]);
    let ran = run_with(
        &x,
        Some("cj-compiled"),
        false,
        os(&["compile", "--json", d.tex.to_str().unwrap()]),
    );
    assert_eq!(ran.code, 0, "{}", ran.err);
    let v = ran.json();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["runs"], 2);
    assert_eq!(v["installed_packages"], serde_json::json!([]));
    let compiled: Compiled = serde_json::from_str(&ran.out).unwrap();
    assert_eq!(compiled.pdf, d.path("doc.pdf"));
    assert_eq!(v["pdf"], d.path("doc.pdf").to_str().unwrap());
}

#[test]
fn plain_compile_prints_the_output_path_on_stdout() {
    let d = doc();
    let x = Arc::new(FakeTex::new());
    x.on("pdflatex", &[], vec![ok(CLEAN_LOG)]);
    let ran = run_with(
        &x,
        Some("cj-plain"),
        false,
        os(&["compile", d.tex.to_str().unwrap()]),
    );
    assert_eq!(ran.code, 0, "{}", ran.err);
    assert_eq!(ran.out.trim(), d.path("doc.pdf").to_str().unwrap());
}

#[derive(Deserialize)]
struct Line {
    schema: u32,
    #[serde(rename = "type")]
    ty: String,
}

#[test]
fn events_json_lines_have_schema_and_a_type_tag() {
    let d = doc();
    let x = Arc::new(FakeTex::new());
    x.on("pdflatex", &[], vec![ok(RERUN_LOG), ok(CLEAN_LOG)]);
    let ran = run_with(
        &x,
        Some("cj-events"),
        false,
        os(&["compile", "--events", "json", d.tex.to_str().unwrap()]),
    );
    assert_eq!(ran.code, 0, "{}", ran.err);
    let lines: Vec<&str> = ran.err.lines().filter(|l| l.starts_with('{')).collect();
    assert!(lines.len() >= 2, "{}", ran.err);
    for l in &lines {
        let line: Line = serde_json::from_str(l).unwrap();
        assert_eq!(line.schema, 1);
        assert!(!line.ty.is_empty());
    }
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["type"], "running_engine");
    assert_eq!(first["run"], 1);
    assert!(first.get("event").is_none(), "the library tag is renamed");
}

#[test]
fn a_consumer_ignoring_unknown_types_and_fields_still_parses() {
    let added = r#"{"schema":1,"type":"a_future_event","extra":[1,2]}"#;
    let line: Line = serde_json::from_str(added).unwrap();
    assert_eq!((line.schema, line.ty.as_str()), (1, "a_future_event"));
    let known = r#"{"schema":1,"type":"running_engine","run":1,"added_later":true}"#;
    let line: Line = serde_json::from_str(known).unwrap();
    assert_eq!(line.ty, "running_engine");
}

#[test]
fn text_events_go_to_stderr_not_stdout() {
    let d = doc();
    let x = Arc::new(FakeTex::new());
    x.on("pdflatex", &[], vec![ok(CLEAN_LOG)]);
    let ran = run_with(
        &x,
        Some("cj-text-events"),
        false,
        os(&["compile", "--json", d.tex.to_str().unwrap()]),
    );
    assert!(ran.err.contains("running the TeX engine"), "{}", ran.err);
    assert!(!ran.out.contains("running the TeX engine"));
}

#[test]
fn paths_keep_their_separators() {
    let ran = run(
        None,
        &[
            "compile",
            "--print-request",
            "--output-dir",
            "sub/dir",
            "d.tex",
        ],
    );
    assert_eq!(ran.json()["request"]["output_dir"], "sub/dir");
}

#[cfg(unix)]
#[test]
fn a_non_utf8_path_serializes_lossily() {
    use std::os::unix::ffi::OsStringExt;
    let mut bad = b"bad".to_vec();
    bad.push(0xff);
    bad.extend_from_slice(b"dir");
    let args: Vec<OsString> = vec![
        "compile".into(),
        "--print-request".into(),
        "--output-dir".into(),
        OsString::from_vec(bad),
        "d.tex".into(),
    ];
    let x = Arc::new(FakeTex::new());
    let ran = run_with(&x, None, false, args);
    assert_eq!(ran.code, 0, "{}", ran.err);
    assert_eq!(ran.json()["request"]["output_dir"], "bad\u{FFFD}dir");
}

#[test]
fn detect_json_is_the_distribution_type_with_a_schema_field() {
    let ran = run(Some("cj-detect"), &["detect", "--json"]);
    assert_eq!(ran.code, 0, "{}", ran.err);
    let v = ran.json();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["kind"], "tiny_te_x");
    let d: Distribution = serde_json::from_str(&ran.out).unwrap();
    assert!(d.bin_dir.ends_with("x86_64-linux"));
}

#[test]
fn plain_detect_names_the_kind_and_bin_dir() {
    let ran = run(Some("cj-detect-plain"), &["detect"]);
    assert_eq!(ran.code, 0, "{}", ran.err);
    assert!(ran.out.contains("x86_64-linux"), "{}", ran.out);
}

fn tlmgr_run(name: &str, x: &Arc<FakeTex>, args: &[&str]) -> super::cli_support::Ran {
    let mut all = vec!["tlmgr"];
    all.extend_from_slice(args);
    all.push("--json");
    run_with(x, Some(name), false, os(&all))
}

#[test]
fn tlmgr_version_json_is_tl_version() {
    let x = Arc::new(FakeTex::new());
    x.on(
        "tlmgr",
        &["--version"],
        vec![Reply::ok().stdout(
            "tlmgr revision 76773 (2025-11-06 20:43:29 +0100)\nTeX Live (https://tug.org/texlive) version 2025\n",
        )],
    );
    let ran = tlmgr_run("cj-version", &x, &["version"]);
    assert_eq!(ran.code, 0, "{}", ran.err);
    assert_eq!(ran.json()["schema"], 1);
    let v: TlVersion = serde_json::from_str(&ran.out).unwrap();
    assert_eq!(v.year, 2025);
}

#[test]
fn tlmgr_install_json_is_install_outcome() {
    let x = Arc::new(FakeTex::new());
    x.on("tlmgr", &["info"], vec![Reply::ok()]);
    let ran = tlmgr_run("cj-install", &x, &["install", "nosuchpkg"]);
    assert_eq!(ran.json()["schema"], 1, "{}", ran.err);
    let _: InstallOutcome = serde_json::from_str(&ran.out).unwrap();
}

#[test]
fn tlmgr_update_json_is_update_outcome() {
    let x = Arc::new(FakeTex::new());
    let ran = run_with(
        &x,
        Some("cj-update"),
        false,
        os(&["tlmgr", "--tlmgr-update", "never", "update", "--json"]),
    );
    assert_eq!(ran.code, 0, "{}", ran.err);
    assert_eq!(ran.json()["schema"], 1);
    let o: UpdateOutcome = serde_json::from_str(&ran.out).unwrap();
    assert_eq!(o, UpdateOutcome::Skipped);
}

#[test]
fn tlmgr_run_passes_arguments_after_the_double_dash() {
    let x = Arc::new(FakeTex::new());
    x.on("tlmgr", &["conf"], vec![Reply::ok().stdout("hello\n")]);
    let ran = run_with(
        &x,
        Some("cj-run"),
        false,
        os(&["tlmgr", "run", "--json", "--", "conf", "texmf"]),
    );
    assert_eq!(ran.code, 0, "{}", ran.err);
    let v = ran.json();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["stdout"], "hello\n");
    assert!(x.lines().iter().any(|l| l == "tlmgr conf texmf"));
}

#[test]
fn log_json_reports_the_diagnostics_of_a_log() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("a.log");
    std::fs::write(
        &log,
        "! Undefined control sequence.\nl.1 \\foo\n\n! LaTeX Error: File `pgfplots.sty' not found.\n\n",
    )
    .unwrap();
    let ran = run(None, &["log", "--json", log.to_str().unwrap()]);
    assert_eq!(ran.code, 0, "{}", ran.err);
    let v = ran.json();
    assert_eq!(v["schema"], 1);
    assert!(
        v["error"]["summary"]
            .as_str()
            .unwrap()
            .contains("Undefined control sequence")
    );
    assert_eq!(v["missing_packages"][0]["file"], "/pgfplots.sty");
}

#[test]
fn plain_log_prints_the_error_summary() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("a.log");
    std::fs::write(&log, "! Undefined control sequence.\nl.1 \\foo\n\n").unwrap();
    let ran = run(None, &["log", log.to_str().unwrap()]);
    assert_eq!(ran.code, 0);
    assert!(
        ran.out.contains("Undefined control sequence"),
        "{}",
        ran.out
    );
}

#[test]
fn log_of_a_missing_file_is_an_environment_error() {
    let ran = run(None, &["log", "--json", "/nonexistent/none.log"]);
    assert_eq!(ran.code, 3);
    assert_eq!(ran.json()["code"], "io");
}
