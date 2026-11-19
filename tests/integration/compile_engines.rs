//! The per-pass engines (`Engine::Latexmk`, `Engine::Tectonic`), tool
//! resolution and search, `input_paths`, and the Windows `.aux` tweak.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

use tinytex::test_support::{FakeTex, Reply, tweak_aux};
use tinytex::{
    Bib, Clean, CompileError, CompileRequest, Compiler, Config, Distribution, Engine, IndexEngine,
    NullObserver, ToolPaths,
};

use super::compile_support::*;
use super::tlmgr_model::model;

fn env_of(spec: &tinytex::CommandSpec, key: &str) -> Option<OsString> {
    spec.env
        .iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

// ---------- Engine::Tectonic ----------

#[test]
fn tectonic_is_one_call_with_no_log() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.reply("tectonic", &[], Reply::ok().creates("doc.pdf", "%PDF"));
    let req = CompileRequest::new()
        .engine(Engine::Tectonic)
        .engine_args(vec!["-Zshell-escape".into()]);
    let c = run("te-one", &d, &req, &x).unwrap();
    assert_eq!(x.lines(), ["tectonic -Zshell-escape doc.tex"]);
    assert_eq!(c.runs, 1);
    assert_eq!(c.log, None);
    assert_eq!(c.pdf, d.path("doc.pdf"));
    assert_eq!(x.calls()[0].cwd.as_deref(), Some(d.dir.path()));
}

#[test]
fn tectonic_maps_output_dir_to_outdir() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.reply("tectonic", &[], Reply::ok().creates("out/doc.pdf", "%PDF"));
    let req = CompileRequest::new()
        .engine(Engine::Tectonic)
        .output_dir("out".into());
    let c = run("te-outdir", &d, &req, &x).unwrap();
    assert_eq!(x.lines(), ["tectonic --outdir out doc.tex"]);
    assert_eq!(c.pdf, d.path("out/doc.pdf"));
}

#[test]
fn tectonic_success_means_the_output_exists() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.reply("tectonic", &[], Reply::ok());
    let req = CompileRequest::new().engine(Engine::Tectonic);
    let err = run("te-none", &d, &req, &x).unwrap_err();
    assert!(matches!(err, CompileError::NoOutput), "{err:?}");

    let x = tex(&model(&[], &[], &[]));
    x.reply(
        "tectonic",
        &[],
        Reply::code(1).stderr("error: could not find file\n"),
    );
    let err = run("te-fail", &d, &req, &x).unwrap_err();
    match err {
        CompileError::Latex {
            exit_code,
            stderr,
            log,
            ..
        } => {
            assert_eq!(exit_code, Some(1));
            assert!(stderr.contains("could not find"));
            assert_eq!(log, None);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn tectonic_renames_to_pdf_file_and_is_not_cleaned_of_the_pdf() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.reply("tectonic", &[], Reply::ok().creates("doc.pdf", "%PDF"));
    let req = CompileRequest::new()
        .engine(Engine::Tectonic)
        .pdf_file("t.pdf".into());
    let c = run("te-rename", &d, &req, &x).unwrap();
    assert_eq!(c.pdf, d.path("t.pdf"));
    assert!(d.exists("t.pdf"));
}

// ---------- Engine::Latexmk ----------

#[test]
fn engine_latexmk_runs_latexmk_per_pass_with_quarto_1_flags() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.reply(
        "latexmk",
        &[],
        Reply::ok()
            .creates("out/doc.log", CLEAN_LOG)
            .creates("out/doc.pdf", "%PDF"),
    );
    let req = CompileRequest::new()
        .engine(Engine::Latexmk)
        .output_dir("out".into())
        .engine_args(vec!["-shell-escape".into()]);
    run("el-form", &d, &req, &x).unwrap();
    assert_eq!(
        x.lines(),
        [
            "latexmk -interaction=batchmode -halt-on-error -pdf -quiet -output-directory=out -shell-escape doc.tex"
        ]
    );
}

#[test]
fn engine_latexmk_omits_pdf_when_a_pdf_variant_is_given() {
    for variant in ["-pdfxe", "-pdflua", "-pdfdvi", "-pdfps", "-pdf", "-pdf-"] {
        let d = doc();
        let x = tex(&model(&[], &[], &[]));
        x.reply("latexmk", &[], ok(CLEAN_LOG));
        let req = CompileRequest::new()
            .engine(Engine::Latexmk)
            .engine_args(vec![variant.into()]);
        run("el-variant", &d, &req, &x).unwrap();
        let line = &x.lines()[0];
        assert_eq!(
            line,
            &format!("latexmk -interaction=batchmode -halt-on-error -quiet {variant} doc.tex"),
            "{variant}"
        );
    }
}

#[test]
fn engine_latexmk_still_runs_our_loop_bibliography_and_recovery() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.on(
        "latexmk",
        &[],
        vec![
            ok_with(
                RERUN_LOG,
                &[(
                    "doc.aux",
                    "\\citation{a}\n\\bibstyle{plain}\n\\bibdata{refs}\n",
                )],
            ),
            ok(CLEAN_LOG),
        ],
    );
    let req = CompileRequest::new()
        .engine(Engine::Latexmk)
        .clean(Clean::Never);
    let c = run("el-loop", &d, &req, &x).unwrap();
    assert_eq!(x.count("latexmk"), 2, "the first run, then bibtex's rerun");
    assert_eq!(x.count("bibtex doc"), 1);
    assert_eq!(c.runs, 2);
}

// ---------- tool resolution ----------

fn bin_dir_with(tools: &[&str]) -> (tempfile::TempDir, Distribution) {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    for t in tools {
        std::fs::write(bin.join(t), "").unwrap();
    }
    let dist = Distribution::from_bin_dir(bin);
    (dir, dist)
}

#[allow(clippy::result_large_err)] // `CompileError` is large by design.
fn compile_with(
    dist: Option<Distribution>,
    d: &Doc,
    req: &CompileRequest,
    x: &FakeTex,
) -> Result<tinytex::Compiled, CompileError> {
    Compiler::new(dist, Config::default()).compile(&d.tex, req, x, &NullObserver)
}

#[test]
fn a_tool_is_found_in_tool_paths_then_bin_dir_then_path() {
    let (_g, dist) = bin_dir_with(&["pdflatex"]);
    let bin = dist.bin_dir.clone();
    let d = doc();

    // bin_dir
    let x = tex(&model(&[], &[], &[]));
    x.reply("pdflatex", &[], ok(CLEAN_LOG));
    compile_with(Some(dist.clone()), &d, &CompileRequest::new(), &x).unwrap();
    assert_eq!(x.calls()[0].program, bin.join("pdflatex"));

    // ToolPaths wins over bin_dir
    let own = d.path("my-pdflatex");
    d.write("my-pdflatex", "");
    let x = tex(&model(&[], &[], &[]));
    x.reply("my-pdflatex", &[], ok(CLEAN_LOG));
    let req = CompileRequest::new().tools(ToolPaths {
        engine: Some(own.clone()),
        ..ToolPaths::default()
    });
    compile_with(Some(dist), &d, &req, &x).unwrap();
    assert_eq!(x.calls()[0].program, own);

    // neither: the bare name, found on PATH, which has bin_dir in front
    let (_g2, empty) = bin_dir_with(&[]);
    let x = tex(&model(&[], &[], &[]));
    x.reply("pdflatex", &[], ok(CLEAN_LOG));
    compile_with(Some(empty.clone()), &d, &CompileRequest::new(), &x).unwrap();
    let spec = &x.calls()[0];
    assert_eq!(spec.program, PathBuf::from("pdflatex"));
    let path = env_of(spec, "PATH").unwrap();
    assert!(
        path.to_string_lossy()
            .starts_with(&*empty.bin_dir.to_string_lossy())
    );
}

#[test]
fn an_explicit_tool_path_that_does_not_exist_is_engine_not_found() {
    let (_g, dist) = bin_dir_with(&["pdflatex", "bibtex"]);
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.reply("pdflatex", &[], ok(CLEAN_LOG));
    let req = CompileRequest::new().tools(ToolPaths {
        engine: Some(d.path("no-such-engine")),
        ..ToolPaths::default()
    });
    match compile_with(Some(dist), &d, &req, &x).unwrap_err() {
        CompileError::EngineNotFound { tool } => assert_eq!(tool, "pdflatex"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        x.count("pdflatex"),
        0,
        "never a fall-through to another copy"
    );
}

#[test]
fn each_tool_path_applies_to_its_own_tool() {
    let (_g, dist) = bin_dir_with(&[]);
    let d = doc();
    for (name, set) in [("tectonic", 0), ("latexmk", 1)] {
        let p = d.path(&format!("my-{name}"));
        d.write(&format!("my-{name}"), "");
        let x = tex(&model(&[], &[], &[]));
        x.reply(&format!("my-{name}"), &[], ok(CLEAN_LOG));
        let mut t = ToolPaths::default();
        if set == 0 {
            t.tectonic = Some(p.clone());
        } else {
            t.latexmk = Some(p.clone());
        }
        let engine = if set == 0 {
            Engine::Tectonic
        } else {
            Engine::Latexmk
        };
        let req = CompileRequest::new().engine(engine).tools(t);
        let _ = compile_with(Some(dist.clone()), &d, &req, &x);
        assert_eq!(x.calls()[0].program, p, "{name}");
    }
}

// ---------- tool search (auto-install) ----------

fn not_found() -> Reply {
    Reply::ok().fails(io::ErrorKind::NotFound)
}

#[test]
fn a_missing_bibliography_tool_is_searched_installed_and_run_again() {
    let d = doc();
    let m = model(&[], &["bibtex"], &[]);
    let x = tex(&m);
    x.reply(
        "pdflatex",
        &[],
        ok_with(
            CLEAN_LOG,
            &[(
                "doc.aux",
                "\\citation{a}\n\\bibstyle{plain}\n\\bibdata{refs}\n",
            )],
        ),
    );
    x.on("bibtex", &[], vec![not_found(), Reply::ok()]);
    search_answer(&x, "bibtex", "bibtex");
    let req = CompileRequest::new().auto_install(true).clean(Clean::Never);
    let c = run("ts-bib", &d, &req, &x).unwrap();
    assert_eq!(x.count("bibtex"), 2, "the failed spawn, then the retry");
    assert!(
        x.lines()
            .iter()
            .any(|l| l == "tlmgr search --file --global /bibtex"),
        "{:?}",
        x.lines()
    );
    assert!(c.installed_packages.contains(&"bibtex".to_owned()));
}

#[test]
fn texindy_maps_to_xindy_when_the_search_finds_nothing() {
    let d = doc();
    let m = model(&[], &["xindy"], &[]);
    let x = tex(&m);
    x.reply("pdflatex", &[], ok_with(CLEAN_LOG, &[("doc.idx", "i")]));
    x.on("texindy", &[], vec![not_found(), Reply::ok()]);
    let req = CompileRequest::new()
        .auto_install(true)
        .clean(Clean::Never)
        .index(IndexEngine {
            command: "texindy".into(),
            args: vec![],
        });
    let c = run("ts-xindy", &d, &req, &x).unwrap();
    assert_eq!(c.installed_packages, ["xindy"]);
    assert_eq!(x.count("texindy"), 2);
}

#[test]
fn a_missing_engine_is_searched_installed_and_run_again() {
    let d = doc();
    let m = model(&[], &["pdftex"], &[]);
    let x = tex(&m);
    x.on("pdflatex", &[], vec![not_found(), ok(CLEAN_LOG)]);
    x.reply(
        "tlmgr",
        &["search", "--file", "--global", "/pdflatex"],
        Reply::ok().stdout("pdftex:\n\tbin/x86_64-linux/pdflatex\n"),
    );
    let c = run(
        "ts-engine",
        &d,
        &CompileRequest::new().auto_install(true),
        &x,
    )
    .unwrap();
    assert_eq!(c.installed_packages, ["pdftex"]);
    assert_eq!(x.count("pdflatex"), 2);
}

#[test]
fn a_tool_that_is_still_missing_after_the_install_is_engine_not_found() {
    let d = doc();
    let m = model(&[], &["pdftex"], &[]);
    let x = tex(&m);
    x.reply("pdflatex", &[], not_found());
    match run(
        "ts-still",
        &d,
        &CompileRequest::new().auto_install(true),
        &x,
    )
    .unwrap_err()
    {
        CompileError::EngineNotFound { tool } => assert_eq!(tool, "pdflatex"),
        other => panic!("{other:?}"),
    }
    assert_eq!(x.count("pdflatex"), 2, "one retry, not a loop");
}

#[test]
fn without_auto_install_or_a_distribution_a_missing_tool_is_not_searched() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.reply(
        "pdflatex",
        &[],
        ok_with(
            CLEAN_LOG,
            &[(
                "doc.aux",
                "\\citation{a}\n\\bibstyle{plain}\n\\bibdata{refs}\n",
            )],
        ),
    );
    x.reply("bibtex", &[], not_found());
    match run("ts-off", &d, &CompileRequest::new(), &x).unwrap_err() {
        CompileError::EngineNotFound { tool } => assert_eq!(tool, "bibtex"),
        other => panic!("{other:?}"),
    }
    assert_eq!(x.count("tlmgr"), 0);

    let x = tex(&model(&[], &[], &[]));
    x.reply(
        "pdflatex",
        &[],
        ok_with(
            CLEAN_LOG,
            &[(
                "doc.aux",
                "\\citation{a}\n\\bibstyle{plain}\n\\bibdata{refs}\n",
            )],
        ),
    );
    x.reply("bibtex", &[], not_found());
    let req = CompileRequest::new().auto_install(true).bib(Bib::Bibtex);
    let err = compile_with(None, &d, &req, &x).unwrap_err();
    assert!(
        matches!(err, CompileError::EngineNotFound { .. }),
        "{err:?}"
    );
    assert_eq!(x.count("tlmgr"), 0, "engine-only mode never installs");
}

// ---------- input paths ----------

#[test]
fn input_paths_become_texinputs_and_bstinputs() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.reply(
        "pdflatex",
        &[],
        ok_with(
            CLEAN_LOG,
            &[
                (
                    "doc.aux",
                    "\\citation{a}\n\\bibstyle{plain}\n\\bibdata{refs}\n",
                ),
                ("doc.idx", "i"),
            ],
        ),
    );
    let req = CompileRequest::new()
        .input_paths(vec!["styles".into(), "/abs/tex".into()])
        .clean(Clean::Never);
    run("ip-env", &d, &req, &x).unwrap();
    let want = OsString::from("styles;/abs/tex;");
    for call in x.calls() {
        let name = call
            .program
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if name == "tlmgr" {
            continue;
        }
        assert_eq!(env_of(&call, "TEXINPUTS"), Some(want.clone()), "{name}");
        assert_eq!(env_of(&call, "BSTINPUTS"), Some(want.clone()), "{name}");
        assert_eq!(env_of(&call, "BIBINPUTS"), None, "{name}");
        assert_eq!(env_of(&call, "INDEXSTYLE"), None, "{name}");
    }
    assert!(x.count("bibtex") >= 1 && x.count("makeindex") >= 1);
}

#[test]
fn no_input_paths_set_no_variables() {
    let d = doc();
    let x = tex(&model(&[], &[], &[]));
    x.reply("pdflatex", &[], ok(CLEAN_LOG));
    run("ip-none", &d, &CompileRequest::new(), &x).unwrap();
    assert_eq!(env_of(&x.calls()[0], "TEXINPUTS"), None);
}

// ---------- the Windows .aux tweak ----------

#[test]
fn tweak_aux_strips_bib_from_bibdata_lines_only() {
    let aux = "\\relax\n\\citation{a}\n\\bibstyle{plain}\n\\bibdata{refs.bib,more.bib}\n\\bibcite{a}{1}\n";
    let out = tweak_aux(aux).unwrap();
    assert!(out.contains("\\bibdata{refs,more}\n"), "{out}");
    assert!(out.contains("\\bibcite{a}{1}"));
    assert_eq!(out.lines().count(), aux.lines().count());
}

#[test]
fn tweak_aux_handles_one_entry_and_leaves_other_names_alone() {
    assert_eq!(
        tweak_aux("\\bibdata{refs.bib}\n").unwrap(),
        "\\bibdata{refs}\n"
    );
    assert_eq!(
        tweak_aux("\\bibdata{my.bibliography.bib}\n").unwrap(),
        "\\bibdata{my.bibliography}\n"
    );
    assert_eq!(tweak_aux("\\bibdata{refs}\n"), None, "nothing to change");
    assert_eq!(tweak_aux("\\citation{a}\n"), None, "no \\bibdata line");
}

#[test]
fn the_tweak_is_applied_for_bibtex_on_windows_without_tlmgr_only() {
    let aux = "\\citation{a}\n\\bibstyle{plain}\n\\bibdata{refs.bib}\n";
    for (windows, dist, rewritten) in [
        (true, false, true),
        (false, false, false),
        (true, true, false),
    ] {
        let d = doc();
        let x = tex(&model(&[], &[], &[]));
        x.on(
            "pdflatex",
            &[],
            vec![ok_with(CLEAN_LOG, &[("doc.aux", aux)]), ok(CLEAN_LOG)],
        );
        let req = CompileRequest::new().bib(Bib::Bibtex).clean(Clean::Never);
        let compiler = if dist {
            compiler("tw-dist")
        } else {
            Compiler::new(None, Config::default())
        }
        .for_windows(windows);
        compiler.compile(&d.tex, &req, &x, &NullObserver).unwrap();
        let after = std::fs::read_to_string(d.path("doc.aux")).unwrap();
        assert_eq!(
            after.contains("\\bibdata{refs}"),
            rewritten,
            "windows={windows} dist={dist}"
        );
    }
}
