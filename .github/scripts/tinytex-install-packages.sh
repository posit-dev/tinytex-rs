#!/bin/sh
# Installs the packages listed in .github/tinytex-packages.txt and proves that
# they landed.
#
# `tlmgr install` exits 0 after rejecting a corrupt download (a checksum
# mismatch only logs a warning), so the exit status proves nothing. This
# script checks every name with `tlmgr info --only-installed`, retries once
# for the names that are missing, and fails if any still is. CI saves the
# TinyTeX cache only after it passes, so an incomplete tree is never cached
# under a key that does not change.
#
# Usage: tinytex-install-packages.sh PACKAGE-LIST-FILE   (tlmgr on PATH)
set -eu

list=${1:?usage: tinytex-install-packages.sh PACKAGE-LIST-FILE}
packages=$(tr '\n' ' ' < "$list")

# Prints the packages of the list that are not installed, one per line.
missing_packages() {
  # `tlmgr info` exits 1 when a name is unknown; the rows it prints are what
  # counts.
  # shellcheck disable=SC2086
  have=$(tlmgr info --list --only-installed --data name $packages 2>/dev/null || true)
  for p in $packages; do
    printf '%s\n' "$have" | grep -qx "$p" || echo "$p"
  done
}

# shellcheck disable=SC2086
tlmgr install $packages || echo "tlmgr install exited non-zero; checking what landed" >&2

missing=$(missing_packages)
if [ -n "$missing" ]; then
  echo "::warning::not installed after the first attempt, retrying once: $(echo $missing)" >&2
  # shellcheck disable=SC2086
  tlmgr install $missing || echo "tlmgr install exited non-zero; checking what landed" >&2
  missing=$(missing_packages)
fi

if [ -n "$missing" ]; then
  echo "::error::not installed after the retry: $(echo $missing)" >&2
  exit 1
fi
echo "all packages installed: $packages"
