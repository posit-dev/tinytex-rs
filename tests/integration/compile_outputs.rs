//! Cleanup and log retention, the stale files, output placement.

use std::io;

use tinytex::test_support::{Reply, move_file_with};
use tinytex::{AUX_EXTENSIONS, Clean, CompileError, CompileRequest, KeepLog, aux_files};

use super::compile_support::*;
use super::tlmgr_model::model;

const WARN_LOG: &str =
    "This is pdfTeX\nLaTeX Warning: Reference `a' on page 1 undefined on input line 3.\n";
const ERR_LOG: &str = "This is pdfTeX\n! Undefined control sequence.\nl.3 \\foo\n\n";

fn build(files: &[(&str, &str)]) -> Reply {
    ok_with(CLEAN_LOG, files)
}

fn failing() -> Reply {
    Reply::code(1)
        .creates("doc.log", ERR_LOG)
        .creates("doc.aux", "a")
        .creates("doc.toc", "t")
}

fn req(clean: Clean, keep: KeepLog) -> CompileRequest {
    CompileRequest::new().clean(clean).keep_log(keep)
}

fn engine(reply: Reply) -> tinytex::test_support::FakeTex {
    let x = tex(&model(&[], &[], &[]));
    x.on("pdflatex", &[], vec![reply]);
    x
}

// ---------- the extension list ----------

#[test]
fn the_extension_list_is_the_union_of_both_upstreams() {
    assert_eq!(AUX_EXTENSIONS.len(), 21);
    for ext in [
        "log", "idx", "aux", "bcf", "blg", "bbl", "fls", "out", "lof", "lot", "toc", "nav", "snm",
        "vrb", "ilg", "ind", "xwm", "brf", "run.xml",
    ] {
        assert!(AUX_EXTENSIONS.contains(&ext), "{ext}");
    }
    assert!(AUX_EXTENSIONS.contains(&"preview.aux"));
    assert!(AUX_EXTENSIONS.contains(&"missfont.log"));
    let files = aux_files("doc");
    assert_eq!(files.len(), 21);
    assert!(files.contains(&"doc.run.xml".to_owned()));
    assert!(files.contains(&"preview.aux".to_owned()));
    assert!(files.contains(&"missfont.log".to_owned()));
}

// ---------- Clean ----------

#[test]
fn always_removes_the_files_the_run_created_after_a_success() {
    let d = doc();
    let x = engine(build(&[("doc.aux", "a"), ("doc.toc", "t")]));
    let c = run("co-always", &d, &req(Clean::Always, KeepLog::Never), &x).unwrap();
    assert!(!d.exists("doc.aux") && !d.exists("doc.toc") && !d.exists("doc.log"));
    assert!(d.exists("doc.pdf"));
    assert_eq!(c.log, None);
}

#[test]
fn always_cleans_after_a_failure_and_keeps_the_log_for_on_warnings() {
    let d = doc();
    let x = engine(failing());
    let err = run(
        "co-always-fail",
        &d,
        &req(Clean::Always, KeepLog::OnWarnings),
        &x,
    )
    .unwrap_err();
    assert!(!d.exists("doc.aux") && !d.exists("doc.toc"));
    assert!(d.exists("doc.log"));
    match err {
        CompileError::Latex { log, .. } => assert_eq!(log, Some(d.path("doc.log"))),
        other => panic!("{other:?}"),
    }
}

#[test]
fn on_success_cleans_a_success_and_leaves_a_failure_alone() {
    let d = doc();
    let x = engine(build(&[("doc.aux", "a")]));
    run(
        "co-os-ok",
        &d,
        &req(Clean::OnSuccess, KeepLog::OnFailure),
        &x,
    )
    .unwrap();
    assert!(
        !d.exists("doc.aux") && !d.exists("doc.log"),
        "Quarto 1 parity"
    );

    let d = doc();
    let x = engine(failing());
    run(
        "co-os-fail",
        &d,
        &req(Clean::OnSuccess, KeepLog::OnFailure),
        &x,
    )
    .unwrap_err();
    assert!(d.exists("doc.aux") && d.exists("doc.toc") && d.exists("doc.log"));
}

#[test]
fn never_removes_nothing_whatever_keep_log_says() {
    let d = doc();
    let x = engine(build(&[("doc.aux", "a")]));
    let c = run("co-never", &d, &req(Clean::Never, KeepLog::Never), &x).unwrap();
    assert!(d.exists("doc.aux") && d.exists("doc.log"));
    assert_eq!(c.log, Some(d.path("doc.log")));
}

#[test]
fn keep_log_decides_whether_the_log_is_exempt() {
    // (keep, outcome is a failure, log text, log survives)
    let cases = [
        (KeepLog::OnWarnings, false, CLEAN_LOG, false),
        (KeepLog::OnWarnings, false, WARN_LOG, true),
        (KeepLog::OnWarnings, true, ERR_LOG, true),
        (KeepLog::OnFailure, false, WARN_LOG, false),
        (KeepLog::OnFailure, true, ERR_LOG, true),
        (KeepLog::Always, false, CLEAN_LOG, true),
        (KeepLog::Always, true, ERR_LOG, true),
        (KeepLog::Never, false, WARN_LOG, false),
        (KeepLog::Never, true, ERR_LOG, false),
    ];
    for (i, (keep, fail, log, survives)) in cases.into_iter().enumerate() {
        let d = doc();
        let reply = if fail {
            Reply::code(1).creates("doc.log", log)
        } else {
            ok(log)
        };
        let x = engine(reply);
        let result = run(&format!("co-keep-{i}"), &d, &req(Clean::Always, keep), &x);
        assert_eq!(d.exists("doc.log"), survives, "{keep:?} fail={fail}");
        match result {
            Ok(c) => assert_eq!(c.log.is_some(), survives, "{keep:?}"),
            Err(CompileError::Latex { log, .. }) => {
                assert_eq!(
                    log.is_some(),
                    survives,
                    "{keep:?}: the error names no deleted log"
                )
            }
            Err(other) => panic!("{other:?}"),
        }
    }
}

#[test]
fn a_cancelled_run_cleans_like_a_failed_one() {
    let cancelled = || {
        Reply::code(1)
            .creates("doc.log", "partial")
            .creates("doc.aux", "a")
            .fails(io::ErrorKind::Interrupted)
            .interrupts()
    };
    let d = doc();
    let x = engine(cancelled());
    let err = run(
        "co-cancel-always",
        &d,
        &req(Clean::Always, KeepLog::OnWarnings),
        &x,
    )
    .unwrap_err();
    assert!(matches!(err, CompileError::Cancelled), "{err:?}");
    assert!(!d.exists("doc.aux"));
    assert!(
        d.exists("doc.log"),
        "a failure keeps the log for OnWarnings"
    );

    let d = doc();
    let x = engine(cancelled());
    run(
        "co-cancel-os",
        &d,
        &req(Clean::OnSuccess, KeepLog::OnWarnings),
        &x,
    )
    .unwrap_err();
    assert!(d.exists("doc.aux"));
}

#[test]
fn a_file_that_existed_before_the_run_is_never_removed() {
    let d = doc();
    d.write("doc.toc", "mine");
    d.write("preview.aux", "mine");
    let x = engine(build(&[("doc.aux", "a"), ("doc.toc", "rebuilt")]));
    run("co-pre", &d, &req(Clean::Always, KeepLog::Never), &x).unwrap();
    assert!(d.exists("doc.toc") && d.exists("preview.aux"));
    assert!(!d.exists("doc.aux"));
}

#[test]
fn only_listed_files_are_removed() {
    let d = doc();
    let x = engine(build(&[
        ("doc.aux", "a"),
        ("doc.other", "o"),
        ("figure.png", "p"),
    ]));
    run("co-listed", &d, &req(Clean::Always, KeepLog::Never), &x).unwrap();
    assert!(d.exists("doc.other") && d.exists("figure.png"));
}

#[test]
fn a_run_created_missfont_log_is_cleaned() {
    let d = doc();
    let x = engine(build(&[("missfont.log", "mktexpk")]));
    run("co-missfont", &d, &req(Clean::Always, KeepLog::Never), &x).unwrap();
    assert!(!d.exists("missfont.log"));
}

#[test]
fn an_invalid_request_deletes_nothing() {
    let d = doc();
    d.write("doc.log", "old");
    d.write("doc.pdf", "old");
    let x = engine(build(&[]));
    let bad = CompileRequest::new()
        .driver(tinytex::Driver::Latexmk)
        .runs(2, 5);
    run("co-invalid", &d, &bad, &x).unwrap_err();
    assert!(d.exists("doc.log") && d.exists("doc.pdf"));
}

// ---------- stale files ----------

#[test]
fn the_stale_log_and_output_are_removed_before_the_first_run() {
    let d = doc();
    d.write("doc.log", "old log");
    d.write("doc.pdf", "old pdf");
    // An engine that exits 0 and writes nothing must fail, not pass on the
    // previous build's file.
    let x = engine(Reply::ok());
    let err = run("co-stale", &d, &CompileRequest::new(), &x).unwrap_err();
    assert!(matches!(err, CompileError::NoOutput), "{err:?}");
    assert!(!d.exists("doc.pdf") && !d.exists("doc.log"));
}

#[test]
fn the_stale_output_in_output_dir_is_removed() {
    let d = doc();
    std::fs::create_dir(d.path("out")).unwrap();
    d.write("out/doc.pdf", "old");
    d.write("out/doc.log", "old");
    let x = engine(Reply::ok());
    let err = run(
        "co-stale-out",
        &d,
        &CompileRequest::new().output_dir("out".into()),
        &x,
    )
    .unwrap_err();
    assert!(matches!(err, CompileError::NoOutput), "{err:?}");
    assert!(!d.exists("out/doc.pdf"));
}

// ---------- output placement ----------

#[test]
fn output_dir_is_created_relative_to_the_tex_directory_and_passed_on() {
    let d = doc();
    let x = engine(
        Reply::ok()
            .creates("build/doc.log", CLEAN_LOG)
            .creates("build/doc.pdf", "%PDF"),
    );
    let req = CompileRequest::new()
        .output_dir("build".into())
        .clean(Clean::Never);
    let c = run("op-dir", &d, &req, &x).unwrap();
    assert!(d.path("build").is_dir());
    assert!(engine_lines(&x)[0].contains("-output-directory=build"));
    assert_eq!(c.pdf, d.path("build/doc.pdf"));
    assert_eq!(c.log, Some(d.path("build/doc.log")));
    assert_eq!(c.output_kind, tinytex::Output::Pdf);
}

#[test]
fn pdf_file_renames_the_output() {
    let d = doc();
    let x = engine(build(&[]));
    let req = CompileRequest::new().pdf_file("final.pdf".into());
    let c = run("op-rename", &d, &req, &x).unwrap();
    assert_eq!(c.pdf, d.path("final.pdf"));
    assert!(d.exists("final.pdf") && !d.exists("doc.pdf"));
    assert_eq!(
        std::fs::read_to_string(d.path("final.pdf")).unwrap(),
        "%PDF-1.5"
    );
}

#[test]
fn pdf_file_replaces_an_existing_file() {
    let d = doc();
    d.write("final.pdf", "previous build");
    let x = engine(build(&[]));
    let req = CompileRequest::new().pdf_file("final.pdf".into());
    run("op-replace", &d, &req, &x).unwrap();
    assert_eq!(
        std::fs::read_to_string(d.path("final.pdf")).unwrap(),
        "%PDF-1.5"
    );
}

#[test]
fn a_failing_rename_falls_back_to_copy_and_remove() {
    let d = doc();
    d.write("from.pdf", "bytes");
    let fail =
        |_: &std::path::Path, _: &std::path::Path| Err(io::Error::other("cross-device link"));
    move_file_with(&d.path("from.pdf"), &d.path("to.pdf"), &fail).unwrap();
    assert_eq!(std::fs::read_to_string(d.path("to.pdf")).unwrap(), "bytes");
    assert!(!d.exists("from.pdf"));

    // A source that cannot be copied is an error, and nothing is removed.
    let e = move_file_with(&d.path("missing.pdf"), &d.path("x.pdf"), &fail).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::NotFound);
}
