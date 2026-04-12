#!/bin/bash
# Sapphire Agent Factory (sp) — Full Installer
# Usage: curl -fsSL https://raw.githubusercontent.com/duggal1/sapphire-harness/master/install.sh | bash

set -e

# ── Color helpers ──────────────────────────────────────────────
BOLD=$(tput bold 2>/dev/null || echo "")
DIM=$(tput dim 2>/dev/null || echo "")
GREEN=$(tput setaf 2 2>/dev/null || echo "")
CYAN=$(tput setaf 6 2>/dev/null || echo "")
PURPLE=$(tput setaf 5 2>/dev/null || echo "")
YELLOW=$(tput setaf 3 2>/dev/null || echo "")
RED=$(tput setaf 1 2>/dev/null || echo "")
RESET=$(tput sgr0 2>/dev/null || echo "")

check()  { printf "${GREEN}✔${RESET} %s\n" "$1"; }
info()   { printf "${DIM}→${RESET} %s\n" "$1"; }
warn()   { printf "${YELLOW}⚠${RESET} %s\n" "$1"; }
err()    { printf "${RED}✕${RESET} %s\n" "$1" >&2; }

# ── Config ─────────────────────────────────────────────────────
REPO="sapphire-harness"
GITHUB_USER="duggal1"
BINARY_NAME="sp"
INSTALL_DIR="${HOME}/.local/bin"
VERSION="${SP_VERSION:-latest}"

# ── Banner ────────────────────────────────────────────────────
print_banner() {
    local border_color="${PURPLE}"
    local title_color="${CYAN}"
    local width=60

    printf "\n"
    printf "${border_color}┌"; printf '─%.0s' $(seq 1 $width); printf "┐${RESET}\n"
    printf "${border_color}│${RESET}"
    printf "%${width}s" "" | tr ' ' ' '
    printf "${border_color}│${RESET}\n"

    local line1="✨ Sapphire Agent Factory"
    local line2="Terminal-first multi-agent orchestration CLI"
    local pad1=$(( (width - ${#line1}) / 2 ))
    local pad2=$(( (width - ${#line2}) / 2 ))

    printf "${border_color}│${RESET}"
    printf "%${pad1}s${title_color}${BOLD}%s${RESET}" "" "$line1"
    printf "%$((width - pad1 - ${#line1}))s" ""
    printf "${border_color}│${RESET}\n"

    printf "${border_color}│${RESET}"
    printf "%${pad2}s${DIM}%s${RESET}" "" "$line2"
    printf "%$((width - pad2 - ${#line2}))s" ""
    printf "${border_color}│${RESET}\n"

    printf "${border_color}│${RESET}"
    printf "%${width}s" "" | tr ' ' ' '
    printf "${border_color}│${RESET}\n"

    printf "${border_color}└"; printf '─%.0s' $(seq 1 $width); printf "┘${RESET}\n"
    printf "\n"
}

# ── Detect platform ────────────────────────────────────────────
detect_platform() {
    local os arch
    os=$(uname -s | tr '[:upper:]' '[:lower:]')
    arch=$(uname -m)

    case "$os" in
        darwin) os="apple-darwin" ;;
        linux)  os="unknown-linux-gnu" ;;
        *)      err "Unsupported OS: $os"; exit 1 ;;
    esac
    case "$arch" in
        x86_64)       arch="x86_64" ;;
        arm64|aarch64) arch="aarch64" ;;
        *)            err "Unsupported architecture: $arch"; exit 1 ;;
    esac

    printf "\n"
    check "Detected: ${BOLD}${os}${RESET} (${BOLD}${arch}${RESET})"
    echo "${arch}-${os}"
}

# ── Prerequisites ──────────────────────────────────────────────
check_prereqs() {
    if ! command -v curl &>/dev/null; then
        err "curl is required"; exit 1
    fi
}

# ── Install directory ──────────────────────────────────────────
ensure_install_dir() {
    if [ ! -d "$INSTALL_DIR" ]; then
        info "Creating install directory: ${DIM}${INSTALL_DIR}${RESET}"
        mkdir -p "$INSTALL_DIR"
    fi
}

# ── Build & install ────────────────────────────────────────────
install_binary() {
    local platform="$1"

    if command -v cargo &>/dev/null; then
        local ver
        ver=$(cargo --version 2>/dev/null || echo "unknown")
        check "Rust found: ${DIM}${ver}${RESET}"
    else
        err "Rust/Cargo is required for source installation"
        printf "  Install with: ${BOLD}curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh${RESET}\n\n"
        exit 1
    fi

    info "Building release binary..."
    printf "  ${DIM}(this may take a minute)${RESET}\n\n"

    local build_dir
    if [ -f "Cargo.toml" ] && grep -q "sapphire-agent-factory" Cargo.toml 2>/dev/null; then
        info "Using current directory as source"
        build_dir="$(pwd)"
    else
        build_dir=$(mktemp -d)
        info "Cloning repository..."
        git clone --depth 1 "https://github.com/${GITHUB_USER}/${REPO}.git" "$build_dir" 2>/dev/null || {
            err "Failed to clone repository"
            exit 1
        }
    fi

    cd "$build_dir"
    cargo build --release --quiet

    if [ ! -f "target/release/${BINARY_NAME}" ]; then
        err "Build failed: binary not found"
        exit 1
    fi

    info "Installing to ${DIM}${INSTALL_DIR}/${BINARY_NAME}${RESET}"
    cp "target/release/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

    if [ "$build_dir" != "$(pwd)" ]; then
        rm -rf "$build_dir"
    fi
}

# ── Verify ─────────────────────────────────────────────────────
verify_installation() {
    local binary_path="${INSTALL_DIR}/${BINARY_NAME}"

    if [ ! -f "$binary_path" ]; then
        err "Installation failed: binary not found at ${DIM}${binary_path}${RESET}"
        exit 1
    fi

    if [ ! -x "$binary_path" ]; then
        err "Installation failed: binary is not executable"
        exit 1
    fi

    printf "\n"
    check "Installed successfully: ${BOLD}${binary_path}${RESET}"
}

# ── PATH check ─────────────────────────────────────────────────
check_path() {
    if [[ ":$PATH:" != *":${INSTALL_DIR}:"* ]]; then
        printf "\n"
        warn "${DIM}${INSTALL_DIR}${RESET} is not in your PATH"
        printf "  ${DIM}echo 'export PATH=\"${INSTALL_DIR}:\\\$PATH\"' >> ~/.zshrc${RESET}\n"
        printf "  ${DIM}source ~/.zshrc${RESET}\n"
    fi
}

# ── Quick start ────────────────────────────────────────────────
print_quickstart() {
    printf "\n"
    printf "  ${CYAN}Quick start:${RESET}\n\n"
    printf "    ${BOLD}sp qwen 2 --repo . --mission \"debug and validate the repo\"${RESET}\n"
    printf "    ${BOLD}sp status${RESET}\n"
    printf "    ${BOLD}sp --help${RESET}\n\n"
    printf "  ${DIM}Docs: https://github.com/${GITHUB_USER}/${REPO}${RESET}\n"
    printf "\n"
}

# ── Main ───────────────────────────────────────────────────────
main() {
    print_banner
    check_prereqs
    local platform
    platform=$(detect_platform)
    ensure_install_dir
    install_binary "$platform"
    verify_installation
    check_path
    print_quickstart

    check "Ready to orchestrate ✨"
    printf "\n"
}

main "$@"
