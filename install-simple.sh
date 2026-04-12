#!/bin/bash
# Ultra-minimal sp installer — visually clean version
# curl -fsSL https://raw.githubusercontent.com/duggal1/sapphire-harness/master/install-simple.sh | bash

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

# ── Banner ─────────────────────────────────────────────────────
print_banner() {
    local border_color="${PURPLE}"
    local title_color="${CYAN}"
    local width=60

    printf "\n"
    printf "${border_color}┌"; printf '─%.0s' $(seq 1 $width); printf "┐${RESET}\n"
    printf "${border_color}│${RESET}"
    printf "%${width}s" "" | tr ' ' ' '
    printf "${border_color}│${RESET}\n"

    # Centered title lines
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

    if command -v cargo &>/dev/null; then
        local ver
        ver=$(cargo --version 2>/dev/null || echo "unknown")
        check "Rust found: ${DIM}${ver}${RESET}"
    else
        info "Checking for Rust..."
        warn "Rust/Cargo is required to build from source"
        printf "  Install with: ${BOLD}curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh${RESET}\n\n"
        exit 1
    fi
}

# ── Main install ───────────────────────────────────────────────
INSTALL_DIR="${HOME}/.local/bin"
BINARY="$INSTALL_DIR/sp"

print_banner

check_prereqs
PLATFORM=$(detect_platform)

mkdir -p "$INSTALL_DIR"

if [ -f "Cargo.toml" ] && grep -q "sapphire-agent-factory" Cargo.toml 2>/dev/null; then
    info "Building from current directory..."
    cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" 2>/dev/null || true
    cargo build --release --quiet
    cp target/release/sp "$BINARY"
else
    info "Cloning repository..."
    BUILD_DIR=$(mktemp -d)
    git clone --depth 1 "https://github.com/duggal1/sapphire-harness.git" "$BUILD_DIR" 2>/dev/null
    cd "$BUILD_DIR"
    cargo build --release --quiet
    cp target/release/sp "$BINARY"
    rm -rf "$BUILD_DIR"
fi

chmod +x "$BINARY"

# ── Verification ───────────────────────────────────────────────
if [ -f "$BINARY" ] && [ -x "$BINARY" ]; then
    printf "\n"
    check "Installed to ${BOLD}${BINARY}${RESET}"
    printf "\n"

    # PATH check
    if [[ ":$PATH:" != *":${INSTALL_DIR}:"* ]]; then
        warn "${DIM}${INSTALL_DIR}${RESET} is not in your PATH"
        printf "  ${DIM}echo 'export PATH=\"${INSTALL_DIR}:\\\$PATH\"' >> ~/.zshrc${RESET}\n"
        printf "  ${DIM}source ~/.zshrc${RESET}\n\n"
    fi

    printf "  ${CYAN}Quick start:${RESET}\n\n"
    printf "    ${BOLD}sp qwen 2 --repo . --mission \"debug and validate the repo\"${RESET}\n"
    printf "    ${BOLD}sp status${RESET}\n"
    printf "    ${BOLD}sp --help${RESET}\n\n"
else
    err "Installation failed"
    exit 1
fi
