# Captured `tlmgr` output

Real output of `tlmgr`, captured once so the test fakes can be checked against it
(the conformance test) and built from it. Each fixture is three files:
`NAME.stdout`, `NAME.stderr`, and `NAME.status` (the exit status).

Do not edit these by hand. To refresh them, rerun the command in a disposable
copy of a TinyTeX tree (never the developer's own tree) and replace all three
files of the fixture.

## Provenance

- Captured 2026-11-24 on macOS 27.2.1 (arm64) with TinyTeX 2025: `tlmgr revision
  76773`, TeX Live 2025, in a `cp -a` copy of the tree under a scratch directory.
- Every command ran under `env -i` with only `PATH` (the copy's `bin` first),
  `HOME`, `TMPDIR`, and `LANG=C`.
- Repository calls used the frozen 2025 repository,
  `https://ftp.math.utah.edu/pub/tex/historic/systems/texlive/2025/tlnet-final`,
  because the default repository is TeX Live 2026, which a 2025 tree refuses.
- Local paths were replaced: the copy is `<copy>`, the local repository is
  `<repo>`, other scratch paths are `<tmp>` or `<scratch>`, and the home
  directory is `<home>`. Nothing else was changed.
- The commands below are reconstructed from the capture notes and the file
  contents. The `info` flags are the ones the crate passes
  (`--list --only-installed --data name`); the exact spelling of the others was
  not recorded.

## Fixtures

| Name | Command | What it shows |
|---|---|---|
| `info-installed` | `tlmgr info --list --only-installed --data name framed` | Exit 0, one row on stdout |
| `info-missing` | the same, for `animate` (not installed) | Exit 1, empty stdout, `package animate not locally!` and two trailer lines on stderr |
| `info-mixed` | the same, for `framed animate ocgx2` | Exit 1, the installed row on stdout, one `not locally!` line per missing name |
| `info-unknown` | the same, for `nosuch_zzz` | The same shape as a missing package |
| `install-nosuch` | `tlmgr --repository FROZEN install nosuch_zzz` | Exit 1, `package nosuch_zzz not present in repository` |
| `install-present` | `tlmgr --repository FROZEN install framed` | Exit 0, `package already present: framed` |
| `install-again` | `tlmgr --repository FROZEN install animate` when installed | Exit 0, `package already present: animate` |
| `install-ok` | `tlmgr --repository FROZEN install animate ocgx2` | Exit 0, progress and `mktexlsr` lines |
| `remove-notinstalled` | `tlmgr remove nosuch_zzz` | Exit 1, `package not present, cannot remove` |
| `remove-again` | `tlmgr remove animate` when already removed | Exit 1, the same shape |
| `search-ocgbase` | `tlmgr --repository FROZEN search --file --global /ocgbase.sty` | Exit 0, `ocgx2:` and the indented path on stdout |
| `pathrepo-good` | `tlmgr --repository REPO --verify-repo none install fakegood` | Control: exit 0, installed |
| `pathrepo-bad` | the same, for `fakebad` (wrong container checksum) | **Exit 0, a `checksums differ` warning on stderr, nothing installed** |
| `pathrepo-bad-info` | `tlmgr info --list --only-installed --data name fakebad` after it | Exit 1, `package fakebad not locally!` |
| `local-good` | the `pathrepo-good` install, with `REPO` written as a `file://` URL | Exit 0, `cannot find package fakegood.tar.(gz\|xz\|lz4)`, nothing installed |
| `opt-repo-before`, `opt-repo-after` | `tlmgr option repository` before and after a call that used `--repository` | The saved value does not change |
| `version` | `tlmgr --version` | Exit 0, a `version YYYY` line |

`FROZEN` is the frozen repository above. `REPO` is the local repository built by
`make-corrupt-repo.sh` in this directory (`make-corrupt-repo.sh OUTDIR [YEAR]`).
The script calls only the read-only `tlmgr --version`, and only when `YEAR` is
omitted. Run the installs that consume the repository against a disposable tree
only, with the repository given as a plain path, not a `file://` URL.

## Things a test must not depend on

- The frozen-repository banner (`TeX Live 2025 is frozen…`) on stderr of every
  call that uses that repository. It is not an error.
- The `package repository URL (verified)` or `(not verified: …)` line on stdout.
- Scratch paths, temporary file names, checksums, and backtrace line numbers.
- Exit status 255: no command in this capture exited with it.
