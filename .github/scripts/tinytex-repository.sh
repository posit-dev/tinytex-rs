#!/bin/sh
# Chooses the tlmgr repository for the TinyTeX that is on PATH.
#
# TinyTeX is built for one TeX Live year, and tlmgr refuses a repository of a
# newer year ("Local TeX Live (2025) is older than remote repository (2026)").
# The default repository always serves the current year, so:
#
#   installed year == current year  -> print nothing: keep the default
#   installed year <  current year  -> print the frozen `tlnet-final` URL
#   anything else                   -> fail with a message
#
# A frozen archive exists only for a past release, so an explicit
# `tlnet-final` URL for the current year would fail or select the wrong
# repository. The caller decides what to do with the printed URL (CI runs
# `tlmgr option repository URL` on its disposable runner).
#
# Usage: tinytex-repository.sh        (tlmgr must be on PATH)
set -eu

default_repo=${TINYTEX_DEFAULT_REPOSITORY:-https://mirror.ctan.org/systems/texlive/tlnet}
frozen_base=${TINYTEX_FROZEN_BASE:-https://ftp.math.utah.edu/pub/tex/historic/systems/texlive}

fail() {
  echo "::error::tinytex-repository: $*" >&2
  exit 1
}

installed=$(tlmgr --version 2>&1 | sed -n 's/^TeX Live .* version \([0-9][0-9][0-9][0-9]\).*$/\1/p' | head -n 1)
[ -n "$installed" ] || fail "cannot read the installed TeX Live year from 'tlmgr --version'"

# The first stanza of the repository's database is `00texlive.config`, whose
# `depend release/YYYY` line names the year it serves. Read only the head of
# the compressed file.
# curl, not Python's urllib: the system Python on some runners cannot verify
# the certificates of the mirror that the redirector picks.
head_file=$(mktemp)
trap 'rm -f "$head_file"' EXIT
curl -fsSL --retry 3 --max-time 120 -r 0-65535 \
  "${default_repo%/}/tlpkg/texlive.tlpdb.xz" -o "$head_file" ||
  fail "cannot download the repository database from $default_repo"
# A server that ignores the range sends the whole file; only the head is read.
current=$(python3 -I - "$head_file" <<'PY'
import lzma, re, sys

with open(sys.argv[1], "rb") as f:
    head = lzma.LZMADecompressor().decompress(f.read(65536))
m = re.search(r"^depend release/(\d{4})$", head.decode("utf-8", "replace"), re.M)
if not m:
    sys.exit("no 'depend release/YYYY' line in the repository database")
print(m.group(1))
PY
) || fail "cannot read the current TeX Live year from $default_repo"

if [ "$installed" = "$current" ]; then
  echo "installed TeX Live $installed matches the default repository ($current): keeping it" >&2
elif [ "$installed" -lt "$current" ]; then
  echo "installed TeX Live $installed is older than the default repository ($current): using the frozen archive" >&2
  echo "$frozen_base/$installed/tlnet-final"
else
  fail "installed TeX Live $installed is newer than the default repository ($current)"
fi
