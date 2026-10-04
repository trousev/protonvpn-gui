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
# The toolchain is pinned, not floating. `linuxdeploy` used to be fetched from its `continuous`
# tag, whose contents change without notice, and nothing checked what came back. A release
# artifact built by a binary that can be replaced under the same URL is not reproducible, so both
# the tool and the runtime are now named by version and verified by SHA-256 before they run. The
# `linuxdeploy-plugin-appimage` release is what carries the `appimagetool` that actually assembles
# the image; pinning it pins that too, and the runtime is overridden explicitly so the bytes that
# execute on the user's machine are chosen here rather than inherited from the build tool.
#
# Usage:
#   packaging/appimage/build.sh              # build, using ./tools for the pinned tools
#   ARCH=aarch64 packaging/appimage/build.sh # cross-arch (needs an aarch64 sysroot to build)
#
# Requirements: a release build of the workspace, `curl` to fetch the tools the first time,
# `sha256sum` to verify them.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

ARCH="${ARCH:-$(uname -m)}"
TOOLS="$REPO_ROOT/tools"
APPDIR="$REPO_ROOT/target/appimage/ProtonVPNGUI.AppDir"
OUT_DIR="$REPO_ROOT/target/appimage"

# The directories the pinned tools are allowed to search. Deliberately explicit rather than
# inherited: linuxdeploy aborts if any `$PATH` entry cannot be listed — a root-only container
# directory such as /opt/containerd/bin is enough — and it also searches `$PATH` for plugins, so
# an inherited path can shadow the pinned plugin with a file someone else controls. Everything
# linuxdeploy needs (`ldd`, `objdump`, `readelf`) lives in these directories; `patchelf` and
# `strip` are bundled with it.
TOOL_PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

# --- pinned toolchain ---------------------------------------------------------------------------
#
# Bump a version only together with its checksum. The checksums are the SHA-256 of the asset as
# published on the release page (linuxdeploy and appimagetool publish them; the plugin release
# does not, so it was downloaded once and hashed here). Nothing is executed before it matches.

LINUXDEPLOY_VERSION=1-alpha-20251107-1
LINUXDEPLOY_PLUGIN_APPIMAGE_VERSION=1-alpha-20250213-1
APPIMAGE_RUNTIME_VERSION=20251108

case "$ARCH" in
    x86_64)
        LINUXDEPLOY_SHA256=c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d
        PLUGIN_SHA256=992d502a248e14ab185448ddf6f6e7d25558cb84d4623c354c3af350c25fccb3
        RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
        ;;
    aarch64)
        LINUXDEPLOY_SHA256=620095110d693282b8ebeb244a95b5e911cf8f65f76c88b4b47d16ae6346fcff
        PLUGIN_SHA256=83c292149274965a865dcd44c135cfca8ba28c6b7de3eb628d4b8b5f248af17c
        RUNTIME_SHA256=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444
        ;;
    *)
        echo "error: no pinned AppImage toolchain for architecture '$ARCH'" >&2
        echo "       add its checksums to this script before promising it" >&2
        exit 2
        ;;
esac

# fetch_verified URL DEST SHA256 — download DEST unless it is already the verified bytes.
fetch_verified() {
    local url="$1" dest="$2" want="$3"

    if [[ -f "$dest" ]] && printf '%s  %s\n' "$want" "$dest" | sha256sum -c --quiet - 2>/dev/null; then
        return 0
    fi

    echo "downloading $(basename "$dest") ..."
    rm -f "$dest"
    curl --proto '=https' --tlsv1.2 -fsSL --retry 3 --retry-delay 2 -o "$dest" "$url"

    if ! printf '%s  %s\n' "$want" "$dest" | sha256sum -c --quiet -; then
        echo "error: checksum mismatch for $(basename "$dest")" >&2
        echo "  expected $want" >&2
        echo "  got      $(sha256sum "$dest" | cut -d' ' -f1)" >&2
        echo "  refusing to run an unverified tool; delete $dest and retry" >&2
        exit 1
    fi
    chmod +x "$dest"
}

mkdir -p "$TOOLS"
fetch_verified \
    "https://github.com/linuxdeploy/linuxdeploy/releases/download/$LINUXDEPLOY_VERSION/linuxdeploy-$ARCH.AppImage" \
    "$TOOLS/linuxdeploy-$ARCH.AppImage" \
    "$LINUXDEPLOY_SHA256"
fetch_verified \
    "https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/$LINUXDEPLOY_PLUGIN_APPIMAGE_VERSION/linuxdeploy-plugin-appimage-$ARCH.AppImage" \
    "$TOOLS/linuxdeploy-plugin-appimage-$ARCH.AppImage" \
    "$PLUGIN_SHA256"
fetch_verified \
    "https://github.com/AppImage/type2-runtime/releases/download/$APPIMAGE_RUNTIME_VERSION/runtime-$ARCH" \
    "$TOOLS/runtime-$ARCH" \
    "$RUNTIME_SHA256"

echo "== release build =="
cargo build --release -p protonvpn-gui --locked

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
# `--appimage-extract-and-run` keeps linuxdeploy working on hosts without FUSE (containers, CI).
# The environment variable does the same for the plugin AppImage that linuxdeploy spawns, which
# the flag cannot reach; without it a FUSE-less runner fails to start the plugin.
export APPIMAGE_EXTRACT_AND_RUN=1
# Override the plugin's bundled runtime with the pinned one, so the runtime that ships in the
# AppImage is the version named above and not whatever the plugin happened to be built with.
export LDAI_RUNTIME_FILE="$TOOLS/runtime-$ARCH"
export OUTPUT="$OUT_DIR/ProtonVPN-GUI-$ARCH.AppImage"

PATH="$TOOL_PATH" "$TOOLS/linuxdeploy-$ARCH.AppImage" \
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
