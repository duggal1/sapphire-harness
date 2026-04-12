#!/bin/bash
# Sapphire Agent Factory (sp) - Dead Simple Installer
# Usage: curl -fsSL https://raw.githubusercontent.com/<user>/<repo>/main/install.sh | bash

set -e

# Configuration
REPO="sapphire-harness"
GITHUB_USER="duggal1"
BINARY="sp"
INSTALL_DIR="${HOME}/.local/bin"
VERSION="${SP_VERSION:-latest}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Helper functions
info() { echo -e "${BLUE}ℹ${NC} $1"; }
success() { echo -e "${GREEN}✓${NC} $1"; }
warn() { echo -e "${YELLOW}⚠${NC} $1"; }
error() { echo -e "${RED}✗${NC} $1" >&2; }

# Detect platform
detect_platform() {
    local os arch platform

    os=$(uname -s | tr '[:upper:]' '[:lower:]')
    arch=$(uname -m)

    case "$os" in
        darwin) os="apple-darwin" ;;
        linux) os="unknown-linux-gnu" ;;
        *) error "Unsupported OS: $os"; exit 1 ;;
    esac

    case "$arch" in
        x86_64) arch="x86_64" ;;
        arm64|aarch64) arch="aarch64" ;;
        *) error "Unsupported architecture: $arch"; exit 1 ;;
    esac

    platform="${arch}-${os}"
    info "Detected platform: $platform"
    echo "$platform"
}

# Check prerequisites
check_prerequisites() {
    if ! command -v curl &> /dev/null; then
        error "curl is required but not installed"
        exit 1
    fi
}

# Create install directory
ensure_install_dir() {
    if [ ! -d "$INSTALL_DIR" ]; then
        info "Creating install directory: $INSTALL_DIR"
        mkdir -p "$INSTALL_DIR"
    fi
}

# Download and install binary
install_binary() {
    local platform="$1"
    local download_url tarball

    # For now, we'll build from source if no release exists
    # Once GitHub Releases are set up, use this pattern:
    # download_url="https://github.com/<user>/${REPO}/releases/${VERSION}/download/${BINARY}-${platform}.tar.gz"
    
    info "Building from source..."
    
    # Check if Rust is installed
    if ! command -v cargo &> /dev/null; then
        error "Rust/Cargo is required for source installation"
        error "Install Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        exit 1
    fi

    local build_dir
    build_dir=$(mktemp -d)
    info "Building in: $build_dir"

    # Clone and build
    git clone --depth 1 "https://github.com/${GITHUB_USER}/${REPO}.git" "$build_dir" 2>/dev/null || {
        # If clone fails, use current directory if it's the repo
        if [ -f "Cargo.toml" ] && grep -q "sapphire-agent-factory" Cargo.toml; then
            info "Using current directory as source"
            build_dir="$(pwd)"
        else
            error "Failed to clone repository and current directory is not the source"
            exit 1
        fi
    }

    cd "$build_dir"
    
    info "Compiling release binary (this may take a minute)..."
    cargo build --release --quiet
    
    if [ ! -f "target/release/${BINARY}" ]; then
        error "Build failed: binary not found at target/release/${BINARY}"
        exit 1
    fi

    # Install
    info "Installing to ${INSTALL_DIR}/${BINARY}"
    cp "target/release/${BINARY}" "${INSTALL_DIR}/${BINARY}"
    chmod +x "${INSTALL_DIR}/${BINARY}"

    # Cleanup
    if [ "$build_dir" != "$(pwd)" ]; then
        rm -rf "$build_dir"
    fi
}

# Verify installation
verify_installation() {
    local binary_path="${INSTALL_DIR}/${BINARY}"
    
    if [ ! -f "$binary_path" ]; then
        error "Installation failed: binary not found at $binary_path"
        exit 1
    fi

    if [ ! -x "$binary_path" ]; then
        error "Installation failed: binary is not executable"
        exit 1
    fi

    success "Installed successfully: $binary_path"
    info "Run 'sp --help' for full documentation"
}

# Check PATH
check_path() {
    if [[ ":$PATH:" != *":${INSTALL_DIR}:"* ]]; then
        warn "$INSTALL_DIR is not in your PATH"
        echo ""
        echo "Add it by running:"
        echo "  echo 'export PATH=\"${INSTALL_DIR}:\$PATH\"' >> ~/.zshrc"
        echo "  source ~/.zshrc"
        echo ""
        echo "Or run directly: ${INSTALL_DIR}/${BINARY}"
    fi
}

# Main installation flow
main() {
    echo ""
    echo -e "${BLUE}╔════════════════════════════════════════════════════════╗${NC}"
    echo -e "${BLUE}║${NC}  ${GREEN}Sapphire Agent Factory (sp) Installer${NC}                  ${BLUE}║${NC}"
    echo -e "${BLUE}║${NC}  Multi-agent orchestration CLI for your terminal     ${BLUE}║${NC}"
    echo -e "${BLUE}╚════════════════════════════════════════════════════════╝${NC}"
    echo ""

    check_prerequisites
    local platform
    platform=$(detect_platform)
    ensure_install_dir
    install_binary "$platform"
    verify_installation
    check_path

    echo ""
    success "Installation complete!"
    echo ""
    echo "Quick start:"
    echo "  ${BINARY} qwen 2 --repo . --mission \"debug and validate the repo\""
    echo "  ${BINARY} status          # Show active missions"
    echo "  ${BINARY} --help          # Full documentation"
    echo ""
    echo "Learn more: https://github.com/sapphire-agent-Factory/${REPO}"
    echo ""
}

main "$@"
