//! Compiles against a real TeX distribution.
//!
//! Each test skips with a visible message when no distribution is found,
//! unless `TINYTEX_RS_REQUIRE_TEX=1` (the crate's TinyTeX CI job sets it), in
//! which case a missing distribution fails the test: a broken CI install must
//! not pass as green.
//!
//! The tests whose names start with `auto_install_` remove and install
//! packages, so they never run against the distribution `find` returns. They
//! run only inside a disposable copy that passes the gate of `sandbox` (build
//! one with `scripts/tex-sandbox.sh`; CI does). `TINYTEX_RS_REQUIRE_TEX` is not
//! permission to mutate a tree: a developer who sets it still skips them.

#![allow(clippy::result_large_err)] // `CompileError` is large by design (see `compile`).

use std::path::Path;

use tempfile::TempDir;
use tinytex::{
    Bib, Clean, CompileError, CompileRequest, Compiled, Compiler, Config, Distribution, Driver,
    Engine, KeepLog, NullObserver, StdExec,
};

use super::sandbox::{self, Sandbox};

fn required() -> bool {
    std::env::var("TINYTEX_RS_REQUIRE_TEX").is_ok_and(|v| v == "1")
}

/// The distribution, or `None` after printing why the test skips.
fn distribution(test: &str, tools: &[&str]) -> Option<Distribution> {
    let usable = tinytex::detect::find(&Config::default())
        .filter(|d| tools.iter().all(|t| d.bin_dir.join(t).exists()));
    if usable.is_none() {
        assert!(
            !required(),
            "{test}: TINYTEX_RS_REQUIRE_TEX=1 but no TeX distribution with {tools:?} was found"
        );
        eprintln!(
            "SKIPPED {test}: no TeX distribution with {tools:?} (set TINYTEX_RS_REQUIRE_TEX=1 to fail instead)"
        );
    }
    usable
}

const NATBIB: &str = r"\documentclass{article}
\usepackage{natbib}
\usepackage{makeidx}
\makeindex
\begin{document}
Hello \citep{knuth}\index{hello}.
\bibliographystyle{plainnat}
\bibliography{refs}
\printindex
\end{document}
";

const REFS: &str = "@book{knuth,\n  author = {Knuth, Donald},\n  title = {The Art of Computer Programming},\n  year = {1968},\n  publisher = {Addison-Wesley}\n}\n";

struct Project {
    dir: TempDir,
}

impl Project {
    fn new(tex: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("doc.tex"), tex).unwrap();
        std::fs::write(dir.path().join("refs.bib"), REFS).unwrap();
        Self { dir }
    }
    fn tex(&self) -> std::path::PathBuf {
        self.dir.path().join("doc.tex")
    }
    fn exists(&self, name: &str) -> bool {
        self.dir.path().join(name).exists()
    }
}

fn compile(d: Distribution, p: &Project, req: &CompileRequest) -> Result<Compiled, CompileError> {
    Compiler::new(Some(d), Config::default()).compile(&p.tex(), req, &StdExec::new(), &NullObserver)
}

fn pdf_ok(path: &Path) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        bytes.starts_with(b"%PDF"),
        "{} is not a PDF",
        path.display()
    );
}

#[test]
fn a_natbib_and_index_document_builds_in_place() {
    let Some(d) = distribution("natbib_in_place", &["pdflatex", "bibtex", "makeindex"]) else {
        return;
    };
    let p = Project::new(NATBIB);
    let c = compile(d, &p, &CompileRequest::new().clean(Clean::Never)).unwrap();
    pdf_ok(&c.pdf);
    assert!(
        c.runs >= 3,
        "engine, bibtex and makeindex reruns: {}",
        c.runs
    );
    assert!(p.exists("doc.bbl") && p.exists("doc.ind"));
    let log = std::fs::read_to_string(p.dir.path().join("doc.log")).unwrap();
    assert!(
        !log.contains("Citation `knuth' on page"),
        "citation resolved"
    );
}

#[test]
fn the_default_cleanup_removes_what_the_run_created() {
    let Some(d) = distribution("default_cleanup", &["pdflatex", "bibtex", "makeindex"]) else {
        return;
    };
    let p = Project::new(NATBIB);
    let c = compile(d, &p, &CompileRequest::new()).unwrap();
    pdf_ok(&c.pdf);
    for gone in [
        "doc.aux", "doc.bbl", "doc.blg", "doc.idx", "doc.ind", "doc.ilg",
    ] {
        assert!(!p.exists(gone), "{gone}");
    }
    assert!(p.exists("refs.bib") && p.exists("doc.tex"));
}

#[test]
fn output_dir_inside_the_tex_directory_works() {
    let Some(d) = distribution("output_dir_inside", &["pdflatex", "bibtex", "makeindex"]) else {
        return;
    };
    let p = Project::new(NATBIB);
    let req = CompileRequest::new()
        .output_dir("build".into())
        .clean(Clean::Never);
    let c = compile(d, &p, &req).unwrap();
    assert_eq!(c.pdf, p.dir.path().join("build/doc.pdf"));
    pdf_ok(&c.pdf);
    assert!(p.exists("build/doc.bbl") && p.exists("build/doc.ind"));
}

#[test]
fn output_dir_outside_the_tex_directory_stops_bibtex_and_makeindex() {
    // TeX Live's default `openout_any = p` (paranoid) refuses to write
    // outside the working directory's tree; the crate does not change it.
    // The index pass runs before the bibliography, so each tool gets its own
    // document.
    let Some(d) = distribution("output_dir_outside", &["pdflatex", "bibtex", "makeindex"]) else {
        return;
    };
    let index_only = NATBIB
        .replace("\\citep{knuth}", "")
        .replace("\\bibliographystyle{plainnat}\n\\bibliography{refs}\n", "");
    let p = Project::new(&index_only);
    let outside = tempfile::tempdir().unwrap();
    let req = CompileRequest::new()
        .output_dir(outside.path().to_path_buf())
        .clean(Clean::Never);
    let err = compile(d.clone(), &p, &req).unwrap_err();
    match err {
        CompileError::Index { log_excerpt, .. } => assert!(log_excerpt.contains("openout_any")),
        other => panic!("makeindex refuses to write there: {other:?}"),
    }

    let bib_only = NATBIB
        .replace("\\usepackage{makeidx}\n\\makeindex\n", "")
        .replace("\\index{hello}", "")
        .replace("\\printindex\n", "");
    let p = Project::new(&bib_only);
    let outside = tempfile::tempdir().unwrap();
    let req = CompileRequest::new()
        .output_dir(outside.path().to_path_buf())
        .clean(Clean::Never);
    match compile(d, &p, &req).unwrap_err() {
        CompileError::Bibliography { log_excerpt, .. } => {
            assert!(log_excerpt.contains("openout_any"), "{log_excerpt}")
        }
        other => panic!("bibtex refuses to write there: {other:?}"),
    }
}

#[test]
fn the_other_engines_write_to_an_output_dir_outside_the_tex_directory() {
    // `-output-directory` exempts the engine itself from the limit.
    for engine in [Engine::PdfLatex, Engine::XeLatex, Engine::LuaLatex] {
        let program = engine.program();
        let Some(d) = distribution("engine_outside", &[program]) else {
            continue;
        };
        let p = Project::new("\\documentclass{article}\\begin{document}x\\end{document}\n");
        let outside = tempfile::tempdir().unwrap();
        let req = CompileRequest::new()
            .engine(engine)
            .output_dir(outside.path().to_path_buf())
            .clean(Clean::Never);
        let c = compile(d, &p, &req).unwrap_or_else(|e| panic!("{program}: {e:?}"));
        pdf_ok(&c.pdf);
        assert!(c.pdf.starts_with(outside.path()), "{program}");
    }
}

#[test]
fn driver_latexmk_builds_a_document_with_a_bibliography() {
    let Some(d) = distribution("driver_latexmk", &["latexmk", "pdflatex", "bibtex"]) else {
        return;
    };
    let p = Project::new(NATBIB);
    let req = CompileRequest::new()
        .driver(Driver::Latexmk)
        .clean(Clean::Never);
    let c = compile(d, &p, &req).unwrap();
    pdf_ok(&c.pdf);
    assert_eq!(c.runs, 1, "one delegated build");
    assert!(p.exists("doc.bbl"));
}

#[test]
fn engine_latexmk_builds_through_our_loop() {
    let Some(d) = distribution("engine_latexmk", &["latexmk", "pdflatex", "bibtex"]) else {
        return;
    };
    let p = Project::new(NATBIB);
    let req = CompileRequest::new()
        .engine(Engine::Latexmk)
        .bib(Bib::Infer)
        .clean(Clean::Never);
    let c = compile(d, &p, &req).unwrap();
    pdf_ok(&c.pdf);
}

#[test]
fn clean_and_keep_log_after_a_failing_document() {
    let Some(d) = distribution("failing_doc", &["pdflatex"]) else {
        return;
    };
    let bad = "\\documentclass{article}\\begin{document}\\undefinedcommand\\end{document}\n";

    let p = Project::new(bad);
    let err = compile(
        d.clone(),
        &p,
        &CompileRequest::new()
            .clean(Clean::Always)
            .keep_log(KeepLog::OnFailure),
    )
    .unwrap_err();
    match err {
        CompileError::Latex { summary, log, .. } => {
            assert!(
                summary
                    .unwrap_or_default()
                    .contains("Undefined control sequence")
            );
            assert_eq!(log, Some(p.dir.path().join("doc.log")));
        }
        other => panic!("{other:?}"),
    }
    assert!(p.exists("doc.log") && !p.exists("doc.aux"));

    let p = Project::new(bad);
    compile(
        d.clone(),
        &p,
        &CompileRequest::new().clean(Clean::OnSuccess),
    )
    .unwrap_err();
    assert!(
        p.exists("doc.aux") && p.exists("doc.log"),
        "a failure keeps everything"
    );

    let p = Project::new(bad);
    let err = compile(
        d,
        &p,
        &CompileRequest::new()
            .clean(Clean::Always)
            .keep_log(KeepLog::Never),
    )
    .unwrap_err();
    assert!(
        matches!(err, CompileError::Latex { log: None, .. }),
        "{err:?}"
    );
    assert!(!p.exists("doc.log"));
}

#[test]
fn a_stale_pdf_does_not_hide_a_failed_build() {
    let Some(d) = distribution("stale_pdf", &["pdflatex"]) else {
        return;
    };
    let p = Project::new(
        "\\documentclass{article}\\begin{document}\\undefinedcommand\\end{document}\n",
    );
    std::fs::write(p.dir.path().join("doc.pdf"), "%PDF-old").unwrap();
    compile(d, &p, &CompileRequest::new()).unwrap_err();
    assert!(!p.exists("doc.pdf"));
}

/// Needs a package TinyTeX does not ship; installs it. Runs only inside a
/// disposable copy (see the module comment).
#[test]
fn auto_install_recovers_from_a_missing_package() {
    let Some(sb) = sandbox::gate("auto_install_recovers_from_a_missing_package") else {
        return;
    };
    let d = sb.dist.clone();
    // Start from a tree without `lipsum`, so the test cannot pass on one that
    // already has it. `tlmgr remove` exits 1 when the package is absent, which
    // is fine here.
    let removed = sb
        .run("tlmgr", &["remove", "lipsum"])
        .expect("run tlmgr remove");
    assert!(
        matches!(removed.status.code(), Some(0 | 1)),
        "tlmgr remove lipsum: {:?}\n{}",
        removed.status,
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(
        kpsewhich(&sb, "lipsum.sty").is_empty(),
        "lipsum.sty must be absent before the compile"
    );

    let p = Project::new(
        "\\documentclass{article}\\usepackage{lipsum}\\begin{document}\\lipsum[1]\\end{document}\n",
    );
    let req = CompileRequest::new().auto_install(true);
    let c = Compiler::new(Some(d), Config::default())
        .compile(&p.tex(), &req, sb.exec(), &NullObserver)
        .unwrap();
    pdf_ok(&c.pdf);
    // Two independent signals that the package landed: the compile reports
    // it, and the tree now holds it.
    assert!(
        c.installed_packages.iter().any(|n| n == "lipsum"),
        "installed_packages: {:?}",
        c.installed_packages
    );
    assert!(
        !kpsewhich(&sb, "lipsum.sty").is_empty(),
        "kpsewhich finds lipsum.sty after the install"
    );
}

/// The path `kpsewhich` prints for `file`, or an empty string when it finds none.
fn kpsewhich(sb: &Sandbox, file: &str) -> String {
    let out = sb.run("kpsewhich", &[file]).expect("run kpsewhich");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
