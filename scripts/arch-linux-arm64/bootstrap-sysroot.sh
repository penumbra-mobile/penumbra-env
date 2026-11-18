#!/usr/bin/env bash
set -euo pipefail

# Builds a glibc aarch64 sysroot from Arch Linux ARM packages without root.
# Packages are downloaded into a private pacman database and extracted with bsdtar.

sysroot=${PENUMBRA_ARCH_ARM64_SYSROOT:-$HOME/.cache/penumbra/arch-linux-arm64-sysroot}
work="$sysroot.work"
mirror=${ALARM_MIRROR:-https://fl.us.mirror.archlinuxarm.org}
repos=(core extra alarm)
packages=(
    glibc
    linux-api-headers
    libdrm
    libglvnd
    mesa
    libinput
    seatd
    systemd-libs
    libxkbcommon
    wayland
    fontconfig
    freetype2
)
pkgconfig_files=(libdrm gbm egl libinput libseat libudev xkbcommon wayland-client)

# Terminal colors match deploy.sh
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

for tool in pacman bsdtar curl gpg tar awk; do
    command -v "$tool" >/dev/null 2>&1 || { log_error "Missing required tool: $tool"; exit 1; }
done

# The package cache is recreated on each run so only one package version set is extracted.
rm -rf "$work/cache" "$work/db" "$work/keyring" "$work/dl" "$sysroot"
mkdir -p "$work/cache" "$work/db" "$work/gnupg" "$work/keyring" "$work/dl" "$sysroot"
chmod 700 "$work/gnupg"

cat >"$work/pacman.conf" <<EOF
[options]
Architecture = aarch64
DBPath = $work/db
CacheDir = $work/cache
LogFile = $work/pacman.log
GPGDir = $work/gnupg
SigLevel = Required DatabaseOptional
EOF
for repo in "${repos[@]}"; do
    printf '\n[%s]\nServer = %s/$arch/$repo\n' "$repo" "$mirror" >>"$work/pacman.conf"
done

log_info "Fetching ALARM repository databases..."
for repo in "${repos[@]}"; do
    curl -fsSL "$mirror/aarch64/$repo/$repo.db.tar.gz" -o "$work/dl/$repo.db.tar.gz"
done

# Find the ALARM keyring package by name in the repository databases.
keyring_repo=""
keyring_file=""
for repo in "${repos[@]}"; do
    keyring_file=$(tar -xzOf "$work/dl/$repo.db.tar.gz" --wildcards 'archlinuxarm-keyring-*/desc' 2>/dev/null \
        | awk '/^%FILENAME%$/ { getline; print; exit }') || true
    if [[ -n "$keyring_file" ]]; then
        keyring_repo=$repo
        break
    fi
done
[[ -n "$keyring_file" ]] || { log_error "archlinuxarm-keyring not found in the ALARM repositories"; exit 1; }

# The keyring package is fetched over HTTPS and trusted on first use.
log_info "Importing Arch Linux ARM package keys..."
curl -fsSL "$mirror/aarch64/$keyring_repo/$keyring_file" -o "$work/dl/$keyring_file"
bsdtar -xf "$work/dl/$keyring_file" -C "$work/keyring" usr/share/pacman/keyrings
keyring_dir="$work/keyring/usr/share/pacman/keyrings"
[[ -f "$keyring_dir/archlinuxarm.gpg" ]] || { log_error "archlinuxarm.gpg missing from keyring package"; exit 1; }
gpg --homedir "$work/gnupg" --batch --quiet --import "$keyring_dir/archlinuxarm.gpg"
if [[ -s "$keyring_dir/archlinuxarm-trusted" ]]; then
    gpg --homedir "$work/gnupg" --batch --quiet --import-ownertrust "$keyring_dir/archlinuxarm-trusted"
fi
gpg --homedir "$work/gnupg" --batch --quiet --update-trustdb

log_info "Resolving aarch64 package closure..."
# Seed the private DBPath with the repository databases. pacman -Sp only reads them.
mkdir -p "$work/db/sync" "$work/db/local"
for repo in "${repos[@]}"; do
    cp "$work/dl/$repo.db.tar.gz" "$work/db/sync/$repo.db"
done
mapfile -t urls < <(pacman --config "$work/pacman.conf" --noconfirm -Sp "${packages[@]}")
[[ ${#urls[@]} -gt 0 ]] || { log_error "pacman resolved no package URLs"; exit 1; }

# Download each package with its signature and verify it against the ALARM keyring.
log_info "Downloading ${#urls[@]} packages..."
for url in "${urls[@]}"; do
    file="${url##*/}"
    curl -fsSL "$url" -o "$work/cache/$file"
    curl -fsSL "$url.sig" -o "$work/cache/$file.sig"
    gpg --homedir "$work/gnupg" --batch --quiet --verify "$work/cache/$file.sig" "$work/cache/$file" >/dev/null 2>&1 \
        || { log_error "Signature verification failed for $file"; exit 1; }
done

log_info "Extracting packages into $sysroot..."
while IFS= read -r -d '' pkg; do
    bsdtar -xf "$pkg" -C "$sysroot"
done < <(find "$work/cache" -maxdepth 1 -name '*.pkg.tar.*' ! -name '*.sig' -print0)
find "$sysroot" -maxdepth 1 -name '.[A-Z]*' -delete

[[ -e "$sysroot/usr/lib/libc.so.6" && -e "$sysroot/usr/lib/crt1.o" ]] \
    || { log_error "glibc was not extracted into the sysroot"; exit 1; }
for pc in "${pkgconfig_files[@]}"; do
    [[ -f "$sysroot/usr/lib/pkgconfig/$pc.pc" ]] || { log_error "Missing $pc.pc in sysroot"; exit 1; }
done

log_success "Arch Linux ARM64 sysroot ready at $sysroot"
