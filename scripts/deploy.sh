#!/usr/bin/env bash
set -euo pipefail

# Penumbra Mobile DE - Optimized Deployment Script
# Automatically deploys cross-compiled ARM64 binaries and assets to target device (e.g. Fairphone 4).

TARGET="aarch64-unknown-linux-musl"
DEVICE_IP="${DEVICE_IP:-172.16.42.1}"
DEVICE_USER="${DEVICE_USER:-user}"
DEVICE_PASSWORD="${DEVICE_PASSWORD:-1}"
SUDO_PASSWORD="${SUDO_PASSWORD:-1}"
SESSION_MANAGER="${SESSION_MANAGER:-auto}"
BUILD_FIRST=false
DEPLOY_ONLY=true
RESTART_SESSION=true
CHECK_STATUS_ONLY=false
SHOW_LOGS=false

# Terminal colors
BOLD='\033[1m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

log_info() {
    printf "${BLUE}[INFO]${NC} %s\n" "$1"
}

log_success() {
    printf "${GREEN}[✓]${NC} %s\n" "$1"
}

log_warn() {
    printf "${YELLOW}[WARN]${NC} %s\n" "$1"
}

log_error() {
    printf "${RED}[ERROR]${NC} %s\n" "$1" >&2
}

usage() {
    cat <<EOF
${BOLD}Usage:${NC} scripts/deploy.sh [OPTIONS]

${BOLD}Options:${NC}
  --build               Build ARM64 alpine binaries before deploying (via Docker).
  --no-build            Skip build; deploy existing pre-built release binaries (default).
  --no-restart          Do not restart the display manager / session after deployment.
  --status              Check connection and running Penumbra status on phone and exit.
  --logs                View the recent session log on the phone.
  --ip <ADDRESS>        Device IP address (default: 172.16.42.1 or \$DEVICE_IP).
  --user <USER>         Remote SSH username (default: user or \$DEVICE_USER).
  --password <PASS>     SSH password (default: 1 or \$DEVICE_PASSWORD).
  --sudo-pass <PASS>    Sudo password (default: 1 or \$SUDO_PASSWORD).
  -h, --help            Show this help message.

${BOLD}Environment Variables:${NC}
  DEVICE_IP             Device IP address (default: 172.16.42.1)
  DEVICE_USER           Device SSH user (default: user)
  DEVICE_PASSWORD       SSH password (default: 1)
  SUDO_PASSWORD         Sudo password on device (default: 1)

${BOLD}Examples:${NC}
  ./scripts/deploy.sh                     # Fast deploy & restart session
  ./scripts/deploy.sh --build             # Cross-compile in Docker first, then deploy & restart
  ./scripts/deploy.sh --status            # Check phone reachability and running processes
  ./scripts/deploy.sh --logs              # Show penumbra session logs on phone
EOF
}

while (($# > 0)); do
    case "$1" in
        --build|-b)
            BUILD_FIRST=true
            DEPLOY_ONLY=false
            ;;
        --no-build|--deploy-only)
            BUILD_FIRST=false
            DEPLOY_ONLY=true
            ;;
        --no-restart|--skip-restart)
            RESTART_SESSION=false
            ;;
        --status)
            CHECK_STATUS_ONLY=true
            ;;
        --logs)
            SHOW_LOGS=true
            ;;
        --ip|--device-ip)
            (($# >= 2)) || { log_error "Missing value for $1"; exit 2; }
            DEVICE_IP="$2"
            shift
            ;;
        --user|--device-user)
            (($# >= 2)) || { log_error "Missing value for $1"; exit 2; }
            DEVICE_USER="$2"
            shift
            ;;
        --password|--device-password)
            (($# >= 2)) || { log_error "Missing value for $1"; exit 2; }
            DEVICE_PASSWORD="$2"
            shift
            ;;
        --sudo-pass|--sudo-password)
            (($# >= 2)) || { log_error "Missing value for $1"; exit 2; }
            SUDO_PASSWORD="$2"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            log_error "Unknown option: $1"
            usage >&2
            exit 2
            ;;
    esac
    shift
done

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
release_dir="$repo_root/target/cross/$TARGET/release"
staging_dir="penumbra-deploy-$(date +%s)"
remote="${DEVICE_USER}@${DEVICE_IP}"

# Build SSH / SCP command arrays
ssh_cmd=(ssh -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=/dev/null -o ConnectTimeout=5)
scp_cmd=(scp -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=/dev/null)

if [[ -n "$DEVICE_PASSWORD" ]]; then
    if ! command -v sshpass >/dev/null 2>&1; then
        log_error "sshpass is required for password authentication. Install it (e.g. sudo apt install sshpass / apk add sshpass)."
        exit 1
    fi
    ssh_cmd=(sshpass -p "$DEVICE_PASSWORD" ssh -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=/dev/null -o ConnectTimeout=5)
    scp_cmd=(sshpass -p "$DEVICE_PASSWORD" scp -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=/dev/null)
fi

# Build ARM64 alpine binaries if requested
if [[ "$BUILD_FIRST" == true ]]; then
    log_info "Cross-compiling ARM64 Alpine release binaries via Docker..."
    "$repo_root/scripts/build-arm64-alpine.sh"
    log_success "ARM64 compilation completed"
fi

# Quick connectivity test
log_info "Testing connectivity to device at ${remote}..."
if ! "${ssh_cmd[@]}" "$remote" 'true' >/dev/null 2>&1; then
    log_error "Cannot connect to ${remote}. Ensure the phone is connected via USB and SSH is running."
    exit 1
fi
log_success "Device reachable at ${DEVICE_IP}"

# Status-only check
if [[ "$CHECK_STATUS_ONLY" == true ]]; then
    log_info "Fetching system and Penumbra status from phone..."
    "${ssh_cmd[@]}" "$remote" bash -s <<'STATUS_SCRIPT'
echo "=== Host Info ==="
uname -a
uptime
echo "=== Display Manager ==="
for svc in gdm tinydm greetd phosh; do
    if systemctl is-active "$svc" >/dev/null 2>&1; then
        echo "Active service: $svc"
    fi
done
echo "=== Penumbra Processes ==="
pgrep -la penumbra || echo "No penumbra processes running."
echo "=== Power / Battery ==="
if [ -d /sys/class/power_supply/battery ]; then
    capacity=$(cat /sys/class/power_supply/battery/capacity 2>/dev/null || echo "?")
    status=$(cat /sys/class/power_supply/battery/status 2>/dev/null || echo "?")
    echo "Battery: ${capacity}% (${status})"
fi
STATUS_SCRIPT
    exit 0
fi

# Show logs
if [[ "$SHOW_LOGS" == true ]]; then
    log_info "Displaying recent session log..."
    "${ssh_cmd[@]}" "$remote" 'cat /tmp/penumbra-session-*.log 2>/dev/null | tail -n 60 || journalctl -u gdm --lines=50 --no-pager'
    exit 0
fi

# Comprehensive list of canonical binaries
required_bins=(
    penumbra-comp
    penumbra-top-bar
    penumbra-homescreen
    penumbra-nav-bar
    penumbra-settings
    penumbra-keyboard
    penumbra-greeter
    penumbra-shell
)

log_info "Validating release binaries in ${release_dir}..."
for binary in "${required_bins[@]}"; do
    if [[ ! -f "$release_dir/$binary" ]]; then
        log_error "Missing binary: $release_dir/$binary"
        log_error "Run './scripts/deploy.sh --build' to compile the binaries first."
        exit 1
    fi
done
log_success "All $((${#required_bins[@]})) release binaries found."

# Create remote staging directory
log_info "Preparing staging directory on device..."
"${ssh_cmd[@]}" "$remote" "mkdir -p '$staging_dir/bin' '$staging_dir/assets'"

# Copy binaries
log_info "Transferring release binaries to phone..."
"${scp_cmd[@]}" "${required_bins[@]/#/$release_dir/}" "$remote:$staging_dir/bin/"
log_success "Binaries transferred."

# Copy session scripts and desktop definitions
log_info "Transferring session scripts and desktop definitions..."
"${scp_cmd[@]}" \
    "$repo_root/scripts/penumbra-session" \
    "$repo_root/scripts/penumbra-nested-session" \
    "$repo_root/scripts/penumbra-native-session" \
    "$repo_root/data/penumbra.desktop" \
    "$repo_root/data/penumbra-settings.desktop" \
    "$repo_root/data/penumbra-keyboard.desktop" \
    "$remote:$staging_dir/"

# Copy assets
if [[ -d "$repo_root/assets" ]]; then
    log_info "Transferring Penumbra assets..."
    "${scp_cmd[@]}" -r "$repo_root/assets/." "$remote:$staging_dir/assets/"
fi
log_success "Assets and configurations staged."

# Installation on device
log_info "Installing binaries and setting permissions on device..."
printf -v remote_env 'SESSION_MANAGER=%q PENUMBRA_LOGIN_USER=%q STAGING_DIR=%q RESTART_SESSION=%q' \
    "$SESSION_MANAGER" "$DEVICE_USER" "$staging_dir" "$RESTART_SESSION"

sudo_script='
set -euo pipefail

# 1. Install binaries to /usr/local/bin
install -d /usr/local/bin /usr/share/wayland-sessions /usr/share/penumbra/assets /usr/share/applications
install -m 0755 "$STAGING_DIR"/bin/* /usr/local/bin/

# 2. Install session wrappers
install -m 0755 "$STAGING_DIR"/penumbra-session /usr/local/bin/penumbra-session
install -m 0755 "$STAGING_DIR"/penumbra-nested-session /usr/local/bin/penumbra-nested-session
install -m 0755 "$STAGING_DIR"/penumbra-native-session /usr/local/bin/penumbra-native-session

# 3. Install desktop entries
install -m 0644 "$STAGING_DIR"/penumbra.desktop /usr/share/wayland-sessions/penumbra.desktop
install -m 0644 "$STAGING_DIR"/penumbra-settings.desktop /usr/share/applications/penumbra-settings.desktop
install -m 0644 "$STAGING_DIR"/penumbra-keyboard.desktop /usr/share/applications/penumbra-keyboard.desktop

# 4. Install assets
cp -a "$STAGING_DIR"/assets/. /usr/share/penumbra/assets/

# 5. Ensure backlight and input permissions
mkdir -p /etc/udev/rules.d
cat > /etc/udev/rules.d/90-backlight.rules <<'\''UDEV_BACKLIGHT'\''
ACTION=="add", SUBSYSTEM=="backlight", RUN+="/bin/chgrp video /sys/class/backlight/%k/brightness", RUN+="/bin/chmod 0664 /sys/class/backlight/%k/brightness"
UDEV_BACKLIGHT
udevadm control --reload-rules 2>/dev/null || true
udevadm trigger --subsystem-match=backlight 2>/dev/null || true
chgrp video /sys/class/backlight/*/brightness 2>/dev/null || true
chmod 0664 /sys/class/backlight/*/brightness 2>/dev/null || true

addgroup "${PENUMBRA_LOGIN_USER}" video 2>/dev/null || usermod -aG video "${PENUMBRA_LOGIN_USER}" 2>/dev/null || true
addgroup "${PENUMBRA_LOGIN_USER}" input 2>/dev/null || usermod -aG input "${PENUMBRA_LOGIN_USER}" 2>/dev/null || true
addgroup "${PENUMBRA_LOGIN_USER}" audio 2>/dev/null || usermod -aG audio "${PENUMBRA_LOGIN_USER}" 2>/dev/null || true

# Ensure local timezone is synchronized (EET/Europe/Bucharest)
timedatectl set-timezone Europe/Bucharest 2>/dev/null || true

# Ensure greetd user and configuration
if id greetd >/dev/null 2>&1; then
    addgroup greetd video 2>/dev/null || usermod -aG video greetd 2>/dev/null || true
    addgroup greetd input 2>/dev/null || usermod -aG input greetd 2>/dev/null || true
    mkdir -p /etc/greetd
    cat > /etc/greetd/config.toml <<'GREETD_CONF'
[terminal]
vt = 1

[default_session]
command = "env SLINT_SCALE_FACTOR=2.62 cage -s -d -- /usr/local/bin/penumbra-greeter"
user = "greetd"
GREETD_CONF
    chmod 0644 /etc/greetd/config.toml
fi

# Clean up staging directory
rm -rf "$STAGING_DIR"

# 6. Session restart if requested
if [[ "$RESTART_SESSION" == true ]]; then
    if systemctl is-active greetd >/dev/null 2>&1; then
        echo "[DEVICE] Restarting greetd service..."
        systemctl restart greetd
    elif systemctl is-active gdm >/dev/null 2>&1; then
        echo "[DEVICE] Restarting GDM service..."
        systemctl restart gdm
    elif systemctl is-active tinydm >/dev/null 2>&1; then
        echo "[DEVICE] Restarting tinydm service..."
        systemctl restart tinydm
    else
        echo "[DEVICE] No standard active display manager detected, killing stale penumbra sessions..."
        pkill -u "${PENUMBRA_LOGIN_USER}" -x penumbra-comp || true
    fi
fi
'

if [[ -n "$SUDO_PASSWORD" ]]; then
    {
        printf '%s\n' "$SUDO_PASSWORD"
        echo "$sudo_script"
    } | "${ssh_cmd[@]}" "$remote" "sudo -S -p '' env $remote_env bash -s"
else
    "${ssh_cmd[@]}" "$remote" "sudo -n env $remote_env bash -s" <<< "$sudo_script"
fi

log_success "Installation complete!"

# Verify shared libraries
log_info "Verifying shared library resolution on device..."
missing_libs=$("${ssh_cmd[@]}" "$remote" 'ldd /usr/local/bin/penumbra-comp 2>&1' | grep -E 'not found|Error loading shared library' || true)
if [[ -n "$missing_libs" ]]; then
    log_error "penumbra-comp has unresolved shared libraries on the device:"
    echo "$missing_libs" >&2
    exit 3
fi
log_success "All shared libraries resolved cleanly."

# Verify active session
if [[ "$RESTART_SESSION" == true ]]; then
    log_info "Waiting for session to initialize..."
    sleep 3
    active_comp=$("${ssh_cmd[@]}" "$remote" 'pgrep -la penumbra-comp || true')
    if [[ -n "$active_comp" ]]; then
        log_success "Penumbra compositor is running: $active_comp"
    else
        log_warn "penumbra-comp not immediately found. It may be starting up or GDM is re-authenticating."
        log_info "Check session status with './scripts/deploy.sh --status' or './scripts/deploy.sh --logs'."
    fi
fi

log_success "Deployment to ${DEVICE_IP} finished successfully!"
