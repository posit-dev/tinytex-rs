//! Output placement, the auxiliary-file list, and the Windows `.aux` tweak.

use std::io;
use std::path::Path;

use regex::Regex;

/// The auxiliary files the compile's cleanup removes: the union of R's
/// `latexmk_emu` list (`R/latex.R:165-171` at `7818777`) and Quarto 1's
/// `cleanup()` (`pdf.ts:599-630` at `v1.12.3`).
///
/// The first 19 entries are extensions of the document's file (`doc.aux`,
/// `doc.run.xml`). The last two, `preview.aux` (the `preview` package) and
/// `missfont.log` (`mktexpk`), are whole file names with no stem. Use
/// [`aux_files`] for the file names of one document.
pub const AUX_EXTENSIONS: &[&str] = &[
    "log",
    "idx",
    "aux",
    "bcf",
    "blg",
    "bbl",
    "fls",
    "out",
    "lof",
    "lot",
    "toc",
    "nav",
    "snm",
    "vrb",
    "ilg",
    "ind",
    "xwm",
    "brf",
    "run.xml",
    "preview.aux",
    "missfont.log",
];

/// How many leading [`AUX_EXTENSIONS`] entries take the document's stem.
const STEMMED: usize = 19;

/// The file names the cleanup considers for the document `stem`, in
/// [`AUX_EXTENSIONS`] order.
pub fn aux_files(stem: &str) -> Vec<String> {
    AUX_EXTENSIONS
        .iter()
        .enumerate()
        .map(|(i, e)| {
            if i < STEMMED {
                format!("{stem}.{e}")
            } else {
                (*e).to_owned()
            }
        })
        .collect()
}

/// Moves `from` to `to`, creating `to`'s directory. When `rename` fails (a
/// move across filesystems, or onto an existing file on Windows) the file is
/// copied, replacing `to`, and `from` is removed.
pub(crate) fn move_file(
    from: &Path,
    to: &Path,
    rename: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(parent) = to.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    if rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

/// R's `tweak_aux` (`R/latex.R:296-307` at `7818777`): `bibtex` on Windows
/// without `tlmgr` wants no `.bib` extension in `\bibdata{...}`. `None` when
/// no line changes.
pub(crate) fn tweak_aux(aux: &str) -> Option<String> {
    let bib = Regex::new(r"[.]bib([,}])").ok()?;
    let mut changed = false;
    let mut out = String::with_capacity(aux.len());
    for line in aux.split_inclusive('\n') {
        let body = line.trim_end();
        if body.starts_with("\\bibdata{") && body.ends_with('}') && body.len() > "\\bibdata{}".len()
        {
            let fixed = bib.replace_all(line, "$1");
            changed |= fixed != line;
            out.push_str(&fixed);
        } else {
            out.push_str(line);
        }
    }
    changed.then_some(out)
}
