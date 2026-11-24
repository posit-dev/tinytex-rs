#!/bin/sh
# make-corrupt-repo.sh: build a tiny local TeX Live repository in which one
# package installs and one package has a deliberately wrong container checksum.
#
# Usage:  make-corrupt-repo.sh OUTDIR [YEAR]
#
#   OUTDIR  must not exist or must be empty; the script writes only under it.
#   YEAR    the TeX Live release of the tree that will install from it (the
#           00texlive.config stanza says `depend release/YEAR`). When omitted it
#           is read from `tlmgr --version` (a read-only call, tlmgr on PATH).
#
# The script never runs a mutating tlmgr command. Layout it creates:
#   OUTDIR/tlpkg/texlive.tlpdb         00texlive.config + fakegood + fakebad
#   OUTDIR/tlpkg/texlive.tlpdb.sha512  required: tlmgr fetches it first
#   OUTDIR/archive/fakegood.tar.xz     correct containersize and SHA-512
#   OUTDIR/archive/fakebad.tar.xz      correct containersize, all-zero SHA-512
#   OUTDIR/pkgsrc/                     the staging trees the archives came from
#
# Consume it with a PLAIN PATH, never a file:// URL:
#   tlmgr --repository OUTDIR --verify-repo none install fakebad
# Why a plain path: with a file:// URL tlmgr 2025 (TLPDB.pm:1782-1787) tests
# `-r "$root/archive/<pkg>.tar.xz"` with the `file://` prefix still on $root, so
# every install silently does nothing (exit 0, "cannot find package
# <pkg>.tar.(gz|xz|lz4)"), even for a correct checksum. --verify-repo none
# is needed because the repository is unsigned.
#
# Expected results (observed on TeX Live 2025, tlmgr revision 76773):
#   install fakegood  exit 0, `install: fakegood [1k]`, `info` lists it installed.
#   install fakebad   exit 0, stderr has `check_file_and_remove: checksums
#                     differ for .../fakebad.tar.xz` and `_install_data:
#                     consistency checks failed`, nothing is installed, and
#                     `tlmgr info --only-installed fakebad` then exits 1.
# Run the installs only against a disposable copy of a TinyTeX tree.

set -eu

out=${1:?usage: make-corrupt-repo.sh OUTDIR [YEAR]}
year=${2:-}

if [ -z "$year" ]; then
  year=$(tlmgr --version 2>/dev/null | sed -n 's/.*version \([0-9][0-9][0-9][0-9]\).*/\1/p' | head -n 1)
  [ -n "$year" ] || { echo "make-corrupt-repo.sh: cannot derive the year; pass it as the 2nd argument" >&2; exit 2; }
fi
case $year in [0-9][0-9][0-9][0-9]) ;; *) echo "make-corrupt-repo.sh: bad year: $year" >&2; exit 2 ;; esac

mkdir -p "$out"
if [ -n "$(ls -A "$out")" ]; then
  echo "make-corrupt-repo.sh: $out is not empty" >&2
  exit 2
fi
out=$(cd "$out" && pwd)
mkdir -p "$out/tlpkg" "$out/archive" "$out/pkgsrc"

if command -v sha512sum >/dev/null 2>&1; then
  sha512() { sha512sum "$1" | cut -d' ' -f1; }
else
  sha512() { shasum -a 512 "$1" | cut -d' ' -f1; }
fi
size() { wc -c < "$1" | tr -d ' '; }

# One archive per package: tlpkg/tlpobj/<name>.tlpobj plus one .sty.
mkpkg() {
  n=$1
  src="$out/pkgsrc/$n"
  mkdir -p "$src/texmf-dist/tex/latex/$n" "$src/tlpkg/tlpobj"
  printf '\\ProvidesPackage{%s}\n' "$n" > "$src/texmf-dist/tex/latex/$n/$n.sty"
  printf 'name %s\ncategory Package\nrevision 1\nshortdesc fake\nrunfiles size=1\n texmf-dist/tex/latex/%s/%s.sty\n' \
    "$n" "$n" "$n" > "$src/tlpkg/tlpobj/$n.tlpobj"
  (cd "$src" && tar -cJf "$out/archive/$n.tar.xz" tlpkg texmf-dist)
}
mkpkg fakegood
mkpkg fakebad

zeros=$(printf '%0128d' 0)
{
  printf 'name 00texlive.config\ncategory TLCore\ndepend container_format/xz\ndepend minrelease/2016\ndepend release/%s\n\n' "$year"
  for n in fakegood fakebad; do
    if [ "$n" = fakegood ]; then sum=$(sha512 "$out/archive/$n.tar.xz"); else sum=$zeros; fi
    printf 'name %s\ncategory Package\nrevision 1\nshortdesc fake %s\nrunfiles size=1\n texmf-dist/tex/latex/%s/%s.sty\ncontainersize %s\ncontainerchecksum %s\n\n' \
      "$n" "$n" "$n" "$n" "$(size "$out/archive/$n.tar.xz")" "$sum"
  done
} > "$out/tlpkg/texlive.tlpdb"

(cd "$out/tlpkg" && printf '%s  texlive.tlpdb\n' "$(sha512 texlive.tlpdb)" > texlive.tlpdb.sha512)

echo "repository built in $out (TeX Live $year); install with:"
echo "  tlmgr --repository $out --verify-repo none install fakebad"
