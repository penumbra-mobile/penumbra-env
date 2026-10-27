# Penumbra Settings

`penumbra-settings` is a fast, native, touch-first mobile Settings application built for **Penumbra Mobile DE** (Linux on smartphones such as the Fairphone 4, PinePhone Pro, and postmarketOS devices).

Developed in **Rust** using **Slint** for hardware-accelerated declarative UI and **Tokio** for asynchronous D-Bus system interactions, `penumbra-settings` strictly adheres to modern Android 17 / Pixel Material You design aesthetics with zero reliance on mock data or desktop-oriented configuration dialogs.

---

## Highlights & Features

- **Android 17 / Material 3 Expressive UI**:
  - OLED pitch-black base canvas (`#0D0B10`) for maximum mobile battery efficiency.
  - Tonal pastel icon badges with high-contrast glyphs.
  - Two-tier viewport architecture with 250ms parallax horizontal slide transitions.
  - Live fuzzy search capsule filtering across all system settings categories.
- **Genuine Hardware & System Telemetry**:
  - **Device Model**: Discovers ARM DeviceTree model via `/proc/device-tree/model` (e.g. `Fairphone 4`).
  - **Processor SoC**: Queries Qualcomm SoC machine and family identifiers via `/sys/devices/soc0` (e.g. `Qualcomm Snapdragon (SM7225) 8-Core`).
  - **Display Resolution**: Inspects DRM connector modes via `/sys/class/drm/` (e.g. `1080 × 2340 Display`).
  - **Bluetooth MAC**: Queries active BlueZ HCI controller hardware addresses.
  - **Battery Health**: Reads mobile PMIC fuel gauges (`qcom_qg` / `bms`), reporting design capacity, full capacity ratio, cycle count, and chemistry.
  - **Storage & Apps**: Calculates real filesystem utilization via `libc::statvfs` and enumerates installed desktop applications.
- **Network & Internet (NetworkManager)**:
  - Asynchronous wireless scanning with live signal strength and security capabilities.
  - Interactive WPA/WPA2 password prompt modal dialog with show/hide password toggle.
  - Virtual on-screen keyboard IPC signaling (`penumbra-keyboard-cmd show/hide`).
  - Beacon rescan trigger and hardware Wi-Fi power switch.
- **Connected Devices (BlueZ)**:
  - ObjectManager peripheral discovery and pairing inspection.
  - Live peripheral battery indicators.
  - Automatic inquiry scanning upon entering subpage.
- **Display & Backlight**:
  - Touch-scrubbing backlight luminance slider with smooth sysfs interpolation.
  - Adaptive ambient brightness toggle.
  - Dark theme mode toggle synchronized with GNOME desktop interface settings (`gsettings set org.gnome.desktop.interface color-scheme`).
  - Interactive screen timeout picker (15s, 30s, 1m, 2m, 5m, 10m, Never) synchronized with `SecurityConfig` and GNOME session `idle-delay`.
- **Sound & Vibration (PipeWire / pactl)**:
  - Media volume slider bound to PulseAudio/PipeWire `@DEFAULT_SINK@`.
  - Independent call volume and ring & notification volume channel configurations.
  - Do Not Disturb (DND) suppression toggle.
  - Call vibration haptic toggle.
  - Persistent state saved to `~/.config/penumbra/sound.json`.
- **Security & Privacy**:
  - Cross-distribution PIN, Password, Swipe, and None lock screen challenge setup.
  - Salted SHA-256 credential hashing without distribution-specific PAM wrappers.
  - Biometric fingerprint sensor detection via `fprintd`.
  - Configurable auto-lock inactivity timeouts.
- **Keyboard (Wayland & KDE Input)**:
  - Discovers installed Wayland/KDE virtual keyboards (`penumbra-keyboard`, `ibus`, etc.).
  - One-tap active virtual keyboard switching.
  - Physical keyboard hardware detection via `/proc/bus/input/devices`.
- **Single-Instance IPC**:
  - Exposes `org.penumbra.Settings` over session D-Bus.
  - Supports opening specific subpages directly from launcher shortcuts or shell widgets: `penumbra-settings -p wifi`, `penumbra-settings -p battery`, etc.

---

## Directory Structure

```text
crates/penumbra-settings/
├── Cargo.toml
├── build.rs
├── README.md
├── DOCUMENTATION.md
├── src/
│   ├── main.rs              # CLI entry point, single-instance D-Bus check, runtime setup
│   ├── app.rs               # Reactive UI coordinator, D-Bus subscriptions & event callbacks
│   ├── keyboard_backend.rs  # Virtual & physical keyboard discovery and switching
│   ├── routes.rs            # Page routing definitions and alias resolution
│   ├── search.rs            # Search engine & fuzzy settings catalog indexing
│   ├── system_info.rs       # Sysfs/Procfs hardware telemetry engine
│   └── dbus/
│       ├── mod.rs           # Single-instance server & backend aggregate
│       ├── network.rs       # NetworkManager D-Bus client & WPA connection logic
│       ├── bluetooth.rs     # BlueZ ObjectManager client & device discovery
│       └── power.rs         # UPower battery client & power-saver profile
└── ui/
    ├── settings_window.slint # Root Slint window & transition manager
    ├── theme.slint          # Material You color tokens, radii, metrics
    ├── icon.slint           # Semantic SVG vector icons
    ├── components/
    │   ├── settings_tile.slint    # Tactile rounded card list items
    │   ├── settings_section.slint # Grouping cards & headers
    │   └── top_bar_nav.slint      # Morphing Android back bar
    └── pages/
        ├── page_main.slint        # Root overview & live search list
        ├── page_wifi.slint        # Wi-Fi list, scanner & password modal
        ├── page_bluetooth.slint   # Bluetooth accessories & scanning
        ├── page_display.slint     # Backlight, theme & timeout picker
        ├── page_battery.slint     # Battery metrics & power profiles
        ├── page_sound.slint       # Volume mixer, DND & vibration
        ├── page_security.slint    # PIN/Password setup & biometric options
        ├── page_keyboard.slint    # Virtual & physical keyboards
        └── page_about.slint       # Hardware, SoC, kernel, memory & storage
```

---

## Building & Running

### Host Development (x86_64 / Native)

```bash
cargo build -p penumbra-settings
cargo run -p penumbra-settings -- --page main
```

### Direct Subpage Navigation via CLI

```bash
penumbra-settings --page wifi        # Opens Wi-Fi subpage
penumbra-settings --page bluetooth   # Opens Bluetooth subpage
penumbra-settings --page display     # Opens Display subpage
penumbra-settings --page battery     # Opens Battery subpage
penumbra-settings --page sound       # Opens Sound subpage
penumbra-settings --page security    # Opens Security & Privacy subpage
penumbra-settings --page keyboard    # Opens Keyboard subpage
penumbra-settings --page about       # Opens About Phone subpage
```

### ARM64 Cross-Compilation for Linux Phones (postmarketOS / Alpine Edge)

Using the Penumbra cross-compilation Docker container:

```bash
docker run --rm \
    --volume "$(pwd):/workspace" \
    --volume "$(pwd)/target/cross:/workspace/target" \
    --workdir /workspace \
    penumbra/aarch64-alpine:edge \
    cargo zigbuild --target aarch64-unknown-linux-musl --release -p penumbra-settings
```

Deploy to device via SSH:

```bash
scp target/cross/aarch64-unknown-linux-musl/release/penumbra-settings user@<device-ip>:/tmp/
ssh -tt user@<device-ip> "sudo cp /tmp/penumbra-settings /usr/local/bin/penumbra-settings && sudo chmod +x /usr/local/bin/penumbra-settings"
```

---

## License

Part of the Penumbra Mobile Shell environment. Licensed under the MIT or Apache-2.0 License.
