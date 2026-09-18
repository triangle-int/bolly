#!/bin/bash
# ╔══════════════════════════════════════════════╗
# ║          bolly — AI companion installer       ║
# ║        https://github.com/triangle-int/bolly  ║
# ╚══════════════════════════════════════════════╝
#
# Usage:
#   curl -fsSL https://bollyai.dev/install.sh | bash
#
# Options (env vars):
#   BOLLY_CHANNEL=nightly    Install nightly instead of stable
#   BOLLY_DIR=/custom/path   Install to custom directory (default: ~/.bolly)
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
          :%#%**+::·······:+*%%+*:          bolly
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
echo -e "${BOLD}  │${NC}     ${CYAN}bolly${NC} installer         ${BOLD}│${NC}"
echo -e "${BOLD}  │${NC}     ${DIM}your AI companion${NC}       ${BOLD}│${NC}"
echo -e "${BOLD}  └─────────────────────────────┘${NC}"

# ─── Detect platform ─────────────────────────────────────────────────────────
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux)   PLATFORM="linux" ;;
    Darwin)  PLATFORM="macos" ;;
    *)       fail "unsupported OS: $OS (bolly supports Linux and macOS)" ;;
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
    macos-x86_64)    fail "Intel Macs are not supported — bolly requires Apple Silicon (M1+)" ;;
    *)               fail "unsupported platform: $PLATFORM-$ARCH" ;;
esac

# ─── Config ───────────────────────────────────────────────────────────────────
REPO="triangle-int/bolly"
CHANNEL="${BOLLY_CHANNEL:-stable}"
BOLLY_DIR="${BOLLY_DIR:-$HOME/.bolly}"
BIN_DIR="$BOLLY_DIR/bin"
BIN="$BIN_DIR/bolly"

step "detecting environment"
log "platform: ${BOLD}$PLATFORM $ARCH${NC}"
log "channel: ${BOLD}$CHANNEL${NC}"
log "install dir: ${BOLD}$BOLLY_DIR${NC}"

# ─── Download binary ─────────────────────────────────────────────────────────
step "downloading bolly"

AUTH_HEADER=""
if [ -n "${GITHUB_TOKEN:-}" ]; then
    AUTH_HEADER="Authorization: token $GITHUB_TOKEN"
fi

ASSET_NAME="bolly-server-$TARGET"
ASSET_NAME_LEGACY="bolly-$TARGET"

if [ "$CHANNEL" = "nightly" ]; then
    # Nightly: must use API to get tag
    API_URL="https://api.github.com/repos/$REPO/releases/tags/nightly"
    RELEASE_JSON=$(curl -fsSL ${AUTH_HEADER:+-H "$AUTH_HEADER"} "$API_URL" 2>/dev/null) || fail "could not fetch release info (try setting GITHUB_TOKEN if rate limited)"
    TAG=$(echo "$RELEASE_JSON" | grep '"tag_name"' | head -1 | sed 's/.*: "//;s/".*//')
    if [ -z "$TAG" ] || [ "$TAG" = "null" ]; then
        fail "could not find a nightly release"
    fi
    DOWNLOAD_URL="https://github.com/$REPO/releases/download/$TAG/$ASSET_NAME"
    DOWNLOAD_URL_LEGACY="https://github.com/$REPO/releases/download/$TAG/$ASSET_NAME_LEGACY"
else
    # Stable: use redirect URL — no API call, no rate limit
    DOWNLOAD_URL="https://github.com/$REPO/releases/latest/download/$ASSET_NAME"
    DOWNLOAD_URL_LEGACY="https://github.com/$REPO/releases/latest/download/$ASSET_NAME_LEGACY"
    TAG="latest"
fi

mkdir -p "$BIN_DIR" "$BOLLY_DIR"

info "downloading ${BOLD}$CHANNEL${NC} for $TARGET..."
# Try new asset name first, fall back to legacy for older releases
if ! curl -fsSL --head "$DOWNLOAD_URL" >/dev/null 2>&1; then
    DOWNLOAD_URL="$DOWNLOAD_URL_LEGACY"
fi
curl -fL --progress-bar "$DOWNLOAD_URL" -o "$BIN" || \
    fail "download failed — check https://github.com/$REPO/releases"

# Resolve actual version from downloaded binary or GitHub redirect
if [ "$TAG" = "latest" ]; then
    RESOLVED=$(curl -fsSIL "$DOWNLOAD_URL" 2>/dev/null | grep -i '^location:' | tail -1 | sed 's|.*/download/\([^/]*\)/.*|\1|' | tr -d '\r')
    TAG="${RESOLVED:-latest}"
fi

chmod +x "$BIN"
echo "$TAG" > "$BIN_DIR/.version"

log "downloaded ${BOLD}$TAG${NC}"

# ─── Config file ──────────────────────────────────────────────────────────────
if [ ! -f "$BOLLY_DIR/config.toml" ]; then
    step "creating config"
    # Generate a secure random auth token (32 chars, a-z0-9)
    AUTH_TOKEN=$(head -c 256 /dev/urandom | LC_ALL=C tr -dc 'a-z0-9' | head -c 32)
    cat > "$BOLLY_DIR/config.toml" <<CONF
host = "0.0.0.0"
port = 26559
auth_token = "$AUTH_TOKEN"

[llm]
model_mode = "auto"

[llm.tokens]
ANTHROPIC = ""       # Required — get key at https://console.anthropic.com
GOOGLE_AI = ""       # Optional — embeddings + media analysis
ELEVENLABS = ""      # Optional — text-to-speech
CONF
    log "created $BOLLY_DIR/config.toml"
    info "authentication token saved in $BOLLY_DIR/config.toml"
else
    log "config already exists, skipping"
    # Backfill auth_token if empty (upgrade from older install)
    EXISTING_TOKEN=$(grep -E '^auth_token\s*=' "$BOLLY_DIR/config.toml" | head -1 | sed 's/[^=]*=\s*//' | tr -d ' "')
    if [ -z "$EXISTING_TOKEN" ]; then
        AUTH_TOKEN=$(head -c 256 /dev/urandom | LC_ALL=C tr -dc 'a-z0-9' | head -c 32)
        sed -i.bak "s/^auth_token\s*=.*/auth_token = \"$AUTH_TOKEN\"/" "$BOLLY_DIR/config.toml"
        rm -f "$BOLLY_DIR/config.toml.bak"
        log "generated auth token for existing install"
    fi
fi

# ─── Update script ────────────────────────────────────────────────────────────
cat > "$BIN_DIR/update" <<UPDATESCRIPT
#!/bin/bash
set -e
REPO="$REPO"
BIN="$BIN"
CHANNEL="\${BOLLY_CHANNEL:-$CHANNEL}"
TARGET="$TARGET"
# Use redirect URL for stable — no API call, no rate limit
DOWNLOAD_URL="https://github.com/\$REPO/releases/latest/download/bolly-server-\$TARGET"
DOWNLOAD_URL_LEGACY="https://github.com/\$REPO/releases/latest/download/bolly-\$TARGET"
if [ "\$CHANNEL" = "nightly" ]; then
    API_URL="https://api.github.com/repos/\$REPO/releases/tags/nightly"
    RELEASE_JSON=\$(curl -fsSL "\$API_URL") || { echo "could not fetch release info"; exit 1; }
    TAG=\$(echo "\$RELEASE_JSON" | grep '"tag_name"' | head -1 | sed 's/.*: "//;s/".*//')
    DOWNLOAD_URL="https://github.com/\$REPO/releases/download/\$TAG/bolly-server-\$TARGET"
    DOWNLOAD_URL_LEGACY="https://github.com/\$REPO/releases/download/\$TAG/bolly-\$TARGET"
fi
echo "checking for updates..."
curl -fsSL "\$DOWNLOAD_URL" -o "\$BIN.tmp" 2>/dev/null || \
    curl -fsSL "\$DOWNLOAD_URL_LEGACY" -o "\$BIN.tmp" || \
    { echo "download failed"; exit 1; }
# Resolve version from redirect
TAG=\$(curl -fsSIL "\$DOWNLOAD_URL" 2>/dev/null | grep -i '^location:' | tail -1 | sed 's|.*/download/\([^/]*\)/.*|\1|' | tr -d '\r')
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
echo "updated to \$TAG — restart bolly to apply"
UPDATESCRIPT
chmod +x "$BIN_DIR/update"

# ─── Platform-specific service ────────────────────────────────────────────────
step "setting up service"
SERVICE_KIND="none"

if [ "$PLATFORM" = "linux" ]; then
    # ── systemd ──
    if command -v systemctl &>/dev/null && [ "$(id -u)" -eq 0 ]; then
        SYSTEMD_SYSTEM_DIR="${BOLLY_SYSTEMD_SYSTEM_DIR:-/etc/systemd/system}"
        mkdir -p "$SYSTEMD_SYSTEM_DIR"
        SERVICE_FILE="$SYSTEMD_SYSTEM_DIR/bolly.service"
        cat > "$SERVICE_FILE" <<EOF
[Unit]
Description=Bolly AI Companion
After=network.target

[Service]
Type=simple
User=$(whoami)
WorkingDirectory=$BOLLY_DIR
Environment=BOLLY_HOME=$BOLLY_DIR
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
        info "start:   sudo systemctl start bolly"
        info "logs:    sudo journalctl -u bolly -f"
    elif command -v systemctl &>/dev/null; then
        # User-level systemd (no root)
        SYSTEMD_DIR="$HOME/.config/systemd/user"
        mkdir -p "$SYSTEMD_DIR"
        cat > "$SYSTEMD_DIR/bolly.service" <<EOF
[Unit]
Description=Bolly AI Companion
After=network.target

[Service]
Type=simple
WorkingDirectory=$BOLLY_DIR
Environment=BOLLY_HOME=$BOLLY_DIR
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
        info "start:   systemctl --user start bolly"
        info "logs:    journalctl --user -u bolly -f"
    else
        log "no systemd found — run manually: $BIN"
    fi

elif [ "$PLATFORM" = "macos" ]; then
    # ── launchd ──
    PLIST_DIR="$HOME/Library/LaunchAgents"
    PLIST="$PLIST_DIR/dev.bollyai.bolly.plist"
    mkdir -p "$PLIST_DIR"
    cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>dev.bollyai.bolly</string>
    <key>ProgramArguments</key>
    <array>
        <string>$BIN</string>
    </array>
    <key>WorkingDirectory</key>
    <string>$BOLLY_DIR</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>BOLLY_HOME</key>
        <string>$BOLLY_DIR</string>
        <key>RUST_LOG</key>
        <string>info</string>
    </dict>
    <key>KeepAlive</key>
    <true/>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardOutPath</key>
    <string>$BOLLY_DIR/bolly.log</string>
    <key>StandardErrorPath</key>
    <string>$BOLLY_DIR/bolly.log</string>
</dict>
</plist>
EOF
    SERVICE_KIND="launchd"
    log "launchd service created"
    info "start:   launchctl bootstrap gui/$(id -u) $PLIST"
    info "stop:    launchctl bootout gui/$(id -u)/dev.bollyai.bolly"
    info "logs:    tail -f $BOLLY_DIR/bolly.log"
fi

# ─── Add to PATH ──────────────────────────────────────────────────────────────
SHELL_NAME=$(basename "$SHELL" 2>/dev/null || echo "bash")
RC_FILE="$HOME/.${SHELL_NAME}rc"

if ! echo "$PATH" | grep -q "$BIN_DIR"; then
    EXPORT_LINE="export PATH=\"$BIN_DIR:\$PATH\""
    if [ -f "$RC_FILE" ] && ! grep -q "$BIN_DIR" "$RC_FILE"; then
        {
            echo ""
            echo "# bolly"
            echo "$EXPORT_LINE"
        } >> "$RC_FILE"
        log "added to PATH in $RC_FILE"
    fi
fi

# ─── Doctor: stop old instance, fix config, restart ──────────────────────────
step "starting bolly"

export PATH="$BIN_DIR:$PATH"

# Read port from config (default 26559)
BOLLY_PORT=26559
if [ -f "$BOLLY_DIR/config.toml" ]; then
    CFG_PORT=$(grep -E '^port\s*=' "$BOLLY_DIR/config.toml" | head -1 | sed 's/.*=\s*//' | tr -d ' ')
    if [ -n "$CFG_PORT" ]; then
        BOLLY_PORT="$CFG_PORT"
    fi
fi
BOLLY_URL="http://localhost:$BOLLY_PORT"

find_exact_bolly_pids() {
    ps -axo pid=,command= | while read -r pid command; do
        if [ "$command" = "$BIN" ]; then
            printf '%s\n' "$pid"
        fi
    done
}

stop_stray_bolly() {
    local pids remaining pid
    pids=$(find_exact_bolly_pids)
    [ -z "$pids" ] && return 0

    for pid in $pids; do
        info "stopping unmanaged bolly process (PID: $pid)..."
        kill "$pid" 2>/dev/null || true
    done

    for _ in $(seq 1 20); do
        remaining=$(find_exact_bolly_pids)
        [ -z "$remaining" ] && return 0
        sleep 0.1
    done

    fail "could not stop the existing Bolly process (PID: $remaining)"
}

# Start through the native service manager so the installed service is the
# process we verify. Fall back to a direct process only without a service manager.
case "$SERVICE_KIND" in
    launchd)
        LAUNCH_DOMAIN="gui/$(id -u)"
        LAUNCH_SERVICE="$LAUNCH_DOMAIN/dev.bollyai.bolly"
        launchctl bootout "$LAUNCH_SERVICE" >/dev/null 2>&1 || true
        stop_stray_bolly
        launchctl bootstrap "$LAUNCH_DOMAIN" "$PLIST"
        launchctl kickstart -k "$LAUNCH_SERVICE"
        launchctl print "$LAUNCH_SERVICE" >/dev/null
        ;;
    systemd-system)
        systemctl stop bolly >/dev/null 2>&1 || true
        stop_stray_bolly
        systemctl enable bolly >/dev/null
        systemctl restart bolly
        systemctl is-active --quiet bolly
        ;;
    systemd-user)
        systemctl --user stop bolly >/dev/null 2>&1 || true
        stop_stray_bolly
        systemctl --user enable bolly >/dev/null
        systemctl --user restart bolly
        systemctl --user is-active --quiet bolly
        ;;
    none)
        stop_stray_bolly
        "$BIN" &>/dev/null &
        ;;
esac

info "waiting for bolly to start..."
for _ in $(seq 1 30); do
    if curl -sf "$BOLLY_URL/healthz" >/dev/null 2>&1; then
        break
    fi
    sleep 0.5
done

    # Read auth token from config for browser URL
AUTH_TOKEN=$(grep -E '^auth_token\s*=' "$BOLLY_DIR/config.toml" | head -1 | sed 's/[^=]*=\s*//' | tr -d ' "')
AUTH_URL="$BOLLY_URL/auth?token=$AUTH_TOKEN"

if curl -sf "$BOLLY_URL/healthz" >/dev/null 2>&1; then
    log "bolly is running on port $BOLLY_PORT"

    # Open browser with auth
    if [ "$PLATFORM" = "macos" ]; then
        open "$AUTH_URL" 2>/dev/null
    elif command -v xdg-open &>/dev/null; then
        xdg-open "$AUTH_URL" 2>/dev/null
    fi

    echo ""
    echo -e "${BOLD}  ┌─────────────────────────────┐${NC}"
    echo -e "${BOLD}  │${NC}  ${GREEN}bolly is ready!${NC}            ${BOLD}│${NC}"
    echo -e "${BOLD}  └─────────────────────────────┘${NC}"
    echo ""
    echo -e "  ${CYAN}${BOLLY_URL}${NC}"
    echo ""
else
    fail "bolly service did not become healthy — check $BOLLY_DIR/bolly.log"
fi
