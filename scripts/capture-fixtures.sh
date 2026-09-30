#!/usr/bin/env bash
#
# Re-captures the parser test corpus from the real `protonvpn` CLI, through a PTY.
#
# Why a PTY and not a pipe: the application always runs the CLI attached to a pseudo-terminal
# (it has to — `signin` prompts for a password), so the parsers must be written against what a
# PTY produces. See docs/architecture.md §10.3.
#
# Usage:
#   scripts/capture-fixtures.sh                 # disconnected set only (safe, no network change)
#   scripts/capture-fixtures.sh --connected     # also connects to the VPN and back
#   scripts/capture-fixtures.sh --cols 80       # override terminal width (default 120)
#
# Requirements: the official `protonvpn` CLI installed, and a signed-in session.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

COLS=120
WITH_CONNECTION=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --connected) WITH_CONNECTION=1; shift ;;
        --cols) COLS="$2"; shift 2 ;;
        -h|--help) sed -n '3,16p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

if ! command -v protonvpn >/dev/null 2>&1; then
    echo 'error: `protonvpn` not found on PATH' >&2
    exit 1
fi

if [[ ! -x target/debug/capture-fixtures ]]; then
    echo "building capture-fixtures..."
    cargo build --bin capture-fixtures
fi

BIN=./target/debug/capture-fixtures
capture() {
    local name="$1"; shift
    "$BIN" --name "$name" --cols "$COLS" -- "$@"
}

echo "== disconnected set =="
capture status_disconnected      protonvpn status
capture info                     protonvpn info
capture config_list              protonvpn config list
capture countries_list           protonvpn countries list
# Same table at a deliberately narrow width. Verified byte-identical to the 120-column capture:
# the CLI's tables are a fixed 40 characters and do not wrap, so the parsers must not depend on
# terminal width. Kept as a regression guard.
"$BIN" --name countries_list_cols80 --cols 80 -- protonvpn countries list
capture cities_list_ch           protonvpn cities list CH
capture disconnect_noop          protonvpn disconnect
capture connect_invalid_country  protonvpn connect --country ZZ
capture connect_invalid_server   protonvpn connect ZZ#99

# Note: `capture-fixtures` records the child's exit code in the metadata but always exits 0
# itself, so error-case captures do not trip `set -e`.

if [[ "$WITH_CONNECTION" -eq 1 ]]; then
    echo
    echo "== connected set (this brings the VPN up and back down) =="
    capture connect_nl              protonvpn connect --country NL
    capture status_connected        protonvpn status
    capture connect_while_connected protonvpn connect --country CH
    capture disconnect              protonvpn disconnect
    capture status_disconnected_after protonvpn status
fi

echo
echo "done. fixtures in crates/protonvpn-core/tests/fixtures/pty/"
