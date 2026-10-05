#!/usr/bin/env bash
# The version a release from this checkout publishes: `X.Y.N`.
#
#   scripts/version.sh            # the tag, e.g. 0.1.42
#   scripts/version.sh --base     # `X.Y`, the line's base tag
#   scripts/version.sh --count    # the commit count that becomes `N`
#   scripts/version.sh --latest   # the latest release tag, empty when there is none
#
#   * `X.Y` is the latest release tag in the repository (the line's base tag),
#   * `N`   is the number of commits in HEAD.
#
# The commit count rather than a hand-kept number, because a number someone has to remember is a
# number that eventually lies. Every release therefore gets a version that is unique and larger than
# the previous one, and there is nothing to bump — a release that waits for three merges simply
# skips the numbers in between.
#
# This file exists because two places need that number and they must agree: `scripts/release.sh`
# publishes the tag, and `packaging/appimage/build.sh` bakes it into the binary so the running app
# can compare itself against the latest release. A binary that believes it is a different version
# than the release it is attached to would either never update or offer an update to itself, for the
# rest of that release's life.
#
# A shallow clone cannot answer either half of the question, and a plausible-looking wrong answer is
# worse than none, so this fails instead of guessing.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

if ! git rev-parse --git-dir >/dev/null 2>&1; then
    echo "error: not a git checkout — the version comes from the tags and the commit count" >&2
    exit 1
fi
if [[ "$(git rev-parse --is-shallow-repository)" == "true" ]]; then
    echo "error: this is a shallow clone, so the commit count and the tags are not there." >&2
    echo "       Fetch the full history (actions/checkout with 'fetch-depth: 0') and retry." >&2
    exit 1
fi

# Release tags look like `0.1` or `0.1.7`. Anything else (a branch tag, a date tag) is ignored:
# guessing a base from an unrelated tag is how you ship `2026.9.1`.
release_tags() {
    git tag --list | grep -E '^[0-9]+\.[0-9]+(\.[0-9]+)?$' | sort -V || true
}

latest="$(release_tags | tail -1)"
base="$(echo "${latest:-0.1}" | cut -d. -f1,2)"
count="$(git rev-list --count HEAD)"

case "${1:-}" in
    ""|--tag) echo "$base.$count" ;;
    --base) echo "$base" ;;
    --count) echo "$count" ;;
    --latest) echo "$latest" ;;
    -h|--help)
        awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "${BASH_SOURCE[0]}"
        ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
esac
