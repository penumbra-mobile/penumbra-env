# Penumbra Settings: Technical Architecture & Developer Manual

This document provides a comprehensive technical overview of the `penumbra-settings` architecture, data pipelines, hardware integrations, configuration persistence, and UI component hierarchy.

---

## 1. System Architecture & Threading Model

`penumbra-settings` utilizes a decoupled asynchronous threading model to ensure that hardware D-Bus queries, network scanning, and I/O bottlenecks never stutter or block the main 60/120 Hz Slint UI rendering pipeline.

```mermaid
graph TD
    subgraph UI_Thread ["Main Thread (Slint Event Loop)"]
        UI[SettingsWindow Component]
        EventHandlers[Callback Dispatchers]
        SlintLoop[Slint Event Loop / Winit]
    end

    subgraph Tokio_Runtime ["Asynchronous Tokio Runtime"]
        NetTask[NetworkManager Worker Task]
        BtTask[BlueZ ObjectManager Task]
        PowerTask[UPower Battery Task]
        DbusService[org.penumbra.Settings Server]
    end

    subgraph Linux_Hardware ["Linux Subsystems & Hardware"]
        NM[NetworkManager D-Bus]
        BlueZ[BlueZ D-Bus]
        UPower[UPower D-Bus]
        PipeWire[PulseAudio / PipeWire pactl]
        SysfsBacklight[/sys/class/backlight]
        SysfsBattery[/sys/class/power_supply/qcom_qg]
        SocSysfs[/sys/devices/soc0]
        DeviceTree[/proc/device-tree/model]
    end

    UI -->|User Touch Events| EventHandlers
    EventHandlers -->|rt_handle.spawn| Tokio_Runtime
    Tokio_Runtime -->|zbus calls / sysfs writes| Linux_Hardware

    NM -->|State updates| NetTask
    BlueZ -->|Device added/removed| BtTask
    UPower -->|PropertiesChanged| PowerTask

    NetTask -->|slint::invoke_from_event_loop| UI
    BtTask -->|slint::invoke_from_event_loop| UI
    PowerTask -->|slint::invoke_from_event_loop| UI
```

### Event Handover via `slint::invoke_from_event_loop`
Because Slint UI state is owned exclusively by the main thread:
1. Asynchronous background workers receive updates from system D-Bus services or sysfs polls.
2. Background workers capture an `slint::Weak<SettingsWindow>` reference.
3. State is bridged into the UI loop using `slint::invoke_from_event_loop(move || { if let Some(ui) = weak.upgrade() { ... } })`.

---

## 2. Hardware Subsystems & D-Bus Clients

### 2.1 Network & Wi-Fi (`src/dbus/network.rs`)
Manages connection state and beacon scanning via `org.freedesktop.NetworkManager`.

- **Wireless Device Resolution**: Queries `/org/freedesktop/NetworkManager` for devices of type `NM_DEVICE_TYPE_WIFI` (2).
- **Periodic Beacon Scanning**: Background loop triggers `RequestScan({})` on `WirelessDeviceProxy` at 10-second intervals and emits an updated `WifiApData` model.
- **Connection Logic**:
  - Checks existing configured profiles via `SettingsProxy::ListConnections`. If a connection for the target SSID already exists, it is activated via `ActivateConnection`.
  - For new secured networks, `AddAndActivateConnection` is invoked with `802-11-wireless` and `802-11-wireless-security` maps (`key-mgmt: "wpa-psk"`, `psk: password`).
- **Interactive Password Modal**: Secured APs prompt the user with a modal dialog in `PageWifi`, automatically signaling `$XDG_RUNTIME_DIR/penumbra-keyboard-cmd show` to deploy the virtual keyboard.

### 2.2 Bluetooth & Peripheral Management (`src/dbus/bluetooth.rs`)
Integrates with the BlueZ system daemon via the D-Bus `ObjectManager` interface (`org.freedesktop.DBus.ObjectManager`).

- **Adapter Discovery**: Automatically discovers the primary HCI adapter under `/org/bluez/hci0`.
- **Dynamic Device Tracking**: Subscribes to `InterfacesAdded` and `InterfacesRemoved` signals to reactively update paired, connected, and RSSI metrics for nearby devices.
- **Battery Reporting**: Inspects `org.bluez.Battery1.Percentage` to display real-time charge indicators for connected Bluetooth headsets and accessories.
- **Lifecycle Discovery**: Navigation into the `bluetooth` subpage triggers `StartDiscovery()`; leaving the subpage triggers `StopDiscovery()`.

### 2.3 Power & Battery Telemetry (`src/dbus/power.rs` & `src/system_info.rs`)
Combines UPower high-level metrics with deep mobile hardware PMIC sysfs telemetry.

- **UPower Integration**: Listens on `/org/freedesktop/UPower/devices/DisplayDevice` for battery percentage, AC online state, time to empty/full, and power-saver profiles.
- **Qualcomm PMIC Fuel Gauge**: Mobile devices (like Qualcomm SM7225 on the Fairphone 4) expose fuel gauges at `/sys/class/power_supply/qcom_qg/` or `bms`. The engine parses:
  - `charge_full` vs `charge_full_design` to compute exact capacity health percentage.
  - `cycle_count` and battery chemistry (`technology`).
  - Nominal mAh capacity rating (`charge_full_design / 1000`).

### 2.4 Display & Backlight (`src/system_info.rs` & `crates/penumbra-services/src/backlight.rs`)
- **Sysfs Backlight Control**: Discovers the active backlight node under `/sys/class/backlight/*/` (e.g. `backlight_lcd`).
- **Ratio Normalization**: Converts raw integer brightness into a normalized ratio `[0.05..=1.0]` based on `max_brightness`.
- **Display Resolution**: Inspects connected DRM modes in `/sys/class/drm/card*-*/modes` to display the native screen resolution (e.g. `1080 × 2340 Display`).

### 2.5 Audio Mixer & Volume Channels (`crates/penumbra-services/src/audio.rs`)
- Interacts with PulseAudio / PipeWire via `pactl get-sink-volume @DEFAULT_SINK@` and `pactl set-sink-volume @DEFAULT_SINK@ <pct>%`.
- Distinct channels are managed: Media volume (hardware sink), Call volume, and Ring/Notification volume.

### 2.6 ARM SoC & DeviceTree Detection (`src/system_info.rs`)
- **ARM Device Model**: Reads `/proc/device-tree/model` (e.g. `Fairphone 4\0`). Falls back to DMI product name on x86_64 systems.
- **Qualcomm SoC Information**: Reads `/sys/devices/soc0/machine` (`SM7225`) and `/sys/devices/soc0/family` (`Snapdragon`) to produce rich processor badges without relying on `/proc/cpuinfo` `model name`.

---

## 3. Persistent Configuration Architecture

Penumbra settings persist state across reboots using unified JSON documents stored under `$XDG_CONFIG_HOME/penumbra/` (`~/.config/penumbra/`).

### 3.1 Security Configuration (`~/.config/penumbra/security.json`)
Managed by `SecurityConfig` in `penumbra-services`:
```json
{
  "lock_type": "swipe",
  "secret_hash": "",
  "salt": "",
  "fingerprint_enabled": false,
  "auto_lock_timeout_secs": 300,
  "hide_sensitive_notifications": false
}
```
- Shared across `penumbra-shell`, `penumbra-greeter`, and `penumbra-settings`.
- Changing the screen timeout in Display Settings immediately updates `auto_lock_timeout_secs`.

### 3.2 Display Configuration (`~/.config/penumbra/display.json`)
Managed by `DisplayConfig` in `penumbra-services`:
```json
{
  "brightness": 0.75,
  "adaptive_brightness": true,
  "dark_theme": true,
  "screen_timeout_secs": 300
}
```
- Calling `DisplayConfig::apply_gsettings()` executes:
  - `gsettings set org.gnome.desktop.interface color-scheme 'prefer-dark'`
  - `gsettings set org.gnome.desktop.interface gtk-theme 'Adwaita-dark'`
  - `gsettings set org.gnome.desktop.session idle-delay <secs>`

### 3.3 Sound Configuration (`~/.config/penumbra/sound.json`)
Managed by `SoundConfig` in `penumbra-services`:
```json
{
  "media_volume": 0.20,
  "call_volume": 0.80,
  "ring_volume": 0.70,
  "dnd_enabled": false,
  "vibrate_for_calls": true
}
```

---

## 4. UI Architecture & Slint Component Hierarchy

The UI is compiled ahead-of-time into native C++/Rust code via `slint-build`.

```text
SettingsWindow (Window: 412x841, Theme.base-surface = #0D0B10)
├── FocusScope (Global Escape key handling for backward navigation)
└── VerticalLayout
    ├── TopBarNav (Morphing title, Android back chevron button)
    └── Rectangle (Viewport Stack)
        ├── Rectangle (Main Catalog View: Parallax slide 0 to -80px)
        │   └── PageMain
        │       ├── Search Header Pill / Active Search Header
        │       ├── Dynamic Search Results List (When searching)
        │       └── Android 17 Root Overview Cards (Network, Apps, Sound, Display, Security, Storage, About)
        └── Rectangle (Subpage Sliding Container: Slide from +100% to 0%)
            ├── PageWifi (Access points list, power switch, WPA password modal)
            ├── PageBluetooth (Adapter toggle, accessories list, discovery indicator)
            ├── PageDisplay (Brightness scrub card, toggles, screen timeout modal picker)
            ├── PageBattery (Percentage, charging text, health info, power-saver toggle)
            ├── PageSound (Media, call, ring volume slider cards, DND, vibration toggle)
            ├── PageSecurity (PIN/Password setup views, fprintd biometrics toggle)
            ├── PageKeyboard (Wayland/KDE virtual keyboard switcher, physical inputs)
            └── PageAbout (Hardware model, Qualcomm SoC, kernel, RAM, storage)
```

### 4.1 Modal Dialog Pattern
Modals (such as the Wi-Fi Password prompt and the Screen Timeout picker) are implemented directly over subpage viewports:
1. Outer `Rectangle` with full width/height, semi-transparent background scrim (`#000000a6`).
2. An empty `TouchArea {}` blocker to intercept touch events and prevent accidental background clicks.
3. Centered modal card (`#25232a`, 28px border radius, high-contrast buttons).
4. For text inputs, focus triggers `password-input-focused` which signals the virtual keyboard daemon.

### 4.2 On-Screen Keyboard IPC Protocol
To guarantee that virtual keyboards automatically pop up when inputs are selected:
- Upon focusing a text input: writes `"show"` to `$XDG_RUNTIME_DIR/penumbra-keyboard-cmd`.
- Upon dismissing or connecting: writes `"hide"` to `$XDG_RUNTIME_DIR/penumbra-keyboard-cmd`.

---

## 5. Single-Instance IPC (`org.penumbra.Settings`)

`penumbra-settings` registers an IPC service on the session D-Bus bus:
- **Service Name**: `org.penumbra.Settings`
- **Object Path**: `/org/penumbra/Settings`
- **Interface**: `org.penumbra.Settings`

```rust
#[zbus::interface(name = "org.penumbra.Settings")]
impl SettingsServer {
    async fn open_page(&self, page: String) -> zbus::fdo::Result<()> {
        // Navigates active instance to requested subpage and un-minimizes window
    }
}
```

If a user launches `penumbra-settings --page wifi` while the app is already open in the background, the secondary process invokes `open_page("wifi")` over D-Bus and exits cleanly, immediately bringing the existing instance into view.

---

## 6. Testing & Quality Assurance

### Unit Tests
The crate includes automated test suites covering routing aliases, fuzzy search scoring, and keyboard detection:
```bash
cargo test -p penumbra-settings
```

Key test modules:
- `routes::tests`: Verifies canonical subpage routing and command-line aliases (`bt` -> `bluetooth`, `screen` -> `display`, etc.).
- `search::tests`: Verifies search ranking, multi-category matching, and category header generation.

### Live On-Device Diagnostics
To test `penumbra-settings` on a physical target:
```bash
# Verify D-Bus service registration
busctl --user status org.penumbra.Settings

# Trigger subpage navigation over D-Bus
busctl --user call org.penumbra.Settings /org/penumbra/Settings org.penumbra.Settings OpenPage s "wifi"
```
