#!/bin/sh
# tex-sandbox.sh: build and delete disposable copies of a TeX tree, so that
# tests which install or remove packages never touch a real installation.
#
#   tex-sandbox.sh create SOURCE_ROOT   copy the tree; print two lines to export:
#                                         TINYTEX_DIR=<copy>
#                                         TINYTEX_SANDBOX_NONCE=<nonce>
#   tex-sandbox.sh delete DIR NONCE     delete a copy this run created
#   tex-sandbox.sh clean                delete stale copies a crash left behind
#
# The copy is a direct child of $RUNNER_TEMP (or $TMPDIR, or /tmp) named
# tinytex-sandbox.XXXXXX, with a sibling <copy>.home directory that the tests
# use as HOME. Two files mark it, and the test gate (tests/integration/sandbox.rs)
# refuses anything without them:
#
#   DISPOSABLE_TINYTEX   written FIRST: the copy's canonical path, then the nonce
#   COMPLETE             written LAST: the copy finished
#
# A crash between the two leaves a tree with a sentinel and no marker, which
# `clean` recognizes and removes. Never accepts a path to create at.
#
# The copy uses `cp -a` (not `cp -R`, which changes mtimes, and never a hard-link
# copy: an in-place rewrite would reach the original). It refuses a source with
# an absolute symlink or a relative one that leaves the tree: a copy that held
# one would pass `kpsewhich` while `tlmgr` operated on the original. It removes
# texmf-var/fonts/conf and texmf-var/luatex-cache, which name the original's
# font directories.
set -eu

PREFIX=tinytex-sandbox

fail() {
  echo "tex-sandbox: $*" >&2
  exit 1
}

# The base directory, canonical.
base() {
  b=${RUNNER_TEMP:-${TMPDIR:-/tmp}}
  (cd "$b" && pwd -P) || fail "cannot resolve the temporary directory $b"
}

# Lexically normalizes an absolute path: collapses "." and "..".
norm() {
  awk -v p="$1" 'BEGIN {
    n = split(p, a, "/"); m = 0
    for (i = 1; i <= n; i++) {
      if (a[i] == "" || a[i] == ".") continue
      if (a[i] == "..") { if (m > 0) m--; continue }
      s[++m] = a[i]
    }
    out = ""
    for (i = 1; i <= m; i++) out = out "/" s[i]
    print (out == "" ? "/" : out)
  }'
}

random_nonce() {
  od -An -N16 -tx1 /dev/urandom | tr -d ' \n'
}

# Deletes DIR (and DIR.home) when it is a real directory directly under the
# base, carries the prefix, and has a sentinel. With NONCE the sentinel must
# hold it; without (clean), any sentinel will do.
remove_tree() {
  dir=$1
  nonce=${2:-}
  b=$(base)
  [ -L "$dir" ] && fail "refusing to delete a symlink: $dir"
  [ -d "$dir" ] || fail "not a directory: $dir"
  canon=$(cd "$dir" && pwd -P)
  [ "$(dirname "$canon")" = "$b" ] || fail "$canon is not a direct child of $b"
  case $(basename "$canon") in
    "$PREFIX".*) ;;
    *) fail "$canon does not carry the $PREFIX prefix" ;;
  esac
  [ -f "$canon/DISPOSABLE_TINYTEX" ] || fail "$canon has no sentinel"
  if [ -n "$nonce" ]; then
    [ "$(sed -n 2p "$canon/DISPOSABLE_TINYTEX")" = "$nonce" ] || fail "$canon holds another run's nonce"
  fi
  rm -rf "$canon" "$canon.home"
}

cmd_create() {
  [ $# -eq 1 ] || fail "usage: tex-sandbox.sh create SOURCE_ROOT"
  [ -d "$1/bin" ] || fail "$1 does not look like a TeX tree (no bin directory)"
  src=$(cd "$1" && pwd -P)
  [ -e "$src/DISPOSABLE_TINYTEX" ] && fail "$src is itself a sandbox"

  # Check every symlink before anything is created.
  find "$src" -type l | while IFS= read -r link; do
    target=$(readlink "$link")
    case $target in
      /*) fail "absolute symlink in the source: $link -> $target" ;;
    esac
    resolved=$(norm "$(dirname "$link")/$target")
    case $resolved in
      "$src" | "$src"/*) ;;
      *) fail "symlink escapes the source tree: $link -> $target" ;;
    esac
  done

  b=$(base)
  dest=$(mktemp -d "$b/$PREFIX.XXXXXX")
  dest=$(cd "$dest" && pwd -P)
  nonce=$(random_nonce)
  printf '%s\n%s\n' "$dest" "$nonce" >"$dest/DISPOSABLE_TINYTEX"
  cp -a "$src/." "$dest/"
  rm -rf "$dest/texmf-var/fonts/conf" "$dest/texmf-var/luatex-cache"
  mkdir "$dest.home"
  : >"$dest/COMPLETE"
  echo "TINYTEX_DIR=$dest"
  echo "TINYTEX_SANDBOX_NONCE=$nonce"
}

cmd_delete() {
  [ $# -eq 2 ] || fail "usage: tex-sandbox.sh delete DIR NONCE"
  remove_tree "$1" "$2"
}

cmd_clean() {
  [ $# -eq 0 ] || fail "usage: tex-sandbox.sh clean"
  b=$(base)
  for d in "$b/$PREFIX".*; do
    [ -d "$d" ] || continue
    [ -L "$d" ] && continue
    case $d in *.home) continue ;; esac
    [ -f "$d/DISPOSABLE_TINYTEX" ] || continue
    remove_tree "$d"
  done
}

case ${1:-} in
  create) shift; cmd_create "$@" ;;
  delete) shift; cmd_delete "$@" ;;
  clean) shift; cmd_clean "$@" ;;
  *) fail "usage: tex-sandbox.sh create SOURCE_ROOT | delete DIR NONCE | clean" ;;
esac
