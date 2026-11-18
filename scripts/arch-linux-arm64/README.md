# Arch Linux ARM64 Scripts

Cross-compile Penumbra on an x86_64 Arch Linux host and deploy it to an Arch Linux ARM64 device (aarch64) over SSH.

## Scripts

| Script | Purpose |
| --- | --- |
| `bootstrap-sysroot.sh` | Builds a glibc aarch64 sysroot from Arch Linux ARM packages without root. |
| `build.sh` | Cross-compiles the release binaries for `aarch64-unknown-linux-gnu`. |
| `deploy.sh` | Installs the binaries on the device, configures the greeter login, and checks status or logs. |

## Requirements

- Host: `cargo`, `aarch64-linux-gnu-gcc`, `aarch64-linux-gnu-ar`, `pkg-config`, `readelf`, `pacman`, `bsdtar`, `curl`, `gpg`, `tar`, `awk`.
- Rust target: `aarch64-unknown-linux-gnu`.
- Device: Arch Linux ARM reachable over SSH, with a user that has `sudo`.

Install the host tools with:

```
sudo pacman -S --needed aarch64-linux-gnu-gcc pkgconf binutils bsdtar curl gnupg
```

## Workflow

1. Create the sysroot (one time):

   ```
   ./scripts/arch-linux-arm64/bootstrap-sysroot.sh
   ```

   The sysroot is written to `~/.cache/penumbra/arch-linux-arm64-sysroot`. Set `PENUMBRA_ARCH_ARM64_SYSROOT` to use another path. Set `ALARM_MIRROR` to use another mirror.

2. Cross-compile:

   ```
   ./scripts/arch-linux-arm64/build.sh
   ```

   Binaries are written to `target/cross/aarch64-unknown-linux-gnu/release`.

3. Deploy to the device:

   ```
   ./scripts/arch-linux-arm64/deploy.sh --ip 172.16.42.1 --user kupfer
   ```

   The device's `sudo` password is typed on the device terminal. Passwords are not stored.

## deploy.sh Options

| Option | Description |
| --- | --- |
| `--build`, `-b` | Run `build.sh` before deploying. |
| `--no-build`, `--deploy-only` | Deploy existing binaries (default). |
| `--no-restart`, `--skip-restart` | Install without restarting the display manager. |
| `--status` | Check the device and running Penumbra processes, then exit. |
| `--logs` | Show recent session logs on the device, then exit. |
| `--ip <ADDRESS>` | Device address (or set `DEVICE_IP`). |
| `--user <USER>` | SSH user (or set `DEVICE_USER`). |
| `--port <PORT>` | SSH port (or set `DEVICE_PORT`, default `22`). |

## What deploy.sh Changes on the Device

- Installs the binaries to `/usr/local/bin` and the Wayland session file to `/usr/share/wayland-sessions/penumbra.desktop`.
- Installs a udev rule that lets the `video` group write the backlight brightness.
- Adds the login user to the `video`, `input`, and `audio` groups.
- Installs `greetd` and `cage` if missing, and writes `/etc/greetd/config.toml`. The greeter runs `penumbra-greeter` under `cage` on VT1 as the `greeter` user.
- Disables `phosh.service` and enables `greetd.service`, so the Penumbra greeter is the default login.

To return to Phosh on the device:

```
sudo systemctl disable --now greetd && sudo systemctl enable --now phosh
```
