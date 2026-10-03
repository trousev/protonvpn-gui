#!/usr/bin/env bash
#
# Fails if the Linux build graph picks up a crate that exists only for another operating system.
#
# This is a Linux-only tool. The window is a Wayland/X11 window, the tray speaks
# StatusNotifierItem over the session bus, and the only program the app ever executes is the
# `protonvpn` CLI. Android, Windows and macOS are not supported and are not built.
#
# Cargo still *resolves* the cross-platform branches of `winit`, `softbuffer`, `zbus` and `tokio`:
# `Cargo.lock` is a union over every target, and nothing can tell a dependency to forget its
# `[target.'cfg(windows)'.dependencies]` table. That is a property of the ecosystem, not a bug we
# can fix here, so the lock file is allowed to mention them. What this script guarantees is the
# thing that actually matters: none of them is ever compiled on Linux.
#
# The crate count is ratcheted. A wrapper around a CLI that grows its dependency graph by
# accident is how this one reached two hundred crates, so growth has to be a deliberate edit to
# `MAX_CRATES` rather than a side effect of someone's feature flag.
#
# Usage: scripts/check-linux-deps.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# The Linux closure as of the change that introduced this script: 218 crates, 23 of them removed
# by dropping iced's unused `auto-detect-theme`. Raise this only with the reason in the commit
# message.
MAX_CRATES=218

HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
if [[ -z "$HOST_TRIPLE" ]]; then
    echo 'error: could not determine the host triple from `rustc -vV`' >&2
    exit 1
fi

# `--locked`: the graph we check has to be the graph we ship. Without it this would happily
# resolve a fresh lock file and measure something no build will ever use.
graph="$(cargo tree --workspace --edges normal,build --target "$HOST_TRIPLE" \
    --prefix none --locked 2>/dev/null \
    | sed -E 's/ v[0-9].*$//; s/ \([*]\)$//' \
    | grep -v '^$' \
    | sort -u)"

foreign="$(grep -E '^(windows|winapi|objc|core-foundation|core-graphics|dispatch|android|ndk|jni|wasm|web-sys|js-sys|redox|hermit|fuchsia|wasi)' <<<"$graph" || true)"

if [[ -n "$foreign" ]]; then
    echo "error: the Linux build would compile crates for another operating system:" >&2
    sed 's/^/  /' <<<"$foreign" >&2
    echo >&2
    echo 'This project is Linux only; find what pulled them in with `cargo tree -i <crate>`.' >&2
    exit 1
fi

count="$(wc -l <<<"$graph")"
echo "$count crates in the Linux build graph (ceiling $MAX_CRATES), none for another platform"

if ((count > MAX_CRATES)); then
    echo "error: the graph grew by $((count - MAX_CRATES)) crate(s)." >&2
    echo 'Adding a dependency is a decision: explain it, then raise MAX_CRATES in this script.' >&2
    exit 1
fi
