#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$SCRIPT_DIR"
BUILD_DIR="$PROJECT_DIR/build"
TAURI_DIR="$PROJECT_DIR/src-tauri"

echo "============================================"
echo "  RustDesk QuickSupport - Linux Build"
echo "============================================"
echo ""

mkdir -p "$BUILD_DIR"

# ── platform detection ──────────────────────────
HOST_OS="$(uname -s)"
if [[ "$HOST_OS" == "Linux" ]]; then
    echo "Host platform: Linux (native)"
else
    echo "Host platform: $HOST_OS (cross-compile)"
fi

# ── deps check ──────────────────────────────────
missing=()
for cmd in cargo rustup; do
    if ! command -v "$cmd" &>/dev/null; then
        missing+=("$cmd")
    fi
done
if [ ${#missing[@]} -gt 0 ]; then
    echo "Error: Missing required tools: ${missing[*]}"
    echo "Install Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

if ! command -v cargo-tauri &>/dev/null; then
    echo "Installing tauri-cli..."
    cargo install tauri-cli --version "^2"
fi

# ── native libs check ───────────────────────────
if [[ "$HOST_OS" == "Linux" ]]; then
    echo "Checking system dependencies..."
    MISSING_LIBS=()

    if ! pkg-config --exists webkit2gtk-4.1 2>/dev/null && ! pkg-config --exists webkit2gtk-4.0 2>/dev/null; then
        MISSING_LIBS+=("webkit2gtk")
    fi
    if ! pkg-config --exists glib-2.0 2>/dev/null; then
        MISSING_LIBS+=("glib")
    fi
    if ! pkg-config --exists gtk+-3.0 2>/dev/null; then
        MISSING_LIBS+=("gtk3")
    fi

    if [ ${#MISSING_LIBS[@]} -gt 0 ]; then
        echo "Missing system libraries: ${MISSING_LIBS[*]}"
        echo ""
        echo "Install them with your package manager:"
        echo "  Ubuntu/Debian:  sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libglib2.0-dev"
        echo "  Fedora:         sudo dnf install webkit2gtk4.1-devel gtk3-devel glib2-devel"
        echo "  Arch:           sudo pacman -S webkit2gtk-4.1 gtk3 glib2"
        exit 1
    fi
else
    echo ""
    echo "Warning: Cross-compiling for Linux requires matching system libraries."
    echo "Recommend building on a Linux host for best results."
    echo ""
fi

# ── install target ──────────────────────────────
TARGET="x86_64-unknown-linux-gnu"

if ! rustup target list --installed | grep -q "$TARGET"; then
    echo "Installing target: $TARGET ..."
    rustup target add "$TARGET"
fi

# ── build ───────────────────────────────────────
echo ""
echo "--- Building for Linux ($TARGET) ---"

cargo tauri build \
    --target "$TARGET" \
    2>&1 | tee "$BUILD_DIR/build-linux.log"

echo "Linux build completed."

# ── summary ─────────────────────────────────────
echo ""
echo "============================================"
echo "  Build Complete!"
echo "============================================"
echo ""

BUNDLE_DIR="$PROJECT_DIR/target/$TARGET/release/bundle"

if [ -d "$BUNDLE_DIR/deb" ]; then
    echo "  DEB package:  $BUNDLE_DIR/deb/"
fi
if [ -d "$BUNDLE_DIR/appimage" ]; then
    echo "  AppImage:     $BUNDLE_DIR/appimage/"
fi

BIN="$PROJECT_DIR/target/$TARGET/release/rustdesk-client"
if [ -f "$BIN" ]; then
    SIZE=$(ls -lh "$BIN" | awk '{print $5}')
    echo "  Binary:       $BIN ($SIZE)"
fi

echo ""
if [[ "$HOST_OS" == "Linux" ]]; then
    echo "  Run directly: ./target/release/rustdesk-client"
else
    echo "  Copy the binary or bundle to a Linux machine to test."
fi
echo ""
