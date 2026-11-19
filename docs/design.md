> **Snapshot of 2026-11-22**, copied from q2's `claude-notes/designs/2026-09-30-tinytex-rs-api.md` at milestone M0. The q2 note stays the source until the first tag; links into q2 are removed from this copy.

# `tinytex-rs` — public API, feature inventory, and merge strategy

**Date:** 2026-09-30
**Status:** Design, pre-implementation; reviewed three times (the epic design review of
2026-11-17, the implementability review of 2026-11-20, and the interface review of
2026-11-22, q2 note `../plans/2026-11-22-tex-tinytex-interface-review.md`).
§9 is an addendum on adoption by the R and
Quarto 1 `tinytex` packages, for discussion with their maintainers. The q2 epic that uses
this crate is `../plans/2026-09-23-pandoc-hybrid-latex.md`.
**Home:** the crate lives in its own repository, **`posit-dev/tinytex-rs`**, with no
dependency on any q2 crate. This note is kept in q2's `claude-notes/` because q2's
plan 6.5a (index) is the first
consumer; copy it into the crate repo as `docs/design.md` at milestone M0.
**Epic:** q2 note `../plans/2026-09-23-pandoc-hybrid-latex.md`.
**Crate plans:** block 5 of the plan index (plans 5.1 to 5.6).
**Research:** q2 note `../research/2026-09-23-latex-pdf-pandoc-q1-inventory.md`.

**Baselines, and how to read them.** Every citation below is to one of two pinned
revisions, read through `git show`, never through a working tree:

- **Quarto 1 at `v1.12.3`** (released 2026-11-17): `git -C
  /Users/gordon/src/q2/external-sources/quarto-cli show v1.12.3:<path>`. The local checkout
  is `v1.11.5-1` plus 34 commits, so reading it gives neither `v1.11.6` nor `v1.12.3`. Line
  numbers in `src/command/render/latexmk/pdf.ts` are the same as at `v1.11.6` up to about
  line 430 and 3 lower after that. `v1.12.3` changed four things this design depends on:
  the bibliography trigger needs `\citation`, `\bibdata` **and** `\bibstyle` (#15380; the
  commit calls the earlier any-of-three a bug), a missing `.bst` is now installed (#15377),
  the `(file …): cannot open` matcher no longer has a dead double escape (#15385), and every
  warned hyphenation language is installed (#15076).
- **R `tinytex` at commit `7818777`** (2026-09-17): `git -C
  /Users/gordon/src/q2/external-sources/tinytex show 7818777:R/latex.R`. R `main` has moved
  on (to `e78c2f7`, 2026-11-09); §3.3 lists the later commits and what this design does
  with each.

## 1. What "tinytex" is, and what the crate is

TinyTeX is not a program. It is an ordinary TeX Live tree, published as release assets
by `rstudio/tinytex-releases`. Two codebases manage it, and both drive its executables
(`tlmgr`, `pdflatex`, `xelatex`, `lualatex`, `biber`, `bibtex`, `makeindex`,
`fmtutil-sys`, `latexmk`) as subprocesses; neither links or embeds TeX:

- **The R package `tinytex`** (`rstudio/tinytex`, MIT, CRAN). Actively maintained:
  `main` was at 2026-09-17 when this was written and `R/latex.R` was last edited
  2026-09-10; Yihui Xie is the maintainer of record and Christophe Dervieux a frequent
  contributor to `R/latex.R`. Source: `R/install.R`,
  `R/tlmgr.R`, `R/latex.R`, plus `tools/` (the scripts that build the bundles).
- **Quarto 1's TypeScript** (`quarto-cli`, MIT): `src/tools/impl/tinytex.ts` and
  `tinytex-info.ts` (install and detection), `src/command/render/latexmk/` (the compile
  loop, tlmgr wrapper, log parser). Its compile loop is a port of `tinytex::latexmk()`
  (its comments cite tinytex commits from 2021), and it has since absorbed a subset of
  later R fixes and added Quarto-only handling (§3).

| Code lines (cloc) | Files | Code |
|---|---|---|
| Q1 TypeScript (install + detection + latexmk dir) | 11 | 2,401 |
| R package `main`, `R/*.R` only | 6 | 1,844 physical lines |
| R package `main`, whole repo excl. `TUGboat` (R, YAML CI, shell/PowerShell/batch build scripts, tests) | 54 | 3,025 |

Both do the same three jobs: **detect/install** a TinyTeX tree, **wrap `tlmgr`**, and
**drive a compile** that recovers from missing packages by parsing the `.log`. The crate
provides all three, with q2's latex/pdf epic needing the last two (and detection) first.

### Terminology

The two codebases and the plans use "latexmk" for four different things. This note uses
the words below, and only these, for the layers of a LaTeX build:

| Term | Meaning | Examples |
|---|---|---|
| **Engine** | A TeX program that turns one `.tex` file into one output file in a single pass. | `pdflatex`, `xelatex`, `lualatex` (PDF); `latex` (DVI). They share TeX's command-line conventions (`-interaction=…`, `-halt-on-error`, `-output-directory=…`, the file last), which is why a caller can swap one for another. `tectonic` is a different kind of engine: self-contained, with its own CLI, and it fetches its own packages. |
| **Driver** | A program that runs an engine repeatedly until cross-references settle, runs the bibliography and index tools between passes, and recovers from errors. **This crate's `compile` module is a driver.** | real `latexmk`; R's `tinytex::latexmk()`; Quarto 1's `generatePdf`; Pandoc's internal rerun loop (capped at four runs). |
| **`latexmk`** | Alone, always the real executable: the Perl driver written by John Collins, shipped with TeX Live. | `Engine::Latexmk` means "run this executable as the per-pass engine inside our driver", which is what Quarto 1's `pdf-engine: latexmk` does. `Driver::Latexmk` means "hand the whole build to this executable" (R's `emulation = FALSE`). Quarto 1\'s `latex-auto-mk: false` is a third thing, Pandoc running the engine itself (q2 plan 6.9). |
| **Emulation** | A driver that imitates `latexmk`\'s job without running it. This crate's name for the choice is `Driver::Builtin`; R's `emulation = FALSE` is `Driver::Latexmk` (§5). | `tinytex::latexmk(emulation = TRUE)`, the default. |
| **Distribution** | A TeX tree plus its package manager. | TeX Live and TinyTeX (a trimmed TeX Live), both managed with `tlmgr`. MiKTeX has a different package manager, so it runs in engine-only mode: the crate compiles with the engines it finds on `PATH` and skips auto-install, as R's emulation does for a distribution that is not TeX Live. |

**There is no standard driver interface.** The engines share conventions; the drivers do
not. They agree on the concepts (input file, engine, engine arguments, output directory,
cleanup) and on little else: real `latexmk` has `-pdf`, `-pdflatex=`, `-outdir` and `-c`;
`tinytex::latexmk()` has `engine=`, `engine_args=` and `clean=`; Quarto 1's unused
`quarto-latexmk` command has `--pdf-engine`, `--output-dir` and `--no-clean`; Pandoc's loop
has no options of its own. Only the tinytex family has minimum and maximum run counts and
package auto-install. So the shape of this crate's driver interface, including its
command line (§5.1), is a decision this note makes, not a standard it follows.

ConTeXt is not covered by this crate: it has its own tooling and Pandoc treats it separately (q2 runs `context` itself).

A naming note for q2 readers: q2 also uses "engine" for knitr and Jupyter execution (error subsystem `engine`). In q2 code and prose, the TeX type is `TexEngine` and the term is "TeX engine" or "PDF engine".

### Consumers in q2

| Consumer | What it needs | When |
|---|---|---|
| `TexCompileStage` (plans 6.5a to 6.9) | `compile`, `detect`, `tlmgr` (via compile), `log` | latex/pdf epic |
| Lua filter params, `quartoEnvironmentParams()` — `"TinyTexBinDir"` is emitted as `""` today (`crates/quarto-core/src/pandoc_filters/params.rs` on `main`); Q1 fills it from `tinyTexBinDir()` | `detect::find_tinytex` **filesystem-only** (runs on every render, including formats that never compile LaTeX) | latex/pdf epic |
| `q2 install` / `uninstall` / `update` / `tools` — `crates/quarto/src/commands/install.rs` returns `NotImplemented` today. q2's installer will be a registry of tools (TinyTeX first; verapdf and chromium / chrome-headless-shell are expected later), so TinyTeX is one backend of a q2-side framework, tracked as strand `bd-6s1fi9c7` | the crate's `install` step functions (§5), plus `Distribution` queries for `check`/`tools` | after the epic (M5) |

## 2. Conventions

- **Blocking core; no async traits.** Every operation is a subprocess or a download. The
  call sites tolerate this: `TypstCompileStage` (the precedent for `TexCompileStage`)
  is an `async fn run` that calls the blocking `crate::child_runner::run_child` with the
  render's `ctx.child_limits()` (`typst_compile.rs:257`); `q2 render` drives the pipeline with
  `pollster::block_on` (`render.rs:1158`, `1377`), a bare executor with no tokio
  runtime, so a `tokio::process`-based API could not be called from it; the preview server
  already wraps each render in `tokio::task::spawn_blocking` (for example
  `preview_static.rs:216`). A blocking crate also sidesteps the repo's
  `#[async_trait(?Send)]` rule entirely. If a future consumer wants async, it wraps
  calls in `spawn_blocking`.
- **Injection, not globals.** Three small traits: `Exec` (subprocesses; `StdExec` is the
  default implementation), `Observer` (progress events; replaces Q1's `logProgress` and
  spinners; every method is a no-op by default), and `Fetcher` (HTTP; only present with
  the `install` feature and implemented by the caller). `Exec` takes a `CommandSpec`
  (program, args, env, cwd), not `std::process::Command`, so tests can record and
  inspect calls. `CommandSpec::env` holds **overrides** applied with `Command::env` on top of
  the inherited environment, never the whole environment. `NullObserver` is the provided
  no-op observer, so a host that wants no progress writes `&NullObserver`. `Exec::run`
  captures all output; a host that wants live output uses the `Event` stream, and a streaming
  `run` is a later, additive default method.
- **Threads, cancellation and locking.** `Exec` and `Observer` are
  `Send + Sync`, because q2 renders documents on rayon workers and runs preview renders on
  a blocking thread pool. `Exec` has a default method `interrupted(&self) -> bool`
  (false by default): a host that supports cancellation implements it, and q2 injects an
  `Exec` that wraps its child runner with the render's cancellation token, deadline and
  limits, so deadlines and cancellation need no crate-side hook. q2's adapter measures one
  deadline for a whole compile and passes the remaining time to each child; the crate has
  no deadline of its own except `StdExec`\'s optional whole-command deadline, which the
  `tinytex compile --timeout` flag uses. A cancelled compile ends with
  `CompileError::Cancelled`, after the crate has removed the auxiliary files its run
  created. Commands that change the TeX installation (package install, update, remove,
  `path`, repository changes, format rebuilds) are marked
  `CommandSpec::changes_installation`, so that a host can protect them from cancellation
  (q2 does: a superseding edit does not kill an install; the deadline still applies). After
  such a command returns, the crate checks `interrupted()` and stops before its next step
  (`TlmgrError::Interrupted`). **The rule for every call:** after any `Exec::run` that returns
  `Err` or a non-zero exit, the crate tests `interrupted()` **first**; when it is true the
  result is `Cancelled` (`TlmgrError::Interrupted` inside `tlmgr`) and the cancelled-run
  cleanup runs, whatever the error was. A host that kills a child on cancellation returns
  `Err` from `run` (q2's `run_child` does, `child_runner.rs:53-56`, `128-136`), so without
  this rule every kill would surface as `Io`, `PackageInstall` or `EngineNotFound`. This
  covers the read-only `tlmgr search` and `info` calls too. Package installation, update and format-rebuild
  operations are serialized by a process-wide lock keyed by the distribution root (compiles
  themselves are not: q2 keeps its own one-build-at-a-time lock, plan 6.7): the lock is taken
  at the top-level operation, and inner steps
  receive a witness token (a guard reference) instead of locking again, because
  `std::sync::Mutex` is not re-entrant; poisoning is recovered from (the lock protects an
  operation, not data); waits poll `try_lock` with a short sleep and check `interrupted()`.
  **Under the lock, recheck.** After acquiring the lock, `install` tests `is_installed` for
  every requested package and returns early when all are present, because R\'s algorithm
  (§3.2) otherwise runs an update and a second install for the thread that waited. The
  registry key is `Distribution::root` when known and the canonicalized `bin_dir`
  otherwise (`root` is `None` for some system installs). The "update done" flag lives in the
  same registry entry: it is set by any completed `tlmgr update` and by a `TexLiveTooOld`
  report, it is shared by every compile of that distribution in the process, and it is
  **not** reset between compiles. (R\'s `update_noted` is set only for `TexLiveTooOld` and
  is reset at the end of each `latexmk()` call, so R updates again on every failure; in a
  long-lived preview server that would repeat `update --self` on every compile.)
  The guard is its own type, so a cross-process advisory file lock can replace it later. No
  async regime is committed: the lock is a blocking lock, and an async caller wraps calls in
  `spawn_blocking` as it must for the compile itself.
- **Command environment.** `Distribution::command()` prepends the distribution\'s
  `bin_dir` to `PATH` for every program. `TEXLIVE_PREFER_OWN=1` is set **only** for
  `tlmgr`, `fmtutil` and `updmap` (R sets it only around `tlmgr`, `R/tlmgr.R:48-52`; Quarto 1
  never sets it), never for the engines. On Windows the Rtools directories are removed from
  `PATH` for those tools (R `fb7a799`).
- **One `Config`.** Environment variables are read in exactly one place
  (`Config::from_env()`), not scattered through the code as Q1 does (`Deno.env.get`)
  and R does (`getOption`). q2 layers its own `QUARTO_*` names on top.
- **No printing, no `Q-*` awareness.** Errors are plain enums (`thiserror`); the
  stage maps them to q2's error catalog (epic B4).
- **`#[non_exhaustive]`** on public enums and option structs so DVI output, relocation,
  and new engines can be added without breaking callers.
- **Cross-platform from day one.** On Windows the `runscript.tlu` 8.3-path failures
  (R #427) are avoided the way R avoids them: `fmtutil` and `updmap` are called by their
  short names with `--sys` (`fmtutil --sys --all`, `updmap --sys`), which needs no temporary
  file. `tlmgr.bat` is spawned directly, which Rust 1.77.2 and later runs through `cmd` with
  its own escaping (an argument it cannot escape safely is an error, and `tlmgr` arguments
  are package names and fixed flags). Quarto 1\'s temporary-script indirection
  (`src/core/windows.ts`, `safeWindowsExec`) is **not** ported: `Distribution::command()`
  returns plain data, so a temporary file created inside it would have no owner to delete
  it, and a mocked `Exec` would leak it. Callers never see any of this.
  `cfg(unix)`/`cfg(windows)` gating follows the repo rules (`cross-platform.md`).
  **Known limit:** on Windows a deadline kill stops the child process only, not its
  descendants (`child_runner.rs:208-209`), so `tlmgr` (Perl) can outlive a killed wrapper;
  q2 records the limit in plans 6.7 and 10.3.
- **Native only.** Everything except `log` does process and filesystem IO. `log`
  (pure functions over text) has no cfg gates.

## 3. Merge strategy: what to take from each implementation

**Principle.** The R package is the structural baseline: it is the maintained upstream, and
its install and tlmgr behavior is what the published bundles are built and tested against.
Quarto 1's TypeScript contributes a real additional layer (below), kept as clearly separate
units. Where the two disagree, this table decides, with one rule for q2: **where Quarto 1\'s
behavior is what q2 users get today, Quarto 1\'s behavior is q2\'s default and R\'s is
offered as an option** (an enum or a flag on `CompileRequest` or `Config`, reachable from the
CLI). The crate\'s own defaults, which the `tinytex` CLI and other adopters get, are R\'s for
those options; q2 sets Quarto 1\'s explicitly (§9.4 item 7). Each such row names its option.
Citations are to the baselines in the header: R at `7818777`, Quarto 1 at `v1.12.3`.

### 3.1 Compile loop and log parsing

| Area | R `main` | Quarto 1 | Adopted |
|---|---|---|---|
| Loop shape | engine → parse log → install → retry; index; bibliography; rerun until stable; cleanup | same shape | R's, with Q1's extras |
| Success check | exit code **and** output file exists (PNAS templates eat errors) | same | same |
| Run counts | `min_times` and `max_times` count reruns after a first run, so R always runs the engine at least twice (measured: (1,10) and (1,1) both run `pdflatex` twice, (3,10) four times, (2,2) three times, (0,0) once); the extra runs sit outside `max_times` | `latex-min-runs` and `latex-max-runs` are total runs, first included, and one run is enough when no rerun is needed (`pdf.ts:118-135`) | Quarto 1\'s semantics, because the keys are Quarto\'s (D30); documented as "total runs". R\'s `(min, max)` is `min_runs = min + 1`, `max_runs = max + 1` (`CompileRequest::reruns(min, max)` does the addition); the CLI table says so |
| Missing hyphenation file after a successful run | none (but see the fatal-error row below) | installs **every** warned language, including polyglossia warnings, and repeats the initial run (`pdf.ts:182-200`, `findMissingHyphenationFiles` at `v1.12.3`); without auto-install, or when the install fails, a warning | Q1\'s: install all and repeat once; otherwise a `Diagnostics.missing_hyphenation` entry, never an error |
| Fatal `Package babel Error: You haven't defined the language` | installs the matching `hyphen-*` package through the shared language map (`R/hyphen.R`, `b757aa5` and `872c51e`, after the baseline) and reruns | the same, since `v1.12.3` (`findMissingHyphenationPackages`, `parse-error.ts:271-285`, used at `pdf.ts:473`) | both agree; one shared language map (§3.3) |
| Missing modules after a successful run | `check_extra`: installs the packages for `babel`, `glossaries`, `datetime2` and `polyglossia` warnings, then runs the engine once more (`R/latex.R:234`, `416` at `7818777`) | none | R's |
| Max runs reached | loop just ends | warning | **warning in `Diagnostics`**, not an error (Q1) |
| Bib engine | optional; inferred (`.bcf` present → `biber`, else `bibtex`) | explicit from `cite-method`; none for citeproc | **`Bib::{Infer, Bibtex, Biber, Skip}`** (D24: q2 passes explicit values) |
| Bib failure | a non-zero exit from the bibliography tool is fatal: after a package-install attempt from the `.blg`, `Failed to build the bibliography` (`latex.R:256-277`) | a non-zero exit is **ignored** unless the `.blg` has `open style file`, which installs the `.bst` package (`pdf.ts:425-454`, `v1.12.3` matches ` open style file `); the PDF is still built | **`OnBibFailure::{Error, Warn}`.** The crate default is R\'s `Error`; **q2 sets `Warn`** (Quarto 1 parity: a `Diagnostics.bibliography_failed` entry, then continue). The package-install attempt from the `.blg` is shared by both; under `Warn`, a missing `.bst` whose install fails is still `CompileError::Bibliography` (Quarto 1 errors there too, `pdf.ts:436-448`): `Warn` covers a tool that fails, not a style that cannot be installed |
| Windows `.bib`-strip fixup in `.aux` | when `tlmgr` is unavailable on Windows | intended the same, but the guard is `!hasTexLive()` on an `async` function that is never awaited (`pdf.ts:396`), so the fixup never runs | R's condition; this Q1 defect is **not** ported |
| Missing commands (`makeindex`, `biber`, `xindy`…) | `tlmgr search --file` for the command name | on ENOENT, same search plus a `texindy`→`xindy` mapping | both |
| Index | re-run engine once before `makeindex` (bookdown#1274); custom command and args | same; `latex-makeindex[-opts]` | same |
| Engine flags | `-halt-on-error -interaction=batchmode` | same; `latexmk` engine adds `-pdf` (unless the user gave a `-pdf*` variant) and `-quiet` | same |
| `tectonic` | direct call, skips the whole loop | Pandoc's own driver handles it | **`Engine::Tectonic` short-circuits like R**; q2's stage still runs (epic B1) |
| `latexmk` | `emulation = FALSE` delegates the whole build, no auto-install: `latexmk -latexoption=-halt-on-error -interaction=batchmode -pdf -pdflatex=E <engine_args> file`, then `latexmk -c` if `clean` (`latex.R:128-146`) | `pdf-engine: latexmk` runs `latexmk` as the per-pass engine inside Q1's loop | **Both, as two settings.** `Engine::Latexmk` is Q1\'s meaning (a user-facing Q1 value). **`Driver::{Builtin, Latexmk}`** is R\'s: `Driver::Latexmk` runs the invocation at left once, with no recovery and no auto-install, and rejects a request that also sets run counts, a bibliography or an index (`CompileError::InvalidRequest`). Both flag forms were run on TinyTeX 2025 (`latexmk` 4.87) and build the PDF; R\'s form also halts on an error (exit 12) |
| Working directory | the file's directory is assumed (auxiliary file names come from the file's basename) | explicit: the `.tex` file's directory (`latex.ts:63-64`, `cwd` for the engine and the tools) | Q1's: the engine and every auxiliary tool run with the `.tex` file's directory as cwd; there is no separate working-directory field |
| Output dir / input paths | R `tinytex.output_dir` option; no input paths | `-output-directory` (relative to the `.tex` directory); `latex-input-paths` → `TEXINPUTS` **and** `BSTINPUTS`, `;`-joined with trailing `;` (append-default; `latex.ts:239-240`); no `BIBINPUTS` or `INDEXSTYLE` | Q1's. With an output directory outside the `.tex` directory, `bibtex` and `makeindex` refuse to write there under TeX Live's default `openout_any = p` (checked on TinyTeX; `pdflatex` itself is exempt through `-output-directory`); Q1 has the same limit and the crate does not set `openout_any` or `TEXMFOUTPUT` |
| Cleanup | removes only aux files this run created, on success and on failure (`latex.R:181-189`); the log is kept when the run failed or has warnings; always removes the stale `.log` and the stale output PDF first (Q1, `latex.ts:68-73`) | removes aux files before the build and, only after a success, after it (`pdf.ts:50-51`, `138-139`; list at `599-630`, plus `missfont.log`); leaves them on failure; removes the `.log` after a success | **`Clean::{Never, OnSuccess, Always}` and `KeepLog::{OnFailure, OnWarnings, Always, Never}`.** Crate defaults are R\'s (`Always`, `OnWarnings`); **q2 sets `OnSuccess` and `KeepLog::OnFailure`** (Quarto 1 parity), which also removes q2\'s post-hoc log deletion. `Clean::Never` keeps the auxiliary files and does not by itself keep the log: `KeepLog` decides the log, so a host that wants everything kept sets both (q2 does for `latex-clean: false` and `--debug`, plan 6.5b). Every setting deletes only files this run created (never a user\'s pre-existing files), the extension list is the union of both, and a cancelled run cleans like a failed one; verify with the biblatex/index e2e fixtures |
| Missing-package recovery scope | after every engine run (`latex.R:199-227`) | only after the first run (`pdf.ts:222`); a failed rerun is rejected at once (`pdf.ts:571-572`; the rerun predicate is at `578-579`) | **`Recover::{FirstRun, EveryRun}`.** `FirstRun` (Q1) is the default for the crate and for q2: it changes failure paths only, and R\'s behavior is the opt-in |
| Bibliography trigger | needs `\citation`, `\bibdata` and `\bibstyle` (`latex.R:294`) | the same three since `v1.12.3` (`pdf.ts:589-592`, #15380); `v1.11.6` ran on any one | **all three**; the two now agree |
| Hyphenation repeat; index-prep, hyphenation and extra-module reruns | n/a | the hyphenation install repeats the first run (`pdf.ts:190-192`); these reruns sit outside the run counter (`pdf.ts:118-119`) | Q1\'s repeat, with a guard against an install that does not clear the report; **one count**: `max_runs` bounds every engine run and `Compiled.runs` is the total |
| R failure-path steps | `delete_texmf_user()` (`latex.R:214`, `install.R:359-368`) and the Ghostscript and Homebrew `epstopdf` step (`tlmgr.R:124-130`) | `Hint::Ghostscript` only | **not adopted in the crate.** `delete_texmf_user` deletes a user directory, so a host that wants it retries on `CompileError::Latex` with an empty `missing`; the Ghostscript step is host-side from `Compiled.installed_packages`, `Event::PackagesInstalled` and the hint |
| DVI / XDV output | supported (`--no-pdf`, `latex` engine); its success check is only right for `engine == 'latex'` (`latex.R:119-121`) | none | out of v1; reserved by a `non_exhaustive` `Output::{Pdf}` on `CompileRequest` and `Compiled.output_kind`, so DVI is additive (`pdf_file` keeps its name) |
| PDF placement | `xfun::file_rename`; output relative to the process cwd (`latex.R:111-121`) | none | the crate renames; when `fs::rename` fails across filesystems it copies and removes. Output is relative to the `.tex` directory (row above) |

Log regex tables: **union**. Both tables are close to identical.

| Only in R `main` | Only in Quarto 1 |
|---|---|
| `glossaries`, `datetime2`, `polyglossia`, `babel` missing-module warnings after a *successful* run (`check_extra`, since 2021) | luaotfload font-fallback crash detection with an actionable message (quarto-cli#14558, upstream latex3/luaotfload#331) and the `;-fallback` suffix strip |
| `l3backend` mismatch regex (#424; `main` keeps it although the package left the bundle list in Aug 2026) | hyphenation via `Package babel Info: Hyphen rules for 'X' set to \l@nil`; `chinese`/`chinese-hans` skipped (R\'s later `R/hyphen.R` map now also covers the fatal `babel` error and `ngerman`→`hyphen-german`, so those two are in both) |
| `needs_rerun` accepts `Please \(?re\)?run` (so "Please rerun LaTeX.") | PDF/UA accessibility warnings (tagpdf missing alt text, `DocumentMetadata` language, others) |
| | `marginnote` resolving matcher; explicit-request pattern for a missing `.aux` (#6226) |
| | Ghostscript hints from stderr; `No pages of output` fallback; single-quote message variants; `luaotfload … Font "X" not found` |

Present in both (keep one implementation, port both tests): fonts with spaces, `.*:`/`:.*`
stripping (R #465, Q1 #12194), `latex-lab`/`DocumentMetadata`, `colorprofiles` via `pdfx`,
new `fontspec` format, babel `Unknown option` → `.ldf`, `lua-uni-algos`, `tikz` libraries,
`epstopdf`, `missfont.log`, `.fd` files, `widetext`.

Rerun detection: union of both pattern sets. Error presentation: R's block-per-`!`
extraction (each block ends at a blank line) rather than Q1's single greedy span from the
first `!` to "Here is how much"; hints: unsupported Unicode (both), inline math (Q1's
generic wording; R's version is Rmd-specific), Ghostscript and luaotfload (Q1).

### 3.2 Detection, tlmgr, install

| Area | R `main` | Quarto 1 | Adopted |
|---|---|---|---|
| Find TinyTeX | `tinytex.tlmgr.path` option; scans `<dir>/bin/*`; macOS also `/Library/TeX/texbin` | hard-coded per-OS dir and bin subdir; `universal-darwin` only | R\'s; **filesystem-only** (no subprocess). `Config.prefer` chooses TinyTeX or the system TeX (Q1\'s `latex-tinytex`) |
| "Is this TinyTeX?" | directory name, or a `.tinytex` marker file | path regex, or `TinyTeX` in `fmtutil.cnf` | both checks |
| Install-dir override | `TINYTEX_DIR` | none | R's |
| Windows non-ASCII `APPDATA` | falls back to `ProgramData` | same | same |
| Command environment | PATH prepended; **`TEXLIVE_PREFER_OWN=1`** around `tlmgr` only (use TeX Live\'s own curl, R #354; `tlmgr.R:48-52`) | PATH prepended; never sets it | R\'s, scoped to `tlmgr`, `fmtutil` and `updmap` (§2) |
| `tlmgr install` | one batch call; update first only if a package is already installed (`tlmgr.R:118`); any non-zero status or a package still missing: `update --self`, then install again once (`tlmgr.R:119-124`); the compile stops unless the status is 0 (`latex.R:205-214`) | one package per call (`texlive.ts:199`); accepts exit 0 **or 255** (`texlive.ts:265`); reruns after any attempt (`pkgmgr.ts:53`) | **the merge:** R\'s batch call; accept 0 or 255; on anything else retry once after `update --self`; rerun the engine after any attempt; deduplicate the package list; `InstallOutcome` carries a per-package result. Verify the batch behavior with an unknown package against real `tlmgr` (§7). `Tlmgr::install` never calls `tlmgr path add` (R does, at the end of its install): q2\'s compile-driven installs must not touch `/usr/local/bin` or `/etc/paths.d` |
| `tlmgr update` | **lazy, but repeated:** on every engine failure (`latex.R:223`, no opt-out) and before every install when a package is already present (`tlmgr.R:118`, the `tinytex.tlmgr_update` opt-out). `update_noted` is set **only** when the update reports that TeX Live is older than the remote (`tlmgr.R:217`, `236`), skips every later update, and is reset at the end of each `latexmk()` call (`latex.R:189`); that case also skips `fmtutil` | **lazy as well:** after the first failed engine run, once, `update --self` then `update --all`, then `fmtutil` (`pdf.ts:220-235`); and per package in `installPackage` (`texlive.ts:232-246`, `271-293`), repeated for every already-installed package | **`TlmgrUpdate::{Auto, Never}`** (replaces the bool). `Auto` is lazy like both and **at most one update per distribution per process**, a crate rule stricter than either upstream (R repeats on every failure and install, Quarto 1 once per compile plus per package; §2). The `update --self` retry inside `install` is not subject to it. `Never` skips both the on-failure and the pre-install update. (An earlier draft called Quarto 1 eager and R once-per-compile; both were wrong.) |
| "Older than remote repository" | detected in `update`, reported once; optional auto-upgrade | message only from `findPackages` | typed `UpdateOutcome::TexLiveTooOld` (R); auto-upgrade belongs to `install` (M5) |
| `fmtutil` | exit code discarded | non-fatal warning (`fmtutilFailureMessage`) | typed `FmtutilWarning`, non-fatal by construction |
| Package search | `tlmgr search --file --global`, parse `/name$`, strip platform suffix; shortcuts `fandol`, `latex-lab` | same; shortcuts `fandol`, `latex-lab`, `colorprofiles` | Q1's (larger) shortcut list |
| Path integration | `tlmgr path add` only when needed; macOS `/etc/paths.d/TinyTeX` when `/usr/local/bin` is unwritable | `path add` only for a system TeX Live; requires writable `/usr/local/bin` on macOS | R's |
| Repository choice | explicit → `tlnet.yihui.org` (probe: HTTP < 400, not `text/html`) → `mirror.ctan.org` redirect | same probe (`isTlnet`); explicit override → `tlnet.yihui.org` → `mirror.ctan.org` → random pick from a fixed mirror list | same order, with Q1's fixed list as the last resort |
| tlgpg | `https://texlive.info/tlgpg/` (Jul 2026) | `http://www.preining.info/tlgpg/` | R's |
| Bundle assets | new `TinyTeX-<os>-<arch>-v<ver>.tar.xz` names, arm64, musl, source-install fallback | new names for darwin/linux/arm64; no musl; no fallback | R's `binary_supported` logic |
| Post-install config | `sys_bin`, `postaction install script xetex`, `max_print_line 10000`, `path add`, R texmf tree, repo | `sys_bin`, `postaction …`, repo, `path add` | R's, **minus the R texmf tree** (R-specific) |
| Installed version | none beyond `tlmgr --version` | `version` file in the install dir | both |
| Reinstall/update | preserves package list and `texmf-local` | `quarto update tinytex` | R's preservation |
| Uninstall | `path remove`, remove R texmf tree, user texmf, directory | `path remove`, directory | union, minus the R texmf tree |
| Extraction errors | requires `xz` for `.tar.xz` (R #501) | hint to install `xz-utils` | Q1's hint |

### 3.3 R commits after the baseline

R `main` is at `e78c2f7` (2026-11-09); the pinned `7818777` is 2026-09-17. What changed since,
and what this crate does with it:

| R commit | Change | This crate |
|---|---|---|
| `b757aa5`, `872c51e` (#516, #517) | install `hyphen-*` for the fatal `babel` "haven't defined the language" error; shared language map in `R/hyphen.R` | **adopt**; Quarto 1 `v1.12.3` already has the same handling (`parse-error.ts:271-285`), so this is agreement, not an R-only addition (§3.1); one language map |
| `56cd10d` | opt-in `tinytex.prescan_packages` | **defer** (no Quarto 1 counterpart; a later `Recover` variant would be its home) |
| `fb7a799` | remove Rtools directories from `PATH` while running `tlmgr` on Windows | **adopt** (§2) |
| `e78c2f7` | sets `TEXLIVE_PREFER_OWN=1` in R\'s build workflow | none (confirms §3.2) |

Re-run `git log 7818777..origin/main -- R/` when the crate\'s first milestone starts; the table
above is the audit up to 2026-11-09.

## 4. Feature inventory: what exists, and what q1/q2 need

Disposition: **v1** = milestones M1–M3 (needed by the latex/pdf epic); **M5** = the
`install` feature, designed now, built later; **reserved** = API leaves room; **cut** =
not planned. "Needed" = used by Quarto 1 today, therefore by q2.

| Capability | R | TS | Needed | Disposition |
|---|---|---|---|---|
| Detect install, `bin_dir`, `is_tinytex`, installed version | ✓ | ✓ | yes (`TinyTexBinDir` param, compile, `check`) | v1 (M2) |
| Command environment: PATH prepend, `TEXLIVE_PREFER_OWN`, Windows wrapping | partial | partial | yes | v1 (M2) |
| `tlmgr` search / install / update / remove / path / version / info / repository | ✓ | ✓ | yes | v1 (M2) |
| `tlmgr` passthrough options (`latex-tlmgr-opts`) | — | ✓ | yes | v1 (`Config`) |
| Compile loop with missing-package recovery | ✓ | ✓ | yes | v1 (M3) |
| Bibliography and index passes | ✓ | ✓ | yes | v1 (M3) |
| Log parsing and hints | ✓ | ✓ | yes | v1 (M1) |
| PDF/UA accessibility, luaotfload, `marginnote`, Ghostscript hints | — | ✓ | yes | v1 (M1, structured data; Quarto-worded messages live in q2's plans 6.8a and 6.8b) |
| `Engine::Tectonic` direct call | ✓ | via Pandoc | yes (stage always runs) | v1 (M3) |
| `Engine::Latexmk` as per-pass engine | — | ✓ | yes (`pdf-engine: latexmk`) | v1 (M3) |
| Surface LaTeX warnings | as R warnings | selected | yes | v1 (`Diagnostics`) |
| Real-`latexmk` delegation (`emulation = FALSE`) | ✓ | — | not by q2 | v1 (M3): `Driver::Latexmk` (§5); q2 never sets it |
| DVI / XDV output, `latex` engine | ✓ | — | no | reserved: `Output` enum, `Compiled.output_kind` |
| Install TinyTeX; latest release and asset resolution; repository probing | ✓ | ✓ | yes, for `q2 install tinytex` (strand `bd-6s1fi9c7`) | M5 |
| Update/reinstall TinyTeX preserving packages | ✓ | ✓ | yes, eventually | M5 |
| Uninstall | ✓ | ✓ | yes, eventually | M5 |
| Source install, musl | ✓ | — | no | `Unsupported` error now; later |
| Bundle choice (`TinyTeX-0/1/2`), pinned versions | ✓ | fixed latest | no | `Bundle`/`Version` enums, default = Quarto's `TinyTeX` bundle, latest (M5) |
| `TINYTEX_PREVENT_INSTALL`, `TINYTEX_DIR` | ✓ | — | guard only | `Config` fields |
| Relocation (`copy_tinytex`, `use_tinytex`), `tinytex_root` | ✓ | — | no | reserved |
| `tlmgr conf`, `usermode`, `tl_sizes`, `tl_platforms`, `ctan_mirrors`, package listing | ✓ | — | no (except package listing, for M5's reinstall) | `Tlmgr::run(&[..])` escape hatch; `installed_packages()` in M5 |
| `texhash`, `updmap`, `fc_cache`, `kpsewhich`, `refresh_all`, `open_tlmgr` | ✓ | partial | no | cut; `Distribution::command()` covers ad-hoc use |
| R's texmf tree (`r_texmf`) | ✓ | — | no | cut (R-specific) |
| `tinytex` command-line interface (§5.1) | — (R API only) | an unused `quarto-latexmk` command with its own flags (executable name `tinitex`, sic) | not by q2, which links the library; needed for adoption by R and Quarto 1 (§9.2) | M6: `compile`, `detect`, `tlmgr`, `log` and the flag-parity tests (`--print-request`, request round-trip, flag-coverage walk). Scheduled after the q2 compile-stage plans have exercised the API, so that the JSON shape is not frozen before real use; the `serde` groundwork and stable error codes land with M3 |

## 5. Public API

Module layout:

```
tinytex-rs
├── exec      CommandSpec, Exec, StdExec, Observer, Event
├── config    Config
├── log       pure: log text → diagnostics (no IO)
├── detect    Distribution, find_tinytex, find
├── tlmgr     Tlmgr
├── compile   CompileRequest, Compiler, Engine, Bib, ToolPaths, Compiled, CompileError
├── cli       (feature "cli") — clap parser into CompileRequest and friends; the `tinytex` binary (§5.1)
├── install   (feature "install")  — M5: step functions; defines the `Fetcher` trait
└── install::download  (feature "download", implies "install") — M5: UreqFetcher, unpack(),
                       and the one-call install()/update()
```

```rust
// ---------- exec ----------
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub cwd: Option<PathBuf>,
    pub changes_installation: bool,               // install, update, remove, path, repository,
}                                                 // fmtutil: a host may shield it from cancellation
pub trait Exec: Send + Sync {                     // blocking; StdExec is the default
    fn run(&self, cmd: &CommandSpec) -> io::Result<Output>;   // captured stdout/stderr
    fn interrupted(&self) -> bool { false }       // cancellation; polled by loops and lock waits,
}                                                 // and after a `changes_installation` command
pub trait Observer: Send + Sync {                 // every method defaults to a no-op
    fn event(&self, _e: &Event) {}
}
pub struct NullObserver;                          // impl Observer; `&NullObserver` for no progress
#[derive(Serialize, Deserialize)]                 // every variant also has a `Display` text
#[non_exhaustive]
pub enum Event {
    Searching { term: SearchTerm },
    InstallingPackage { name: String, index: usize, of: usize },
    UpdatingTlmgr,
    UpdatingPackages,
    RebuildingFormats,
    RunningEngine { run: u32 },                   // total run number, first included
    MakingIndex { command: String },
    RunningBibEngine { engine: String },
    InstallingHyphenation { file: String },
    PackagesInstalled { names: Vec<String> },     // after an install attempt; hosts add their own steps
    Downloading { url: String, received: u64, total: Option<u64> },   // install feature
}

// ---------- config ----------
/// Every field has a CLI flag or is environment-only (§5.1). `Default` is written by hand:
/// `tlmgr_update` `Auto`, `prefer` `TinyTeX`, everything else empty or false.
#[derive(Serialize, Deserialize)]
#[non_exhaustive]
pub struct Config {
    pub install_dir: Option<PathBuf>,             // TINYTEX_DIR; `--install-dir`
    pub repository: Option<String>,               // CTAN_REPO / caller override; `--repository`
    pub tlmgr_opts: Vec<String>,                  // latex-tlmgr-opts; `--tlmgr-opt`
    pub prevent_install: bool,                    // TINYTEX_PREVENT_INSTALL (M5); environment only
    pub tlmgr_update: TlmgrUpdate,                // `--tlmgr-update`
    pub prefer: Prefer,                           // `--prefer`; Q1's `latex-tinytex`
    pub tlmgr_path: Option<PathBuf>,              // R's `tinytex.tlmgr.path`; `--tlmgr-path`; wins over `prefer`
    pub usermode: bool,                           // R's `usermode`; `--usermode`
    pub delete_tlpdb: bool,                       // R's `delete_tlpdb`; `--delete-tlpdb`
    pub verbose: bool,                            // `--verbose`
}
/// `Auto`: update lazily, at most once per process and distribution (§2, §3.2). `Never`: skip the
/// on-failure and the pre-install update (R\'s `tinytex.tlmgr_update = FALSE`, extended to both).
#[derive(Default, Serialize, Deserialize)] #[non_exhaustive]
pub enum TlmgrUpdate { #[default] Auto, Never }
impl Config { pub fn from_env() -> Self; }

// ---------- detect ----------
#[non_exhaustive] pub enum Kind { TinyTeX, TexLive }    // TexLive = any other tlmgr-based install
#[derive(Default, Serialize, Deserialize)]
pub enum Prefer { #[default] TinyTeX, System }           // Q1's `latex-tinytex: false` = System
pub struct Distribution {
    pub root: Option<PathBuf>,                    // unknown for some system installs
    pub bin_dir: PathBuf,
    pub tlmgr: PathBuf,
    pub kind: Kind,
}
pub fn default_install_dir(cfg: &Config) -> Option<PathBuf>;   // ~/.TinyTeX, ~/Library/TinyTeX,
                                                               // %APPDATA% → ProgramData if non-ASCII
pub fn find_tinytex(cfg: &Config) -> Option<Distribution>;     // FILESYSTEM ONLY; called every render
pub fn find(cfg: &Config) -> Option<Distribution>;             // `cfg.prefer`: TinyTeX dir, else tlmgr on PATH
impl Distribution {
    pub fn is_tinytex(&self) -> bool;                          // name, `.tinytex`, or fmtutil.cnf
    pub fn is_writable(&self) -> bool;                         // R's `tlmgr_writable()`: is_tinytex() and the
                                                               // `tlmgr` script passes a real access probe
                                                               // (write test, not metadata); tests inject the probe
    pub fn installed_version(&self) -> Option<String>;         // the `version` file
    pub fn tex_live_year(&self, x: &dyn Exec) -> Option<u32>;  // spawns `tlmgr --version`
    pub fn command(&self, program: &str) -> CommandSpec;       // PATH-prepended; TEXLIVE_PREFER_OWN=1 for
}                                                              // tlmgr/fmtutil/updmap only (§2); no temp files

// ---------- tlmgr ----------
pub struct Tlmgr<'a> { /* &Distribution, &dyn Exec, &Config, &dyn Observer */ }
impl<'a> Tlmgr<'a> {
    pub fn new(dist: &'a Distribution, cfg: &'a Config, x: &'a dyn Exec, o: &'a dyn Observer) -> Self;
    // emits Searching, InstallingPackage, UpdatingTlmgr, UpdatingPackages, RebuildingFormats
}
impl Tlmgr<'_> {
    pub fn version(&self) -> Result<TlVersion, TlmgrError>;               // year + tlmgr date
    pub fn search_file(&self, term: &SearchTerm) -> Result<Vec<String>, TlmgrError>;
    pub fn info(&self, pkgs: &[String], fields: &[&str], only_installed: bool)
        -> Result<Vec<Vec<String>>, TlmgrError>;                          // `info --list --data f1,f2`; one row per line
    pub fn repository(&self) -> Result<String, TlmgrError>;               // `option repository`
    pub fn packages_for_log(&self, log: &str, missfont: Option<&str>)     // R `parse_packages`
        -> Result<Vec<String>, TlmgrError>;
    pub fn install_for_log(&self, log: &str, missfont: Option<&str>)      // R `parse_install`
        -> Result<InstallOutcome, TlmgrError>;
    pub fn is_installed(&self, pkg: &str) -> bool;
    pub fn install(&self, pkgs: &[String]) -> Result<InstallOutcome, TlmgrError>;  // batch; recheck under the lock;
                                                                                    // accept 0 or 255; retry once (§3.2)
    pub fn update(&self, opts: UpdateOpts) -> Result<UpdateOutcome, TlmgrError>;   // Updated | TexLiveTooOld{..} | Skipped
    pub fn remove(&self, pkgs: &[String]) -> Result<(), TlmgrError>;
    pub fn path(&self, action: PathAction) -> Result<(), TlmgrError>;              // `path add|remove`; macOS /etc/paths.d inside
    pub fn set_repository(&self, url: &str) -> Result<(), TlmgrError>;
    pub fn fmtutil(&self) -> Result<(), FmtutilWarning>;                           // non-fatal by design
    pub fn run(&self, args: &[&str], changes_installation: bool)                    // escape hatch; the caller
        -> Result<Output, TlmgrError>;                                              // classifies the command (§2)
}
/// Result types (serde on all): `TlVersion { year: u32, date: String }`;
/// `InstallOutcome { installed, already_present, failed: Vec<String>, warnings: Vec<FmtutilWarning> }`
/// (per-package; a package is `failed` when still missing after the retry; `warnings` are the
/// non-fatal `fmtutil` failures of the format rebuilds the install ran);
/// `UpdateOutcome::{ Updated { warning: Option<FmtutilWarning> }, TexLiveTooOld { local: u32, remote: u32 },
/// Skipped }` (`Skipped`: `TlmgrUpdate::Never`, or an update already ran in this process);
/// `UpdateOpts { all: bool, self_only: bool, run_fmtutil: bool, delete_tlpdb: bool, more_args: Vec<String> }`
/// (default: `all`, `run_fmtutil`; `self_only` means `update --self` alone and overrides `all`; without
/// it `all` runs `update --all --self`, as R does, and `--self` is dropped in user mode);
/// `PathAction::{ Add, Remove }`; `FmtutilWarning { message: String, exit_code: Option<i32> }`.
/// `Tlmgr::install` and `update` hold the install lock; `install_locked(&self, &InstallGuard, ..)`
/// is private and takes the witness (§2).
/// `TlmgrError` (`#[non_exhaustive]`, `Serialize` only; stable codes `tlmgr.io`, `tlmgr.command_failed`,
/// `tlmgr.texlive_too_old`, `tlmgr.interrupted`, `tlmgr.unparseable`): `Io`, `Command { program, args, code,
/// stderr }`, `TexLiveTooOld { local, remote }` (returned by `install` when packages stay missing and the
/// distribution is known to be a release behind), `Interrupted`, `Unparseable { what, output }`.
/// `TlmgrError::Interrupted`: `interrupted()` was true after a command returned, `Ok` or
/// `Err` (§2); the compile maps it to `Cancelled`.

// ---------- compile ----------
#[non_exhaustive]
pub enum Engine { PdfLatex, XeLatex, LuaLatex, Latexmk, Tectonic }
pub enum Bib { Infer, Bibtex, Biber, Skip }        // Skip = citeproc; Infer = R main's .bcf/.aux inference
/// Who runs the engine repeatedly. `Builtin`: this crate\'s loop (R: `emulation = TRUE`).
/// `Latexmk`: the real `latexmk` runs the whole build, once (R: `emulation = FALSE`); the
/// request must then leave run counts, `bib` and `index` at their defaults, or `compile`
/// returns `CompileError::InvalidRequest`. Not `Engine::Latexmk` (§1).
#[derive(Default, Serialize, Deserialize)] #[non_exhaustive]
pub enum Driver { #[default] Builtin, Latexmk }
/// Package recovery after a failed engine run: `FirstRun` (Quarto 1) or `EveryRun` (R).
#[derive(Default, Serialize, Deserialize)] #[non_exhaustive]
pub enum Recover { #[default] FirstRun, EveryRun }
/// A non-zero exit from the bibliography tool: `Error` (R) fails the compile
/// (`CompileError::Bibliography`); `Warn` (Quarto 1) records
/// `Diagnostics.bibliography_failed` and continues.
#[derive(Default, Serialize, Deserialize)] #[non_exhaustive]
pub enum OnBibFailure { #[default] Error, Warn }
/// Which files the cleanup removes: only ones this run created (never a user\'s).
#[derive(Default, Serialize, Deserialize)] #[non_exhaustive]
pub enum Clean { Never, OnSuccess, #[default] Always }
#[derive(Default, Serialize, Deserialize)] #[non_exhaustive]
pub enum KeepLog { OnFailure, #[default] OnWarnings, Always, Never }
#[derive(Default, Serialize, Deserialize)] #[non_exhaustive]
pub enum Output { #[default] Pdf }                 // DVI is additive later
pub struct IndexEngine { pub command: String, pub args: Vec<String> }
#[derive(Default)]
pub struct ToolPaths {                             // host overrides (the CLI's `--tool name=path`);
                                                   // q2 sets none: PATH, `latex-tinytex` and TINYTEX_DIR find tools (D6).
                                                   // Resolution order for every tool: this struct, then the distribution's
                                                   // `bin_dir`, then `PATH`; an entry that does not exist is `EngineNotFound`
    pub engine: Option<PathBuf>, pub bibtex: Option<PathBuf>, pub biber: Option<PathBuf>,
    pub makeindex: Option<PathBuf>, pub latexmk: Option<PathBuf>, pub tectonic: Option<PathBuf>,
}
/// The whole compile contract, as plain serializable data. The library takes it directly
/// and the CLI parses its flags into it (§5.1); there is no option that exists in only one.
///
/// `Default` is written by hand, not derived: `engine` `PdfLatex`, `driver` `Builtin`,
/// `engine_args` empty, `bib` `Infer`, `index` `None`, `min_runs` 1 and `max_runs` 10 (a stored
/// 0 means unset and reads as 1 and 10, as Quarto 1's `(minRuns || 1)` and `(maxRuns || 10)`
/// do, `pdf.ts:118-119`), `output_dir` and `pdf_file` `None`, `input_paths` empty, `tools`
/// empty, `auto_install` `false` (the host sets it: q2 follows D45; the CLI uses
/// `Distribution::is_writable`), `recover` `FirstRun`, `on_bib_failure` `Error`, `clean`
/// `Always`, `keep_log` `OnWarnings`, `output` `Pdf`. **These are the crate\'s defaults, R\'s
/// where R and Quarto 1 differ; q2 sets `on_bib_failure` `Warn`, `clean` `OnSuccess` and
/// `keep_log` `OnFailure` (Quarto 1\'s), plan 6.6.**
#[derive(Serialize, Deserialize)]
#[non_exhaustive]
pub struct CompileRequest {
    pub engine: Engine,
    pub driver: Driver,                            // `--driver`
    pub engine_args: Vec<String>,                  // pdf-engine-opt(s)
    pub bib: Bib,
    pub index: Option<IndexEngine>,                // latex-makeindex[-opts]; `None` = makeindex, no options. The pass runs
                                                   // whenever the first run left an `.idx`; there is no way to skip it
    pub min_runs: u32, pub max_runs: u32,          // total engine runs, first included
    pub output_dir: Option<PathBuf>,               // -output-directory; relative to the `.tex` directory; the crate creates it
    pub pdf_file: Option<PathBuf>,                 // the crate renames the output PDF to this path
    pub input_paths: Vec<PathBuf>,                 // TEXINPUTS and BSTINPUTS (not BIBINPUTS)
    pub tools: ToolPaths,
    pub auto_install: bool,                        // latex-auto-install
    pub recover: Recover,                          // `--recover`
    pub on_bib_failure: OnBibFailure,              // `--on-bib-failure`
    pub clean: Clean,                              // latex-clean (`false` = `Never`, `true` = `OnSuccess`)
    pub keep_log: KeepLog,                         // `--keep-log`
    pub output: Output,                            // `--output`
}
// The engine and every auxiliary tool run with the `.tex` file's directory as their cwd.
impl CompileRequest {                              // chained setters, one per field
    pub fn new() -> Self;                          // = Default
    pub fn engine(self, e: Engine) -> Self;
    pub fn driver(self, d: Driver) -> Self;
    pub fn engine_args(self, a: Vec<String>) -> Self;
    pub fn bib(self, b: Bib) -> Self;
    pub fn index(self, i: IndexEngine) -> Self;
    pub fn runs(self, min: u32, max: u32) -> Self;      // total runs, first included
    pub fn reruns(self, min: u32, max: u32) -> Self;    // R\'s meaning: runs(min + 1, max + 1)
    pub fn output_dir(self, d: PathBuf) -> Self;
    pub fn pdf_file(self, p: PathBuf) -> Self;
    pub fn input_paths(self, d: Vec<PathBuf>) -> Self;
    pub fn tools(self, t: ToolPaths) -> Self;
    pub fn auto_install(self, on: bool) -> Self;
    pub fn recover(self, r: Recover) -> Self;
    pub fn on_bib_failure(self, f: OnBibFailure) -> Self;
    pub fn clean(self, c: Clean) -> Self;
    pub fn keep_log(self, k: KeepLog) -> Self;
    pub fn output(self, o: Output) -> Self;
}
pub struct Compiler { /* distribution + config */ }
impl Compiler {
    pub fn new(dist: Option<Distribution>, cfg: Config) -> Self;   // None = tools from PATH only
    pub fn auto(cfg: Config) -> Self;                              // `detect::find(&cfg)`, then `new`
    pub fn compile(&self, tex: &Path, req: &CompileRequest, x: &dyn Exec, o: &dyn Observer)
        -> Result<Compiled, CompileError>;
}
/// The auxiliary files the compile's cleanup removes: the union of R's (`latexmk_emu`) and
/// Quarto 1's (`cleanup()`) lists, 21 entries. The first 19 are extensions of the document's
/// file (`<stem>.log`, `.idx`, `.aux`, `.bcf`, `.blg`, `.bbl`, `.fls`, `.out`, `.lof`, `.lot`,
/// `.toc`, `.nav`, `.snm`, `.vrb`, `.ilg`, `.ind`, `.xwm`, `.brf`, `.run.xml`); the last two are
/// whole file names, R's `preview.aux` and Quarto 1's `missfont.log`. Hosts that watch the
/// working directory take the list from here.
pub const AUX_EXTENSIONS: &[&str];
/// The 21 file names of `AUX_EXTENSIONS` for the document `stem` (`doc.aux`, ..., `preview.aux`).
pub fn aux_files(stem: &str) -> Vec<String>;
#[derive(Serialize, Deserialize)]
pub struct Compiled {
    pub pdf: PathBuf,                              // the output file; named `pdf` while `Output` has one variant
    pub output_kind: Output,
    pub log: Option<PathBuf>,                      // Some only if the file survives cleanup; None for tectonic
    pub runs: u32,
    pub installed_packages: Vec<String>,
    pub diagnostics: Diagnostics,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Diagnostics {
    pub warnings: Vec<String>,                     // `latex_warnings` (R's set)
    pub accessibility: AccessibilityWarnings,
    pub missing_hyphenation: Vec<String>,          // files not installed (auto-install off or failed)
    pub max_runs_reached: bool,
    pub bibliography_failed: Option<String>,       // `Warn`: the tool and a log excerpt (Quarto 1 continues)
    pub notices: Vec<String>,                      // non-fatal facts with no other channel (a failed `fmtutil`)
}
#[derive(Default, Serialize, Deserialize)]
pub struct AccessibilityWarnings {                 // Quarto 1's shape (`parse-error.ts:111-115`)
    pub missing_alt_text: Vec<String>,             // image file names
    pub missing_language: bool,
    pub other_warnings: Vec<String>,               // other tagpdf warnings
}
#[non_exhaustive]
pub enum CompileError {
    NoTexInstallation,
    EngineNotFound { tool: String },
    Latex { summary: Option<String>, hints: Vec<Hint>, log: Option<PathBuf>,
            missing: Vec<SearchTerm>,              // `log::missing_packages` of the failed run's log
            stdout: String, stderr: String, exit_code: Option<i32> },
    PackageInstall { reason: InstallFailure, source: TlmgrError },
    NoOutput,                                      // the engine exited 0 and wrote no output file
    InvalidRequest { message: String },            // e.g. `Driver::Latexmk` with run counts
    Bibliography { engine: String, log_excerpt: String, log: Option<PathBuf> },   // `log`: the engine log, kept
    Index { log_excerpt: String, log: Option<PathBuf> },
    Cancelled,                                     // Exec::interrupted() turned true
    Io(io::Error),
}

// ---------- log (pure; no IO) ----------
pub fn missing_packages(log: &str, missfont: Option<&str>) -> Vec<SearchTerm>;
pub fn latex_error(log: &str, stderr: Option<&str>) -> Option<LatexError>;   // summary + Vec<Hint>
pub struct LatexError { pub summary: String, pub hints: Vec<Hint> }              // one bounded block per `!`
/// What `missing_packages` returns: an argument for `tlmgr search --file --global`
/// (for example `/pgfplots.sty`), or a package named directly by one of the shortcuts
/// (`fandol`, `latex-lab`, `colorprofiles`) that needs no search.
#[non_exhaustive] pub enum SearchTerm { File(String), Package(String) }
/// Why a package install failed: Quarto 1 tells these apart (`pdf.ts:483,490,498`,
/// `texlive.ts:392-399`) and the host words each differently.
#[non_exhaustive] pub enum InstallFailure { NoMatch, InstallFailed, TexLiveTooOld }
pub fn needs_rerun(log: &str) -> bool;
pub fn missing_hyphenation(log: &str) -> Vec<String>;
pub fn missing_modules(log: &str) -> Vec<String>;     // glossaries, datetime2, polyglossia, babel (check_extra)
pub fn latex_warnings(log: &str) -> Vec<String>;
pub fn accessibility(log: &str) -> AccessibilityWarnings;   // structured (see above); wording belongs to the caller
pub enum Hint { UnsupportedUnicode, InlineMath, Ghostscript, LuaotfloadFallback /* … */ }
// `SearchTerm` has a `Display` that strips the leading `/` of a `File` term (`pgfplots.sty`).
// Hint wording is the host\'s (q2 plan 6.8a; Quarto 1\'s text is at `parse-error.ts:527-546`).
// When the log has no `!` block (the luaotfload crash, `parse-error.ts:297-304`),
// `LatexError.summary` is that crash\'s one-line text.

// ---------- install (feature "install"; M5; not used by the latex/pdf epic) ----------
// Layer 1 — steps. No HTTP client and no archive dependencies. This is what q2's installer
// framework (strand bd-6s1fi9c7) drives. That framework hosts several tools (TinyTeX
// first; verapdf and chromium expected) and owns the shared download / extract /
// prerequisite / progress code they all need.
pub trait Fetcher {                                              // implemented by the caller
    fn get_to_file(&self, url: &str, dest: &Path) -> Result<(), FetchError>;
    fn head(&self, url: &str) -> Result<HeadInfo, FetchError>;   // repository probe
    fn latest_release(&self, repo: &str) -> Result<ReleaseInfo, FetchError>;
}
pub fn latest_release(f: &dyn Fetcher) -> Result<ReleaseInfo, InstallError>;   // rstudio/tinytex-releases
pub fn select_asset(rel: &ReleaseInfo, bundle: Bundle, platform: Platform)
    -> Result<Asset, InstallError>;           // new asset names, then old; Unsupported (musl, unknown OS) for now
pub fn resolve_repository(cfg: &Config, f: &dyn Fetcher) -> String;
pub fn configure(dist: &Distribution, o: ConfigureOptions, x: &dyn Exec, obs: &dyn Observer)
    -> Result<ConfigureReport, InstallError>; // sys_bin, postaction xetex, max_print_line, repository,
                                              // path add; reports restart_required
pub fn uninstall(dist: &Distribution, x: &dyn Exec, obs: &dyn Observer) -> Result<(), InstallError>;

// Layer 2 — one-call convenience for standalone users (feature "download")
pub struct UreqFetcher;                       // impl Fetcher
pub fn unpack(archive: &Path, dest: &Path) -> Result<(), InstallError>;   // zip, tar.gz, tar.xz, Windows self-extracting .exe
pub fn install(cfg: &Config, o: InstallOptions, x: &dyn Exec, f: &dyn Fetcher, obs: &dyn Observer)
    -> Result<InstallReport, InstallError>;   // latest_release → select_asset → download → unpack → configure
pub fn update(/* … */) -> Result<InstallReport, InstallError>;   // reinstall, preserving packages and texmf-local
```

### 5.1 The `tinytex` command-line interface

The binary is a **thin layer over the library**: the same operations, reached through flags
instead of function calls. Its shape is decided by three rules.

1. **The library defines the features; the CLI only names them.** Every flag that
   describes a compile sets exactly one field of `CompileRequest` or `Config`, and every
   field of both has a flag, except the ones documented as environment-only
   (`Config.prevent_install`); the coverage test walks `Config` as well as
   `CompileRequest`. The five flags that choose how the command takes its input or prints its output (`--request`,
   `--print-request`, `--json`, `--events`, `--timeout`) set none. The CLI has no behavior
   the library lacks, and no working-directory flag: the working directory is the `.tex`
   file's directory.
2. **Flag names come from Quarto 1's `quarto-latexmk` command**, the only existing driver
   interface that covers the full feature set (run counts, auto-install, `tlmgr` options).
   The CLI adds no `latexmk`-style aliases (`-outdir`, `-pdf`, `-xelatex`, `-lualatex`): they
   would be a second spelling of flags that already exist, and the CLI is not a drop-in
   `latexmk`. They can be added if R's maintainers ask.
3. **Output is the library's serialized types.** `--json` prints `Compiled` (or the
   serialized `CompileError`, with its stable code) on stdout; progress events are `Event`
   values, printed as text or, with `--events json`, as JSON lines on stderr.

Commands mirror the API modules:

| Command | Library call |
|---|---|
| `tinytex compile <file.tex>` | `Compiler::compile` |
| `tinytex detect` | `detect::find` / `find_tinytex` |
| `tinytex tlmgr <operation> …` | `Tlmgr` methods; `tlmgr run -- <args>` is the escape hatch |
| `tinytex log <file.log>` | the `log` module, printing diagnostics |
| `tinytex install`, `uninstall`, `update` (M5, `download` feature) | `install` |

`compile` flags:

| Flag | `CompileRequest` field | Alias | Quarto 1 `quarto-latexmk` | R `tinytex::latexmk()` |
|---|---|---|---|---|
| `<file.tex>` | the `tex` argument | | `<input>` | `file` |
| `--pdf-engine <name>` | `engine` | | `--pdf-engine` | `engine` |
| `--pdf-engine-opt <arg>` (repeatable; `=` form also accepted) | `engine_args` | | `--pdf-engine-opt=` | `engine_args` |
| `--bib-engine <bibtex\|biber\|infer\|skip>` (also accepts `natbib` = `bibtex` and `biblatex` = `biber`) | `bib` | | `--bib-engine` (takes `biblatex`\|`natbib`, a citation method) | `bib_engine` |
| `--index-engine <cmd>`, `--index-engine-opt <arg>` | `index` | | same names | none |
| `--min <n>`, `--max <n>` | `min_runs`, `max_runs` (total runs, first included; R\'s `min_times = 1, max_times = 10` is `--min 2 --max 11`) | | same names | `min_times`, `max_times` |
| `--driver <builtin\|latexmk>` | `driver` | | none | `emulation` (`TRUE` = `builtin`) |
| `--recover <first-run\|every-run>` | `recover` | | none (first run) | every run |
| `--on-bib-failure <error\|warn>` | `on_bib_failure` | | none (warns) | error |
| `--keep-log <on-failure\|on-warnings\|always\|never>` | `keep_log` | | none | none (on warnings) |
| `--output <pdf>` | `output` | | none | `--no-pdf` is DVI, reserved |
| `--output-dir <dir>` | `output_dir` | | `--output-dir` | `tinytex.output_dir` option |
| `--pdf-file <path>` | `pdf_file` | | none | `pdf_file` |
| `--input-path <dir>` (repeatable) | `input_paths` | | none (`latex-input-paths` is a Quarto YAML key) | none |
| `--clean <never\|on-success\|always>`; `--no-clean` = `--clean never` | `clean` | | `--no-clean` | `clean` (`TRUE` = `always`) |
| `--auto-install` | `auto_install = true` | | none (Quarto 1's default is on) | `install_packages = TRUE` |
| `--no-auto-install` | `auto_install = false` (with neither flag the CLI sets `auto_install` from `Distribution::is_writable()`, as R does) | | `--no-auto-install` | `install_packages = FALSE` |
| `--tlmgr-opt <arg>` (repeatable) | `Config.tlmgr_opts` (the `config` half of the printed `{request, config}`) | | `--tlmgr-opt=` | none |
| `--tlmgr-update <auto\|never>` | `Config.tlmgr_update` | | none | `tinytex.tlmgr_update` |
| `--prefer <tinytex\|system>` | `Config.prefer` | | `latex-tinytex` (YAML) | none |
| `--install-dir <dir>`, `--repository <url>` | `Config.install_dir`, `Config.repository` | | none | `TINYTEX_DIR`, `tinytex.repository` |
| `--tool <name>=<path>` (repeatable) | `tools` | | none | none |
| `--request <file\|->` | the whole request, from JSON | | none | none |
| `--print-request` | prints `{request, config}` as the flags produce them, then exits | | none | none |
| `--json`, `--events json` | output format only | | none | none |

`--bib-engine` takes the *program* (`bibtex`, `biber`) and accepts Quarto 1\'s citation
methods (`biblatex`, `natbib`) as aliases; q2\'s stage passes explicit values (D24). Quarto 1\'s
`quarto-latexmk --no-auto-mk` is not a flag here: that command\'s parser reads it and
nothing in the command uses it. (The `latex-auto-mk` **YAML key** is live in Quarto 1,
`latexmk.ts:18`, and q2 plans 2.2 and 6.9 handle it; it is Pandoc running the engine, not
`--driver`.)

**Keeping the two in step.** `--print-request` makes parity a test, not a promise: for each
flag, a test runs the CLI parser and asserts that the printed `{request, config}` equals the
`CompileRequest` and `Config` built through the library's setters (`Config` derives
`Serialize` and `Deserialize` for this), and a second test round-trips the
JSON through `--request`. A flag that is neither in the parity table above nor on the
five-flag exception list fails a coverage test that walks the `clap` definition.

**JSON and errors in the CLI.** Requests, `Config` and events derive `Serialize` and
`Deserialize`; results (`Compiled`, `Distribution`, `TlVersion`, `InstallOutcome`,
`UpdateOutcome`) derive both too, so a Rust or non-Rust host can read the output back;
`CompileError` has a hand-written `Serialize` only (it holds an `io::Error`). The `schema`
field is added by the CLI\'s output wrapper (the library types have no such field): every
JSON object on stdout is `{ "schema": 1, ... }`, and an error is `{ "schema": 1, "code":
..., "message": ... }` with the stable `code` below. Each `--events json` line is a JSON
object with `schema` and a `type` tag (the `Event` variant in snake case). Consumers ignore
unknown `type` values and unknown fields within one schema version; removing or retyping a
field bumps `schema`. Paths serialize as UTF-8 strings with a lossy conversion for non-UTF-8
paths, and a path never has its separators rewritten. Codes and exit codes:

| `CompileError` / other | `code` | Exit code |
|---|---|---|
| `NoTexInstallation` | `no_tex_installation` | 3 |
| `EngineNotFound` | `engine_not_found` | 3 |
| `Latex` | `latex_error` | 1 |
| `PackageInstall` (`NoMatch`, `InstallFailed`) | `package_install` | 1 |
| `PackageInstall` (`TexLiveTooOld`) | `texlive_too_old` | 5 |
| `Bibliography` | `bibliography` | 1 |
| `Index` | `index` | 1 |
| `NoOutput` | `no_output` | 1 |
| `InvalidRequest` | `invalid_request` | 2 |
| `Cancelled` (also the `--timeout` deadline) | `cancelled` | 4 |
| `Io` | `io` | 3 |
| `tinytex tlmgr`: `TlmgrError` | `tlmgr` | 1 |
| `tinytex detect`: no installation found | `no_tex_installation` | 3 |

A usage error exits 2 before any of these.

**Details settled 2026-11-17.**

- **Binary name `tinytex`.** The R package is not an executable, so there is no `PATH`
  collision. Quarto 1's unused command names its executable `tinitex` (sic), apparently a
  typo of the same name; the spelling is not carried over. The crate and repository stay
  `tinytex-rs`. Docs should say plainly which of the four similar names is which: the
  TinyTeX distribution, the `tinytex` R package, this CLI, and Quarto 1's `tinitex`.
- **Streams and exit codes.** Stdout carries the result (JSON with `--json`, otherwise the
  output path); stderr carries progress and diagnostics. Exit code 0 is success, 1 a LaTeX
  compile failure, 2 a usage error, 3 an environment problem (no TeX installation, a tool
  not found), 4 cancelled or timed out, 5 the TeX Live installation is older than the
  remote repository.
- **JSON versioning.** Every JSON output carries an integer `schema` field, starting at 1;
  error objects carry the stable error code (§9.4 item 5). The schema is documented as
  unstable until the first tagged release that R or Quarto 1 pins. Environment variables are
  read through `Config::from_env()` using R's names where R has them (`TINYTEX_DIR`,
  `TINYTEX_PREVENT_INSTALL`); the CLI cannot read R `options()`, so an R wrapper passes
  flags explicitly.
- **`--timeout <seconds>`** is a whole-command deadline passed to `StdExec`, the
  counterpart of the deadline q2's `Exec` adapter measures for a whole compile; a timeout
  exits with code 4.
- **Quarto 1\'s `quarto-latexmk --no-auto-mk` is not a flag** (see the note under the
  flag table).
- **Wrapping.** R's `latexmk()` keeps its R-level signature and calls
  `tinytex compile --json ...` through `system2()`, turning events into `message()` and
  `warning()`. Quarto 1's `generatePdf` would become a thin wrapper using `Deno.Command`.
  Neither host has a driver command line to stay compatible with, because none exists
  (§1). How the binary reaches users is for the maintainers (§9.7).

**Packaging.** One package, with the library and a `[[bin]] name = "tinytex"` behind a
`cli` feature (`required-features`), so `clap` stays out of q2's dependency tree. `serde`
is a normal dependency, because §9.4 item 5 makes every public type serializable. Settings
that come from the environment are read in one place, `Config::from_env()`, for the binary
exactly as for the library.

## 6. Decisions

**Settled earlier (2026-09-24):** crate name `tinytex-rs`; no
dependency on any q2 crate; `Bib` inference as a fallback with q2 passing explicit values;
the port is derived from live `tinytex` on `main`; `tectonic` short-circuit in v1; DVI out
of v1; crate error enum with no `Q-*` awareness, mapped in q2's plan 6.8a; hermetic log-parsing tests
seeded from the union of both test corpora.

**Settled 2026-11-17 (Gordon):**
- **The `tinytex` CLI is a first-class deliverable, a thin layer over the library** (§5.1).
  Its features come from the library's `CompileRequest`; its flag names come from Quarto 1's
  `quarto-latexmk`, without `latexmk`-style aliases.
- **Real-`latexmk` command-line compatibility is a non-goal.** There is no standard driver
  interface (§1), and neither R nor Quarto 1 uses `latexmk`\'s as a contract.
- **q2 links the library**; it never runs the `tinytex` binary.
- **Threading and cancellation** are part of the public contract (§2): `Send + Sync`
  traits, `Exec::interrupted()`, `CommandSpec::changes_installation`, and the install lock
  with a witness token. Deadlines and cancellation come from the injected `Exec`.
- **Distributions without `tlmgr`** compile in engine-only mode (`Compiler::new(None, ...)`)
  with auto-install skipped, as R does; MiKTeX is not an error.
- **`Engine::Tectonic` is used by q2** (the crate short-circuits it, as R does): q2's stage
  runs the compile through the crate for tectonic as for the other engines, which is a
  deliberate improvement on Quarto 1 (where tectonic is built by Pandoc with none of
  Quarto's LaTeX extras).

**Settled 2026-11-20 (Gordon, implementability review):**
- **The crate follows Quarto 1 and R for where it runs.** The engine and every auxiliary
  tool run with the `.tex` file's directory as the working directory; there is no
  working-directory field, no `BIBINPUTS` or `INDEXSTYLE` input paths, and no record of the
  commands that ran. q2 builds in place, in the document's directory, for `q2 render` and
  for preview (q2 plans 6.5a and 6.5b).
- **Installs are shielded from a superseding edit, not from the deadline** (§2).
- **q2 owns the deadline and the compile lock.** The crate has no deadline hook beyond
  `StdExec`\'s, and never locks a compile.
- **The first tag is a `0.x` pre-release.** q2's plans 6.5a to 6.9 are the first real
  consumers; crate fixes after the tag are made in the crate repository, tagged, and the
  pin is bumped in the q2 plan that found them.

**Settled 2026-11-22 (Gordon, interface review):**
- **One rule for R-versus-Quarto-1 differences** (§3 principle): where Quarto 1\'s behavior is
  q2\'s default it stays q2\'s default and R\'s is an option. The crate and CLI default to R\'s,
  and q2 sets Quarto 1\'s explicitly.
- **Options added:** `Driver`, `Recover`, `OnBibFailure`, `Clean`, `KeepLog`, `TlmgrUpdate`,
  `Output`, and `Config.prefer`. The R failure-path steps, relocation and `cwd` stay out.
- **Baselines moved:** Quarto 1 `v1.12.3` (all-three bibliography trigger), R `7818777`
  plus the §3.3 audit.
- **Windows:** R\'s route (`fmtutil --sys --all`, `updmap --sys`) and `tlmgr.bat` spawned
  directly; no temporary script (§2).
- **`posit-dev/tinytex-rs` is public before the tag q2 pins** (plan 5.5 gate).
- **Plan 5.4 is split** into 5.4a and 5.4b (§8).

**Settled 2026-09-30 (Gordon):**
- **Standalone repository**, `posit-dev/tinytex-rs`.
- **Blocking core API** (§2), justified by the q2 call sites.
- **The installer is designed now and built later.** The `install` module's API is
  specified in §5 and implemented in milestone M5, tracked by strand `bd-6s1fi9c7`. The
  latex/pdf epic's scope-out of a TeX-distribution installer stands.
- **The install API has two layers.** Step functions (release lookup, asset selection,
  repository resolution, post-install configuration, uninstall) carry no HTTP client and no
  archive dependencies; `Fetcher` is implemented by the caller. The one-call
  `install()`/`update()`, `UreqFetcher`, and `unpack()` sit behind a `download` feature for
  standalone users. The reason is that q2's installer framework will install several tools
  (TinyTeX, then probably verapdf and chromium), so the framework owns the shared
  download and extraction code and uses the crate only for TinyTeX-specific knowledge. The
  compile-only v1 has no network or archive dependencies at all.
- **Accessibility warnings are structured data in the crate** (`AccessibilityWarnings`);
  Quarto-worded messages (`fig-alt`, `lang:`) stay in q2's plans 6.8a and 6.8b.
- **Cleanup deletes only files the run created** (§3.1, `Clean` and `KeepLog`): the crate
  removes the stale `.log` first and never deletes a user\'s pre-existing files. The
  biblatex and index e2e fixtures in q2\'s plans 6.3b and 7.4 verify this.
- **R-style error-block extraction** (§3.1): one bounded block per `!` line, each ending
  at a blank line. M1's fixtures include multi-error logs to confirm the output is bounded
  and readable compared with Q1's single span from the first `!` to "Here is how much".

## 7. Testing and verification

- **Hermetic log tests (M1).** Fixtures are the union of
  `tinytex/tests/test-cran/test-latex.R` (`detect_files`, `needs_rerun`, `font_ext`) and
  `quarto-cli/tests/unit/latexmk/parse-error.test.ts` plus its `expl3-aborted.log`, and
  `fmtutil-handler.test.ts`. The suites overlap on font spaces, `colorprofiles`, `tikz`
  and `lua-uni-algos`; tinytex-only cases include `latex-lab`/`DocumentMetadata`,
  `(pdf backend)` embedding, and `needs_rerun`; Quarto-only cases include `expl3`,
  luaotfload fallback, LuaTeX-ja, single-quote variants and babel hyphenation. Fixture
  text is **copied into the crate repo's own test files** with attribution (both
  upstreams are MIT), never read from `external-sources/` at test time.
- **Mocked `Exec` tests (M2, M3).** Record `CommandSpec` sequences and assert them: the
  install retry (install → verify → `update --self` → `fmtutil` → install), the
  lazy-update rule, `TexLiveTooOld` handling, the Windows command forms. A mocked `Exec`
  returns `std::process::Output`, which needs an `ExitStatus`; building one portably takes a
  `#[cfg(unix)]`/`#[cfg(windows)]` `from_raw` helper pair (`cross-platform.md`). The compile
  loop reads `.log`, `.aux`, `.bcf`, `.idx` and the PDF from disk, so the fake must create
  them in the `.tex` directory: plan 5.3b builds one shared `FakeTex` helper, and every loop
  test uses it. Include a test where `run` returns `Err` while `interrupted()` is true (§2) and
  one where a waiting thread finds the package already installed (§2).
- **Integration tests** run against a real TeX distribution when one is present and skip,
  with a visible message, otherwise; the crate's CI job installs TinyTeX and sets
  `TINYTEX_RS_REQUIRE_TEX=1`, which turns a missing distribution into a failure, so a
  broken install cannot pass as green. (q2's own tests assume real tools on `PATH` and
  never skip, so there is no q2 precedent to copy; the crate differs because its
  contributors often have no TeX.) The same CI job covers the auto-install path.
- **A re-invoked test binary cannot be the slow child.** `libtest` rejects unknown
  arguments, so use the portable pattern in `crates/quarto-core/src/engine/ts_process.rs`
  (`spawn_long_lived_child`, `echo_stdin_to_stderr_cmd`). `StdExec` needs pipe-draining
  threads like `run_child` (`child_runner.rs:84-160`), because `Command::output()` has no
  deadline; the `--timeout` clock starts at `StdExec` construction.
- **To verify against a real `tlmgr` in M2/M3:** (a) that a batch `tlmgr install` with one
  unknown package still installs the others and returns a status R's logic expects (R
  relies on this; if it aborts the batch, add `InstallMode::{Batch, Each}`); (b) whether setting `max_print_line` in the *engine's environment*
  prevents hard-wrapped log lines on a system TeX Live (R only sets it in TinyTeX's
  `texmf.cnf` at install time, so the line-oriented regexes see wrapped lines on other
  installs); (c) `Prefer::System` semantics against Q1's `hasTinyTex`/`preferTinyTex`
  pair.
- **Cancelling an in-flight compile.** A blocking compile with package
  installs can run for minutes and cannot be dropped like an async future. q2's PDF preview
  needs it (a newer edit supersedes the running render), and q2 already has the mechanism:
  a render's cancellation token and the child runner that kills the child process. The
  crate needs no cancel flag of its own: the host injects an `Exec` whose `run` honors the
  token and whose `interrupted()` reports it (§2), and the loops and the lock wait stop on
  it. Commands that change the installation are marked `changes_installation` so the host
  can let them finish; the crate stops before the next step. Tests use a fake slow `Exec`,
  including one whose `interrupted()` turns true during an installation command.
- **Output directory outside the `.tex` directory.** A natbib and index document is built
  with `output_dir` inside and outside the `.tex` directory; the second case documents that
  `bibtex` and `makeindex` refuse to write there under `openout_any = p`.

## 8. Milestones

| | Scope | Plan |
|---|---|---|
| M0 | Repo bootstrap: license, attribution notice for ported code, CI matrix (Linux/macOS/Windows), README stub, this document | 5.1 |
| M1 | `log` module and its fixtures | 5.2 |
| M2 | `exec`, `config`, `detect` (5.3a), then `tlmgr` and the install lock (5.3b) | 5.3a, 5.3b |
| M3 | `compile`, including the serializable `CompileRequest`, `Compiled` and `CompileError` (with stable codes): types, defaults, loop, recovery, cancellation (5.4a); cleanup, output paths, engines, tool search, real-TeX tests, CI (5.4b) | 5.4a, 5.4b, then q2's 6.5a |
| M4 | README, rustdoc, first tagged revision (a `0.x` pre-release) for q2 to pin | 5.5 |
| M5 | `install` feature (step functions) and `download` feature (one-call `install()`, `UreqFetcher`, `unpack()`). Tracked by strand **`bd-6s1fi9c7`**, which also covers the q2-side installer framework (TinyTeX first; verapdf and chromium / chrome-headless-shell expected later). Not part of the latex/pdf epic | `q2 install tinytex` |
| M6 | `tinytex` command-line interface with JSON output, and `serde` on all public types with stable error codes (§9.2, §9.4). Needed for adoption by R and Quarto 1; not needed by q2's epic | 5.6 |

## 9. Addendum: adoption by `tinytex` (R) and Quarto 1

**Status:** analysis for discussion with the `tinytex` maintainers (Christophe Dervieux,
Yihui Xie), 2026-09-30. They hope `tinytex-rs` can become the common base of both the R
package `tinytex` and Quarto 1's TypeScript, which adds two consumers beyond q2. Nothing in
§1–§8 depends on this addendum except where §4 and §8 cross-reference it, and the q2
latex/pdf epic's scope is unchanged. The maintainers will need to agree with every
particular of this design; the last subsection lists the questions to put to them.

### 9.1 What this changes

- **The crate must be distributable as a native binary**, and that shapes the public
  surface: serializable types, stable error codes, a command-line interface (§5.1, §9.2).
- The installer (M5) becomes a prerequisite for R's migration, not only for q2's
  `q2 install` (§9.4).
- A handful of R- and TS-facing gaps in the API (§9.3, §9.4), which are small.

### 9.2 Distribution: a native binary, not WASM

**WASM cannot be the delivery vehicle for this crate.** Everything `tinytex-rs` does, apart
from the pure `log` module and a few pure helpers (asset-name selection, `needs_rerun`), is
an interaction with the host operating system:

- **Spawning processes**: `tlmgr`, `fmtutil`, `pdflatex`/`xelatex`/`lualatex`, `biber`,
  `bibtex`, `makeindex`, `latexmk`, `tectonic`. `wasm32-unknown-unknown` has no operating
  system at all, and WASI has no process-spawning API.
- **Reading and writing the filesystem**: auxiliary files, logs, the TinyTeX tree itself.
- **Network and environment access**: release lookup, repository probing, `PATH`
  manipulation, symlinks into the system `PATH`.

A WASM build could only do this work if every host supplied process, filesystem and network
operations as imports. That is the sans-IO design (§9.6): feasible, but it moves all of the
OS-facing work into each host, which is precisely what a shared crate is meant to remove.
Neither migrating host needs WASM in any case: Quarto 1 runs under Deno, which can spawn
binaries and load native libraries, and R has no built-in WASM runtime. Q2's own WASM
(hub-client) build is unaffected; `TexCompileStage` and this crate are excluded there, as
`TypstCompileStage` is (plan 6.5a).

**So the deliverable is a native executable per platform**, at least the platforms TinyTeX
itself supports (Windows, macOS, Linux x86_64, Linux arm64, Linux musl x86_64, per the R
package's `binary_supported`), optionally alongside the Rust library for in-process use. The
ways the three consumers would use it:

| Consumer | Route | Notes |
|---|---|---|
| q2 | Rust dependency (plans 6.5a to 6.9), in process | as already planned |
| R `tinytex` | `system2()` calling the binary with JSON output, or extendr bindings to the library | extendr works but adds a Rust toolchain requirement to source installs; the R maintainers must judge that |
| Quarto 1 (Deno) | `Deno.Command` calling the binary, or FFI (`Deno.dlopen`) to the library | Quarto 1 already shells out to every other tool; how it would ship or fetch a platform binary needs checking against its packaging, which has not been examined here |

The CLI route needs no per-language binding code and matches how R and Quarto 1 already
use every other external tool, so it is the lowest-friction path and the one to design for
first. Its shape is specified in §5.1. Neither host has a driver command line to stay
compatible with: R's `latexmk()` is an R function, and Quarto 1 calls `generatePdf`
in-process. Each host therefore replaces its compile loop with a thin wrapper that builds a
`CompileRequest`, runs `tinytex compile --json`, and renders the result. How the binary
reaches users (built with the R package, bundled with Quarto 1, or downloaded on first use)
is a question for the two maintainers.

### 9.3 Coverage of the two public surfaces

**R.** `NAMESPACE` on `main` exports 26 functions.

| Coverage | R exports |
|---|---|
| Covered by §5 | `check_installed`, `is_tinytex`, `tinytex_root`, `latexmk`, `pdflatex`, `xelatex`, `lualatex`, `tlmgr`, `tlmgr_install`, `tlmgr_path`, `tlmgr_remove`, `tlmgr_update`, `tlmgr_version`, `install_tinytex`, `reinstall_tinytex`, `uninstall_tinytex` (the last three in M5) |
| Covered after the small additions in §9.4 | `parse_packages`, `parse_install` (they search and install in one call), `tlmgr_search` (general flags), `tlmgr_repo` (needs a getter), `tlmgr_conf`, `tl_pkgs` and `tl_sizes` (a typed `Tlmgr::info` covers both) |
| Not covered; stays in R | `copy_tinytex`, `use_tinytex` (relocation; thin, implementable in R on top of `Tlmgr::path`), `r_texmf` (R-specific; implementable with `tlmgr conf`) |

Helpers §4 marks "cut" (`fmtutil`, `texhash`, `updmap`, `fc_cache`, `kpsewhich`,
`refresh_all`, `ctan_mirrors`, `tl_platforms`) are unexported in R, so omitting them does not
affect R's public API.

**Quarto 1.** What the rest of Quarto imports from its TinyTeX code is `hasTinyTex`,
`tinyTexInstallDir`, `tinyTexBinDir` (`filters.ts`), `texLiveContext` and `tlVersion`
(`check.ts`), the `tinyTexInstallable` object (`tools.ts`), and `generatePdf`
(`output.ts`). Detection, version, and compile are covered. The `InstallableTool` interface
decomposes installation into `latestRelease`, `preparePackage`, `install`, `afterInstall`,
and `uninstall`; §5's step functions map onto these, with two gaps:

- **No placement step.** The TS `install` step extracts, moves the tree to `.TinyTeX` or
  `TinyTeX`, repairs file permissions, writes the `version` file, and locates `tlmgr`.
  This is TinyTeX-specific knowledge, so the crate needs an `install_extracted` step.
- **`configure` lacks knobs.** It needs the candidate user-bin directories
  (`suggestUserBinPaths`), an explicit `bin_path` (`QUARTO_TEXLIVE_BINPATH`), tlgpg
  installation on macOS and Windows, and the Windows restart check (`restart_required`).

### 9.4 Amendments to §4 and §5

These are small, do not depend on the maintainers' agreement, and are worth making as the
crate is built:

1. **`Config` gains R's options**: `tlmgr_path` (R's `tinytex.tlmgr.path`), `usermode`,
   `delete_tlpdb`, `verbose`. `UpdateOpts` gains `delete_tlpdb`, `run_fmtutil`, and
   `more_args`.
2. **`InstallOptions` gains** `dir`, `extra_packages`, `repository`, and `force`.
3. **New methods:** `Tlmgr::info(pkgs, fields, only_installed)` (covers `tl_pkgs` and
   `tl_sizes`), `Tlmgr::repository()`, `Tlmgr::packages_for_log` and `install_for_log`
   (R's `parse_packages`/`parse_install`), `Distribution::is_writable` (R defaults
   `install_packages` from `tlmgr_writable()`), and `log::index_error` (Q1's
   `findIndexError`).
4. **`install::install_extracted(dir, cfg, version)`** (§9.3), and the extra `configure`
   options.
5. **Serializable public types.** Every option, request, config, event, and result type
   derives `Serialize` and `Deserialize`; `CompileError` derives `Serialize` only (it holds
   an `io::Error`); errors carry stable string codes so non-Rust hosts
   can switch on them; `Observer` events carry enough data, and a `Display` text, for a
   host to render its own messages (R's `message()` output is part of its existing
   behavior). `CompileRequest` is the central one (§5).
6. **The `tinytex` command-line interface** (§5.1, M6): a thin layer over the library whose
   flags map one-to-one onto request fields, so the CLI and the API do exactly the same
   thing. Quarto 1's unused `quarto-latexmk` command contributes flag names, not a design.
7. **Neutral defaults.** `Compiler::new` defaults are the crate\'s own, which are R\'s where
   R and Quarto 1 differ (§3 principle, `CompileRequest` doc); Quarto\'s defaults and
   differing settings (for example `autoInstall`, `OnBibFailure::Warn`, `Clean::OnSuccess`)
   are set by the host.
8. **R-compatibility recipes** (documented, not features): R\'s `delete_texmf_user()` retry
   is a host retry on `CompileError::Latex` with an empty `missing`; R\'s Ghostscript and
   `epstopdf` step is host-side from `Compiled.installed_packages`; relocation
   (`copy_tinytex`, `use_tinytex`) is host-side over `Config.install_dir` and
   `TINYTEX_DIR`; R\'s `emulation = FALSE` is `Driver::Latexmk`. A host builds one
   `Compiler` per document when `Config.tlmgr_opts` varies, since `Config` is per `Compiler`.

### 9.5 What stays in the host layers

The crate owns the logic-heavy behavior: the compile loop, log parsing, tlmgr semantics,
detection, and installation. Each host keeps its thin glue: R's `options(tinytex.*)` and
their mapping onto `Config`/`Compiler`, `message()`/`warning()` wording, `selectDirectory`,
the RStudio stdout quirk (`use_file_stdout`), and R's relocation and texmf functions;
Quarto's YAML keys, `QUARTO_*` environment variables, and the `InstallableTool` framework
(strand `bd-6s1fi9c7`). R's bundle-building `tools/` scripts (shell, PowerShell, and batch)
are outside this crate's API and assumed to stay where they are.

### 9.6 Alternative considered: a sans-IO core

Today the crate owns process execution (`Exec` is injectable only for Rust callers), and the
R and Quarto 1 quirks (R's RStudio redirect, Quarto's Windows `.bat` wrapper) would live
inside the crate. A sans-IO core, in which the compile and tlmgr logic yield "run this
command" steps and the host executes them and feeds back the output, would give each host
full control of execution, plus async and cancellation without redesign, and would make a
WASM build possible (§9.2). It is materially more complex, and Rust has no stable coroutine
syntax for it (it would be an explicit state machine). The current design, with a native
binary and an injectable `Exec` for Rust callers, is kept. Revisit only if the maintainers
need host-controlled execution.

### 9.7 Questions for the tinytex maintainers

1. **Delivery.** Is a native binary acceptable for the R package (built with it, bundled, or
   downloaded), and for Quarto 1 (shipped or fetched per platform)? Is CLI with JSON output
   the preferred route, or bindings?
2. **Behavior changes.** Which of the behavior differences in §3 are acceptable to existing R
   and Quarto users, since migration makes them visible.
3. **Scope of the crate versus the hosts** (§9.5), including whether R keeps relocation and
   the texmf-tree functions.
4. **Release and stability model**: versioning, semver promises, and MSRV, given that two
   products and q2 would pin the crate.
5. **The bundle-building scripts**: whether they stay in the R repository.
6. **The command-line vocabulary** (§5.1): whether Quarto 1's flag names, with
   no aliases, suit R's wrapper too, and whether either host needs something
   the library does not already offer.
