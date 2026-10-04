#!/usr/bin/env bash
#
# Publishes a release: version `X.Y.N`, where
#
#   * `X.Y` is the latest release tag in the repository (the line's base tag, e.g. `0.1`),
#   * `N`   is the number of commits in `main`.
#
# The commit count rather than a hand-kept number, because a number someone has to remember is a
# number that eventually lies. Every merge to `main` adds at least one commit, so every release
# gets a version that is unique and larger than the previous one, and there is nothing to bump.
#
# It also builds and packages, so that the artifact attached to a release is exactly what this
# script produced locally — and so that the whole path except the upload can be exercised without
# a token (`--dry-run`, which is what `act` and a laptop use).
#
# Usage:
#   scripts/release.sh                 # build, package, publish (needs GH_TOKEN)
#   scripts/release.sh --dry-run       # build and package only, never publish
#   scripts/release.sh --skip-build    # publish what is already in target/release
#   scripts/release.sh --appimage FILE # attach an AppImage built by packaging/appimage/build.sh
#   scripts/release.sh --print-version # print the tag that would be released, and stop
#
# The AppImage is an input, not something this script builds. It is produced by
# `packaging/appimage/build.sh` in a job that cannot write to the repository and passed here, so
# the one job allowed to publish never runs the third-party toolchain. A local release is:
#
#   packaging/appimage/build.sh
#   scripts/release.sh --appimage target/appimage/ProtonVPN-GUI-$(uname -m).AppImage
#
# Requirements: cargo, git, tar, sha256sum; `gh` only when publishing.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DRY_RUN=0
SKIP_BUILD=0
PRINT_VERSION=0
APPIMAGE=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run) DRY_RUN=1; shift ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        --print-version) PRINT_VERSION=1; shift ;;
        --appimage)
            APPIMAGE="${2:-}"
            if [[ -z "$APPIMAGE" ]]; then
                echo "error: --appimage requires a path" >&2
                exit 2
            fi
            shift 2
            ;;
        -h|--help) sed -n '3,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

# Release tags look like `0.1` or `0.1.7`. Anything else (a branch tag, a date tag) is ignored:
# guessing a base from an unrelated tag is how you ship `2026.9.1`.
release_tags() {
    git tag --list | grep -E '^[0-9]+\.[0-9]+(\.[0-9]+)?$' | sort -V || true
}

latest="$(release_tags | tail -1)"
base="${latest:-0.1}"
base="$(echo "$base" | cut -d. -f1,2)"

count="$(git rev-list --count HEAD)"
tag="$base.$count"

if [[ "$PRINT_VERSION" -eq 1 ]]; then
    echo "$tag"
    exit 0
fi

echo "== release $tag =="
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
    echo "Собрано из коммита \`$(git rev-parse --short HEAD)\`."
    echo
    echo "Изменения:"
    echo
    previous="$(release_tags | grep -E "^${base}\.[0-9]+$" | sort -V | tail -1 || true)"
    if [[ -n "$previous" ]]; then
        git log --pretty='- %s' "$previous..HEAD"
    else
        git log --pretty='- %s' -20
    fi
    echo
    echo "Проверка загрузки:"
    echo
    echo '```sh'
    echo "sha256sum -c SHA256SUMS"
    echo "gh attestation verify $name.tar.gz --repo $origin_repo"
    if [[ -n "$appimage_name" ]]; then
        echo "gh attestation verify $appimage_name --repo $origin_repo"
    fi
    echo '```'
    echo
    echo "Внутри: \`protonvpn-gui\`, \`.desktop\`, иконка, README и LICENSE. Сам CLI \`protonvpn\`"
    echo "не входит — это зависимость системы."
    if [[ -n "$appimage_name" ]]; then
        echo
        echo "Рядом \`$appimage_name\` — тот же бинарник, упакованный в AppImage: скачал,"
        echo "\`chmod +x\` и запустил, без установки. CLI \`protonvpn\` всё равно нужен в системе."
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
        echo "== dry run: релиз не публикуется =="
    else
        echo "== GH_TOKEN не задан: релиз собран, но не опубликован =="
    fi
    echo "   тег был бы: $tag"
    echo "   файлы:     ${assets[*]}"
    exit 0
fi

echo
echo "== publish =="
if gh release view "$tag" >/dev/null 2>&1; then
    # Re-running the same commit (a manual dispatch, a retried job) must not fail: upload over
    # the existing assets instead of trying to create the release twice.
    echo "   релиз $tag уже существует — перезаписываю артефакты"
    gh release upload "$tag" "${assets[@]}" --clobber
else
    gh release create "$tag" \
        --title "$tag" \
        --notes-file "$notes_file" \
        "${assets[@]}"
fi
echo "   https://github.com/$(gh repo view --json nameWithOwner --jq .nameWithOwner)/releases/tag/$tag"
