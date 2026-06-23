#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$SCRIPT_DIR"
BUILD_DIR="$PROJECT_DIR/build"
FRONTEND_DIR="$PROJECT_DIR/ui"
TAURI_DIR="$PROJECT_DIR/src-tauri"

echo "============================================"
echo "  RustDesk QuickSupport - Build All"
echo "============================================"
echo ""

mkdir -p "$BUILD_DIR"

check_deps() {
    local missing=()
    for cmd in cargo rustup node npm; do
        if ! command -v "$cmd" &>/dev/null; then
            missing+=("$cmd")
        fi
    done
    if [ ${#missing[@]} -gt 0 ]; then
        echo "Error: Missing required tools: ${missing[*]}"
        exit 1
    fi
}

install_target() {
    local target="$1"
    if ! rustup target list --installed | grep -q "$target"; then
        echo "Installing target: $target ..."
        rustup target add "$target"
    fi
}

install_tauri_cli() {
    if ! command -v cargo-tauri &>/dev/null; then
        echo "Installing tauri-cli..."
        cargo install tauri-cli --version "^2"
    fi
}

build_macos() {
    local target="$1"
    local target_label="$2"
    echo ""
    echo "--- Building for macOS ($target_label) ---"

    if [[ "$(uname)" != "Darwin" ]]; then
        echo "Warning: Building macOS targets requires running on macOS. Skipping."
        return 0
    fi

    install_target "$target"

    cargo tauri build \
        --target "$target" \
        --bundles dmg \
        2>&1 | tee "$BUILD_DIR/build-macos-$target_label.log"

    echo "macOS ($target_label) build completed."
}

build_windows() {
    local target="x86_64-pc-windows-msvc"
    echo ""
    echo "--- Building for Windows (x86_64) ---"

    install_target "$target"

    cargo tauri build \
        --target "$target" \
        --bundles msi \
        2>&1 | tee "$BUILD_DIR/build-windows.log"

    echo "Windows build completed."
}

build_linux() {
    local target="x86_64-unknown-linux-gnu"
    echo ""
    echo "--- Building for Linux (x86_64) ---"

    install_target "$target"

    cargo tauri build \
        --target "$target" \
        --bundles deb,appimage \
        2>&1 | tee "$BUILD_DIR/build-linux.log"

    echo "Linux build completed."
}

build_all() {
    local os_name
    os_name="$(uname -s)"

    case "$os_name" in
        Darwin)
            echo "Building on macOS - producing macOS + Linux + Windows builds..."
            build_macos "aarch64-apple-darwin" "arm64"
            build_macos "x86_64-apple-darwin" "x86_64"
            build_linux
            build_windows
            ;;
        Linux)
            echo "Building on Linux - producing Linux + Windows builds..."
            build_linux
            build_windows
            ;;
        *)
            echo "Unknown OS: $os_name"
            echo "Building for current target only..."
            cargo tauri build 2>&1 | tee "$BUILD_DIR/build-native.log"
            ;;
    esac
}

check_deps
install_tauri_cli

echo "Ensuring frontend dependencies are available..."
echo "(No frontend dependencies needed - using vanilla HTML/CSS/JS)"

echo ""
echo "Starting cross-platform builds..."
echo "Build output will be in: $BUILD_DIR"
echo "Bundle output will be in: $TAURI_DIR/target/<target>/release/bundle/"
echo ""

build_all

echo ""
echo "============================================"
echo "  Build Complete!"
echo "============================================"
echo ""
echo "Build artifacts:"
echo "  Logs:       $BUILD_DIR/"
echo "  Bundles:    $TAURI_DIR/target/*/release/bundle/"
echo ""
if [[ "$(uname -s)" == "Darwin" ]]; then
    echo "  macOS ARM:  $TAURI_DIR/target/aarch64-apple-darwin/release/bundle/"
    echo "  macOS x64:  $TAURI_DIR/target/x86_64-apple-darwin/release/bundle/"
fi
if [[ "$(uname -s)" == "Darwin" ]] || [[ "$(uname -s)" == "Linux" ]]; then
    echo "  Linux:      $TAURI_DIR/target/x86_64-unknown-linux-gnu/release/bundle/"
    echo "  Windows:    $TAURI_DIR/target/x86_64-pc-windows-msvc/release/bundle/"
fi
echo ""
