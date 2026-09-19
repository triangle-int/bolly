#!/bin/bash
# ╔══════════════════════════════════════════════╗
# ║          nolune — AI companion installer       ║
# ║        https://github.com/triangle-int/nolune  ║
# ╚══════════════════════════════════════════════╝
#
# Usage:
#   curl -fsSL https://nolune.dev/install.sh | bash
#
# Options (env vars):
#   NOLUNE_CHANNEL=nightly    Install nightly instead of stable
#   NOLUNE_DIR=/custom/path   Install to custom directory (default: ~/.nolune)
#
set -e

# ─── Colors ───────────────────────────────────────────────────────────────────
BOLD='\033[1m'
DIM='\033[2m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m'

log()  { echo -e "  ${GREEN}✓${NC} $1"; }
info() { echo -e "  ${DIM}$1${NC}"; }
warn() { echo -e "  ${YELLOW}!${NC} $1"; }
fail() { echo -e "  ${RED}✗${NC} $1"; exit 1; }
step() { echo -e "\n${CYAN}${BOLD}$1${NC}"; }

# ─── Orb animation (frames extracted from orb-onboarding.mp4) ─────────────────
animate_orb() {
    local H=16  # fixed frame height

    # Helper: pad frame to fixed height, centered vertically
    show_frame() {
        printf '\033[2J\033[H'
        local content="$1"
        local lines
        lines=$(echo "$content" | wc -l)
        local pad=$(( (H - lines) / 2 ))
        for _ in $(seq 1 $pad); do echo; done
        echo "$content"
        local bottom=$(( H - lines - pad ))
        for _ in $(seq 1 $bottom); do echo; done
    }

    show_frame '                   .......
                 ....··....
                 ....··....
                   ......'
    sleep 0.12

    show_frame '                   ......
                ............
               ......··......
             ......·:++:·......
              .....··::··.....
                .............
                 ...........
                  ........'
    sleep 0.12

    show_frame '                   ......
                  ..·**:...
                  ..·::·...
                    ....'
    sleep 0.1

    show_frame '                        ..
                     ..:+·.
                 ...··+%*:·.
                .·:+*%#%*+:.
                ·+%%%***++:.
                ..··.·::···..
                     .....'
    sleep 0.12

    show_frame '               ......··.......
             ··...··:+++::::::··..
            .:+:·:+*%##%%*******+:..
             .:+**%#%***%%*:··:++:·.
          .··..·+%%##%%%%*+:··...
          ·**·..:*++++*%*:·...
           .·····++:::+*:..
              ....·::::·...'
    sleep 0.15

    show_frame '                .·:+***++:·.
             .·:+**%%%%%***+:·.
            .:**+:++++++++++**:..
          ..:**+::::·::++***+**+:.
          .:+*++++:·····:+*%++**+·
          .:**++*+:·····:+*%*+*+:·
           ·+%%*%*+:···:+*%%***:·
           .·*%##%%*++**%%****:.
            .·+**%%%%%%%%%**+:.
              .·::+****++::·.
                 ........'
    sleep 0.15

    show_frame '                .·:++**++:·.
             .·:*%%#####%%*+::.
           .·+**********%%%*+*+·.
          .+**+**++*%******%%***:.
          :%****++*++:::+***%%%%%·
          +%+*%*++:·....·:+**%%%%+
          +%+*%**:·.....·:+*%%%%#+
          ·***%%%*+::··::++*%%%%%:
          .:****%##%%******%%%%%+.
           .·+***********%%%%%*:.
             .·+*%%%%%%%%%%*+:.
                .··:+++::··.'
    sleep 0.2

    show_frame '                    ....
                .·:+****++:.
             .·:*%########%*+:.
           .:+******+++++**+***:.
          .+%***+::::··::+++**%%:.
          :%%***+::······:++*%***·
          :%#%**+::·······:+*%%+*:          nolune
          :*#%%%*::::·····:+%%#*+:          your AI companion
          ·+*%%%**+:::::::+*%#%++·
           ·:+%%%**++::++*%%#%+:·.
            .·:+*%%*****%%%%*+:..
              ..:+**%%%%%*+:·.
                 ..·:::··..'
    sleep 1.0
    printf '\033[2J\033[H'
}

# Only animate if terminal is interactive
if [ -t 1 ]; then
    animate_orb
fi

# ─── Banner ───────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}  ┌─────────────────────────────┐${NC}"
echo -e "${BOLD}  │${NC}     ${CYAN}nolune${NC} installer        ${BOLD}│${NC}"
echo -e "${BOLD}  │${NC}     ${DIM}your AI companion${NC}       ${BOLD}│${NC}"
echo -e "${BOLD}  └─────────────────────────────┘${NC}"

# ─── Detect platform ─────────────────────────────────────────────────────────
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux)   PLATFORM="linux" ;;
    Darwin)  PLATFORM="macos" ;;
    *)       fail "unsupported OS: $OS (nolune supports Linux and macOS)" ;;
esac

case "$ARCH" in
    x86_64|amd64)   ARCH="x86_64" ;;
    aarch64|arm64)   ARCH="aarch64" ;;
    *)               fail "unsupported architecture: $ARCH" ;;
esac

case "$PLATFORM-$ARCH" in
    linux-x86_64)    TARGET="x86_64-unknown-linux-gnu" ;;
    linux-aarch64)   TARGET="aarch64-unknown-linux-gnu" ;;
    macos-aarch64)   TARGET="aarch64-apple-darwin" ;;
    macos-x86_64)    fail "Intel Macs are not supported — nolune requires Apple Silicon (M1+)" ;;
    *)               fail "unsupported platform: $PLATFORM-$ARCH" ;;
esac

# ─── Config ───────────────────────────────────────────────────────────────────
REPO="triangle-int/nolune"
CHANNEL="${NOLUNE_CHANNEL:-stable}"
NOLUNE_DIR="${NOLUNE_DIR:-$HOME/.nolune}"
BIN_DIR="$NOLUNE_DIR/bin"
BIN="$BIN_DIR/nolune"

step "detecting environment"
log "platform: ${BOLD}$PLATFORM $ARCH${NC}"
log "channel: ${BOLD}$CHANNEL${NC}"
log "install dir: ${BOLD}$NOLUNE_DIR${NC}"

# ─── Download binary ─────────────────────────────────────────────────────────
step "downloading nolune"

# github.com can be slow or intermittently unreachable on some networks;
# fail fast on a dead connection and retry transient errors instead of hanging.
CURL_NET_OPTS="--connect-timeout 15 --retry 3 --retry-delay 2 --retry-connrefused"

AUTH_HEADER=""
if [ -n "${GITHUB_TOKEN:-}" ]; then
    AUTH_HEADER="Authorization: token $GITHUB_TOKEN"
fi

ASSET_NAME="nolune-server-$TARGET"

if [ "$CHANNEL" = "nightly" ]; then
    # Nightly: must use API to get tag
    API_URL="https://api.github.com/repos/$REPO/releases/tags/nightly"
    RELEASE_JSON=$(curl -fsSL $CURL_NET_OPTS ${AUTH_HEADER:+-H "$AUTH_HEADER"} "$API_URL" 2>/dev/null) || fail "could not fetch release info (try setting GITHUB_TOKEN if rate limited)"
    TAG=$(echo "$RELEASE_JSON" | grep '"tag_name"' | head -1 | sed 's/.*: "//;s/".*//')
    if [ -z "$TAG" ] || [ "$TAG" = "null" ]; then
        fail "could not find a nightly release"
    fi
    DOWNLOAD_URL="https://github.com/$REPO/releases/download/$TAG/$ASSET_NAME"
else
    # Stable: use redirect URL — no API call, no rate limit
    DOWNLOAD_URL="https://github.com/$REPO/releases/latest/download/$ASSET_NAME"
    TAG="latest"
fi

mkdir -p "$BIN_DIR" "$NOLUNE_DIR"

info "downloading ${BOLD}$CHANNEL${NC} for $TARGET..."
curl -fL --progress-bar $CURL_NET_OPTS "$DOWNLOAD_URL" -o "$BIN" || \
    fail "download failed — could not fetch $DOWNLOAD_URL (check that github.com is reachable, then see https://github.com/$REPO/releases)"

# Resolve actual version from downloaded binary or GitHub redirect
if [ "$TAG" = "latest" ]; then
    # HEAD -L follows two redirects: /releases/download/<tag>/... and then the
    # signed CDN URL. Only the first carries the tag.
    RESOLVED=$(curl -fsSIL $CURL_NET_OPTS "$DOWNLOAD_URL" 2>/dev/null | grep -i '^location:' | grep '/releases/download/' | head -1 | sed 's|.*/releases/download/\([^/]*\)/.*|\1|' | tr -d '\r')
    TAG="${RESOLVED:-latest}"
fi

chmod +x "$BIN"
echo "$TAG" > "$BIN_DIR/.version"

log "downloaded ${BOLD}$TAG${NC}"

# ─── Config file ──────────────────────────────────────────────────────────────
if [ ! -f "$NOLUNE_DIR/config.toml" ]; then
    step "creating config"
    # Generate a secure random auth token (32 chars, a-z0-9)
    AUTH_TOKEN=$(head -c 256 /dev/urandom | LC_ALL=C tr -dc 'a-z0-9' | head -c 32)
    cat > "$NOLUNE_DIR/config.toml" <<CONF
host = "0.0.0.0"
port = 26559
auth_token = "$AUTH_TOKEN"

[llm]
model_mode = "auto"

[llm.tokens]
ANTHROPIC = ""       # Required — get key at https://console.anthropic.com
ELEVENLABS = ""      # Optional — text-to-speech
CONF
    log "created $NOLUNE_DIR/config.toml"
    info "authentication token saved in $NOLUNE_DIR/config.toml"
else
    log "config already exists, skipping"
    # Backfill auth_token if empty (upgrade from older install)
    EXISTING_TOKEN=$(grep -E '^auth_token\s*=' "$NOLUNE_DIR/config.toml" | head -1 | sed 's/[^=]*=\s*//' | tr -d ' "')
    if [ -z "$EXISTING_TOKEN" ]; then
        AUTH_TOKEN=$(head -c 256 /dev/urandom | LC_ALL=C tr -dc 'a-z0-9' | head -c 32)
        sed -i.bak "s/^auth_token\s*=.*/auth_token = \"$AUTH_TOKEN\"/" "$NOLUNE_DIR/config.toml"
        rm -f "$NOLUNE_DIR/config.toml.bak"
        log "generated auth token for existing install"
    fi
fi

# ─── Update script ────────────────────────────────────────────────────────────
cat > "$BIN_DIR/update" <<UPDATESCRIPT
#!/bin/bash
set -e
REPO="$REPO"
BIN="$BIN"
CHANNEL="\${NOLUNE_CHANNEL:-$CHANNEL}"
TARGET="$TARGET"
CURL_NET_OPTS="$CURL_NET_OPTS"
# Use redirect URL for stable — no API call, no rate limit
DOWNLOAD_URL="https://github.com/\$REPO/releases/latest/download/nolune-server-\$TARGET"
if [ "\$CHANNEL" = "nightly" ]; then
    API_URL="https://api.github.com/repos/\$REPO/releases/tags/nightly"
    RELEASE_JSON=\$(curl -fsSL \$CURL_NET_OPTS "\$API_URL") || { echo "could not fetch release info"; exit 1; }
    TAG=\$(echo "\$RELEASE_JSON" | grep '"tag_name"' | head -1 | sed 's/.*: "//;s/".*//')
    DOWNLOAD_URL="https://github.com/\$REPO/releases/download/\$TAG/nolune-server-\$TARGET"
fi
echo "checking for updates..."
curl -fsSL \$CURL_NET_OPTS "\$DOWNLOAD_URL" -o "\$BIN.tmp" 2>/dev/null || \
    { echo "download failed — could not fetch \$DOWNLOAD_URL"; exit 1; }
# Resolve version from the /releases/download/<tag>/ redirect hop (the final hop is the CDN URL)
TAG=\$(curl -fsSIL \$CURL_NET_OPTS "\$DOWNLOAD_URL" 2>/dev/null | grep -i '^location:' | grep '/releases/download/' | head -1 | sed 's|.*/releases/download/\([^/]*\)/.*|\1|' | tr -d '\r')
TAG="\${TAG:-unknown}"
CURRENT=\$(cat "$BIN_DIR/.version" 2>/dev/null || echo "none")
if [ "\$TAG" = "\$CURRENT" ]; then
    rm -f "\$BIN.tmp"
    echo "already at \$TAG"
    exit 0
fi
chmod +x "\$BIN.tmp"
mv "\$BIN.tmp" "\$BIN"
echo "\$TAG" > "$BIN_DIR/.version"
echo "updated to \$TAG — restart nolune to apply"
UPDATESCRIPT
chmod +x "$BIN_DIR/update"

# ─── Platform-specific service ────────────────────────────────────────────────
step "setting up service"
SERVICE_KIND="none"

if [ "$PLATFORM" = "linux" ]; then
    # ── systemd ──
    if command -v systemctl &>/dev/null && [ "$(id -u)" -eq 0 ]; then
        SYSTEMD_SYSTEM_DIR="${NOLUNE_SYSTEMD_SYSTEM_DIR:-/etc/systemd/system}"
        mkdir -p "$SYSTEMD_SYSTEM_DIR"
        SERVICE_FILE="$SYSTEMD_SYSTEM_DIR/nolune.service"
        cat > "$SERVICE_FILE" <<EOF
[Unit]
Description=Nolune AI Companion
After=network.target

[Service]
Type=simple
User=$(whoami)
WorkingDirectory=$NOLUNE_DIR
Environment=NOLUNE_HOME=$NOLUNE_DIR
Environment=RUST_LOG=info
ExecStart=$BIN
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF
        systemctl daemon-reload
        SERVICE_KIND="systemd-system"
        log "systemd service created"
        info "start:   sudo systemctl start nolune"
        info "logs:    sudo journalctl -u nolune -f"
    elif command -v systemctl &>/dev/null; then
        # User-level systemd (no root)
        SYSTEMD_DIR="$HOME/.config/systemd/user"
        mkdir -p "$SYSTEMD_DIR"
        cat > "$SYSTEMD_DIR/nolune.service" <<EOF
[Unit]
Description=Nolune AI Companion
After=network.target

[Service]
Type=simple
WorkingDirectory=$NOLUNE_DIR
Environment=NOLUNE_HOME=$NOLUNE_DIR
Environment=RUST_LOG=info
ExecStart=$BIN
Restart=always
RestartSec=3

[Install]
WantedBy=default.target
EOF
        systemctl --user daemon-reload
        SERVICE_KIND="systemd-user"
        log "user systemd service created"
        info "start:   systemctl --user start nolune"
        info "logs:    journalctl --user -u nolune -f"
    else
        log "no systemd found — run manually: $BIN"
    fi

elif [ "$PLATFORM" = "macos" ]; then
    # ── launchd ──
    PLIST_DIR="$HOME/Library/LaunchAgents"
    PLIST="$PLIST_DIR/dev.nolune.nolune.plist"
    mkdir -p "$PLIST_DIR"
    cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>dev.nolune.nolune</string>
    <key>ProgramArguments</key>
    <array>
        <string>$BIN</string>
    </array>
    <key>WorkingDirectory</key>
    <string>$NOLUNE_DIR</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>NOLUNE_HOME</key>
        <string>$NOLUNE_DIR</string>
        <key>RUST_LOG</key>
        <string>info</string>
    </dict>
    <key>KeepAlive</key>
    <true/>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardOutPath</key>
    <string>$NOLUNE_DIR/nolune.log</string>
    <key>StandardErrorPath</key>
    <string>$NOLUNE_DIR/nolune.log</string>
</dict>
</plist>
EOF
    SERVICE_KIND="launchd"
    log "launchd service created"
    info "start:   launchctl bootstrap gui/$(id -u) $PLIST"
    info "stop:    launchctl bootout gui/$(id -u)/dev.nolune.nolune"
    info "logs:    tail -f $NOLUNE_DIR/nolune.log"
fi

# ─── Add to PATH ──────────────────────────────────────────────────────────────
SHELL_NAME=$(basename "$SHELL" 2>/dev/null || echo "bash")
RC_FILE="$HOME/.${SHELL_NAME}rc"

if ! echo "$PATH" | grep -q "$BIN_DIR"; then
    EXPORT_LINE="export PATH=\"$BIN_DIR:\$PATH\""
    if [ -f "$RC_FILE" ] && ! grep -q "$BIN_DIR" "$RC_FILE"; then
        {
            echo ""
            echo "# nolune"
            echo "$EXPORT_LINE"
        } >> "$RC_FILE"
        log "added to PATH in $RC_FILE"
    fi
fi

# ─── Doctor: stop old instance, fix config, restart ──────────────────────────
step "starting nolune"

export PATH="$BIN_DIR:$PATH"

# Read port from config (default 26559)
NOLUNE_PORT=26559
if [ -f "$NOLUNE_DIR/config.toml" ]; then
    CFG_PORT=$(grep -E '^port\s*=' "$NOLUNE_DIR/config.toml" | head -1 | sed 's/.*=\s*//' | tr -d ' ')
    if [ -n "$CFG_PORT" ]; then
        NOLUNE_PORT="$CFG_PORT"
    fi
fi
NOLUNE_URL="http://localhost:$NOLUNE_PORT"

find_exact_nolune_pids() {
    ps -axo pid=,command= | while read -r pid command; do
        if [ "$command" = "$BIN" ]; then
            printf '%s\n' "$pid"
        fi
    done
}

stop_stray_nolune() {
    local pids remaining pid
    pids=$(find_exact_nolune_pids)
    [ -z "$pids" ] && return 0

    for pid in $pids; do
        info "stopping unmanaged nolune process (PID: $pid)..."
        kill "$pid" 2>/dev/null || true
    done

    for _ in $(seq 1 20); do
        remaining=$(find_exact_nolune_pids)
        [ -z "$remaining" ] && return 0
        sleep 0.1
    done

    fail "could not stop the existing Nolune process (PID: $remaining)"
}

# Start through the native service manager so the installed service is the
# process we verify. Fall back to a direct process only without a service manager.
case "$SERVICE_KIND" in
    launchd)
        LAUNCH_DOMAIN="gui/$(id -u)"
        LAUNCH_SERVICE="$LAUNCH_DOMAIN/dev.nolune.nolune"
        launchctl bootout "$LAUNCH_SERVICE" >/dev/null 2>&1 || true
        stop_stray_nolune
        launchctl bootstrap "$LAUNCH_DOMAIN" "$PLIST"
        launchctl kickstart -k "$LAUNCH_SERVICE"
        launchctl print "$LAUNCH_SERVICE" >/dev/null
        ;;
    systemd-system)
        systemctl stop nolune >/dev/null 2>&1 || true
        stop_stray_nolune
        systemctl enable nolune >/dev/null
        systemctl restart nolune
        systemctl is-active --quiet nolune
        ;;
    systemd-user)
        systemctl --user stop nolune >/dev/null 2>&1 || true
        stop_stray_nolune
        systemctl --user enable nolune >/dev/null
        systemctl --user restart nolune
        systemctl --user is-active --quiet nolune
        ;;
    none)
        stop_stray_nolune
        "$BIN" &>/dev/null &
        ;;
esac

info "waiting for nolune to start..."
for _ in $(seq 1 30); do
    if curl -sf "$NOLUNE_URL/healthz" >/dev/null 2>&1; then
        break
    fi
    sleep 0.5
done

if curl -sf "$NOLUNE_URL/healthz" >/dev/null 2>&1; then
    log "nolune is running on port $NOLUNE_PORT"

    # Open the plain URL. Authentication is entered explicitly in the client;
    # long-lived credentials must never be placed in browser navigation.
    if [ "$PLATFORM" = "macos" ]; then
        open "$NOLUNE_URL" 2>/dev/null
    elif command -v xdg-open &>/dev/null; then
        xdg-open "$NOLUNE_URL" 2>/dev/null
    fi

    echo ""
    echo -e "${BOLD}  ┌─────────────────────────────┐${NC}"
    echo -e "${BOLD}  │${NC}  ${GREEN}nolune is ready!${NC}           ${BOLD}│${NC}"
    echo -e "${BOLD}  └─────────────────────────────┘${NC}"
    echo ""
    echo -e "  ${CYAN}${NOLUNE_URL}${NC}"
    echo ""
else
    fail "nolune service did not become healthy — check $NOLUNE_DIR/nolune.log"
fi
