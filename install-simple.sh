#!/bin/bash
# Ultra-minimal sp installer — visually clean version
# curl -fsSL https://raw.githubusercontent.com/duggal1/sapphire-harness/master/install-simple.sh | bash

set -e

# ── Visual helpers ─────────────────────────────────────────────
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    BOLD=$'\033[1m'
    DIM=$'\033[2m'
    RESET=$'\033[0m'
    FG=$'\033[38;2;244;241;255m'
    MUTED=$'\033[38;2;156;149;179m'
    BORDER=$'\033[38;2;86;79;111m'
    PURPLE=$'\033[38;2;191;104;255m'
    PURPLE_BRIGHT=$'\033[38;2;234;213;255m'
    GREEN=$'\033[38;2;122;230;156m'
    YELLOW=$'\033[38;2;244;189;102m'
    RED=$'\033[38;2;255;136;170m'
else
    BOLD=""
    DIM=""
    RESET=""
    FG=""
    MUTED=""
    BORDER=""
    PURPLE=""
    PURPLE_BRIGHT=""
    GREEN=""
    YELLOW=""
    RED=""
fi

BOX_WIDTH=64

check() { printf " ${GREEN}✓${RESET} %b\n" "$1"; }
info() { printf " ${PURPLE}›${RESET} ${MUTED}%b${RESET}\n" "$1"; }
warn() { printf " ${YELLOW}!${RESET} %b\n" "$1"; }
err() { printf " ${RED}✕${RESET} %b\n" "$1" >&2; }

repeat_char() {
    local char="$1"
    local count="$2"
    local out=""

    while [ "$count" -gt 0 ]; do
        out="${out}${char}"
        count=$((count - 1))
    done

    printf "%s" "$out"
}

fit_text() {
    local text="$1"
    local width="$2"

    if [ "${#text}" -le "$width" ]; then
        printf "%s" "$text"
    else
        printf "%s" "${text:0:$((width - 3))}..."
    fi
}

print_box_border() {
    printf "${BORDER}┌"
    repeat_char "─" $((BOX_WIDTH + 2))
    printf "┐${RESET}\n"
}

print_box_footer() {
    printf "${BORDER}└"
    repeat_char "─" $((BOX_WIDTH + 2))
    printf "┘${RESET}\n"
}

print_box_line() {
    local text
    text=$(fit_text "$1" "$BOX_WIDTH")
    printf "${BORDER}│ ${FG}%-*s${RESET} ${BORDER}│${RESET}\n" "$BOX_WIDTH" "$text"
}

print_centered_box_line() {
    local text="$1"
    local style_prefix="$2"
    local text_width=${#text}
    local left_pad=0
    local right_pad=0

    if [ "$text_width" -lt "$BOX_WIDTH" ]; then
        left_pad=$(((BOX_WIDTH - text_width) / 2))
        right_pad=$((BOX_WIDTH - text_width - left_pad))
    fi

    printf "${BORDER}│ %*s%b%s%b%*s ${BORDER}│${RESET}\n" \
        "$left_pad" "" \
        "$style_prefix" "$text" "$RESET" \
        "$right_pad" ""
}

print_empty_box_line() {
    printf "${BORDER}│ %-*s │${RESET}\n" "$BOX_WIDTH" ""
}

# ── Banner ─────────────────────────────────────────────────────
print_banner() {
    printf "\n"
    print_box_border
    print_empty_box_line
    print_centered_box_line "Sapphire Agent Factory" "${PURPLE_BRIGHT}${BOLD}"
    print_centered_box_line "Terminal-first multi-agent orchestration CLI" "${MUTED}"
    print_empty_box_line
    print_box_footer
    printf "\n"
}

print_capabilities() {
    print_box_border
    printf "${BORDER}│ ${PURPLE_BRIGHT}${BOLD}%-*s${RESET} ${BORDER}│${RESET}\n" "$BOX_WIDTH" "Capabilities"
    print_empty_box_line
    print_box_line "• Plans work before launch"
    print_box_line "• Runs supervisor and worker sessions"
    print_box_line "• Persists mail, replay, and mission state"
    print_box_line "• Keeps validation and watchdog flow intact"
    print_box_footer
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

    printf "\n" >&2
    check "Detected: ${BOLD}${os}${RESET} (${BOLD}${arch}${RESET})" >&2
    printf "%s" "${arch}-${os}"
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
print_capabilities

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

    print_box_border
    printf "${BORDER}│ ${PURPLE_BRIGHT}${BOLD}%-*s${RESET} ${BORDER}│${RESET}\n" "$BOX_WIDTH" "Quick start"
    print_empty_box_line
    print_box_line "sp qwen 2 --repo . --mission \"debug and validate the repo\""
    print_box_line "sp status"
    print_box_line "sp --help"
    print_box_footer
    printf "\n"
else
    err "Installation failed"
    exit 1
fi
