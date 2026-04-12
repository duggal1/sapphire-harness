#!/bin/bash
# Ultra-minimal sp installer - one-liner version
# curl -fsSL https://raw.githubusercontent.com/<user>/<repo>/main/install-simple.sh | bash

set -e

echo "Installing Sapphire Agent Factory (sp)..."

# Detect platform
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)
[ "$OS" = "darwin" ] && OS="apple-darwin" || OS="unknown-linux-gnu"
[ "$ARCH" = "arm64" ] && ARCH="aarch64"

INSTALL_DIR="${HOME}/.local/bin"
BINARY="$INSTALL_DIR/sp"

# Ensure install directory exists
mkdir -p "$INSTALL_DIR"

# Check if cargo is available
if ! command -v cargo &> /dev/null; then
    echo "Error: Rust/Cargo is required. Install with:"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

# Build from source (using current directory or clone repo)
if [ -f "Cargo.toml" ] && grep -q "sapphire-agent-factory" Cargo.toml 2>/dev/null; then
    echo "Building from current directory..."
    cargo build --release --quiet
    cp target/release/sp "$BINARY"
else
    echo "Cloning repository..."
    BUILD_DIR=$(mktemp -d)
    git clone --depth 1 "https://github.com/duggal1/sapphire-harness.git" "$BUILD_DIR"
    cd "$BUILD_DIR"
    cargo build --release --quiet
    cp target/release/sp "$BINARY"
    rm -rf "$BUILD_DIR"
fi

chmod +x "$BINARY"

# Verify
if [ -f "$BINARY" ] && [ -x "$BINARY" ]; then
    echo "✓ Installed successfully: $BINARY"
    echo ""
    echo "Add to PATH (if needed):"
    echo "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.zshrc"
    echo ""
    echo "Quick start:"
    echo "  sp qwen 2 --repo . --mission \"debug and validate the repo\""
    echo "  sp --help"
else
    echo "✗ Installation failed"
    exit 1
fi
