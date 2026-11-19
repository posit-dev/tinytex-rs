#!/bin/sh
# Release gate for plan 5.5: fails while the repository is private or the tag
# is not fetchable without credentials. Usage: scripts/check-public.sh <tag>
set -eu
tag="${1:?usage: check-public.sh <tag>}"
repo="posit-dev/tinytex-rs"
vis=$(gh repo view "$repo" --json visibility --jq .visibility)
[ "$vis" = "PUBLIC" ] || { echo "FAIL: $repo is $vis, not PUBLIC" >&2; exit 1; }
# An unauthenticated read: no helper, no token, no prompt.
GIT_TERMINAL_PROMPT=0 GIT_ASKPASS=true GH_TOKEN= \
  git -c credential.helper= ls-remote --tags "https://github.com/$repo" "refs/tags/$tag" \
  | grep -q "refs/tags/$tag" || { echo "FAIL: tag $tag not listed anonymously" >&2; exit 1; }
echo "ok: $repo is PUBLIC and $tag is fetchable without credentials"
