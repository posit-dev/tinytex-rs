//! Tests for the pure log-parsing module (`tinytex::log`).
//!
//! Seeded from the union of two MIT-licensed suites, with the text copied here
//! so that nothing reads `external-sources/` at test time:
//!
//! * R package `tinytex` at `7818777` (`tests/test-cran/test-latex.R`: `font_ext`,
//!   `detect_files`, `needs_rerun`), plus `R/hyphen.R` from R `main` at
//!   `e78c2f7` for the language map. Copyright (c) Yihui Xie and Posit
//!   Software, PBC. MIT.
//! * Quarto 1 at `v1.12.3` (`tests/unit/latexmk/parse-error.test.ts` and
//!   `expl3-aborted.log`, quarto-dev/quarto#7417). Copyright (c) Posit
//!   Software, PBC. MIT.
//!
//! Tests for `accessibility`, `index_error`, `missing_modules`,
//! `latex_warnings` and the error extraction have no upstream test and are
//! written for this crate.

use tinytex::log::{
    AccessibilityWarnings, Hint, SearchTerm, accessibility, index_error, latex_error,
    latex_warnings, missing_hyphenation, missing_modules, missing_packages, needs_rerun,
};

const FONT_TAIL: &str = "(-(Bold|Italic|Regular).*)?[.](tfm|afm|mf|otf|ttf)";

/// The search term for a font with no extension (R `font_ext`, Quarto 1
/// `fontSearchTerm`): runs of spaces become `\s*`.
fn font_term(font: &str) -> String {
    format!(
        "{}{FONT_TAIL}",
        font.split_whitespace().collect::<Vec<_>>().join("\\s*")
    )
}

fn file(name: &str) -> SearchTerm {
    SearchTerm::File(format!("/{name}"))
}

fn pkg(name: &str) -> SearchTerm {
    SearchTerm::Package(name.to_string())
}

/// Asserts that the first term found in `text` is the file search for `expected`.
fn assert_found(text: &str, expected: &str) {
    let found = missing_packages(text, None);
    assert_eq!(
        found.first(),
        Some(&file(expected)),
        "expected `/{expected}` first in {text:?}, got {found:?}"
    );
}

const EXPL3: &str = include_str!("../fixtures/log/expl3-aborted.log");
const MULTI_ERROR: &str = include_str!("../fixtures/log/multi-error.log");

// ---------------------------------------------------------------- fonts and files

#[test]
fn nothing_found_in_unrelated_text() {
    assert!(missing_packages("asdf qwer", None).is_empty());
    assert!(missing_packages("", None).is_empty());
}

#[test]
fn expl3_mismatch_names_the_package() {
    // quarto-dev/quarto#7417
    assert_found(EXPL3, "expl3.sty");
}

#[test]
fn font_errors_become_font_search_terms() {
    assert_found(
        "! Font U/psy/m/n/10=psyr at 10.0pt not loadable: Metric (TFM) file not found",
        &font_term("psyr"),
    );
    assert_found(
        "! The font \"FandolSong-Regular\" cannot be found.",
        &font_term("FandolSong-Regular"),
    );
    assert_found(
        "! Package fontspec Error: The font \"Caladea\" cannot be found.",
        &font_term("Caladea"),
    );
    assert_found(
        "!pdfTeX error: /usr/local/bin/pdflatex (file tcrm0700): Font tcrm0700 at 600 not found",
        &font_term("tcrm0700"),
    );
    assert_found(
        "(fontspec)                The font \"LibertinusSerif-Regular\" cannot be",
        &font_term("LibertinusSerif-Regular"),
    );
    // R #478, Quarto 1: spaces in font names
    assert_found(
        "! The font \"Noto Emoji\" cannot be found.",
        &font_term("Noto Emoji"),
    );
    assert_found(
        "! Package fontspec Error: The font \"DejaVu Sans\" cannot be found.",
        &font_term("DejaVu Sans"),
    );
    assert_eq!(
        font_term("Noto Emoji"),
        "Noto\\s*Emoji(-(Bold|Italic|Regular).*)?[.](tfm|afm|mf|otf|ttf)"
    );
    // xdvipdfmx
    assert_found(
        "xdvipdfmx:fatal: Unable to find TFM file \"rsfs10\"",
        &font_term("rsfs10"),
    );
}

#[test]
fn font_with_extension_is_searched_as_a_file() {
    // R #465, Quarto 1 #12194: strip the `file:` prefix and the `:-kern;...` suffix
    assert_found(
        "! Font \\JY3/mc/m/n/10=file:HaranoAjiMincho-Regular.otf:-kern;jfm=ujis at 9.24713pt not loadable: metric data not found or bad.",
        "HaranoAjiMincho-Regular.otf",
    );
}

#[test]
fn luaotfload_fallback_font_strips_the_fallback_suffix() {
    // Quarto 1 only: quarto-dev/quarto-cli#14558
    assert_found(
        "luaotfload | db : Reload initiated (formats: otf,ttf,ttc); reason: Font \"DejaVu Sans Mono;-fallback\" not found.",
        &font_term("DejaVu Sans Mono"),
    );
}

#[test]
fn missing_files_become_file_search_terms() {
    assert_found("! LaTeX Error: File `framed.sty' not found.", "framed.sty");
    assert_found("! LaTeX Error: File 'framed.sty' not found.", "framed.sty");
    assert_found(
        "/usr/local/bin/mktexpk: line 123: mf: command not found",
        "mf",
    );
    assert_found(
        "or the language definition file ngerman.ldf was not found",
        "ngerman.ldf",
    );
    assert_found(
        "! Package babel Error: Unknown option 'english'.",
        "english.ldf",
    );
    assert_found(
        "!pdfTeX error: pdflatex (file 8r.enc): cannot open encoding file for reading",
        "8r.enc",
    );
    // the `(file ...): cannot open` matcher, Quarto 1 #15385: a real line, no `!` prefix
    assert_found(
        "pdflatex (file 8r.enc): cannot open encoding file for reading",
        "8r.enc",
    );
    assert_found(
        "Package widetext error: Install the flushend package which is a part of sttools",
        "flushend.sty",
    );
    assert_found(
        "! Package isodate.sty Error: Package file substr.sty not found.",
        "substr.sty",
    );
    assert_found(
        "! Package fontenc Error: Encoding file `t2aenc.def' not found.",
        "t2aenc.def",
    );
    assert_found(
        "! I can't find file `hyph-de-1901.ec.tex'.",
        "hyph-de-1901.ec.tex",
    );
    assert_found(
        "! I can't find file 'hyph-de-1901.ec.tex'.",
        "hyph-de-1901.ec.tex",
    );
    assert_found(
        "luaotfload-features.lua:835: module 'lua-uni-normalize' not found:",
        "lua-uni-algos.lua",
    );
    assert_found(
        "! LuaTeX-ja error: File 'jfm-zh_CN.lua' not found.",
        "jfm-zh_CN.lua",
    );
    assert_found(
        "Package biblatex Info: ... file 'trad-abbrv.bbx' not found",
        "trad-abbrv.bbx",
    );
    assert_found(
        "support file `supp-pdf.mkii' (supp-pdf.tex) is missing",
        "supp-pdf.mkii",
    );
    assert_found(
        "support file 'supp-pdf.mkii' (supp-pdf.tex) is missing",
        "supp-pdf.mkii",
    );
    assert_found(
        "No file LGRcmr.fd. ! LaTeX Error: This NFSS system isn't set up properly.",
        "lgrcmr.fd",
    );
    assert_found(
        "! Package babel Error: Unknown option 'ngerman'.\n(babel)                Suggested actions:\n(babel)                * Make sure you haven't misspelled it",
        "ngerman.ldf",
    );
}

#[test]
fn babel_unknown_option_with_continuation_lines() {
    assert_found(
        "Package babel Error: Unknown option 'ngerman'. Either you misspelled it\n    (babel)                or the language definition file ngerman.ldf\n    (babel)                was not found.\n    (babel)                There is a locale ini file for this language.",
        "ngerman.ldf",
    );
}

#[test]
fn epstopdf_and_tikz_and_pdfx() {
    assert_found(
        "! Package pdftex.def Error: File `logo-mdpi-eps-converted-to.pdf' not found",
        "epstopdf",
    );
    assert_found(
        "! Package pdftex.def Error: File 'logo-mdpi-eps-converted-to.pdf' not found",
        "epstopdf",
    );
    assert_found(
        "! xdvipdfmx:fatal: pdf_ref_obj(): passed invalid object.",
        "epstopdf",
    );
    assert_found(
        "! Package tikz Error: I did not find the tikz library 'hobby'. This error message was issued because the library or one of its sublibraries could not be found, probably because of a misspelling. Processed options: \"library={hobby}\". The possibly misspelled library name is \"hobby\". The library name should be one of the following (or you misspelled it): named tikzlibraryhobby.code.tex",
        "tikzlibraryhobby.code.tex",
    );
    assert_found(
        "! Package pdfx Error: No color profile sRGB_IEC61966-2-1_black_scaled.icc found",
        "colorprofiles.sty",
    );
}

#[test]
fn tikz_library_file_needs_a_tikz_error() {
    // R: the tikz matcher only counts when the log has `! Package tikz Error:`
    assert!(missing_packages(" tikzlibraryhobby.code.tex", None).is_empty());
}

#[test]
fn shortcut_packages_are_named_directly() {
    // Quarto 1 and R: these need no `tlmgr search`
    assert_eq!(
        missing_packages(
            "! CTeX fontset `fandol' is unavailable in current mode",
            None
        ),
        vec![pkg("fandol")]
    );
    assert_eq!(
        missing_packages(
            "! CTeX fontset 'fandol' is unavailable in current mode",
            None
        ),
        vec![pkg("fandol")]
    );
    assert_eq!(
        missing_packages("No support files for \\DocumentMetadata found.", None),
        vec![pkg("latex-lab")]
    );
    // Quarto 1: the PDF/A backend error names the package, R names its `.sty`
    assert_eq!(
        missing_packages("(pdf backend): cannot open file for embedding", None),
        vec![pkg("colorprofiles")]
    );
}

#[test]
fn other_ctex_fontsets_are_searched() {
    assert_found(
        "! CTeX fontset `founder' is unavailable in current mode",
        "founder",
    );
}

#[test]
fn missing_bst_in_a_blg_log() {
    // quarto-dev/quarto-cli#15376, #15377: bibtex writes the next log line
    // right after the style file name
    let blg = "This is BibTeX, Version 0.99e (TeX Live 2026)\nThe top-level auxiliary file: doc.aux\nI couldn't open style file agsm.bst\n---line 7 of file doc.aux\n : \\bibstyle{agsm\n :               }\nI'm skipping whatever remains of this command\nI found no style file---while reading file doc.aux\n(There were 2 error messages)\n";
    assert_eq!(missing_packages(blg, None), vec![file("agsm.bst")]);
}

#[test]
fn l3backend_mismatch_uses_the_last_file_line_before_an_undefined_control_sequence() {
    // R only (#424)
    let log = "File: l3backend-pdftex.def 2024-02-20 L3 backend support: PDF output (pdfTeX)\n! Undefined control sequence.\nl.1 \\foo\n";
    assert_eq!(
        missing_packages(log, None),
        vec![file("l3backend-pdftex.def")]
    );
    // ... and not without the error
    let ok = "File: l3backend-pdftex.def 2024-02-20 L3 backend support: PDF output (pdfTeX)\n";
    assert!(missing_packages(ok, None).is_empty());
}

#[test]
fn results_are_deduplicated_in_first_seen_order() {
    let log = "! LaTeX Error: File `a.sty' not found.\n! LaTeX Error: File `b.sty' not found.\n! LaTeX Error: File `a.sty' not found.\n";
    assert_eq!(
        missing_packages(log, None),
        vec![file("a.sty"), file("b.sty")]
    );
}

#[test]
fn crlf_logs_parse_like_lf_logs() {
    let log = "! LaTeX Error: File `a.sty' not found.\r\n! I can't find file `b.tex'.\r\n";
    assert_eq!(
        missing_packages(log, None),
        vec![file("a.sty"), file("b.tex")]
    );
}

#[test]
fn missfont_log_adds_fonts_and_commands() {
    // R `miss_font()`: the font is the last word of a line, the command that
    // makes it the first
    let miss = "mktexpk --mfmode / --bdpi 600 --mag 1+0/600 --dpi 600 ecrm0900\n";
    assert_eq!(
        missing_packages("", Some(miss)),
        vec![file(&font_term("ecrm0900")), file("mktexpk")]
    );
    // log terms come first; lines without two words are ignored
    let found = missing_packages("! LaTeX Error: File `a.sty' not found.", Some("single\n\n"));
    assert_eq!(found, vec![file("a.sty")]);
}

#[test]
fn search_term_display_strips_the_leading_slash() {
    assert_eq!(file("pgfplots.sty").to_string(), "pgfplots.sty");
    assert_eq!(pkg("latex-lab").to_string(), "latex-lab");
}

// ---------------------------------------------------------------- rerun detection

#[test]
fn rerun_is_detected() {
    // R `needs_rerun` cases
    assert!(needs_rerun("Package biblatex Warning: Please rerun LaTeX."));
    assert!(needs_rerun("Please (re)run the file"));
    assert!(needs_rerun("error: Rerun LaTeX."));
    assert!(needs_rerun("Rerun to get the final file"));
    // Quarto 1: a beamer run with lualatex (#6226)
    assert!(needs_rerun("No file doc.aux.\n"));
    assert!(!needs_rerun("nothing to do here"));
    assert!(!needs_rerun(""));
}

#[test]
fn marginnote_rerun_resolves_later_in_the_log() {
    let resolved = include_str!("../fixtures/log/rerun-resolved.log");
    assert!(!needs_rerun(resolved));
    let unresolved = include_str!("../fixtures/log/rerun-unresolved.log");
    assert!(needs_rerun(unresolved), "line 214 never resolves");
}

// ---------------------------------------------------------------- hyphenation

fn babel_warning(lang: &str) -> String {
    format!(
        "Package babel Warning: No hyphenation patterns were preloaded for\n(babel)                the language '{lang}' into the format.\n(babel)                Please, configure your TeX system to add them and\n(babel)                rebuild the format. Now I will use the patterns\n(babel)                preloaded for \\language=0 instead on input line 51."
    )
}

fn babel_fatal(lang: &str) -> String {
    format!("! Package babel Error: You haven't defined the language '{lang}' yet.")
}

#[test]
fn babel_warning_quote_styles() {
    let backtick = "Package babel Warning: No hyphenation patterns were preloaded for\n(babel)                the language `Spanish' into the format.\n(babel)                Please, configure your TeX system to add them and";
    assert_eq!(missing_hyphenation(backtick), vec!["hyphen-spanish"]);
    assert_eq!(
        missing_hyphenation(&babel_warning("Spanish")),
        vec!["hyphen-spanish"]
    );
    assert_eq!(
        missing_hyphenation(&babel_warning("ngerman")),
        vec!["hyphen-german"]
    );
}

#[test]
fn languages_without_hyphenation_files_are_skipped() {
    assert!(missing_hyphenation(&babel_warning("chinese")).is_empty());
    // the Info form, quarto-dev/quarto-cli#10291
    let info = "Package babel Info: Hyphen rules for 'chinese-hans' set to \\l@nil\n(babel)             (\\language10). Reported on input line 143.";
    assert!(missing_hyphenation(info).is_empty());
    assert!(missing_hyphenation("Some other log text without babel warnings").is_empty());
    assert!(missing_hyphenation("asdf qwer").is_empty());
}

#[test]
fn hyphen_rules_info_is_the_fallback() {
    let info = "Package babel Info: Hyphen rules for 'nswissgerman' set to \\l@nil\n(babel)             (\\language5). Reported on input line 1260.";
    assert_eq!(missing_hyphenation(info), vec!["hyphen-nswissgerman"]);
}

#[test]
fn every_warned_language_is_returned() {
    let babel = format!(
        "{}\n\n{}",
        babel_warning("Spanish"),
        babel_warning("Italian")
    );
    assert_eq!(
        missing_hyphenation(&babel),
        vec!["hyphen-spanish", "hyphen-italian"]
    );

    // polyglossia, a real xelatex log with \setotherlanguages{spanish,italian}
    let polyglossia = "Package polyglossia Warning: No hyphenation patterns were loaded for `spanish'\n(polyglossia)                I will use \\language=\\l@nohyphenation instead.\n\nPackage polyglossia Warning: No hyphenation patterns were loaded for `italian'\n(polyglossia)                I will use \\language=\\l@nohyphenation instead.";
    assert_eq!(
        missing_hyphenation(polyglossia),
        vec!["hyphen-spanish", "hyphen-italian"]
    );
    assert_eq!(
        missing_hyphenation(
            "Package polyglossia Warning: No hyphenation patterns were loaded for British English"
        ),
        vec!["hyphen-english"]
    );
    // the same package once, even from both warning kinds
    assert_eq!(
        missing_hyphenation(&format!("{babel}\n\n{polyglossia}")),
        vec!["hyphen-spanish", "hyphen-italian"]
    );
    // two languages, one package
    let indic = format!("{}\n\n{}", babel_warning("hindi"), babel_warning("tamil"));
    assert_eq!(missing_hyphenation(&indic), vec!["hyphen-indic"]);
}

#[test]
fn description_languages_use_the_first_word() {
    // babel-german names the patterns by description
    let log = "Package babel Warning: No hyphenation patterns were preloaded for\n(babel)                the language 'German (current orthography)' into the format.\n(babel)                Please, configure your TeX system to add them and";
    assert_eq!(missing_hyphenation(log), vec!["hyphen-german"]);
}

#[test]
fn only_warning_blocks_are_searched() {
    let info = "Package babel Info: No hyphenation patterns were set for\n(babel)             language 'nohyphenation'. Reported on input line 12.";
    assert!(missing_hyphenation(info).is_empty());
    let with_warning = format!(
        "Package babel Warning: Something else\n(babel)                on input line 3.\n\n{info}"
    );
    assert!(missing_hyphenation(&with_warning).is_empty());
}

#[test]
fn swiss_german_warning_and_fatal_error() {
    // https://github.com/rstudio/tinytex/issues/515: `lang: de-CH` without hyphen-german
    let log = "Package babel Warning: No hyphenation patterns were preloaded for\n(babel)                the language 'German (current orthography), needed by Swiss Standard German (current orthography)' into the format.\n(babel)                Please, configure your TeX system to add them and\n(babel)                rebuild the format. Now I will use the patterns\n(babel)                preloaded for \\language=0 instead on input line 1260.\n\nPackage babel Info: Hyphen rules for 'nswissgerman' set to \\l@nil\n(babel)             (\\language5). Reported on input line 1260.\n\n! Package babel Error: You haven't defined the language 'ngerman' yet.\n(babel)                Perhaps you misspelled it or your installation\n(babel)                is not complete.\n\nSee the babel package documentation for explanation.\nType  H <return>  for immediate help.";
    // the fatal error is returned through `missing_packages`, by package name
    assert_eq!(missing_packages(log, None), vec![pkg("hyphen-german")]);
    // the warning names a description: the first word, not the `\l@nil` name
    assert_eq!(missing_hyphenation(log), vec!["hyphen-german"]);
}

#[test]
fn fatal_babel_error_quote_forms() {
    for log in [
        "! Package babel Error: You haven't defined the language `ngerman' yet.",
        "! Package babel Error: You haven't defined the language ngerman yet.",
        "! Package babel Error: You haven't defined the language 'ngerman' yet.",
    ] {
        assert_eq!(
            missing_packages(log, None),
            vec![pkg("hyphen-german")],
            "{log}"
        );
    }
    assert_eq!(
        missing_packages(&babel_fatal("Spanish"), None),
        vec![pkg("hyphen-spanish")]
    );
    assert!(missing_packages(&babel_fatal("chinese-hans"), None).is_empty());
    // a warning alone is not the fatal error
    assert!(missing_packages(&babel_warning("Spanish"), None).is_empty());
    assert_eq!(
        missing_packages(&babel_fatal("UKenglish"), None),
        vec![pkg("hyphen-english")]
    );
    assert_eq!(
        missing_packages(
            &format!("{}\n{}", babel_fatal("hindi"), babel_fatal("tamil")),
            None
        ),
        vec![pkg("hyphen-indic")]
    );
}

#[test]
fn pinned_german_patterns_map_to_dehyph_exptl() {
    let log = "! Package babel Error: You haven't defined the language 'ngerman-x-latest' yet.\n(babel)                Perhaps you misspelled it or your installation\n(babel)                is not complete.\n\nSee the babel package documentation for explanation.\nType  H <return>  for immediate help.\n ...\n\nl.185 \\begin{hyphenrules}{ngerman-x-latest}";
    assert_eq!(missing_packages(log, None), vec![pkg("dehyph-exptl")]);
}

#[test]
fn language_names_that_are_not_package_names_are_dropped() {
    // the names reach `tlmgr` (through a .bat file on Windows)
    for lang in [
        "x\"&calc",
        "ngerman;id",
        "%PATH%",
        "a|b",
        "a^b",
        "--repository=x",
    ] {
        assert!(
            missing_packages(&babel_fatal(lang), None).is_empty(),
            "fatal {lang}"
        );
        assert!(
            missing_hyphenation(&babel_warning(lang)).is_empty(),
            "warning {lang}"
        );
    }
    assert_eq!(
        missing_packages(
            &format!("{}\n{}", babel_fatal("x\"&calc"), babel_fatal("french")),
            None
        ),
        vec![pkg("hyphen-french")]
    );
}

/// `R/hyphen.R` from R `main` at `e78c2f7`, which both the warning and the
/// fatal-error paths share (the same table is in Quarto 1 `v1.12.3`).
const HYPHEN_MAP: &str = "\
amharic hyphen-ethiopic
assamese hyphen-indic
bengali hyphen-indic
bokmal hyphen-norwegian
british hyphen-english
classiclatin hyphen-latin
espanol hyphen-spanish
francais hyphen-french
geez hyphen-ethiopic
german-x-2024-02-28 dehyph-exptl
german-x-latest dehyph-exptl
gujarati hyphen-indic
hindi hyphen-indic
ibycus hyphen-ancientgreek
kannada hyphen-indic
liturgicallatin hyphen-latin
malayalam hyphen-indic
marathi hyphen-indic
mongolianlmc hyphen-mongolian
monogreek hyphen-greek
ngerman hyphen-german
ngerman-x-2024-02-28 dehyph-exptl
ngerman-x-latest dehyph-exptl
norsk hyphen-norwegian
nynorsk hyphen-norwegian
oriya hyphen-indic
pali hyphen-indic
panjabi hyphen-indic
patois hyphen-french
persian hyphen-farsi
pinyin hyphen-chinese
polygreek hyphen-greek
portuges hyphen-portuguese
schoolfinnish hyphen-finnish
serbianc hyphen-serbian
slovene hyphen-slovenian
swissgerman hyphen-german
tamil hyphen-indic
telugu hyphen-indic
ukenglish hyphen-english
usenglishmax hyphen-english
";

#[test]
fn language_map_covers_the_warning_and_the_fatal_error() {
    let mut n = 0;
    for line in HYPHEN_MAP.lines() {
        let (lang, expected) = line.split_once(' ').unwrap();
        n += 1;
        assert_eq!(
            missing_packages(&babel_fatal(lang), None),
            vec![pkg(expected)],
            "fatal error for {lang}"
        );
        assert_eq!(
            missing_hyphenation(&babel_warning(lang)),
            vec![expected.to_string()],
            "warning for {lang}"
        );
    }
    assert_eq!(n, 41);
}

// ---------------------------------------------------------------- modules and warnings

#[test]
fn missing_modules_after_a_successful_run() {
    // R `check_extra`
    let glossaries = "Package glossaries Warning: No language module detected for `english'.\n(glossaries)                Language modules need to be installed separately.\n(glossaries)                Please check on CTAN for a bundle called\n(glossaries)                `glossaries-english' or similar.\n";
    assert_eq!(missing_modules(glossaries), vec!["glossaries-english"]);
    let datetime2 = "Package datetime2 Warning: Date-Time Language Module `english' not installed on\n(datetime2)  input line 5.\n";
    assert_eq!(missing_modules(datetime2), vec!["datetime2-english"]);
    assert_eq!(
        missing_modules(&babel_warning("Spanish")),
        vec!["hyphen-spanish"]
    );
    let polyglossia = "Package polyglossia Warning: No hyphenation patterns were loaded for `hungarian'\n(polyglossia)                I will use \\language=\\l@nohyphenation instead.";
    assert_eq!(missing_modules(polyglossia), vec!["hyphen-hungarian"]);
    assert!(missing_modules("all fine\n").is_empty());
    // the shared language table applies
    assert_eq!(
        missing_modules(&babel_warning("pinyin")),
        vec!["hyphen-chinese"]
    );
}

#[test]
fn latex_warning_blocks_end_at_a_blank_line() {
    let log = "(./doc.tex\nLaTeX Warning: Reference `x' on page 1 undefined on input line 3.\n\nPackage foo Warning: first line\n(foo)  second line\n\nPackage hyperref Warning: ignored\n\nplain text\n";
    assert_eq!(
        latex_warnings(log),
        vec![
            "LaTeX Warning: Reference `x' on page 1 undefined on input line 3.",
            "Package foo Warning: first line",
            "(foo)  second line",
        ]
    );
    assert!(latex_warnings("nothing\n").is_empty());
}

// ---------------------------------------------------------------- accessibility and index

#[test]
fn accessibility_warnings_are_structured() {
    let log = "Package tagpdf Warning: Alternative text for graphic is missing.\n(tagpdf)                Using 'figure1.png' instead.\n\nPackage tagpdf Warning: Alternative text for graphic is missing.\n(tagpdf)                Using `a-very-long-\n(tagpdf)                name.png' instead.\n\nLaTeX DocumentMetadata Warning: The language has not been set in\n(DocumentMetadata)  the \\DocumentMetadata declaration.\n\nPackage tagpdf Warning: Something else happened\n";
    let got = accessibility(log);
    // a name that tagpdf wraps across continuation lines is rejoined
    assert_eq!(
        got.missing_alt_text,
        vec!["figure1.png", "a-very-long-name.png"]
    );
    assert!(got.missing_language);
    assert_eq!(got.other_warnings, vec!["Something else happened"]);
    assert_eq!(accessibility("clean log"), AccessibilityWarnings::default());
}

#[test]
fn accessibility_alt_text_may_wrap_before_instead() {
    let log = "Package tagpdf Warning: Alternative text for graphic is missing.\n(tagpdf)                Using `figure.png'\n(tagpdf)                instead.\n";
    assert_eq!(accessibility(log).missing_alt_text, vec!["figure.png"]);
}

#[test]
fn index_error_is_the_first_dash_line() {
    let log = "This is makeindex\nScanning input file doc.idx...done.\n   -- Input index error (file = doc.idx, line = 3):\n   -- Unmatched closing brace.\n";
    assert_eq!(
        index_error(log).as_deref(),
        Some("Input index error (file = doc.idx, line = 3):")
    );
    assert_eq!(index_error("no problem"), None);
}

// ---------------------------------------------------------------- error extraction

#[test]
fn error_extraction_is_one_block_per_bang_line() {
    let e = latex_error(MULTI_ERROR, None).expect("errors");
    // each block ends at a blank line; the final fatal-error line is skipped
    let expected = "Undefined control sequence.\nl.5 \\foo\n\nEmergency stop.\nl.5 \\foo\n\nMissing $ inserted.\n<inserted text> \n                $\nl.9 a_b\n\nLaTeX Error: Environment bogus undefined.";
    assert_eq!(e.summary, expected);
    assert!(!e.summary.contains("Fatal error"));
    assert!(!e.summary.contains("Here is how much"));
    assert!(e.summary.lines().count() < 20, "bounded: {}", e.summary);
    assert_eq!(e.hints, vec![Hint::InlineMath]);
}

#[test]
fn error_blocks_stop_at_the_memory_line() {
    let log = "! Emergency stop.\nl.1 x\nHere is how much of TeX's memory you used:\n 10 strings\n";
    assert_eq!(
        latex_error(log, None).unwrap().summary,
        "Emergency stop.\nl.1 x"
    );
}

#[test]
fn the_last_line_may_be_an_error() {
    assert_eq!(latex_error("! Oops.", None).unwrap().summary, "Oops.");
}

#[test]
fn no_error_means_none() {
    assert_eq!(latex_error("everything is fine\n", None), None);
    assert_eq!(latex_error("", None), None);
    // the only `!` line is the fatal-error trailer
    assert_eq!(
        latex_error(
            "!  ==> Fatal error occurred, no output PDF file produced!\n",
            None
        ),
        None
    );
}

#[test]
fn empty_output_is_reported_when_no_block_exists() {
    // Quarto 1's `No pages of output` fallback
    let e = latex_error("Output written on doc.dvi.\nNo pages of output.\n", None).unwrap();
    assert_eq!(
        e.summary,
        "No pages of output - the document appears to have produced no output."
    );
    assert!(e.hints.is_empty());
}

#[test]
fn unicode_hint() {
    // Quarto 1: `! Package inputenc Error: Unicode character`; R also accepts `! LaTeX Error:`
    for head in [
        "! Package inputenc Error: Unicode character € (U+20AC)",
        "! LaTeX Error: Unicode character € (U+20AC)",
    ] {
        let log = format!(
            "{head}\n               not set up for use with LaTeX.\n\nSee the inputenc package documentation for explanation.\n"
        );
        let e = latex_error(&log, None).unwrap();
        assert_eq!(e.hints, vec![Hint::UnsupportedUnicode], "{head}");
    }
}

#[test]
fn ghostscript_hints_come_from_stderr() {
    let log = "! Emergency stop.\nl.1 x\n";
    assert!(latex_error(log, None).unwrap().hints.is_empty());
    let e = latex_error(
        log,
        Some("!!! Error: Cannot open Ghostscript for piped input\n"),
    )
    .unwrap();
    assert_eq!(e.hints, vec![Hint::Ghostscript]);
    let e = latex_error(
        log,
        Some("GPL Ghostscript 10.02.1: Can't find initialization file gs_init.ps.\n"),
    )
    .unwrap();
    assert_eq!(e.hints, vec![Hint::GhostscriptMisconfigured]);
    // the same text in the log (not stderr) does not count
    assert!(
        latex_error(
            "! Emergency stop.\n!!! Error: Cannot open Ghostscript for piped input\n",
            None
        )
        .unwrap()
        .hints
        .is_empty()
    );
}

#[test]
fn luaotfload_fallback_crash_has_no_bang_block() {
    // quarto-dev/quarto-cli#14558, latex3/luaotfload#331
    let log = "luaotfload | db : Reload initiated (formats: otf,ttf,ttc); reason: Font \"Latin Modern Roman;-fallback\" not found.\nluaotfload | resolve : sequence of 3 lookups yielded nothing appropriate....texmf-dist/tex/luatex/luaotfload/luaotfload-fallback.lua:50: attempt to index a nil value (local 'f').\n!  ==> Fatal error occurred, no output PDF file produced!";
    let e = latex_error(log, None).unwrap();
    assert!(e.summary.contains("font fallback"), "{}", e.summary);
    assert!(e.summary.contains("luaotfload/issues/331"), "{}", e.summary);
    assert!(!e.summary.contains('\n'), "one line");
    assert_eq!(e.hints, vec![Hint::LuaotfloadFallback]);
}

#[test]
fn log_types_round_trip_through_json() {
    let e = latex_error(MULTI_ERROR, None).unwrap();
    let json = serde_json::to_string(&e).unwrap();
    let back: tinytex::log::LatexError = serde_json::from_str(&json).unwrap();
    assert_eq!(back, e);
    let term = file("pgfplots.sty");
    let back: SearchTerm = serde_json::from_str(&serde_json::to_string(&term).unwrap()).unwrap();
    assert_eq!(back, term);
    let a = AccessibilityWarnings {
        missing_alt_text: vec!["a.png".into()],
        missing_language: true,
        other_warnings: vec![],
    };
    let back: AccessibilityWarnings =
        serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
    assert_eq!(back, a);
}
