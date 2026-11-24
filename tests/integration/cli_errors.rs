//! The error table of the design note (section 5.1): codes and exit codes.
#![cfg(feature = "cli")]
#![allow(clippy::result_large_err)]

use std::ffi::OsString;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tinytex::cli::{compile_error_status, tlmgr_error_status};
use tinytex::log::InstallFailure;
use tinytex::test_support::{FakeTex, Reply};
use tinytex::{CompileError, TlmgrError};

use super::cli_support::{Ran, host, run, run_on, run_with};
use super::compile_support::{CLEAN_LOG, doc, fails, ok};

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn command_failed() -> TlmgrError {
    TlmgrError::Command {
        program: "tlmgr".into(),
        args: vec!["x".into()],
        code: Some(1),
        stderr: String::new(),
    }
}

#[test]
fn each_compile_error_variant_has_its_code_and_exit_code() {
    let cases: Vec<(CompileError, &str, i32)> = vec![
        (CompileError::NoTexInstallation, "no_tex_installation", 3),
        (
            CompileError::EngineNotFound {
                tool: "pdflatex".into(),
            },
            "engine_not_found",
            3,
        ),
        (
            CompileError::Latex {
                summary: None,
                hints: vec![],
                log: None,
                missing: vec![],
                stdout: String::new(),
                stderr: String::new(),
                exit_code: Some(1),
            },
            "latex_error",
            1,
        ),
        (
            CompileError::PackageInstall {
                reason: InstallFailure::NoMatch,
                source: command_failed(),
            },
            "package_install",
            1,
        ),
        (
            CompileError::PackageInstall {
                reason: InstallFailure::InstallFailed,
                source: command_failed(),
            },
            "package_install",
            1,
        ),
        (
            CompileError::PackageInstall {
                reason: InstallFailure::TexLiveTooOld,
                source: TlmgrError::TexLiveTooOld {
                    local: 2024,
                    remote: 2025,
                },
            },
            "texlive_too_old",
            5,
        ),
        (
            CompileError::Bibliography {
                engine: "bibtex".into(),
                log_excerpt: String::new(),
                log: None,
            },
            "bibliography",
            1,
        ),
        (
            CompileError::Index {
                log_excerpt: String::new(),
                log: None,
            },
            "index",
            1,
        ),
        (CompileError::NoOutput, "no_output", 1),
        (
            CompileError::InvalidRequest {
                message: "x".into(),
            },
            "invalid_request",
            2,
        ),
        (CompileError::Cancelled, "cancelled", 4),
        (CompileError::Io(io::Error::other("x")), "io", 3),
    ];
    for (err, code, exit) in cases {
        assert_eq!(compile_error_status(&err), (code, exit), "{err:?}");
    }
}

#[test]
fn a_tlmgr_error_is_code_tlmgr_with_exit_one() {
    assert_eq!(tlmgr_error_status(&command_failed()), ("tlmgr", 1));
    assert_eq!(
        tlmgr_error_status(&TlmgrError::Unparseable {
            what: "x".into(),
            output: String::new()
        }),
        ("tlmgr", 1)
    );
    assert_eq!(
        tlmgr_error_status(&TlmgrError::TexLiveTooOld {
            local: 1,
            remote: 2
        }),
        ("texlive_too_old", 5)
    );
    assert_eq!(
        tlmgr_error_status(&TlmgrError::Interrupted),
        ("cancelled", 4)
    );
}

fn assert_error_object(ran: &Ran, code: &str) {
    let v = ran.json();
    assert_eq!(v["schema"], 1, "{v}");
    assert_eq!(v["code"], code, "{v}");
    assert!(v["message"].as_str().is_some_and(|m| !m.is_empty()), "{v}");
}

#[test]
fn a_latex_failure_exits_1_with_a_latex_error_object() {
    let d = doc();
    let x = Arc::new(FakeTex::new());
    x.on(
        "pdflatex",
        &[],
        vec![fails(
            "! Undefined control sequence.\nl.1 \\undefinedcs\n\n",
        )],
    );
    let ran = run_with(
        &x,
        Some("cli-latex"),
        false,
        os(&["compile", "--json", d.tex.to_str().unwrap()]),
    );
    assert_eq!(ran.code, 1, "{}", ran.err);
    assert_error_object(&ran, "latex_error");
    assert!(
        ran.json()["details"]["summary"]
            .as_str()
            .unwrap()
            .contains("Undefined control sequence")
    );
}

#[test]
fn a_plain_latex_failure_prints_the_error_on_stderr_and_nothing_on_stdout() {
    let d = doc();
    let x = Arc::new(FakeTex::new());
    x.on(
        "pdflatex",
        &[],
        vec![fails("! Undefined control sequence.\n\n")],
    );
    let ran = run_with(
        &x,
        Some("cli-latex-plain"),
        false,
        os(&["compile", d.tex.to_str().unwrap()]),
    );
    assert_eq!(ran.code, 1);
    assert!(ran.out.is_empty(), "{}", ran.out);
    assert!(
        ran.err.contains("Undefined control sequence"),
        "{}",
        ran.err
    );
}

#[test]
fn an_invalid_request_exits_2() {
    let d = doc();
    let ran = run(
        Some("cli-invalid"),
        &[
            "compile",
            "--json",
            "--driver",
            "latexmk",
            "--min",
            "3",
            d.tex.to_str().unwrap(),
        ],
    );
    assert_eq!(ran.code, 2, "{}", ran.err);
    assert_error_object(&ran, "invalid_request");
}

#[test]
fn cancellation_exits_4() {
    let d = doc();
    let x = Arc::new(FakeTex::new());
    x.on("pdflatex", &[], vec![Reply::code(1).interrupts()]);
    let ran = run_with(
        &x,
        Some("cli-cancel"),
        false,
        os(&["compile", "--json", d.tex.to_str().unwrap()]),
    );
    assert_eq!(ran.code, 4, "{}", ran.err);
    assert_error_object(&ran, "cancelled");
}

#[test]
fn timeout_passes_a_whole_command_deadline_and_a_timed_out_run_exits_4() {
    let d = doc();
    let x = Arc::new(FakeTex::new());
    x.on(
        "pdflatex",
        &[],
        vec![Reply::ok().fails(io::ErrorKind::TimedOut)],
    );
    let asked = Arc::new(Mutex::new(None));
    let h = host(&x, Some("cli-timeout"), false, &asked);
    let ran = run_on(
        &h,
        &x,
        &asked,
        os(&[
            "compile",
            "--json",
            "--timeout",
            "1",
            d.tex.to_str().unwrap(),
        ]),
    );
    assert_eq!(ran.deadline, Some(Some(Duration::from_secs(1))));
    assert_eq!(ran.code, 4, "{}", ran.err);
    assert_error_object(&ran, "cancelled");
}

#[test]
fn without_timeout_the_exec_has_no_deadline() {
    let d = doc();
    let x = Arc::new(FakeTex::new());
    x.on("pdflatex", &[], vec![ok(CLEAN_LOG)]);
    let asked = Arc::new(Mutex::new(None));
    let h = host(&x, Some("cli-nodeadline"), false, &asked);
    let ran = run_on(&h, &x, &asked, os(&["compile", d.tex.to_str().unwrap()]));
    assert_eq!(ran.code, 0, "{}", ran.err);
    assert_eq!(ran.deadline, Some(None));
}

#[test]
fn detect_finding_nothing_exits_3() {
    let ran = run(None, &["detect", "--json"]);
    assert_eq!(ran.code, 3);
    assert_error_object(&ran, "no_tex_installation");
}

#[test]
fn tlmgr_without_an_installation_exits_3() {
    let ran = run(None, &["tlmgr", "version", "--json"]);
    assert_eq!(ran.code, 3);
    assert_error_object(&ran, "no_tex_installation");
}

#[test]
fn a_failing_tlmgr_exits_1_with_code_tlmgr() {
    let x = Arc::new(FakeTex::new());
    x.on("tlmgr", &["--version"], vec![Reply::code(1).stderr("boom")]);
    let ran = run_with(
        &x,
        Some("cli-tlmgr-fail"),
        false,
        os(&["tlmgr", "version", "--json"]),
    );
    assert_eq!(ran.code, 1, "{}", ran.err);
    assert_error_object(&ran, "tlmgr");
}

#[test]
fn usage_errors_exit_2_before_any_other_code() {
    for args in [
        vec!["compile"],
        vec!["compile", "--driver", "bogus", "doc.tex"],
        vec!["nonsense"],
        vec![],
    ] {
        let ran = run(Some("cli-usage"), &args);
        assert_eq!(ran.code, 2, "{args:?}: {}", ran.err);
        assert!(!ran.err.is_empty());
    }
}

#[test]
fn help_exits_0_on_stdout() {
    let ran = run(None, &["--help"]);
    assert_eq!(ran.code, 0);
    assert!(ran.out.contains("compile"), "{}", ran.out);
}
