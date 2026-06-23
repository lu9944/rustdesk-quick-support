#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$SCRIPT_DIR"
BUILD_DIR="$PROJECT_DIR/build"
TAURI_DIR="$PROJECT_DIR/src-tauri"
ICON_DIR="$TAURI_DIR/icons"

echo "============================================"
echo "  RustDesk QuickSupport - Windows Build"
echo "============================================"
echo ""

mkdir -p "$BUILD_DIR"

# ── platform detection ──────────────────────────
HOST_OS="$(uname -s)"
case "$HOST_OS" in
    MINGW*|MSYS*|CYGWIN*)
        HOST="windows"
        ;;
    Darwin)
        HOST="macos"
        ;;
    Linux)
        HOST="linux"
        ;;
    *)
        echo "Unknown host OS: $HOST_OS"
        exit 1
        ;;
esac

echo "Host platform: $HOST"

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

# ── icon.ico ────────────────────────────────────
if [ ! -f "$ICON_DIR/icon.ico" ]; then
    echo "icon.ico not found, generating from 32x32.png..."

    if command -v magick &>/dev/null; then
        magick "$ICON_DIR/32x32.png" -resize 32x32 "$ICON_DIR/icon.ico"
    elif command -v convert &>/dev/null; then
        convert "$ICON_DIR/32x32.png" -resize 32x32 "$ICON_DIR/icon.ico"
    else
        echo "Error: ImageMagick not found. Install it first:"
        if [[ "$HOST" == "macos" ]]; then
            echo "  brew install imagemagick"
        elif [[ "$HOST" == "linux" ]]; then
            echo "  sudo apt install imagemagick"
        fi
        exit 1
    fi

    echo "icon.ico created."
fi

# ── build ───────────────────────────────────────
if [[ "$HOST" == "windows" ]]; then
    # ── native MSVC build ────────────────────────
    TARGET="x86_64-pc-windows-msvc"

    if ! rustup target list --installed | grep -q "$TARGET"; then
        echo "Installing target: $TARGET ..."
        rustup target add "$TARGET"
    fi

    echo ""
    echo "--- Building for Windows (native MSVC) ---"

    cargo tauri build \
        --target "$TARGET" \
        2>&1 | tee "$BUILD_DIR/build-windows.log"

else
    # ── cross-compile from macOS / Linux ─────────
    TARGET="x86_64-pc-windows-msvc"

    if ! rustup target list --installed | grep -q "$TARGET"; then
        echo "Installing target: $TARGET ..."
        rustup target add "$TARGET"
    fi

    if ! command -v cargo-xwin &>/dev/null; then
        echo "Installing cargo-xwin..."
        cargo install cargo-xwin
    fi

    if ! command -v lld-link &>/dev/null; then
        echo "Error: lld-link not found."
        if [[ "$HOST" == "macos" ]]; then
            echo "  brew install lld"
        fi
        exit 1
    fi

    # Homebrew LLVM tools (llvm-rc) may not be in default PATH
    if [[ "$HOST" == "macos" ]]; then
        for p in /opt/homebrew/opt/llvm/bin /usr/local/opt/llvm/bin; do
            if [ -d "$p" ] && [ -x "$p/llvm-rc" ]; then
                export PATH="$p:$PATH"
                break
            fi
        done
    fi

    if ! command -v llvm-rc &>/dev/null; then
        echo "Error: llvm-rc not found."
        if [[ "$HOST" == "macos" ]]; then
            echo "  brew install llvm"
        fi
        exit 1
    fi

    echo "Setting up xwin environment..."
    eval "$(cargo xwin env --target "$TARGET" 2>/dev/null)"

    echo ""
    echo "--- Cross-compiling for Windows (MSVC, static WebView2) ---"

    cargo tauri build \
        --target "$TARGET" \
        --no-bundle \
        2>&1 | tee "$BUILD_DIR/build-windows.log"
fi

# ── summary ─────────────────────────────────────
echo ""
echo "============================================"
echo "  Build Complete!"
echo "============================================"
echo ""

EXE="$PROJECT_DIR/target/$TARGET/release/rustdesk-client.exe"
DLL="$PROJECT_DIR/target/$TARGET/release/WebView2Loader.dll"
if [ -f "$EXE" ]; then
    SIZE=$(ls -lh "$EXE" | awk '{print $5}')
    echo "  Single exe:  $EXE ($SIZE)"
fi

if [[ "$HOST" == "windows" ]]; then
    echo "  NSIS setup:  $PROJECT_DIR/target/$TARGET/release/bundle/nsis/"
fi

echo ""
echo "  Single .exe file, no DLL needed. Double-click to run."
echo "  Requires Windows 10+ (bundled WebView2)."
echo ""
