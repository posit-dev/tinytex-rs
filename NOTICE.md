# Third-party attribution

This crate's behavior, and some of its code and test fixtures, derive from two
MIT-licensed projects. Their copyright notices apply to the derived parts.

- **`tinytex`** (the R package), <https://github.com/rstudio/tinytex>.
  Copyright (c) Yihui Xie and RStudio, PBC (now Posit Software, PBC). MIT.
  Source of the `tlmgr` handling, TinyTeX installation behavior and the
  structure of the compile loop.
- **Quarto** (`quarto-cli`), <https://github.com/quarto-dev/quarto-cli>.
  Copyright (c) Posit Software, PBC. MIT. Source of the LaTeX log matchers,
  the missing-package detection and the accessibility and rerun warnings.

Files that copy text from either project name it in a comment at the top and
keep the upstream notice.
