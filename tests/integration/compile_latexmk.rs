//! `Driver::Latexmk`: one delegated command, no recovery, no auto-install.

use tinytex::test_support::Reply;
use tinytex::{Bib, CompileError, CompileRequest, Driver, Engine, IndexEngine};

use super::compile_support::*;
use super::tlmgr_model::model;

fn latexmk_req() -> CompileRequest {
    CompileRequest::new().driver(Driver::Latexmk)
}

#[test]
fn the_one_recorded_command_is_rs_latexmk_form() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.reply("latexmk", &[], ok(CLEAN_LOG));
    let req = latexmk_req()
        .engine(Engine::XeLatex)
        .engine_args(vec!["-shell-escape".into()]);
    let c = run("lm-form", &d, &req, &x).unwrap();
    assert_eq!(
        x.lines(),
        [
            "latexmk -latexoption=-halt-on-error -interaction=batchmode -pdf -pdflatex=xelatex -shell-escape doc.tex"
        ]
    );
    assert_eq!(x.calls()[0].cwd.as_deref(), Some(d.dir.path()));
    assert_eq!(c.runs, 1);
    assert_eq!(c.pdf, d.path("doc.pdf"));
}

#[test]
fn auto_install_is_ignored_with_a_notice_and_nothing_is_installed() {
    let d = doc();
    let x = tex(&model(&[], &["pgfplots"], &[]));
    x.reply("latexmk", &[], fails(&missing_sty("pgfplots")));
    let err = run("lm-noinst", &d, &latexmk_req().auto_install(true), &x).unwrap_err();
    assert!(matches!(err, CompileError::Latex { .. }));
    assert_eq!(x.count("tlmgr"), 0);

    let x = tex(&model(&[], &[], &[]));
    x.reply("latexmk", &[], ok(CLEAN_LOG));
    let d = doc();
    let c = run("lm-notice", &d, &latexmk_req().auto_install(true), &x).unwrap();
    assert!(
        c.diagnostics
            .notices
            .iter()
            .any(|n| n.contains("auto_install")),
        "{:?}",
        c.diagnostics.notices
    );
}

#[test]
fn run_counts_a_bibliography_or_an_index_are_rejected_before_any_command() {
    let bad = [
        latexmk_req().runs(2, 10),
        latexmk_req().runs(1, 5),
        latexmk_req().bib(Bib::Bibtex),
        latexmk_req().index(IndexEngine {
            command: "makeindex".into(),
            args: vec![],
        }),
    ];
    for (i, req) in bad.iter().enumerate() {
        let d = doc();
        let x = tex(&model(&[], &[], &[]));
        let err = run(&format!("lm-bad{i}"), &d, req, &x).unwrap_err();
        assert!(
            matches!(err, CompileError::InvalidRequest { .. }),
            "{err:?}"
        );
        assert!(x.calls().is_empty());
    }
}

#[test]
fn a_failing_exit_is_a_latex_error_with_the_summary_from_the_log() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.reply(
        "latexmk",
        &[],
        Reply::code(12)
            .creates("doc.log", "! Undefined control sequence.\nl.3 \\foo\n\n")
            .stderr("boom"),
    );
    match run("lm-fail", &d, &latexmk_req(), &x).unwrap_err() {
        CompileError::Latex {
            summary,
            exit_code,
            stderr,
            log,
            ..
        } => {
            assert!(summary.unwrap().contains("Undefined control sequence"));
            assert_eq!(exit_code, Some(12));
            assert_eq!(stderr, "boom");
            assert_eq!(log, Some(d.path("doc.log")));
        }
        other => panic!("{other:?}"),
    }
}
