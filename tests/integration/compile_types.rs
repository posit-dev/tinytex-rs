//! `CompileRequest` defaults, serialization and setters, and the shape of
//! `CompileError` (plan 5.4a, phase 1).

use tinytex::{
    Bib, Clean, CompileError, CompileRequest, Driver, Engine, IndexEngine, KeepLog, OnBibFailure,
    Output, Recover,
};

#[test]
fn defaults_are_the_crates_own_with_rs_choices_where_the_upstreams_differ() {
    let r = CompileRequest::default();
    assert_eq!(r.engine, Engine::PdfLatex);
    assert_eq!(r.driver, Driver::Builtin);
    assert!(r.engine_args.is_empty());
    assert_eq!(r.bib, Bib::Infer);
    assert_eq!(r.index, None, "None is makeindex with no options");
    assert_eq!((r.min_runs, r.max_runs), (1, 10));
    assert_eq!(r.output_dir, None);
    assert_eq!(r.pdf_file, None);
    assert!(r.input_paths.is_empty());
    assert_eq!(r.tools, tinytex::ToolPaths::default());
    assert!(!r.auto_install);
    assert_eq!(r.recover, Recover::FirstRun);
    assert_eq!(r.on_bib_failure, OnBibFailure::Error);
    assert_eq!(r.clean, Clean::Always);
    assert_eq!(r.keep_log, KeepLog::OnWarnings);
    assert_eq!(r.output, Output::Pdf);
    assert_eq!(CompileRequest::new(), r);
}

#[test]
fn reruns_is_r_s_meaning_of_runs() {
    assert_eq!(
        CompileRequest::new().reruns(1, 10),
        CompileRequest::new().runs(2, 11)
    );
    assert_eq!(
        CompileRequest::new().reruns(0, 0),
        CompileRequest::new().runs(1, 1)
    );
}

#[test]
fn a_request_round_trips_through_json() {
    let r = CompileRequest::new()
        .engine(Engine::XeLatex)
        .driver(Driver::Latexmk)
        .engine_args(vec!["-shell-escape".into()])
        .bib(Bib::Biber)
        .index(IndexEngine {
            command: "texindy".into(),
            args: vec!["-g".into()],
        })
        .runs(2, 7)
        .output_dir("build".into())
        .pdf_file("out.pdf".into())
        .input_paths(vec!["styles".into()])
        .auto_install(true)
        .recover(Recover::EveryRun)
        .on_bib_failure(OnBibFailure::Warn)
        .clean(Clean::OnSuccess)
        .keep_log(KeepLog::OnFailure);
    let json = serde_json::to_string(&r).unwrap();
    let back: CompileRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(back, r);
    // Missing fields read as defaults.
    let sparse: CompileRequest = serde_json::from_str(r#"{"engine":"lua_latex"}"#).unwrap();
    assert_eq!(sparse.engine, Engine::LuaLatex);
    assert_eq!((sparse.min_runs, sparse.max_runs), (1, 10));
}

#[test]
fn errors_have_stable_string_codes_and_serialize() {
    let cases: Vec<(CompileError, &str)> = vec![
        (
            CompileError::NoTexInstallation,
            "compile.no_tex_installation",
        ),
        (
            CompileError::EngineNotFound { tool: "x".into() },
            "compile.engine_not_found",
        ),
        (CompileError::NoOutput, "compile.no_output"),
        (
            CompileError::InvalidRequest {
                message: "m".into(),
            },
            "compile.invalid_request",
        ),
        (
            CompileError::Bibliography {
                engine: "bibtex".into(),
                log_excerpt: String::new(),
                log: None,
            },
            "compile.bibliography",
        ),
        (
            CompileError::Index {
                log_excerpt: String::new(),
                log: None,
            },
            "compile.index",
        ),
        (CompileError::Cancelled, "compile.cancelled"),
        (CompileError::Io(std::io::Error::other("x")), "compile.io"),
        (
            CompileError::Latex {
                summary: Some("s".into()),
                hints: vec![],
                log: None,
                missing: vec![],
                stdout: String::new(),
                stderr: String::new(),
                exit_code: Some(1),
            },
            "compile.latex",
        ),
    ];
    for (err, code) in cases {
        assert_eq!(err.code().as_str(), code);
        let v = serde_json::to_value(&err).unwrap();
        assert_eq!(v["code"], code);
        assert!(v["message"].is_string());
    }
}
