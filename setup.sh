#!/bin/bash
set -e
umask 077
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
cd "$ROOT_DIR"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# ── Colors ───────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${CYAN}[*]${NC} $1"; }
ok()    { echo -e "${GREEN}[+]${NC} $1"; }
warn()  { echo -e "${YELLOW}[!]${NC} $1"; }
fail()  { echo -e "${RED}[-]${NC} $1" >&2; exit 1; }

# ── Password input ──────────────────────────────────────────────────
if [ $# -ge 2 ]; then
    ROUTER_PASSWORD="$1"
    AGENT_PASSWORD="$2"
elif [ $# -eq 0 ]; then
    echo -e "${CYAN}Router admin password:${NC} "
    read -rs ROUTER_PASSWORD; echo
    echo -e "${CYAN}Agent API password:${NC} "
    read -rs AGENT_PASSWORD; echo
    [ -z "$ROUTER_PASSWORD" ] && fail "Router password cannot be empty."
    [ -z "$AGENT_PASSWORD" ] && fail "Agent password cannot be empty."
else
    echo "Usage: ./setup.sh [router-password agent-password]"
    echo "       ./setup.sh    (interactive — prompts for passwords)"
    exit 1
fi

GATEWAY="${ZTE_GATEWAY:-192.168.0.1}"
AGENT_PORT=9090
SSH_PORT=2222
TARGET=aarch64-unknown-linux-musl
BINARY=target/$TARGET/release/zte-agent
ESIM_HELPER=target/$TARGET/release/zte-esim
REMOTE_BIN=/data/zte-agent
STARTUP_SCRIPT=/data/local/tmp/start_zte_agent.sh
BINARY_CHANGED=false
DOWNLOAD_URL="https://github.com/dklasens/MU5250-OpenUI/releases/latest/download/zte-agent"

# ── Binary source menu ──────────────────────────────────────────────
# Both options install this repo's agent. Option 1 downloads the pre-built
# binary from this repo's GitHub releases (no dev tools needed); option 2
# builds from source and stays the default so the binary is always auditable
# and installable even when the release download is unreachable.
echo ""
echo -e "${CYAN}How would you like to get the zte-agent binary?${NC}"
echo "  1) Download pre-built from this repo's GitHub releases (no dev tools"
echo "     needed)"
echo "  2) Build from source (recommended — auditable; requires Rust toolchain)"
echo ""
echo -n "Choice [2]: "
read -r BUILD_CHOICE
BUILD_CHOICE="${BUILD_CHOICE:-2}"

if [ "$BUILD_CHOICE" != "1" ] && [ "$BUILD_CHOICE" != "2" ]; then
    fail "Invalid choice. Enter 1 or 2."
fi

# ── Step 0: Prerequisites ───────────────────────────────────────────
info "Checking prerequisites..."

OS="$(uname -s)"
HAS_BREW=false
HAS_APT=false
command -v brew >/dev/null 2>&1 && HAS_BREW=true
command -v apt-get >/dev/null 2>&1 && HAS_APT=true

# Cross-compiler tool varies by platform
if [ "$OS" = "Darwin" ]; then
    CROSS_CC="aarch64-linux-musl-gcc"
else
    CROSS_CC="aarch64-linux-gnu-gcc"
fi

# Dependency table: cmd | brew_pkg | apt_pkg | custom_install
# Build path needs all deps; download path only needs curl, python3, adb
if [ "$BUILD_CHOICE" = "1" ]; then
    DEPS=(
        "curl|curl|curl|"
        "python3|python3|python3|"
        "adb|android-platform-tools|android-tools-adb|"
        "openssl|openssl|openssl|"
    )
else
    DEPS=(
        "curl|curl|curl|"
        "python3|python3|python3|"
        "adb|android-platform-tools|android-tools-adb|"
        "openssl|openssl|openssl|"
        "cargo|||curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
        "$CROSS_CC|filosottile/musl-cross/musl-cross|gcc-aarch64-linux-gnu musl-tools|"
    )
fi

MISSING_CMDS=()
MISSING_INSTALLS=()
ALL_OK=true

# Returns 0 if every missing tool has a resolved install command
all_have_installers() {
    for i in "${!MISSING_CMDS[@]}"; do
        [ -z "${MISSING_INSTALLS[$i]}" ] && return 1
    done
    return 0
}

for entry in "${DEPS[@]}"; do
    IFS='|' read -r cmd brew_pkg apt_pkg custom <<< "$entry"
    if command -v "$cmd" >/dev/null 2>&1; then
        echo -e "  ${GREEN}✓${NC} $cmd"
    else
        echo -e "  ${RED}✗${NC} $cmd"
        ALL_OK=false
        MISSING_CMDS+=("$cmd")

        # Determine install command
        install_cmd=""
        if [ -n "$custom" ]; then
            install_cmd="$custom"
        elif [ "$OS" = "Darwin" ] && [ "$HAS_BREW" = true ] && [ -n "$brew_pkg" ]; then
            install_cmd="brew install $brew_pkg"
        elif [ "$OS" != "Darwin" ] && [ "$HAS_APT" = true ] && [ -n "$apt_pkg" ]; then
            install_cmd="sudo apt-get install -y $apt_pkg"
        fi
        MISSING_INSTALLS+=("$install_cmd")
    fi
done

# For download path, ADB is only needed if SSH isn't set up — defer check
# (we'll check SSH reachability later and skip ADB requirement if SSH works)

if [ "$ALL_OK" = false ]; then
    # On macOS without brew, offer to install it first
    if [ "$OS" = "Darwin" ] && [ "$HAS_BREW" = false ]; then
        echo ""
        warn "Homebrew not found. It's needed to install dependencies on macOS."
        echo -e "${CYAN}Install Homebrew now? (y/N)${NC}"
        read -r INSTALL_BREW
        if [ "$INSTALL_BREW" = "y" ] || [ "$INSTALL_BREW" = "Y" ]; then
            /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
            # Source brew shellenv for current session
            if [ -x /opt/homebrew/bin/brew ]; then
                eval "$(/opt/homebrew/bin/brew shellenv)"
            elif [ -x /usr/local/bin/brew ]; then
                eval "$(/usr/local/bin/brew shellenv)"
            fi
            HAS_BREW=true
            # Re-evaluate install commands now that brew is available
            for i in "${!MISSING_CMDS[@]}"; do
                if [ -z "${MISSING_INSTALLS[$i]}" ]; then
                    for entry in "${DEPS[@]}"; do
                        IFS='|' read -r cmd brew_pkg _ _ <<< "$entry"
                        if [ "$cmd" = "${MISSING_CMDS[$i]}" ] && [ -n "$brew_pkg" ]; then
                            MISSING_INSTALLS[$i]="brew install $brew_pkg"
                            break
                        fi
                    done
                fi
            done
        else
            fail "Homebrew is required on macOS. Install it from https://brew.sh"
        fi
    fi

    if ! all_have_installers; then
        echo ""
        echo "Cannot auto-install all dependencies. Please install manually:"
        for i in "${!MISSING_CMDS[@]}"; do
            if [ -z "${MISSING_INSTALLS[$i]}" ]; then
                echo "  ${MISSING_CMDS[$i]} — no auto-install available for this platform"
            fi
        done
        exit 1
    fi

    echo ""
    echo "The following will be installed:"
    for i in "${!MISSING_CMDS[@]}"; do
        echo "  ${MISSING_CMDS[$i]}  →  ${MISSING_INSTALLS[$i]}"
    done
    echo ""
    echo -e "${CYAN}Install all missing dependencies now? (y/N)${NC}"
    read -r DO_INSTALL
    if [ "$DO_INSTALL" != "y" ] && [ "$DO_INSTALL" != "Y" ]; then
        fail "Missing dependencies: ${MISSING_CMDS[*]}. Install them and re-run."
    fi

    for i in "${!MISSING_CMDS[@]}"; do
        info "Installing ${MISSING_CMDS[$i]}..."
        if ! eval "${MISSING_INSTALLS[$i]}"; then
            fail "Failed to install ${MISSING_CMDS[$i]}."
        fi
        # Source cargo env if we just installed rustup
        if [ "${MISSING_CMDS[$i]}" = "cargo" ] && [ -f "$HOME/.cargo/env" ]; then
            # shellcheck disable=SC1091
            source "$HOME/.cargo/env"
        fi
        if command -v "${MISSING_CMDS[$i]}" >/dev/null 2>&1; then
            ok "${MISSING_CMDS[$i]} installed."
        else
            fail "Failed to install ${MISSING_CMDS[$i]}."
        fi
    done
fi

# Ensure Rust cross-compilation target is installed (build path only)
if [ "$BUILD_CHOICE" = "2" ]; then
    if ! rustup target list --installed 2>/dev/null | grep -q aarch64-unknown-linux-musl; then
        info "Adding Rust cross-compilation target..."
        rustup target add aarch64-unknown-linux-musl
    fi
fi
ok "All prerequisites found."

# Validate credentials before any unlock or deployment write.
ZTE_AGENT_PASSWORD="$AGENT_PASSWORD" python3 -c 'import importlib.util,os; s=importlib.util.spec_from_file_location("startup", "scripts/render-agent-startup.py"); m=importlib.util.module_from_spec(s); s.loader.exec_module(m); m.render(os.environ["ZTE_AGENT_PASSWORD"],os.environ.get("ZTE_AGENT_PIN", ""))'
PREPARE=(python3 scripts/deploy-components.py --prepare-only --bundle-output "$WORK/packages")
if [ -n "${ZTE_BUNDLE_PATH:-}" ]; then PREPARE+=(--bundle "$ZTE_BUNDLE_PATH"); fi
if [ "$BUILD_CHOICE" = 1 ]; then
    PREPARE+=(--release-agent "$BINARY" --release-esim-helper "$ESIM_HELPER")
else
    cargo build --locked --release --target "$TARGET" -p zte-agent -p zte-esim
fi
"${PREPARE[@]}"

# Probe transports without accepting an arbitrary TCP listener as a modem.
# deploy-components verifies model, firmware, architecture, uid and fingerprint.
EXTRA=()
SSH=(ssh -p "$SSH_PORT" -o BatchMode=yes -o StrictHostKeyChecking=accept-new
     -o "UserKnownHostsFile=$HOME/.ssh/known_hosts.d/zte" -o ConnectTimeout=5 "root@$GATEWAY")
mkdir -p "$HOME/.ssh/known_hosts.d"
if "${SSH[@]}" true >/dev/null 2>&1; then
    ok "SSH is reachable. Device identity will be verified before writes."
elif adb devices 2>/dev/null | grep -q 'device$'; then
    EXTRA+=(--auto-adb)
else
    SUFFIX="${ZTE_BACKUP_SUFFIX:-}"
    if [ -z "$SUFFIX" ]; then read -rsp 'Backup-key suffix: ' SUFFIX; echo; fi
    [ -n "$SUFFIX" ] || fail 'Backup-key suffix cannot be empty.'
    ROUTER_PW="$ROUTER_PASSWORD" ZTE_BACKUP_SUFFIX="$SUFFIX" \
        python3 scripts/zunlock.py --gw "$GATEWAY" --dry-run --keep-work "$WORK/unlock"
    if [ "${ZTE_DRY_RUN:-0}" = 1 ]; then
        ok 'Unlock dry run completed. Device deployment/storage checks require ADB or SSH and have not run.'
        exit 0
    fi
    ROUTER_PW="$ROUTER_PASSWORD" ZTE_BACKUP_SUFFIX="$SUFFIX" \
        python3 scripts/zunlock.py --gw "$GATEWAY" --keep-work "$WORK/unlock"
    info 'Waiting up to three minutes for ADB…'
    python3 -c 'import subprocess; subprocess.run(["adb", "wait-for-device"], timeout=180, check=True)'
    EXTRA+=(--auto-adb --identity-file "$WORK/unlock/identity.json")
fi
if [ "${ZTE_DRY_RUN:-0}" = 1 ]; then EXTRA+=(--dry-run); fi
ZTE_AGENT_PASSWORD="$AGENT_PASSWORD" python3 scripts/deploy-components.py \
    --agent "$BINARY" --esim-helper "$ESIM_HELPER" --harden --bundle "$WORK/packages" --gateway "$GATEWAY" "${EXTRA[@]}"
ok 'Agent and key-only SSH verified. Run bash deploy-dashboard.sh to install the dashboard.'
