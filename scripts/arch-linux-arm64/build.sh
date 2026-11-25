#!/usr/bin/env bash
set -euo pipefail

# Cross-compiles Penumbra for Arch Linux ARM64 (glibc) from an x86_64 Arch host.

TARGET=aarch64-unknown-linux-gnu
script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd "$script_dir/../.." && pwd)
sysroot=${PENUMBRA_ARCH_ARM64_SYSROOT:-$HOME/.cache/penumbra/arch-linux-arm64-sysroot}
target_dir="$repo_root/target/cross"
release_dir="$target_dir/$TARGET/release"

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
    penumbra-clock
)

BLUE='\033[0;34m'
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'

log_info() {
    printf "${BLUE}[INFO]${NC} %s\n" "$1"
}

log_success() {
    printf "${GREEN}[✓]${NC} %s\n" "$1"
}

log_error() {
    printf "${RED}[ERROR]${NC} %s\n" "$1" >&2
}

usage() {
    cat <<EOF
Usage: scripts/arch-linux-arm64/build.sh [-h]

Cross-compiles Penumbra release binaries for $TARGET.

Environment:
  PENUMBRA_ARCH_ARM64_SYSROOT  Sysroot path (default: $HOME/.cache/penumbra/arch-linux-arm64-sysroot).
                               Missing sysroots are created by bootstrap-sysroot.sh.
EOF
}

case "${1:-}" in
    -h|--help)
        usage
        exit 0
        ;;
    "")
        ;;
    *)
        log_error "Unknown option: $1"
        usage >&2
        exit 2
        ;;
esac

for tool in cargo aarch64-linux-gnu-gcc aarch64-linux-gnu-ar pkg-config readelf; do
    command -v "$tool" >/dev/null 2>&1 || { log_error "Missing host tool: $tool (pacman -S --needed aarch64-linux-gnu-gcc pkgconf binutils)"; exit 1; }
done

if command -v rustup >/dev/null 2>&1; then
    rustup target add "$TARGET"
fi

if [[ ! -e "$sysroot/usr/lib/libc.so.6" ]]; then
    log_info "No sysroot at $sysroot; bootstrapping..."
    "$script_dir/bootstrap-sysroot.sh"
fi

# Linker and sysroot. rpath-link lets the linker resolve transitive shared library dependencies.
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=--sysroot=$sysroot -C link-arg=-Wl,-rpath-link,$sysroot/usr/lib -L native=$sysroot/usr/lib"

# pkg-config and C build scripts, scoped to the target triple.
export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_SYSROOT_DIR_aarch64_unknown_linux_gnu="$sysroot"
export PKG_CONFIG_LIBDIR_aarch64_unknown_linux_gnu="$sysroot/usr/lib/pkgconfig:$sysroot/usr/share/pkgconfig"
export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
export AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar
export CFLAGS_aarch64_unknown_linux_gnu="--sysroot=$sysroot"

cd "$repo_root"

log_info "Building workspace for $TARGET (excluding penumbra-comp)..."
cargo build --workspace --exclude penumbra-comp --release --target "$TARGET" --target-dir "$target_dir"

log_info "Building penumbra-comp with native-drm..."
cargo build -p penumbra-comp --features native-drm --release --target "$TARGET" --target-dir "$target_dir"

log_info "Validating binaries in $release_dir..."
for bin in "${required_bins[@]}"; do
    path="$release_dir/$bin"
    [[ -f "$path" ]] || { log_error "Missing binary: $path"; exit 1; }
    machine=$(readelf -h "$path" | awk -F: '/Machine:/ { sub(/^ +/, "", $2); print $2 }')
    [[ "$machine" == "AArch64" ]] || { log_error "$path is not an AArch64 ELF (machine: $machine)"; exit 1; }
done

log_success "All ${#required_bins[@]} binaries built for $TARGET in $release_dir"
