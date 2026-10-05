#!/usr/bin/env bash
# Releases on demand. Nothing is published because `main` moved: the workflow has no `push`
# trigger, and this script is how a release is asked for.
#
#   scripts/release.sh                   # dispatch the release workflow on `main` (needs `gh`)
#   scripts/release.sh --dry-run         # print the dispatch that would be sent, and stop
#   scripts/release.sh --print-version   # print the tag a release right now would publish
#
# The dispatch is `gh workflow run`, so `gh` has to be installed and logged in (`gh auth login`)
# and nothing else is needed — no token in the environment, no build on this machine. The version
# is then the one the runner computes from `main`, which is the point: a local checkout is often
# behind, and a preview that can be wrong is worse than no preview. `--print-version` prints that
# preview anyway, from local tags and local commits, for when the number itself is the question.
#
# The packaging half lives in this file too, because the artifact a release publishes is exactly
# what this script produces, and that path has to stay runnable locally and under `act`:
#
#   scripts/release.sh --local             # build, package, publish from here (needs GH_TOKEN)
#   scripts/release.sh --local --dry-run   # build and package only, never publish
#   scripts/release.sh --skip-build        # (implies --local) publish what is in target/release
#   scripts/release.sh --appimage FILE     # (implies --local) attach an AppImage from
#                                          # packaging/appimage/build.sh
#
# Either way the version is `X.Y.N`, where
#
#   * `X.Y` is the latest release tag in the repository (the line's base tag, e.g. `0.1`),
#   * `N`   is the number of commits in the branch being released.
#
# The commit count rather than a hand-kept number, because a number someone has to remember is a
# number that eventually lies. Every release therefore gets a version that is unique and larger
# than the previous one, and there is nothing to bump — a release that waits for three merges
# simply skips the numbers in between.
#
# The formula itself lives in `scripts/version.sh`, which is also what bakes that number into the
# binary (`packaging/appimage/build.sh`). The tag published here and the version the app reports
# about itself are therefore the same string, and the check below refuses an AppImage where they
# disagree.
#
# The AppImage is an input, not something this script builds. It is produced by
# `packaging/appimage/build.sh` in a job that cannot write to the repository and passed here, so
# the one job allowed to publish never runs the third-party toolchain. A local release is:
#
#   packaging/appimage/build.sh
#   scripts/release.sh --local --appimage target/appimage/ProtonVPN-GUI-$(uname -m).AppImage
#
# Requirements: `gh` (logged in) to dispatch; cargo, git, tar, sha256sum and `gh` for a local
# release.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DRY_RUN=0
LOCAL=0
SKIP_BUILD=0
PRINT_VERSION=0
APPIMAGE=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run) DRY_RUN=1; shift ;;
        --local) LOCAL=1; shift ;;
        # These two only mean something for a local release, so they select it rather than making
        # the caller say `--local` as well.
        --skip-build) LOCAL=1; SKIP_BUILD=1; shift ;;
        --appimage)
            LOCAL=1
            APPIMAGE="${2:-}"
            if [[ -z "$APPIMAGE" ]]; then
                echo "error: --appimage requires a path" >&2
                exit 2
            fi
            shift 2
            ;;
        --print-version) PRINT_VERSION=1; shift ;;
        -h|--help)
            # The usage block is this file's own header, so there is one place to keep it right.
            awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "${BASH_SOURCE[0]}"
            exit 0
            ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

# Release tags look like `0.1` or `0.1.7`. Anything else (a branch tag, a date tag) is ignored:
# guessing a base from an unrelated tag is how you ship `2026.9.1`.
release_tags() {
    git tag --list | grep -E '^[0-9]+\.[0-9]+(\.[0-9]+)?$' | sort -V || true
}

# Computed for `--print-version` and for a local release. A dispatch does not use it: the version
# there belongs to the runner, which is looking at `main` and not at this checkout. That is also why
# this is only asked for when it is actually needed — a dispatch from a shallow clone is a perfectly
# good dispatch, and it must not fail on a question it was never going to answer.
tag=""
base=""
latest=""
count=""
if [[ "$PRINT_VERSION" -eq 1 || "$LOCAL" -eq 1 ]]; then
    # One definition of the number, shared with `packaging/appimage/build.sh`: the tag published
    # here and the version baked into the binary have to be the same string, or the app in the
    # user's hands spends the life of that release offering an update to itself.
    tag="$("$REPO_ROOT/scripts/version.sh")"
    base="$("$REPO_ROOT/scripts/version.sh" --base)"
    latest="$("$REPO_ROOT/scripts/version.sh" --latest)"
    count="$("$REPO_ROOT/scripts/version.sh" --count)"
fi

if [[ "$PRINT_VERSION" -eq 1 ]]; then
    echo "$tag"
    exit 0
fi

if [[ "$LOCAL" -eq 0 ]]; then
    # `gh workflow run` is the whole dispatch. It authenticates with the login `gh` already has,
    # which is why there is no token to set here and no token to leak: the credentials never pass
    # through this script or through the shell that started it.
    if ! command -v gh >/dev/null 2>&1; then
        echo "error: gh not found — a release is dispatched with the GitHub CLI" >&2
        exit 1
    fi
    if ! gh auth status >/dev/null 2>&1; then
        echo "error: gh is not logged in — run 'gh auth login' first" >&2
        exit 1
    fi

    echo "== dispatch =="
    echo "   workflow: .github/workflows/release.yml on main"
    echo "   command:  gh workflow run release.yml --ref main"

    if [[ "$DRY_RUN" -eq 1 ]]; then
        echo "   dry run: nothing was dispatched"
        exit 0
    fi

    # Exactly one release line, always on `main`, never on the branch this happens to be run from:
    # the ref is stated rather than left to the default, so a checkout on a feature branch cannot
    # quietly release itself. The workflow file has to exist on `main` for this to find it, which
    # is where it lives.
    #
    # `gh workflow run` prints the URL of the run it created, which is the only place that URL is
    # known without asking the API and racing the run's appearance in it. Both streams are
    # captured, because which one carries the URL is not part of gh's contract — and the same text
    # is the error message when the dispatch is rejected.
    if ! dispatched="$(gh workflow run release.yml --ref main 2>&1)"; then
        echo "error: the dispatch was rejected — 'gh auth status' must show a token allowed to run workflows" >&2
        printf '%s\n' "$dispatched" >&2
        exit 1
    fi
    run_url="$(printf '%s\n' "$dispatched" | grep -Eo 'https://[^[:space:]]+/actions/runs/[0-9]+' | tail -1 || true)"

    echo "   version:  decided by the runner, from main"
    if [[ -n "$run_url" ]]; then
        echo "   run:      $run_url"
    else
        echo "   runs:     https://github.com/$(gh repo view --json nameWithOwner --jq .nameWithOwner)/actions/workflows/release.yml"
    fi
    exit 0
fi

echo "== local release $tag =="
echo "   base tag: $base (latest release tag: ${latest:-none})"
echo "   commits in HEAD: $count"

if [[ "$SKIP_BUILD" -eq 0 ]]; then
    echo
    echo "== build =="
    cargo build --release --workspace --locked
fi

echo
echo "== package =="
name="protonvpn-gui-$tag-x86_64-linux"
stage="dist/$name"
rm -rf "$stage"
mkdir -p "$stage"
install -m 755 target/release/protonvpn-gui "$stage/"
install -m 644 packaging/protonvpn-gui.desktop "$stage/"
install -m 644 packaging/icons/protonvpn-gui.png "$stage/"
install -m 644 README.md LICENSE "$stage/"
tar -C dist -czf "dist/$name.tar.gz" "$name"

appimage_name=""
if [[ -n "$APPIMAGE" ]]; then
    if [[ ! -f "$APPIMAGE" ]]; then
        echo "error: AppImage not found: $APPIMAGE" >&2
        exit 1
    fi
    appimage_name="$(basename "$APPIMAGE")"
    # The image names itself after the version baked into it, so its name is a claim about its
    # bytes, and the claim has to be this release's version. The running app compares that number
    # against `releases/latest`: an image that believes it is 0.1.42 while being published as 0.1.43
    # would offer an update to itself forever, and never apply one.
    case "$appimage_name" in
        *-"$tag"-*) ;;
        *)
            echo "error: the AppImage's name does not carry this release's version ($tag):" >&2
            echo "       $appimage_name" >&2
            echo "       rebuild it with packaging/appimage/build.sh from this commit." >&2
            exit 1
            ;;
    esac
    install -m 755 "$APPIMAGE" "dist/$appimage_name"
fi

( cd dist && sha256sum "$name.tar.gz" > SHA256SUMS )
if [[ -n "$appimage_name" ]]; then
    ( cd dist && sha256sum "$appimage_name" >> SHA256SUMS )
fi

echo "   dist/$name.tar.gz"
if [[ -n "$appimage_name" ]]; then
    echo "   dist/$appimage_name"
fi

# Release notes: what changed since the previous release of this line, plus how to check the
# download. Generated rather than hand-written, because the commits are already the record.
notes_file="dist/${tag}-notes.md"
origin_repo="$(git remote get-url origin | sed -E 's#(git@|https?://)[^/:]+[:/]##; s#\.git$##')"
{
    echo "Built from commit \`$(git rev-parse --short HEAD)\`."
    echo
    echo "Changes:"
    echo
    previous="$(release_tags | grep -E "^${base}\.[0-9]+$" | sort -V | tail -1 || true)"
    if [[ -n "$previous" ]]; then
        git log --pretty='- %s' "$previous..HEAD"
    else
        git log --pretty='- %s' -20
    fi
    echo
    echo "Checking the download:"
    echo
    echo '```sh'
    echo "sha256sum -c SHA256SUMS"
    echo "gh attestation verify $name.tar.gz --repo $origin_repo"
    if [[ -n "$appimage_name" ]]; then
        echo "gh attestation verify $appimage_name --repo $origin_repo"
    fi
    echo '```'
    echo
    echo "Inside: \`protonvpn-gui\`, the \`.desktop\` entry, the icon, README and LICENSE. The CLI"
    echo "itself is not bundled — \`protonvpn\` is a dependency of the system."
    if [[ -n "$appimage_name" ]]; then
        echo
        echo "Beside it, \`$appimage_name\` is the same binary packed as an AppImage: download,"
        echo "\`chmod +x\`, run — no installation. The \`protonvpn\` CLI is still needed on the system."
    fi
} > "$notes_file"

# The assets to publish, in one place so the dry-run listing and the upload cannot drift apart.
assets=("dist/$name.tar.gz" dist/SHA256SUMS)
if [[ -n "$appimage_name" ]]; then
    assets+=("dist/$appimage_name")
fi

token="${GH_TOKEN:-${GITHUB_TOKEN:-}}"
if [[ "$DRY_RUN" -eq 1 || -z "$token" ]]; then
    echo
    if [[ "$DRY_RUN" -eq 1 ]]; then
        echo "== dry run: nothing is published =="
    else
        echo "== no GH_TOKEN: the release is built but not published =="
    fi
    echo "   the tag would be: $tag"
    echo "   the files:        ${assets[*]}"
    exit 0
fi

echo
echo "== publish =="
if gh release view "$tag" >/dev/null 2>&1; then
    # Re-running the same commit (a retried job, a second dispatch for the same main) must not
    # fail: upload over the existing assets instead of trying to create the release twice.
    echo "   release $tag already exists — overwriting the artifacts"
    gh release upload "$tag" "${assets[@]}" --clobber
else
    gh release create "$tag" \
        --title "$tag" \
        --notes-file "$notes_file" \
        "${assets[@]}"
fi
echo "   https://github.com/$(gh repo view --json nameWithOwner --jq .nameWithOwner)/releases/tag/$tag"
