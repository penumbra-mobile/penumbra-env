#!/usr/bin/env bash
set -euo pipefail

# Deploys Penumbra to an Arch Linux ARM64 device over SSH.
# Passwords are never stored: SSH uses keys or prompts, and sudo prompts on the device TTY.

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd "$script_dir/../.." && pwd)
TARGET=aarch64-unknown-linux-gnu
release_dir="$repo_root/target/cross/$TARGET/release"
remote_stage="penumbra-arch-deploy"

DEVICE_IP="${DEVICE_IP:-}"
DEVICE_USER="${DEVICE_USER:-$(id -un)}"
DEVICE_PORT="${DEVICE_PORT:-22}"
BUILD_FIRST=false
RESTART_SESSION=true
CHECK_STATUS_ONLY=false
SHOW_LOGS=false

required_bins=(
    penumbra-comp
    penumbra-top-bar
    penumbra-homescreen
    penumbra-nav-bar
    penumbra-settings
    penumbra-keyboard
    penumbra-greeter
    penumbra-shell
    penumbra-desktop-shell
    penumbra-calls
    penumbra-messaging
    penumbra-contacts
)

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
${BOLD}Usage:${NC} scripts/arch-linux-arm64/deploy.sh [OPTIONS]

${BOLD}Options:${NC}
  --build               Cross-compile with build.sh before deploying.
  --no-build            Deploy existing release binaries (default).
  --no-restart          Do not restart the display manager after installation.
  --status              Check the device and running Penumbra processes, then exit.
  --logs                Show recent session logs on the device, then exit.
  --ip <ADDRESS>        Device address (required unless DEVICE_IP is set).
  --user <USER>         SSH user on the device (default: local user or \$DEVICE_USER).
  --port <PORT>         SSH port (default: 22 or \$DEVICE_PORT).
  -h, --help            Show this help message.

${BOLD}Examples:${NC}
  DEVICE_IP=192.168.1.150 ./scripts/arch-linux-arm64/deploy.sh --build
  ./scripts/arch-linux-arm64/deploy.sh --ip 192.168.1.150 --user mobile --status
EOF
}

while (($# > 0)); do
    case "$1" in
        --build|-b)
            BUILD_FIRST=true
            ;;
        --no-build|--deploy-only)
            BUILD_FIRST=false
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
        --ip)
            (($# >= 2)) || { log_error "Missing value for $1"; exit 2; }
            DEVICE_IP="$2"
            shift
            ;;
        --user)
            (($# >= 2)) || { log_error "Missing value for $1"; exit 2; }
            DEVICE_USER="$2"
            shift
            ;;
        --port)
            (($# >= 2)) || { log_error "Missing value for $1"; exit 2; }
            DEVICE_PORT="$2"
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

[[ -n "$DEVICE_IP" ]] || { log_error "No device address. Pass --ip or set DEVICE_IP."; exit 2; }

remote="$DEVICE_USER@$DEVICE_IP"
control_dir=$(mktemp -d)
stage_dir=""
cleanup() {
    [[ -z "$stage_dir" ]] || rm -rf "$stage_dir"
    ssh -p "$DEVICE_PORT" -o ControlPath="$control_dir/%C" -O exit "$remote" >/dev/null 2>&1 || true
    rm -rf "$control_dir"
}
trap cleanup EXIT

# One multiplexed connection so password prompts appear at most once.
ssh_opts=(
    -p "$DEVICE_PORT"
    -o ConnectTimeout=5
    -o StrictHostKeyChecking=accept-new
    -o ControlMaster=auto
    -o ControlPath="$control_dir/%C"
    -o ControlPersist=120
)

check_connection() {
    log_info "Testing connectivity to $remote..."
    if ! ssh "${ssh_opts[@]}" "$remote" true; then
        log_error "Cannot connect to $remote. Check the address, SSH service, and keys."
        exit 1
    fi
    log_success "Device reachable at $DEVICE_IP"
}

if [[ "$CHECK_STATUS_ONLY" == true ]]; then
    check_connection
    ssh "${ssh_opts[@]}" "$remote" bash -s <<'STATUS_SCRIPT'
echo "=== Host ==="
uname -m
cat /etc/arch-release 2>/dev/null || echo "No /etc/arch-release found"
uptime
echo "=== Display Manager ==="
for svc in greetd gdm sddm; do
    systemctl is-active --quiet "$svc" && echo "Active service: $svc"
done
echo "=== Penumbra Processes ==="
pgrep -la penumbra || echo "No penumbra processes running."
STATUS_SCRIPT
    exit 0
fi

if [[ "$SHOW_LOGS" == true ]]; then
    check_connection
    ssh "${ssh_opts[@]}" "$remote" 'tail -n 60 /tmp/penumbra-session-*.log 2>/dev/null || journalctl -b -n 60 --no-pager'
    exit 0
fi

if [[ "$BUILD_FIRST" == true ]]; then
    "$script_dir/build.sh"
fi

log_info "Validating release binaries in $release_dir..."
for bin in "${required_bins[@]}"; do
    path="$release_dir/$bin"
    [[ -f "$path" ]] || { log_error "Missing binary: $path (run with --build)"; exit 1; }
    machine=$(readelf -h "$path" | awk -F: '/Machine:/ { sub(/^ +/, "", $2); print $2 }')
    [[ "$machine" == "AArch64" ]] || { log_error "$path is not an AArch64 ELF (machine: $machine)"; exit 1; }
done
[[ -d "$repo_root/assets" ]] || { log_error "Missing assets directory: $repo_root/assets"; exit 1; }
log_success "All ${#required_bins[@]} release binaries found."

check_connection

stage_dir=$(mktemp -d)
mkdir -p "$stage_dir/bin" "$stage_dir/assets"
for bin in "${required_bins[@]}"; do
    cp "$release_dir/$bin" "$stage_dir/bin/"
done
cp "$repo_root/scripts/penumbra-session" \
    "$repo_root/scripts/penumbra-nested-session" \
    "$repo_root/scripts/penumbra-native-session" \
    "$repo_root/data/penumbra.desktop" \
    "$repo_root/data/penumbra-settings.desktop" \
    "$repo_root/data/penumbra-keyboard.desktop" \
    "$repo_root/data/penumbra-calls.desktop" \
    "$repo_root/data/penumbra-messaging.desktop" \
    "$repo_root/data/penumbra-contacts.desktop" \
    "$repo_root/data/penumbra-settings.svg" \
    "$repo_root/data/penumbra-calls.svg" \
    "$repo_root/data/penumbra-messaging.svg" \
    "$repo_root/data/penumbra-contacts.svg" \
    "$stage_dir/"
cp -a "$repo_root/assets/." "$stage_dir/assets/"

cat >"$stage_dir/install.sh" <<'INSTALL_SCRIPT'
#!/usr/bin/env bash
set -euo pipefail

stage=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
login_user=${PENUMBRA_LOGIN_USER:?PENUMBRA_LOGIN_USER is required}
restart_session=${RESTART_SESSION:-true}

install -d /usr/local/bin /usr/share/wayland-sessions /usr/share/applications /usr/share/penumbra/assets
install -m 0755 "$stage"/bin/* /usr/local/bin/
install -m 0755 "$stage"/penumbra-session "$stage"/penumbra-nested-session "$stage"/penumbra-native-session /usr/local/bin/
install -m 0644 "$stage"/penumbra.desktop /usr/share/wayland-sessions/penumbra.desktop
install -m 0644 "$stage"/penumbra-settings.desktop "$stage"/penumbra-keyboard.desktop "$stage"/penumbra-calls.desktop "$stage"/penumbra-messaging.desktop "$stage"/penumbra-contacts.desktop /usr/share/applications/
install -d /usr/share/icons/hicolor/scalable/apps
install -m 0644 "$stage"/penumbra-settings.svg "$stage"/penumbra-calls.svg "$stage"/penumbra-messaging.svg "$stage"/penumbra-contacts.svg /usr/share/icons/hicolor/scalable/apps/
gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true
pkill -x gnome-calls 2>/dev/null || true
cp -a "$stage"/assets/. /usr/share/penumbra/assets/
if [ -d "$stage/assets/sounds/penumbra" ]; then
    install -d /usr/share/sounds/penumbra
    cp -a "$stage"/assets/sounds/penumbra/. /usr/share/sounds/penumbra/
fi

install -d /etc/udev/rules.d
cat > /etc/udev/rules.d/90-backlight.rules <<'UDEV_RULE'
ACTION=="add", SUBSYSTEM=="backlight", RUN+="/bin/chgrp video /sys/class/backlight/%k/brightness", RUN+="/bin/chmod 0664 /sys/class/backlight/%k/brightness"
UDEV_RULE
udevadm control --reload-rules
udevadm trigger --subsystem-match=backlight || true

id "$login_user" >/dev/null
for group in video input audio; do
    if getent group "$group" >/dev/null; then
        usermod -aG "$group" "$login_user"
    fi
done

if ! pacman -Q greetd cage >/dev/null 2>&1; then
    pacman -Sy --needed --noconfirm greetd cage
fi
id greeter >/dev/null
for group in video input; do
    usermod -aG "$group" greeter
done
install -d /etc/greetd
cat > /etc/greetd/config.toml <<'GREETD_CONF'
[terminal]
vt = 1

[default_session]
command = "env SLINT_SCALE_FACTOR=2.62 cage -s -d -- /usr/local/bin/penumbra-greeter"
user = "greeter"
GREETD_CONF
chmod 0644 /etc/greetd/config.toml
systemctl disable --now phosh.service 2>/dev/null || true
systemctl enable greetd.service

if [[ "$restart_session" == true ]]; then
    echo "[DEVICE] Restarting greetd (penumbra-greeter is the default login)..."
    systemctl restart greetd
fi
INSTALL_SCRIPT

log_info "Transferring staged files to $remote..."
tar -C "$stage_dir" -cf - . | ssh "${ssh_opts[@]}" "$remote" \
    "rm -rf $remote_stage && mkdir -p $remote_stage && tar -C $remote_stage -xf -"
log_success "Files staged on device."

log_info "Installing on device (sudo will prompt on the device terminal)..."
printf -v remote_install 'sudo env PENUMBRA_LOGIN_USER=%q RESTART_SESSION=%q bash %s/install.sh; status=$?; rm -rf %s; exit $status' \
    "$DEVICE_USER" "$RESTART_SESSION" "$remote_stage" "$remote_stage"
ssh -t "${ssh_opts[@]}" "$remote" "$remote_install"
log_success "Installation complete."

log_info "Verifying shared library resolution on device..."
missing_libs=$(ssh "${ssh_opts[@]}" "$remote" 'ldd /usr/local/bin/penumbra-comp 2>&1 | grep -E "not found|Error loading shared library" || true')
if [[ -n "$missing_libs" ]]; then
    log_error "penumbra-comp has unresolved shared libraries on the device:"
    echo "$missing_libs" >&2
    exit 3
fi
log_success "All shared libraries resolved."

if [[ "$RESTART_SESSION" == true ]]; then
    log_info "Waiting for session to initialize..."
    sleep 3
    active_comp=$(ssh "${ssh_opts[@]}" "$remote" 'pgrep -la penumbra-comp || true')
    if [[ -n "$active_comp" ]]; then
        log_success "Penumbra compositor is running: $active_comp"
    else
        log_warn "penumbra-comp not found yet. Check with --status or --logs."
    fi
fi

log_success "Deployment to $DEVICE_IP finished."
