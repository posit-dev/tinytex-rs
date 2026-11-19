//! Pure functions over TeX log text: no IO, no processes.
//!
//! The matchers are the union of two MIT-licensed implementations, whose
//! tables are close to identical (design section 3.1):
//!
//! * the R package `tinytex` at `7818777` (`R/latex.R`: `detect_files`,
//!   `needs_rerun`, `latex_warning`, `check_extra`, `show_latex_error`), plus
//!   its language map `R/hyphen.R` (R `main`, `e78c2f7`);
//! * Quarto 1 at `v1.12.3` (`src/command/render/latexmk/parse-error.ts`).
//!
//! Matchers that only Quarto 1 or only R has are in labeled groups below. A
//! log is searched as text, so every function normalizes `\r\n` to `\n`
//! first.
//!
//! The caller words the messages: this module returns structured values
//! ([`Hint`], [`AccessibilityWarnings`], [`SearchTerm`]), not prose, except for
//! [`LatexError::summary`], which is the engine's own text.

use std::borrow::Cow;
use std::fmt;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Compiles a regular expression once, on first use.
macro_rules! re {
    ($pattern:expr) => {{
        static RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new($pattern).expect("a valid built-in regex"));
        &*RE
    }};
}

/// What [`missing_packages`] returns: an argument for
/// `tlmgr search --file --global` (for example `/pgfplots.sty`), or a package
/// named directly (the shortcuts `fandol`, `latex-lab` and `colorprofiles`,
/// and the hyphenation packages of a fatal `babel` error) that needs no
/// search.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SearchTerm {
    /// A file name, or a regular expression for one (fonts), with the leading
    /// `/` that anchors it at a path component.
    File(String),
    /// A package name that needs no `tlmgr search`.
    Package(String),
}

impl fmt::Display for SearchTerm {
    /// The bare name: a `File` term loses its leading `/` (`pgfplots.sty`),
    /// so a host can name the missing package from it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SearchTerm::File(file) => f.write_str(file.strip_prefix('/').unwrap_or(file)),
            SearchTerm::Package(package) => f.write_str(package),
        }
    }
}

/// A structured hint attached to a [`LatexError`]. The wording belongs to the
/// host (Quarto 1's text is in `suggestHint`, `parse-error.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Hint {
    /// A Unicode character the engine's input encoding cannot typeset; try
    /// another engine such as XeLaTeX.
    UnsupportedUnicode,
    /// `Missing $ inserted`: some text needs math delimiters.
    InlineMath,
    /// The engine could not start Ghostscript (read from `stderr`).
    Ghostscript,
    /// Ghostscript started but cannot find its initialization file (read from
    /// `stderr`).
    GhostscriptMisconfigured,
    /// The `luaotfload` font-fallback crash; the summary carries its text.
    LuaotfloadFallback,
}

/// The extracted error of a failed run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatexError {
    /// One bounded block per `!` line, in log order: each block runs from the
    /// `!` line (without its `! `) to the line before the next blank line or
    /// TeX's `Here is how much of TeX's memory you used:` line, and blocks are
    /// separated by one blank line. The trailing `==> Fatal error occurred`
    /// line is not a block.
    ///
    /// When the log has no `!` block, the summary is a single line of text:
    /// either the `luaotfload` fallback crash message
    /// (`luaotfload-fallback.lua`, which Quarto 1 returns before the `!`
    /// extraction) or, for a run that wrote no pages, `No pages of output`
    /// with an explanation. Hint wording belongs to the host.
    pub summary: String,
    /// Structured hints, stderr-derived ones first.
    pub hints: Vec<Hint>,
}

/// Why a package install failed: Quarto 1 tells these apart (`pdf.ts:483,490,498`,
/// `texlive.ts:392-399`) and the host words each differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InstallFailure {
    /// `tlmgr search` found no package for what the log asks for.
    NoMatch,
    /// `tlmgr install` failed, or left a package missing.
    InstallFailed,
    /// The local TeX Live is older than the remote repository.
    TexLiveTooOld,
}

/// The accessibility (PDF/UA) warnings of a run, Quarto 1's shape
/// (`parse-error.ts`, `PdfAccessibilityWarnings`). The caller words them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityWarnings {
    /// File names of images that lack alternative text.
    pub missing_alt_text: Vec<String>,
    /// The document language is not set.
    pub missing_language: bool,
    /// Other `tagpdf` warnings, verbatim.
    pub other_warnings: Vec<String>,
}

fn normalize(text: &str) -> Cow<'_, str> {
    if text.contains('\r') {
        Cow::Owned(text.replace("\r\n", "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

fn push_unique<T: PartialEq>(into: &mut Vec<T>, value: T) {
    if !into.contains(&value) {
        into.push(value);
    }
}

// ------------------------------------------------------------------ missing packages

/// Packages that `tlmgr search` cannot find by file name, named directly
/// (Quarto 1's shortcut list, a superset of R's `fandol` and `latex-lab`).
const SHORTCUT_PACKAGES: [&str; 3] = ["fandol", "latex-lab", "colorprofiles"];

fn search_term(name: String) -> SearchTerm {
    if SHORTCUT_PACKAGES.contains(&name.as_str()) {
        SearchTerm::Package(name)
    } else {
        SearchTerm::File(format!("/{name}"))
    }
}

/// The search pattern for a font name with no extension (R `font_ext`, Quarto
/// 1 `fontSearchTerm`): runs of whitespace become `\s*`.
fn font_pattern(font: &str) -> String {
    let font = re!(r"\s+").replace_all(font, regex::NoExpand(r"\s*"));
    format!("{font}(-(Bold|Italic|Regular).*)?[.](tfm|afm|mf|otf|ttf)")
}

/// Fonts: strip a directory, then the `file:` prefix and `:-kern;...` suffix
/// (R #465, Quarto 1 #12194); a name with an extension is a file already.
fn font_term(matched: &str) -> String {
    let base = matched.rsplit(['/', '\\']).next().unwrap_or(matched);
    let base = re!(r"^.*?:|:.*$").replace_all(base, "");
    if base.contains('.') {
        base.into_owned()
    } else {
        font_pattern(&base)
    }
}

/// What a matcher does with its first capture group.
#[derive(Clone, Copy)]
enum Filter {
    /// The capture, as is.
    Plain,
    /// A font (see [`font_term`]).
    Font,
    /// A font, after removing `luaotfload`'s internal `;-fallback` suffix
    /// (Quarto 1 only, quarto-dev/quarto-cli#14558).
    LuaotfloadFont,
    /// The capture plus `.sty` (the widetext special case).
    Sty,
    /// A fixed term, ignoring the capture.
    Fixed(&'static str),
    /// The capture plus `.ldf` (babel language definition files).
    Ldf,
    /// The capture, lowercased (`LGRcmr.fd` is searched as `lgrcmr.fd`).
    Lower,
    /// The capture, only when the log has a `! Package tikz Error:`.
    Tikz,
}

impl Filter {
    fn apply(self, capture: &str, log: &str) -> Option<String> {
        match self {
            Filter::Plain => Some(capture.to_string()),
            Filter::Font => Some(font_term(capture)),
            Filter::LuaotfloadFont => Some(font_term(
                capture.strip_suffix(";-fallback").unwrap_or(capture),
            )),
            Filter::Sty => Some(format!("{capture}.sty")),
            Filter::Fixed(term) => Some(term.to_string()),
            Filter::Ldf => Some(format!("{capture}.ldf")),
            Filter::Lower => Some(capture.to_lowercase()),
            Filter::Tikz => log
                .contains("! Package tikz Error:")
                .then(|| capture.to_string()),
        }
    }
}

/// The matchers, in Quarto 1's order (R's table is the same set; the
/// differences are marked). Each regex matches a whole line (`.*` at both
/// ends, as upstream) and its first capture group names the file.
static PACKAGE_MATCHERS: LazyLock<Vec<(Regex, Filter)>> = LazyLock::new(|| {
    let table: &[(&str, Filter)] = &[
        // Fonts.
        (r#".*! Font [^=]+=([^ ]+).+ not loadable.*"#, Filter::Font),
        (
            r#".*! .*The font "([^"]+)" cannot be found.*"#,
            Filter::Font,
        ),
        (r#".*!.+ error:.+\(file ([^)]+)\): .*"#, Filter::Font),
        (r#".*Unable to find TFM file "([^"]+)".*"#, Filter::Font),
        (
            r#".*\(fontspec\)\s+The font "([^"]+)" cannot be.*"#,
            Filter::Font,
        ),
        // Quarto 1 only: a missing luaotfload fallback-chain font.
        (
            r#".*luaotfload.*reason: Font "([^"]+)" not found.*"#,
            Filter::LuaotfloadFont,
        ),
        (
            r".*Package widetext error: Install the ([^ ]+) package.*",
            Filter::Sty,
        ),
        // epstopdf.
        (
            r".* File [`'](.+eps-converted-to.pdf)'.*",
            Filter::Fixed("epstopdf"),
        ),
        (
            r".*xdvipdfmx:fatal: pdf_ref_obj.*",
            Filter::Fixed("epstopdf"),
        ),
        // tikz libraries.
        (r".* (tikzlibrary[^ ]+?[.]code[.]tex).*", Filter::Tikz),
        (
            r".*module 'lua-uni-normalize' not found:.*",
            Filter::Fixed("lua-uni-algos.lua"),
        ),
        // PDF/A and PDF standards. Quarto 1 names the `colorprofiles` package
        // for the backend error; R names its `.sty`.
        (
            r".* Package pdfx Error: No color profile ([^\s]*).*",
            Filter::Fixed("colorprofiles.sty"),
        ),
        (
            r".*No support files for \\DocumentMetadata found.*",
            Filter::Fixed("latex-lab"),
        ),
        (
            r".*\(pdf backend\): cannot open file for embedding.*",
            Filter::Fixed("colorprofiles"),
        ),
        // Font definition files.
        (r".*No file ([^`'. ]+[.]fd)[.].*", Filter::Lower),
        // Style files and commands.
        (r".* Loading '([^']+)' aborted!.*", Filter::Plain),
        (
            r".*! LaTeX Error: File [`']([^']+)' not found.*",
            Filter::Plain,
        ),
        (r".* [fF]ile ['`]?([^' ]+)'? not found.*", Filter::Plain),
        (r".*the language definition file ([^\s]*).*", Filter::Plain),
        (
            r".*! Package babel Error: Unknown option [`']([^'`]+)'[.].*",
            Filter::Ldf,
        ),
        (r".* \(file ([^)]+)\): cannot open .*", Filter::Plain),
        // A `.blg` log; `\S` keeps the match off the next log line
        // (quarto-dev/quarto-cli#15377).
        (r".* open style file (\S+).*", Filter::Plain),
        (r".*file [`']([^']+)' .*is missing.*", Filter::Plain),
        (
            r".*! CTeX fontset [`']([^']+)' is unavailable.*",
            Filter::Plain,
        ),
        (r".*: ([^:]+): command not found.*", Filter::Plain),
        (r".*! I can't find file [`']([^']+)'.*", Filter::Plain),
    ];
    table
        .iter()
        .map(|(pattern, filter)| (Regex::new(pattern).expect("built-in regex"), *filter))
        .collect()
});

/// The terms a log asks for, in first-seen order, without repeats: file
/// searches for `tlmgr search --file --global`, and packages named directly.
///
/// `missfont` is the text of `missfont.log`, when the run wrote one (R
/// `miss_font()`): its fonts and font-making commands are appended.
pub fn missing_packages(log: &str, missfont: Option<&str>) -> Vec<SearchTerm> {
    let log = normalize(log);
    let mut terms: Vec<SearchTerm> = Vec::new();

    for (regex, filter) in PACKAGE_MATCHERS.iter() {
        for captures in regex.captures_iter(&log) {
            // `Fixed` matchers have no capture group.
            let capture = captures.get(1).map_or("", |capture| capture.as_str());
            if let Some(name) = filter.apply(capture, &log) {
                push_unique(&mut terms, search_term(name));
            }
        }
    }

    // R only (#424): an L3 programming layer mismatch surfaces as an
    // undefined control sequence; the file named by the last `File:` line
    // before it is from `l3backend`.
    if re!(r"(?m)^! Undefined control sequence").is_match(&log)
        && let Some(captures) = re!(r"(?m)^File: ([^ ]+) \d{4,}-\d{2}-\d{2} .*$")
            .captures_iter(&log)
            .last()
    {
        push_unique(&mut terms, search_term(captures[1].to_string()));
    }

    // The fatal babel error names its hyphenation packages directly.
    for package in fatal_babel_packages(&log) {
        push_unique(&mut terms, SearchTerm::Package(package));
    }

    if let Some(missfont) = missfont {
        for term in missfont_terms(&normalize(missfont)) {
            push_unique(&mut terms, search_term(term));
        }
    }
    terms
}

/// R `miss_font()`: the last word of a two-word line is a font, the first the
/// command that makes it; fonts first.
fn missfont_terms(text: &str) -> Vec<String> {
    let mut fonts = Vec::new();
    let mut commands = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let (Some(command), Some(font)) = (words.next(), words.last()) else {
            continue;
        };
        let font = if font.contains('.') {
            font.to_string()
        } else {
            font_pattern(font)
        };
        push_unique(&mut fonts, font);
        push_unique(&mut commands, command.to_string());
    }
    for command in commands {
        push_unique(&mut fonts, command);
    }
    fonts
}

// ------------------------------------------------------------------ rerun

/// Whether the log asks for another engine run: an explicit request (the
/// union of R's and Quarto 1's patterns, so `Please rerun LaTeX.` counts), a
/// missing `.aux` (a beamer run with lualatex, quarto-dev/quarto#6226), or a
/// `marginnote` position that never resolved.
pub fn needs_rerun(log: &str) -> bool {
    let log = normalize(log);
    if re!(r"(Rerun to get |Please \(?re\)?run |[rR]erun LaTeX\.)").is_match(&log)
        || re!(r"(?m)^No file .*?\.aux\.\s*$").is_match(&log)
    {
        return true;
    }
    // Quarto 1 only: `marginnote` reports a position it has not computed yet
    // (`\@mn@currxpos`) and reports the real one later, for the same line.
    re!(r"(?m)^.*xpos seems to be \\@mn@currxpos.*?line ([0-9]*)\.$")
        .captures_iter(&log)
        .any(|unresolved| {
            let resolved = format!(
                r"(?m)^.*xpos seems to be [0-9]*\.[0-9]*pt.*?line {}\.$",
                &unresolved[1]
            );
            !Regex::new(&resolved).is_ok_and(|resolved| resolved.is_match(&log))
        })
}

// ------------------------------------------------------------------ warnings

/// The lines of every `LaTeX Warning:` and `Package <name> Warning:` block
/// (a block runs to the next blank line), R's `latex_warning()`. With
/// `skip_noisy`, blocks from `fixltx2e`, `caption` and `hyperref` are left
/// out, as R does.
fn warning_lines(log: &str, skip_noisy: bool) -> Vec<String> {
    let lines: Vec<&str> = log.split('\n').collect();
    let start = re!(r"^(LaTeX|Package [A-Za-z0-9]+) Warning:");
    let noisy = re!(r"^Package (fixltx2e|caption|hyperref) Warning:");
    let mut keep = vec![false; lines.len()];
    for (i, line) in lines.iter().enumerate() {
        if !start.is_match(line) || (skip_noisy && noisy.is_match(line)) {
            continue;
        }
        for (j, kept) in keep.iter_mut().enumerate().skip(i) {
            if lines[j].trim().is_empty() {
                break;
            }
            *kept = true;
        }
    }
    lines
        .iter()
        .zip(keep)
        .filter(|(_, keep)| *keep)
        .map(|(line, _)| (*line).to_string())
        .collect()
}

/// The lines of the log's warning blocks (LaTeX and package warnings), minus
/// the noisy `fixltx2e`, `caption` and `hyperref` ones, in log order.
pub fn latex_warnings(log: &str) -> Vec<String> {
    warning_lines(&normalize(log), true)
}

// ------------------------------------------------------------------ hyphenation

/// Hyphenation pattern names whose package is not `hyphen-<name>`. Generated
/// in tinytex from `execute AddHyphen` in `texlive.tlpdb` (`R/hyphen.R`, R
/// `main` at `e78c2f7`); the same table is in Quarto 1 `v1.12.3`. The
/// warning and the fatal-error paths share it.
const HYPHEN_MAP: &[(&str, &str)] = &[
    ("amharic", "hyphen-ethiopic"),
    ("assamese", "hyphen-indic"),
    ("bengali", "hyphen-indic"),
    ("bokmal", "hyphen-norwegian"),
    ("british", "hyphen-english"),
    ("classiclatin", "hyphen-latin"),
    ("espanol", "hyphen-spanish"),
    ("francais", "hyphen-french"),
    ("geez", "hyphen-ethiopic"),
    ("german-x-2024-02-28", "dehyph-exptl"),
    ("german-x-latest", "dehyph-exptl"),
    ("gujarati", "hyphen-indic"),
    ("hindi", "hyphen-indic"),
    ("ibycus", "hyphen-ancientgreek"),
    ("kannada", "hyphen-indic"),
    ("liturgicallatin", "hyphen-latin"),
    ("malayalam", "hyphen-indic"),
    ("marathi", "hyphen-indic"),
    ("mongolianlmc", "hyphen-mongolian"),
    ("monogreek", "hyphen-greek"),
    ("ngerman", "hyphen-german"),
    ("ngerman-x-2024-02-28", "dehyph-exptl"),
    ("ngerman-x-latest", "dehyph-exptl"),
    ("norsk", "hyphen-norwegian"),
    ("nynorsk", "hyphen-norwegian"),
    ("oriya", "hyphen-indic"),
    ("pali", "hyphen-indic"),
    ("panjabi", "hyphen-indic"),
    ("patois", "hyphen-french"),
    ("persian", "hyphen-farsi"),
    ("pinyin", "hyphen-chinese"),
    ("polygreek", "hyphen-greek"),
    ("portuges", "hyphen-portuguese"),
    ("schoolfinnish", "hyphen-finnish"),
    ("serbianc", "hyphen-serbian"),
    ("slovene", "hyphen-slovenian"),
    ("swissgerman", "hyphen-german"),
    ("tamil", "hyphen-indic"),
    ("telugu", "hyphen-indic"),
    ("ukenglish", "hyphen-english"),
    ("usenglishmax", "hyphen-english"),
];

/// Maps language names found in a log to the packages that provide their
/// hyphenation patterns, without repeats.
fn hyphen_packages<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut packages = Vec::new();
    for raw in names {
        // Names in `language.dat` may be mixed case (`UKenglish`); a warning
        // may give a description (`German (current orthography)`), whose
        // first word is the language.
        let lowered = raw.replace(['`', '\''], "").to_lowercase();
        let language = lowered.split_whitespace().next().unwrap_or("");
        // The name comes from the log and reaches `tlmgr` (through a `.bat`
        // file on Windows), so keep only what is a package name.
        if !re!(r"^[a-z0-9-]+$").is_match(language) {
            continue;
        }
        // Some languages have no hyphenation files (quarto-dev/quarto-cli#10291).
        if language == "chinese" || language == "chinese-hans" {
            continue;
        }
        let package = HYPHEN_MAP
            .iter()
            .find(|(name, _)| *name == language)
            .map_or_else(
                || format!("hyphen-{language}"),
                |(_, package)| (*package).to_string(),
            );
        push_unique(&mut packages, package);
    }
    packages
}

/// Languages named by `babel`'s missing-patterns warning (R `check_babel`,
/// Quarto 1 `findMissingHyphenationFiles`), searched in warning text only.
fn babel_warning_languages(warnings: &str) -> Vec<String> {
    re!(r"(?m)^(?:\(babel\).* |Package babel Warning: No hyphenation patterns were preloaded for the )language [`']([^']+)'.*$")
        .captures_iter(warnings)
        .map(|captures| captures[1].to_string())
        .collect()
}

/// Languages named by `polyglossia`'s missing-patterns warning.
fn polyglossia_warning_languages(warnings: &str) -> Vec<String> {
    re!(r"(?m)^Package polyglossia Warning: No hyphenation patterns were loaded for ([`'][^']+'|British English).*$")
        .captures_iter(warnings)
        .map(|captures| captures[1].to_string())
        .collect()
}

/// The hyphenation packages whose absence makes a run fail: `babel`'s fatal
/// `You haven't defined the language 'X' yet` (also the backquote and unquoted
/// forms; R `b757aa5` and `872c51e`, Quarto 1 `findMissingHyphenationPackages`).
fn fatal_babel_packages(log: &str) -> Vec<String> {
    let languages: Vec<String> =
        re!(r"Package babel Error: You haven't defined the language [`']?([^`'\s]+)[`']? yet")
            .captures_iter(log)
            .map(|captures| captures[1].to_string())
            .collect();
    hyphen_packages(languages.iter().map(String::as_str))
}

/// The hyphenation packages for the languages a successful run warned about,
/// every one of them: from `babel` and `polyglossia` warning blocks, else from
/// `Package babel Info: Hyphen rules for 'X' set to \l@nil` (Quarto 1 only).
/// Names are packages (`hyphen-spanish`); languages with no pattern files
/// (`chinese`) are skipped.
pub fn missing_hyphenation(log: &str) -> Vec<String> {
    let log = normalize(log);
    let warnings = warning_lines(&log, false).join("\n");
    let mut languages = babel_warning_languages(&warnings);
    languages.extend(polyglossia_warning_languages(&warnings));
    let packages = hyphen_packages(languages.iter().map(String::as_str));
    if !packages.is_empty() {
        return packages;
    }
    let info: Vec<String> = re!(r"(?m)Package babel Info: Hyphen rules for '(.*?)' set to \\l@nil")
        .captures_iter(&log)
        .map(|captures| captures[1].to_string())
        .collect();
    hyphen_packages(info.iter().map(String::as_str))
}

/// The packages for modules that a successful run warned about (R
/// `check_extra`): `babel` and `polyglossia` hyphenation (`hyphen-spanish`,
/// from warning blocks only), `glossaries` language modules
/// (`glossaries-english`) and `datetime2` modules (`datetime2-english`).
pub fn missing_modules(log: &str) -> Vec<String> {
    let log = normalize(log);
    let warnings = warning_lines(&log, true).join("\n");
    let mut modules = Vec::new();
    let babel = babel_warning_languages(&warnings);
    for package in hyphen_packages(babel.iter().map(String::as_str)) {
        push_unique(&mut modules, package);
    }
    // Package glossaries Warning: No language module detected for `english'.
    // (glossaries)  ... `glossaries-english' or similar.
    for captures in re!(r"(?m)^\(glossaries\).* [`']([^']+)'.*$").captures_iter(&warnings) {
        push_unique(&mut modules, captures[1].to_string());
    }
    for captures in re!(
        r"(?m)^Package datetime2 Warning: Date-Time Language Module [`']([^']+)' not installed.*$"
    )
    .captures_iter(&warnings)
    {
        push_unique(&mut modules, format!("datetime2-{}", &captures[1]));
    }
    let polyglossia = polyglossia_warning_languages(&warnings);
    for package in hyphen_packages(polyglossia.iter().map(String::as_str)) {
        push_unique(&mut modules, package);
    }
    modules
}

// ------------------------------------------------------------------ accessibility

/// The PDF/UA warnings of `tagpdf` and `DocumentMetadata` (Quarto 1 only).
pub fn accessibility(log: &str) -> AccessibilityWarnings {
    let log = normalize(log);
    let mut result = AccessibilityWarnings::default();

    // Package tagpdf Warning: Alternative text for graphic is missing.
    // (tagpdf)                Using 'filename' instead.
    // tagpdf wraps long file names over `(tagpdf)` continuation lines; the
    // continuation prefix is not part of the name.
    for captures in re!(
        r"Package tagpdf Warning: Alternative text for graphic is missing\.\s*\n\(tagpdf\)\s*Using ['`]([^'`]+)['`]\s*(?:\n\(tagpdf\)\s*)?instead\."
    )
    .captures_iter(&log)
    {
        let name = re!(r"\n\(tagpdf\)\s*").replace_all(&captures[1], "");
        result.missing_alt_text.push(name.into_owned());
    }

    result.missing_language =
        log.contains("LaTeX DocumentMetadata Warning: The language has not been set in");

    for captures in re!(r"Package tagpdf Warning: ([^\n]+)").captures_iter(&log) {
        let warning = &captures[1];
        if !warning.starts_with("Alternative text for graphic is missing") {
            result.other_warnings.push(warning.to_string());
        }
    }
    result
}

/// The message of a `makeindex` failure: the first line of the form
/// `   -- message`.
pub fn index_error(log: &str) -> Option<String> {
    let log = normalize(log);
    re!(r"(?m)^\s\s\s--\s(.*)")
        .captures(&log)
        .map(|captures| captures[1].to_string())
}

// ------------------------------------------------------------------ errors

/// The one-line text Quarto 1 gives for the `luaotfload` fallback crash
/// (luaotfload v3.29, latex3/luaotfload#331): the resolver dereferences a nil
/// font, a Lua runtime error that has no `!` block.
const LUAOTFLOAD_FALLBACK_MESSAGE: &str = "A font fallback (e.g. 'mainfontfallback' or 'monofontfallback') triggered a known luaotfload bug on this TeX Live version, which crashes LuaLaTeX before a PDF is produced. Until it is fixed upstream, set a single font that covers the required glyphs (e.g. 'monofont: JuliaMono') instead of a fallback list. See https://github.com/latex3/luaotfload/issues/331";

/// The errors of a failed run, from its log and, for hints, its `stderr`.
/// `None` when the log has no error block and did not report empty output.
///
/// Error extraction is R's block per `!` line (see [`LatexError::summary`]),
/// not Quarto 1's single span from the first `!` to `Here is how much`.
pub fn latex_error(log: &str, stderr: Option<&str>) -> Option<LatexError> {
    let log = normalize(log);

    if re!(r"luaotfload-fallback\.lua:\d+: attempt to index a nil value").is_match(&log) {
        return Some(LatexError {
            summary: LUAOTFLOAD_FALLBACK_MESSAGE.to_string(),
            hints: vec![Hint::LuaotfloadFallback],
        });
    }

    let lines: Vec<&str> = log.split('\n').collect();
    let ends: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            line.trim().is_empty() || **line == "Here is how much of TeX's memory you used:"
        })
        .map(|(i, _)| i)
        .collect();
    let mut blocks: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(first) = line.strip_prefix("! ") else {
            continue;
        };
        // The last message of a failed run, about the fatal error itself.
        if line.contains("==> Fatal error occurred") {
            continue;
        }
        let last = ends.iter().find(|end| **end > i).map_or(i, |end| end - 1);
        let mut block = vec![first];
        block.extend(&lines[i + 1..=last]);
        blocks.push(block.join("\n"));
    }

    if blocks.is_empty() {
        return re!(r"(No pages of output)\.")
            .captures(&log)
            .map(|captures| LatexError {
                summary: format!(
                    "{} - the document appears to have produced no output.",
                    &captures[1]
                ),
                hints: Vec::new(),
            });
    }

    Some(LatexError {
        summary: blocks.join("\n\n"),
        hints: hints(&log, stderr),
    })
}

/// Hints, stderr-derived first (Quarto 1 `suggestHint`, but all that match).
fn hints(log: &str, stderr: Option<&str>) -> Vec<Hint> {
    let mut hints = Vec::new();
    if let Some(stderr) = stderr {
        let stderr = normalize(stderr);
        if re!(r"(?m)^!!! Error: Cannot open Ghostscript for piped input").is_match(&stderr) {
            hints.push(Hint::Ghostscript);
        }
        if re!(r"(?m)^GPL Ghostscript .*: Can't find initialization file gs_init.ps")
            .is_match(&stderr)
        {
            hints.push(Hint::GhostscriptMisconfigured);
        }
    }
    // Quarto 1 matches `inputenc`; R also matches `! LaTeX Error:`.
    if re!(r"! (Package inputenc|LaTeX) Error: Unicode character").is_match(log) {
        hints.push(Hint::UnsupportedUnicode);
    }
    if re!(r"Missing \$ inserted\.").is_match(log) {
        hints.push(Hint::InlineMath);
    }
    hints
}
