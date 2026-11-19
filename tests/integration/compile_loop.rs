//! The rerun loop through `FakeTex`: run counts, package recovery, index,
//! bibliography, hyphenation and module reruns, working directory.

use tinytex::test_support::Reply;
use tinytex::{Bib, CompileError, CompileRequest, Event, IndexEngine, Recover};

use super::compile_support::*;
use super::tlmgr_model::{model, without_info};

fn pdflatex(x: &tinytex::test_support::FakeTex, replies: Vec<Reply>) {
    x.on("pdflatex", &[], replies);
}

// ---------- run counts ----------

#[test]
fn a_document_that_settles_after_one_run_is_built_once() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(&x, vec![ok(CLEAN_LOG)]);
    let c = run(
        "lp-once",
        &d,
        &CompileRequest::new().clean(tinytex::Clean::Never),
        &x,
    )
    .unwrap();
    assert_eq!(c.runs, 1);
    assert_eq!(x.count("pdflatex"), 1);
    assert_eq!(c.pdf, d.path("doc.pdf"));
    assert_eq!(c.log, Some(d.path("doc.log")));
    assert!(!c.diagnostics.max_runs_reached);
}

#[test]
fn the_engine_command_form() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(&x, vec![ok(CLEAN_LOG)]);
    let req = CompileRequest::new().engine_args(vec!["-shell-escape".into()]);
    run("lp-form", &d, &req, &x).unwrap();
    assert_eq!(
        engine_lines(&x),
        ["pdflatex -halt-on-error -interaction=batchmode -shell-escape doc.tex"]
    );
}

#[test]
fn min_runs_counts_total_runs_first_included() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(&x, vec![ok(CLEAN_LOG)]);
    let c = run("lp-min", &d, &CompileRequest::new().runs(3, 10), &x).unwrap();
    assert_eq!(c.runs, 3);
    assert_eq!(x.count("pdflatex"), 3);
}

#[test]
fn reruns_one_ten_is_runs_two_eleven() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(&x, vec![ok(CLEAN_LOG)]);
    let c = run("lp-reruns", &d, &CompileRequest::new().reruns(1, 10), &x).unwrap();
    assert_eq!(c.runs, 2);
}

#[test]
fn a_document_that_never_settles_stops_at_max_runs_with_a_diagnostic() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(&x, vec![ok(RERUN_LOG)]);
    let c = run("lp-max", &d, &CompileRequest::new().runs(1, 4), &x).unwrap();
    assert_eq!(c.runs, 4);
    assert!(c.diagnostics.max_runs_reached);
}

#[test]
fn a_stored_zero_reads_as_the_default_counts() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(&x, vec![ok(RERUN_LOG)]);
    let c = run("lp-zero", &d, &CompileRequest::new().runs(0, 0), &x).unwrap();
    assert_eq!(c.runs, 10, "max_runs 0 is 10");
    assert!(c.diagnostics.max_runs_reached);
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(&x, vec![ok(CLEAN_LOG)]);
    assert_eq!(
        run("lp-zero2", &d, &CompileRequest::new().runs(0, 0), &x)
            .unwrap()
            .runs,
        1,
        "min_runs 0 is 1"
    );
}

#[test]
fn every_engine_run_counts_and_the_event_numbers_agree_with_runs() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(
        &x,
        vec![
            ok_with(CLEAN_LOG, &[("doc.idx", "\\indexentry{a}{1}\n")]),
            ok(CLEAN_LOG),
        ],
    );
    let rec = Recorder::default();
    let c = run_observed("lp-count", &d, &CompileRequest::new(), &x, &rec).unwrap();
    // initial, the index-prep rerun, makeindex, then one more run.
    assert_eq!(c.runs, 3);
    let numbers: Vec<u32> = rec
        .events()
        .into_iter()
        .filter_map(|e| match e {
            Event::RunningEngine { run } => Some(run),
            _ => None,
        })
        .collect();
    assert_eq!(numbers, [1, 2, 3]);
}

#[test]
fn the_index_prep_rerun_counts_toward_max_runs() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(
        &x,
        vec![ok_with(CLEAN_LOG, &[("doc.idx", "x")]), ok(CLEAN_LOG)],
    );
    let c = run("lp-idxmax", &d, &CompileRequest::new().runs(1, 2), &x).unwrap();
    assert_eq!(c.runs, 2, "initial and prep; no room for the final run");
    assert!(c.diagnostics.max_runs_reached);
    assert_eq!(x.count("makeindex"), 1);
}

// ---------- missing packages ----------

#[test]
fn a_missing_package_is_installed_and_the_engine_retried() {
    let d = doc();
    let m = model(&[], &["pgfplots"], &[]);
    let x = tex(&m);
    pdflatex(&x, vec![fails(&missing_sty("pgfplots")), ok(CLEAN_LOG)]);
    search_answer(&x, "pgfplots.sty", "pgfplots");
    let rec = Recorder::default();
    let req = CompileRequest::new().auto_install(true);
    let c = run_observed("lp-install", &d, &req, &x, &rec).unwrap();
    assert_eq!(
        without_info(x.lines()),
        [
            "pdflatex -halt-on-error -interaction=batchmode doc.tex",
            "tlmgr search --file --global /pgfplots.sty",
            "tlmgr install pgfplots",
            "pdflatex -halt-on-error -interaction=batchmode doc.tex",
        ]
    );
    assert_eq!(c.runs, 2);
    assert_eq!(c.installed_packages, ["pgfplots"]);
    assert!(
        rec.events()
            .iter()
            .any(|e| matches!(e, Event::PackagesInstalled { names } if names == &["pgfplots"]))
    );
}

#[test]
fn the_engine_is_rerun_after_a_failed_install_attempt_too() {
    let d = doc();
    // Not installable: the attempt leaves the package missing.
    let m = model(&[], &[], &[]);
    let x = tex(&m);
    pdflatex(&x, vec![fails(&missing_sty("pgfplots")), ok(CLEAN_LOG)]);
    search_answer(&x, "pgfplots.sty", "pgfplots");
    let req = CompileRequest::new().auto_install(true);
    let c = run("lp-failedattempt", &d, &req, &x).unwrap();
    assert_eq!(c.runs, 2);
    assert!(c.installed_packages.is_empty());
}

#[test]
fn the_same_packages_twice_end_the_recovery() {
    let d = doc();
    let m = model(&[], &["pgfplots"], &[]);
    let x = tex(&m);
    pdflatex(&x, vec![fails(&missing_sty("pgfplots"))]);
    search_answer(&x, "pgfplots.sty", "pgfplots");
    let req = CompileRequest::new().auto_install(true);
    let err = run("lp-dedupe", &d, &req, &x).unwrap_err();
    assert!(matches!(
        err,
        CompileError::PackageInstall {
            reason: tinytex::log::InstallFailure::InstallFailed,
            ..
        }
    ));
    assert_eq!(x.count("tlmgr install"), 1, "installed once, not again");
    assert_eq!(x.count("pdflatex"), 2);
}

#[test]
fn a_fatal_babel_error_installs_the_hyphenation_package_by_name() {
    let d = doc();
    let m = model(&[], &["hyphen-spanish"], &[]);
    let x = tex(&m);
    pdflatex(
        &x,
        vec![
            fails(
                "! Package babel Error: You haven't defined the language 'spanish' yet.\n\nSee the babel package documentation.\n",
            ),
            ok(CLEAN_LOG),
        ],
    );
    let req = CompileRequest::new().auto_install(true);
    let c = run("lp-babel", &d, &req, &x).unwrap();
    assert_eq!(c.installed_packages, ["hyphen-spanish"]);
    assert_eq!(
        x.count("tlmgr search"),
        0,
        "a named package needs no search"
    );
}

#[test]
fn missfont_log_feeds_the_search() {
    let d = doc();
    let m = model(&[], &["cm-super"], &[]);
    let x = tex(&m);
    pdflatex(
        &x,
        vec![
            Reply::code(1)
                .creates("doc.log", "! Emergency stop.\n")
                .creates("missfont.log", "mktextfm ecrm1000\n"),
            ok(CLEAN_LOG),
        ],
    );
    let req = CompileRequest::new().auto_install(true);
    let _ = run("lp-missfont", &d, &req, &x);
    assert!(
        x.lines()
            .iter()
            .any(|l| l.starts_with("tlmgr search") && l.contains("ecrm1000")),
        "{:?}",
        x.lines()
    );
}

// ---------- recovery scope ----------

fn rerun_then_missing(x: &tinytex::test_support::FakeTex, then: Reply) {
    pdflatex(
        x,
        vec![ok(RERUN_LOG), fails(&missing_sty("pgfplots")), then],
    );
    search_answer(x, "pgfplots.sty", "pgfplots");
}

#[test]
fn first_run_recovery_rejects_a_failed_rerun_at_once() {
    let d = doc();
    let x = tex(&model(&[], &["pgfplots"], &[]));
    rerun_then_missing(&x, ok(CLEAN_LOG));
    let req = CompileRequest::new().auto_install(true);
    let err = run("lp-first", &d, &req, &x).unwrap_err();
    assert!(matches!(err, CompileError::Latex { .. }), "{err:?}");
    assert_eq!(x.count("tlmgr"), 0);
    assert_eq!(x.count("pdflatex"), 2);
}

#[test]
fn every_run_recovery_installs_after_a_failed_rerun() {
    let d = doc();
    let x = tex(&model(&[], &["pgfplots"], &[]));
    rerun_then_missing(&x, ok(CLEAN_LOG));
    let req = CompileRequest::new()
        .auto_install(true)
        .recover(Recover::EveryRun);
    let c = run("lp-every", &d, &req, &x).unwrap();
    assert_eq!(c.installed_packages, ["pgfplots"]);
    assert_eq!(c.runs, 3);
}

#[test]
fn every_run_recovery_stops_when_the_package_does_not_clear_the_report() {
    let d = doc();
    let x = tex(&model(&[], &["pgfplots"], &[]));
    rerun_then_missing(&x, fails(&missing_sty("pgfplots")));
    let req = CompileRequest::new()
        .auto_install(true)
        .recover(Recover::EveryRun);
    assert!(run("lp-every2", &d, &req, &x).is_err());
    assert_eq!(x.count("tlmgr install"), 1);
}

// ---------- index ----------

#[test]
fn the_index_pass_reruns_the_engine_first_then_runs_makeindex() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(
        &x,
        vec![ok_with(CLEAN_LOG, &[("doc.idx", "x")]), ok(CLEAN_LOG)],
    );
    run("lp-idx", &d, &CompileRequest::new(), &x).unwrap();
    let lines: Vec<String> = x
        .lines()
        .into_iter()
        .map(|l| l.split(' ').next().unwrap().to_owned())
        .collect();
    assert_eq!(lines, ["pdflatex", "pdflatex", "makeindex", "pdflatex"]);
    assert!(x.lines().contains(&"makeindex doc.idx".to_owned()));
}

#[test]
fn a_custom_index_command_and_arguments() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(&x, vec![ok_with(CLEAN_LOG, &[("doc.idx", "x")])]);
    let req = CompileRequest::new().index(IndexEngine {
        command: "texindy".into(),
        args: vec!["-g".into()],
    });
    run("lp-idx2", &d, &req, &x).unwrap();
    assert!(
        x.lines().contains(&"texindy -g doc.idx".to_owned()),
        "{:?}",
        x.lines()
    );
}

#[test]
fn a_failed_makeindex_is_an_index_error_with_the_ilg_message() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(&x, vec![ok_with(CLEAN_LOG, &[("doc.idx", "x")])]);
    x.reply(
        "makeindex",
        &[],
        Reply::code(1).creates(
            "doc.ilg",
            "Scanning input file doc.idx\n   -- Input index error (file = doc.idx, line = 1):\n",
        ),
    );
    let err = run("lp-idx3", &d, &CompileRequest::new(), &x).unwrap_err();
    match err {
        CompileError::Index { log_excerpt, log } => {
            assert!(log_excerpt.contains("Input index error"), "{log_excerpt}");
            assert_eq!(log, Some(d.path("doc.log")));
        }
        other => panic!("{other:?}"),
    }
}

// ---------- bibliography ----------

const AUX_BIB: &str = "\\citation{a}\n\\bibdata{refs}\n\\bibstyle{plain}\n";

#[test]
fn bibtex_runs_when_the_aux_has_all_three_markers() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(
        &x,
        vec![ok_with(CLEAN_LOG, &[("doc.aux", AUX_BIB)]), ok(CLEAN_LOG)],
    );
    let c = run("lp-bib", &d, &CompileRequest::new(), &x).unwrap();
    let names: Vec<String> = x
        .lines()
        .iter()
        .map(|l| l.split(' ').next().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["pdflatex", "bibtex", "pdflatex"]);
    assert!(x.lines().contains(&"bibtex doc".to_owned()));
    assert_eq!(c.runs, 2);
}

#[test]
fn a_citation_without_a_bibstyle_does_not_run_bibtex() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(
        &x,
        vec![ok_with(
            CLEAN_LOG,
            &[("doc.aux", "\\citation{a}\n\\bibdata{refs}\n")],
        )],
    );
    run("lp-bib2", &d, &CompileRequest::new(), &x).unwrap();
    assert_eq!(x.count("bibtex"), 0);
}

#[test]
fn biber_runs_when_a_bcf_exists_and_skip_runs_nothing() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(
        &x,
        vec![
            ok_with(CLEAN_LOG, &[("doc.bcf", "<bcf/>"), ("doc.aux", AUX_BIB)]),
            ok(CLEAN_LOG),
        ],
    );
    run("lp-biber", &d, &CompileRequest::new(), &x).unwrap();
    assert!(x.lines().contains(&"biber doc".to_owned()));
    assert_eq!(x.count("bibtex"), 0);

    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(
        &x,
        vec![ok_with(
            CLEAN_LOG,
            &[("doc.bcf", "<bcf/>"), ("doc.aux", AUX_BIB)],
        )],
    );
    run("lp-skip", &d, &CompileRequest::new().bib(Bib::Skip), &x).unwrap();
    assert_eq!(x.count("biber") + x.count("bibtex"), 0);
}

#[test]
fn an_explicit_bibtex_runs_bibtex_even_with_a_bcf() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(
        &x,
        vec![
            ok_with(CLEAN_LOG, &[("doc.bcf", "<bcf/>"), ("doc.aux", AUX_BIB)]),
            ok(CLEAN_LOG),
        ],
    );
    run("lp-bibtex", &d, &CompileRequest::new().bib(Bib::Bibtex), &x).unwrap();
    assert_eq!(x.count("bibtex"), 1);
    assert_eq!(x.count("biber"), 0);
}

#[test]
fn a_bibliography_failure_is_fatal_by_default_and_a_diagnostic_with_warn() {
    let failing = |x: &tinytex::test_support::FakeTex| {
        pdflatex(
            x,
            vec![ok_with(CLEAN_LOG, &[("doc.aux", AUX_BIB)]), ok(CLEAN_LOG)],
        );
        x.reply(
            "bibtex",
            &[],
            Reply::code(2).creates("doc.blg", "I found no \\citation commands\n"),
        );
    };
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    failing(&x);
    let err = run("lp-bibfail", &d, &CompileRequest::new(), &x).unwrap_err();
    match err {
        CompileError::Bibliography {
            engine,
            log_excerpt,
            ..
        } => {
            assert_eq!(engine, "bibtex");
            assert!(log_excerpt.contains("no \\citation"), "{log_excerpt}");
        }
        other => panic!("{other:?}"),
    }

    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    failing(&x);
    let req = CompileRequest::new().on_bib_failure(tinytex::OnBibFailure::Warn);
    let c = run("lp-bibwarn", &d, &req, &x).unwrap();
    let msg = c.diagnostics.bibliography_failed.expect("recorded");
    assert!(
        msg.contains("bibtex") && msg.contains("no \\citation"),
        "{msg}"
    );
    assert_eq!(c.runs, 2, "the loop continues");
}

const BLG_NO_STYLE: &str = "I couldn't open style file agsm.bst\n---line 7 of file doc.aux\n";

fn bst_failure(x: &tinytex::test_support::FakeTex, second: Reply) {
    pdflatex(
        x,
        vec![ok_with(CLEAN_LOG, &[("doc.aux", AUX_BIB)]), ok(CLEAN_LOG)],
    );
    x.on(
        "bibtex",
        &[],
        vec![Reply::code(2).creates("doc.blg", BLG_NO_STYLE), second],
    );
    search_answer(x, "agsm.bst", "bibtex-extra");
}

#[test]
fn a_missing_bst_installs_its_package_and_reruns_bibtex() {
    let d = doc();
    let x = tex(&model(&[], &["bibtex-extra"], &[]));
    bst_failure(&x, Reply::ok());
    let req = CompileRequest::new().auto_install(true);
    let c = run("lp-bst", &d, &req, &x).unwrap();
    assert_eq!(x.count("bibtex"), 2);
    assert_eq!(c.installed_packages, ["bibtex-extra"]);
}

#[test]
fn a_missing_bst_whose_install_cannot_clear_it_is_a_bibliography_error_even_with_warn() {
    let d = doc();
    let x = tex(&model(&[], &["bibtex-extra"], &[]));
    bst_failure(&x, Reply::code(2).creates("doc.blg", BLG_NO_STYLE));
    let req = CompileRequest::new()
        .auto_install(true)
        .on_bib_failure(tinytex::OnBibFailure::Warn);
    let err = run("lp-bst2", &d, &req, &x).unwrap_err();
    assert!(matches!(err, CompileError::Bibliography { .. }), "{err:?}");
    assert_eq!(x.count("tlmgr install"), 1, "installed once");
}

// ---------- reruns after a success ----------

fn babel_warning(lang: &str) -> String {
    format!(
        "Package babel Warning: No hyphenation patterns were preloaded for\n(babel)                the language '{lang}' into the format.\n(babel)                Please, configure your TeX system to add them and\n(babel)                rebuild the format. Now I will use the patterns\n(babel)                preloaded for \\language=0 instead on input line 51.\n"
    )
}

#[test]
fn missing_hyphenation_is_installed_and_the_first_run_repeated() {
    let d = doc();
    let x = tex(&model(&[], &["hyphen-spanish", "hyphen-italian"], &[]));
    let log = format!("{}{}", babel_warning("Spanish"), babel_warning("Italian"));
    pdflatex(&x, vec![ok(&log), ok(CLEAN_LOG)]);
    let req = CompileRequest::new().auto_install(true);
    let c = run("lp-hyph", &d, &req, &x).unwrap();
    assert_eq!(c.runs, 2);
    assert!(
        x.lines()
            .contains(&"tlmgr install hyphen-spanish hyphen-italian".to_owned()),
        "{:?}",
        x.lines()
    );
    assert!(c.diagnostics.missing_hyphenation.is_empty());
}

#[test]
fn the_same_hyphenation_files_reported_again_stop_the_repeat() {
    let d = doc();
    let x = tex(&model(&[], &["hyphen-spanish"], &[]));
    pdflatex(&x, vec![ok(&babel_warning("Spanish"))]);
    let req = CompileRequest::new().auto_install(true);
    let c = run("lp-hyph2", &d, &req, &x).unwrap();
    assert_eq!(c.runs, 2, "one repeat, then the guard");
    assert_eq!(c.diagnostics.missing_hyphenation, ["hyphen-spanish"]);
}

#[test]
fn without_auto_install_hyphenation_is_a_diagnostic_and_the_build_succeeds() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(&x, vec![ok(&babel_warning("Spanish"))]);
    let c = run("lp-hyph3", &d, &CompileRequest::new(), &x).unwrap();
    assert_eq!(c.runs, 1);
    assert_eq!(x.count("tlmgr"), 0);
    assert_eq!(c.diagnostics.missing_hyphenation, ["hyphen-spanish"]);
}

#[test]
fn a_failed_hyphenation_install_is_a_diagnostic_too() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(&x, vec![ok(&babel_warning("Spanish"))]);
    let req = CompileRequest::new().auto_install(true);
    let c = run("lp-hyph4", &d, &req, &x).unwrap();
    assert_eq!(c.diagnostics.missing_hyphenation, ["hyphen-spanish"]);
}

#[test]
fn module_warnings_install_the_modules_and_run_the_engine_once_more() {
    let d = doc();
    let x = tex(&model(&[], &["datetime2-english"], &[]));
    let log = "Package datetime2 Warning: Date-Time Language Module `english' not installed on\n(datetime2)  input line 5.\n";
    pdflatex(&x, vec![ok(log), ok(CLEAN_LOG)]);
    let req = CompileRequest::new().auto_install(true);
    let c = run("lp-module", &d, &req, &x).unwrap();
    assert_eq!(c.runs, 2);
    assert_eq!(c.installed_packages, ["datetime2-english"]);
}

// ---------- working directory ----------

#[test]
fn every_engine_and_tool_command_runs_in_the_tex_directory() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(
        &x,
        vec![
            ok_with(CLEAN_LOG, &[("doc.aux", AUX_BIB), ("doc.idx", "x")]),
            ok(CLEAN_LOG),
        ],
    );
    run("lp-cwd", &d, &CompileRequest::new(), &x).unwrap();
    let calls = x.calls();
    assert!(calls.len() >= 5);
    for c in &calls {
        assert_eq!(c.cwd.as_deref(), Some(d.dir.path()), "{c:?}");
    }
}

#[test]
fn a_relative_output_dir_is_relative_to_the_tex_directory_and_created() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    pdflatex(
        &x,
        vec![
            Reply::ok()
                .creates("build/doc.log", CLEAN_LOG)
                .creates("build/doc.pdf", "%PDF")
                .creates("build/doc.aux", AUX_BIB),
        ],
    );
    let req = CompileRequest::new().output_dir("build".into());
    let c = run("lp-outdir", &d, &req, &x).unwrap();
    assert_eq!(c.pdf, d.path("build").join("doc.pdf"));
    assert!(
        x.lines()[0].contains("-output-directory=build"),
        "{:?}",
        x.lines()
    );
    let bib = format!(
        "bibtex {}",
        std::path::Path::new("build").join("doc").display()
    );
    assert!(x.lines().contains(&bib), "{:?}", x.lines());
}
