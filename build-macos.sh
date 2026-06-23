#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$SCRIPT_DIR"
BUILD_DIR="$PROJECT_DIR/build"
TAURI_DIR="$PROJECT_DIR/src-tauri"

echo "============================================"
echo "  RustDesk QuickSupport - macOS Build"
echo "============================================"
echo ""

mkdir -p "$BUILD_DIR"

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

# ── targets ─────────────────────────────────────
ARCH=$(uname -m)
TARGETS=()

if [[ "$ARCH" == "arm64" ]]; then
    TARGETS=("aarch64-apple-darwin" "x86_64-apple-darwin")
elif [[ "$ARCH" == "x86_64" ]]; then
    TARGETS=("x86_64-apple-darwin" "aarch64-apple-darwin")
fi

for target in "${TARGETS[@]}"; do
    if ! rustup target list --installed | grep -q "$target"; then
        echo "Installing target: $target ..."
        rustup target add "$target"
    fi
done

# ── build ───────────────────────────────────────
for target in "${TARGETS[@]}"; do
    label=$(echo "$target" | sed 's/-apple-darwin//;s/arch64/arm64/')
    echo ""
    echo "--- Building for macOS ($label) ---"

    cargo tauri build \
        --target "$target" \
        --bundles dmg \
        2>&1 | tee "$BUILD_DIR/build-macos-$label.log"

    echo "macOS ($label) build completed."
done

# ── summary ─────────────────────────────────────
echo ""
echo "============================================"
echo "  Build Complete!"
echo "============================================"
echo ""
echo "Build artifacts:"
for target in "${TARGETS[@]}"; do
    label=$(echo "$target" | sed 's/-apple-darwin//;s/arch64/arm64/')
    echo "  macOS $label: $PROJECT_DIR/target/$target/release/bundle/dmg/"
done
echo ""
