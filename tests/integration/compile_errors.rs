//! The shapes of `CompileError`, cancellation, and `Diagnostics`.

use std::io;

use tinytex::log::{Hint, InstallFailure, SearchTerm};
use tinytex::test_support::Reply;
use tinytex::{CompileError, CompileRequest, Compiler, Config, NullObserver, TlmgrError};

use super::compile_support::*;
use super::tlmgr_model::model;

// ---------- error shapes ----------

#[test]
fn a_latex_error_carries_the_missing_terms_whether_or_not_auto_install_is_on() {
    for (name, auto) in [("er-miss-off", false), ("er-miss-on", true)] {
        let d = doc();
        let x = tex(&model(&[], &[], &[]));
        x.on("pdflatex", &[], vec![fails(&missing_sty("pgfplots"))]);
        // With auto-install on and a search that finds nothing, NoMatch ends it.
        let err = run(name, &d, &CompileRequest::new().auto_install(auto), &x).unwrap_err();
        match (auto, err) {
            (
                false,
                CompileError::Latex {
                    missing,
                    summary,
                    log,
                    exit_code,
                    hints,
                    ..
                },
            ) => {
                assert_eq!(missing, [SearchTerm::File("/pgfplots.sty".into())]);
                assert!(summary.unwrap().contains("pgfplots.sty"));
                assert_eq!(log, Some(d.path("doc.log")));
                assert_eq!(exit_code, Some(1));
                assert!(hints.is_empty());
            }
            (true, CompileError::PackageInstall { reason, .. }) => {
                assert_eq!(reason, InstallFailure::NoMatch);
            }
            (_, other) => panic!("{other:?}"),
        }
    }
}

#[test]
fn hints_and_streams_are_carried() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.on(
        "pdflatex",
        &[],
        vec![
            Reply::code(1)
                .creates(
                    "doc.log",
                    "! Missing $ inserted.\n<inserted text> \n                $\n\n",
                )
                .stdout("out")
                .stderr("err"),
        ],
    );
    match run("er-hint", &d, &CompileRequest::new(), &x).unwrap_err() {
        CompileError::Latex {
            hints,
            stdout,
            stderr,
            ..
        } => {
            assert_eq!(hints, [Hint::InlineMath]);
            assert_eq!((stdout.as_str(), stderr.as_str()), ("out", "err"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_failed_install_is_install_failed_with_the_tlmgr_error() {
    // `tlmgr info` itself fails, so `install` returns an error.
    let d = doc();
    let x = tex(&model(&[], &["pgfplots"], &[]));
    x.on("pdflatex", &[], vec![fails(&missing_sty("pgfplots"))]);
    search_answer(&x, "pgfplots.sty", "pgfplots");
    x.reply("tlmgr", &["info"], Reply::code(1).stderr("tlmgr: boom"));
    let err = run(
        "er-instfail",
        &d,
        &CompileRequest::new().auto_install(true),
        &x,
    )
    .unwrap_err();
    match err {
        CompileError::PackageInstall { reason, source } => {
            assert_eq!(reason, InstallFailure::InstallFailed);
            assert!(matches!(source, TlmgrError::Command { .. }), "{source:?}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn no_match_when_the_search_finds_nothing() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.on("pdflatex", &[], vec![fails(&missing_sty("nopkg"))]);
    x.reply("tlmgr", &["search"], Reply::ok().stdout(""));
    match run(
        "er-nomatch",
        &d,
        &CompileRequest::new().auto_install(true),
        &x,
    )
    .unwrap_err()
    {
        CompileError::PackageInstall { reason, .. } => assert_eq!(reason, InstallFailure::NoMatch),
        other => panic!("{other:?}"),
    }
}

#[test]
fn texlive_too_old_is_its_own_reason() {
    let d = doc();
    let x = tex(&model(&[], &["pgfplots"], &[(1, false)]));
    x.on("pdflatex", &[], vec![fails(&missing_sty("pgfplots"))]);
    search_answer(&x, "pgfplots.sty", "pgfplots");
    x.reply(
        "tlmgr",
        &["update"],
        Reply::code(1)
            .stderr("tlmgr: Local TeX Live (2024) is older than remote repository (2025).\n"),
    );
    match run("er-old", &d, &CompileRequest::new().auto_install(true), &x).unwrap_err() {
        CompileError::PackageInstall { reason, source } => {
            assert_eq!(reason, InstallFailure::TexLiveTooOld);
            assert!(matches!(
                source,
                TlmgrError::TexLiveTooOld {
                    local: 2024,
                    remote: 2025
                }
            ));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn no_output_when_the_engine_exits_zero_and_writes_no_file() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.on(
        "pdflatex",
        &[],
        vec![Reply::ok().creates("doc.log", "This is pdfTeX\n")],
    );
    let err = run("er-noout", &d, &CompileRequest::new(), &x).unwrap_err();
    assert!(matches!(err, CompileError::NoOutput), "{err:?}");
}

#[test]
fn a_missing_package_eaten_by_a_template_is_recovered_when_auto_install_is_on() {
    // R's PNAS case: exit 0, no output, the log names a missing package.
    let d = doc();
    let x = tex(&model(&[], &["pgfplots"], &[]));
    x.on(
        "pdflatex",
        &[],
        vec![
            Reply::ok().creates("doc.log", missing_sty("pgfplots")),
            ok(CLEAN_LOG),
        ],
    );
    search_answer(&x, "pgfplots.sty", "pgfplots");
    let c = run("er-pnas", &d, &CompileRequest::new().auto_install(true), &x).unwrap();
    assert_eq!(c.runs, 2);
}

#[test]
fn a_missing_engine_is_engine_not_found_or_no_installation() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.on(
        "pdflatex",
        &[],
        vec![Reply::ok().fails(io::ErrorKind::NotFound)],
    );
    match run("er-nf", &d, &CompileRequest::new(), &x).unwrap_err() {
        CompileError::EngineNotFound { tool } => assert_eq!(tool, "pdflatex"),
        other => panic!("{other:?}"),
    }
    let err = Compiler::new(None, Config::default())
        .compile(&d.tex, &CompileRequest::new(), &x, &NullObserver)
        .unwrap_err();
    assert!(matches!(err, CompileError::NoTexInstallation), "{err:?}");
}

#[test]
fn any_other_io_failure_is_io() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.on(
        "pdflatex",
        &[],
        vec![Reply::ok().fails(io::ErrorKind::PermissionDenied)],
    );
    let err = run("er-io", &d, &CompileRequest::new(), &x).unwrap_err();
    assert!(matches!(err, CompileError::Io(_)), "{err:?}");
}

// ---------- cancellation ----------

#[test]
fn a_killed_engine_is_cancelled_not_io() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.on(
        "pdflatex",
        &[],
        vec![Reply::ok().fails(io::ErrorKind::Other).interrupts()],
    );
    let err = run("ca-kill", &d, &CompileRequest::new(), &x).unwrap_err();
    assert!(matches!(err, CompileError::Cancelled), "{err:?}");
}

#[test]
fn a_failed_run_after_cancellation_is_cancelled_not_latex_or_install() {
    let d = doc();
    let x = tex(&model(&[], &["pgfplots"], &[]));
    x.on(
        "pdflatex",
        &[],
        vec![fails(&missing_sty("pgfplots")).interrupts()],
    );
    search_answer(&x, "pgfplots.sty", "pgfplots");
    let err = run("ca-fail", &d, &CompileRequest::new().auto_install(true), &x).unwrap_err();
    assert!(matches!(err, CompileError::Cancelled), "{err:?}");
    assert_eq!(x.count("tlmgr"), 0);
}

#[test]
fn a_completed_install_followed_by_cancellation_stops_before_the_next_run() {
    let d = doc();
    let x = tex(&model(&[], &["pgfplots"], &[]));
    x.on(
        "pdflatex",
        &[],
        vec![fails(&missing_sty("pgfplots")), ok(CLEAN_LOG)],
    );
    search_answer(&x, "pgfplots.sty", "pgfplots");
    x.reply("tlmgr", &["install"], Reply::ok().interrupts());
    let err = run("ca-inst", &d, &CompileRequest::new().auto_install(true), &x).unwrap_err();
    assert!(matches!(err, CompileError::Cancelled), "{err:?}");
    assert_eq!(x.count("pdflatex"), 1);
}

#[test]
fn a_killed_bibliography_tool_or_indexer_is_cancelled() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.on(
        "pdflatex",
        &[],
        vec![ok_with(
            CLEAN_LOG,
            &[("doc.aux", "\\citation{a}\n\\bibdata{r}\n\\bibstyle{p}\n")],
        )],
    );
    x.reply(
        "bibtex",
        &[],
        Reply::ok().fails(io::ErrorKind::Other).interrupts(),
    );
    let err = run("ca-bib", &d, &CompileRequest::new(), &x).unwrap_err();
    assert!(matches!(err, CompileError::Cancelled), "{err:?}");

    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.on(
        "pdflatex",
        &[],
        vec![ok_with(CLEAN_LOG, &[("doc.idx", "x")])],
    );
    x.reply("makeindex", &[], Reply::code(1).interrupts());
    let err = run("ca-idx", &d, &CompileRequest::new(), &x).unwrap_err();
    assert!(matches!(err, CompileError::Cancelled), "{err:?}");
}

// ---------- diagnostics ----------

#[test]
fn diagnostics_carry_warnings_and_accessibility() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    let log = "LaTeX Warning: Reference `x' on page 1 undefined on input line 3.\n\nPackage tagpdf Warning: Alternative text for graphic is missing.\n(tagpdf)                Using 'fig.png' instead.\n\nLaTeX DocumentMetadata Warning: The language has not been set in\n";
    x.on("pdflatex", &[], vec![ok(log)]);
    let c = run("di-warn", &d, &CompileRequest::new(), &x).unwrap();
    assert!(
        c.diagnostics
            .warnings
            .iter()
            .any(|w| w.contains("Reference"))
    );
    assert_eq!(c.diagnostics.accessibility.missing_alt_text, ["fig.png"]);
    assert!(c.diagnostics.accessibility.missing_language);
    assert!(!c.diagnostics.max_runs_reached);
    assert_eq!(c.diagnostics.bibliography_failed, None);
}

#[test]
fn a_failed_fmtutil_from_an_install_retry_is_a_notice() {
    let d = doc();
    // install fails once, `update --self` runs, `fmtutil` fails, install works.
    let x = tex(&model(&[], &["pgfplots"], &[(1, false), (0, true)]));
    x.on(
        "pdflatex",
        &[],
        vec![fails(&missing_sty("pgfplots")), ok(CLEAN_LOG)],
    );
    search_answer(&x, "pgfplots.sty", "pgfplots");
    x.reply("fmtutil", &[], Reply::code(1).stderr("fmtutil: boom"));
    let c = run(
        "di-notice",
        &d,
        &CompileRequest::new().auto_install(true),
        &x,
    )
    .unwrap();
    assert!(
        c.diagnostics
            .notices
            .iter()
            .any(|n| n.contains("format tree")),
        "{:?}",
        c.diagnostics.notices
    );
}
