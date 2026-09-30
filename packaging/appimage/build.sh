#!/usr/bin/env bash
#
# Builds the AppImage.
#
# AppImage only, on purpose (`docs/plan.md` Phase 3): a Flatpak sandbox would fight both the host
# `protonvpn` CLI and the tray's StatusNotifierItem, and a wrapper gains nothing from it.
#
# The bundle contains our binary and nothing else: `protonvpn` is a runtime dependency of the
# host system, declared in the README and in the app, never vendored. Bundling it would mean
# shipping a Python stack and lying about what the package is.
#
# Usage:
#   packaging/appimage/build.sh              # build, using ./tools for linuxdeploy
#   ARCH=aarch64 packaging/appimage/build.sh # cross-arch (needs an aarch64 sysroot to build)
#
# Requirements: a release build of the workspace, `curl` to fetch linuxdeploy the first time.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

ARCH="${ARCH:-$(uname -m)}"
TOOLS="$REPO_ROOT/tools"
APPDIR="$REPO_ROOT/target/appimage/ProtonVPNGUI.AppDir"
OUT_DIR="$REPO_ROOT/target/appimage"

echo "== release build =="
cargo build --release -p protonvpn-gui

echo "== staging AppDir =="
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" \
         "$APPDIR/usr/share/icons/hicolor/256x256/apps" "$APPDIR/usr/share/metainfo"

install -m 755 target/release/protonvpn-gui "$APPDIR/usr/bin/protonvpn-gui"
install -m 644 packaging/protonvpn-gui.desktop \
    "$APPDIR/usr/share/applications/protonvpn-gui.desktop"
install -m 644 packaging/icons/protonvpn-gui.png \
    "$APPDIR/usr/share/icons/hicolor/256x256/apps/protonvpn-gui.png"

# linuxdeploy wants these at the AppDir root as well.
cp packaging/protonvpn-gui.desktop "$APPDIR/protonvpn-gui.desktop"
cp packaging/icons/protonvpn-gui.png "$APPDIR/protonvpn-gui.png"

echo "== linuxdeploy =="
mkdir -p "$TOOLS"
LINUXDEPLOY="$TOOLS/linuxdeploy-$ARCH.AppImage"
if [[ ! -x "$LINUXDEPLOY" ]]; then
    echo "downloading linuxdeploy for $ARCH..."
    curl -fL -o "$LINUXDEPLOY" \
        "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-$ARCH.AppImage"
    chmod +x "$LINUXDEPLOY"
fi

# `--appimage-extract-and-run` keeps this working on hosts without FUSE (containers, CI).
export OUTPUT="$OUT_DIR/ProtonVPN-GUI-$ARCH.AppImage"
"$LINUXDEPLOY" \
    --appimage-extract-and-run \
    --appdir "$APPDIR" \
    --desktop-file "$APPDIR/protonvpn-gui.desktop" \
    --icon-file "$APPDIR/protonvpn-gui.png" \
    --output appimage

echo
echo "built: $OUTPUT"
echo
echo "Reminder for whoever ships this: the AppImage does not contain the CLI."
echo "Declare the runtime dependency on the official apt package \`protonvpn\`."
