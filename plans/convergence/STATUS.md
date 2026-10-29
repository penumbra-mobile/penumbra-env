# Penumbra Convergence Implementation Status

## Phase Tracker

| Phase | Description | Status | Verification & Target |
|---|---|---|---|
| **Phase 0** | Greetd Boot & LibSeat Integration | **Completed** | Fairphone 4 (`172.16.42.1`) boots cleanly into `penumbra-greeter` |
| **Phase 1** | Compositor Multi-Output Engine & KMS Hotplug Core | **Completed** | Adapted from `cosmic-comp`. Unit tests passed, cross-compiled for musl aarch64, deployed to device |
| **Phase 2** | Multi-Output Routing Policies & Disconnect App Migration | **Completed** | Pointer unclamping across $N$ displays, touch containment, focus routing, window drag, fallback cursor, overview card badge |
| **Phase 3** | Desktop Shell (`penumbra-desktop-shell`) | **Completed** | Slint UI top panel, dock, applets, and launcher deployed to device |
| **Phase 4** | Universal Adaptive `penumbra-settings` | **Completed** | Convergent 2-column desktop vs 1-column mobile, display arrangement canvas, night light, scaling presets |
| **Phase 5** | Out-of-Process Wireless Casting | Queued (Skipped for now) | PipeWire / `wlr-screencopy` streaming |
| **Phase 6** | DisplayLink & Hardware Hardening | Queued | USB-C docks, DP Alt Mode, audio routing |

---

## Third-Party Reference Matrix (`third_party/`)

All convergence work actively references the battle-tested implementations in `third_party/`:

| Component | Upstream Source in `third_party/` | Target in Penumbra | Primary Reference Points |
|---|---|---|---|
| KMS Multi-Output | [`third_party/cosmic-comp`](file:///home/nyl/coding/Penumbra-Shell/third_party/cosmic-comp) | [`crates/penumbra-comp`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-comp) | `src/backend/kms/`, `src/drm_helpers/` (`display_configuration`) |
| Window Manager & Policies | [`third_party/cosmic-comp`](file:///home/nyl/coding/Penumbra-Shell/third_party/cosmic-comp) | [`crates/penumbra-comp`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-comp) | `src/shell/toplevel.rs`, `src/shell/seat.rs`, `src/input/` |
| Panel & Dock Shell | [`third_party/cosmic-panel`](file:///home/nyl/coding/Penumbra-Shell/third_party/cosmic-panel) | `crates/penumbra-desktop-shell` | `cosmic-panel-bin/src/`, layer-shell `--output` binding, autohide |
| System Applets | [`third_party/cosmic-applets`](file:///home/nyl/coding/Penumbra-Shell/third_party/cosmic-applets) | `crates/penumbra-desktop-shell` | Workspaces, battery, network, audio, notifications |
| Adaptive Settings UI | [`third_party/cosmic-settings`](file:///home/nyl/coding/Penumbra-Shell/third_party/cosmic-settings) | [`crates/penumbra-settings`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-settings) | Displays canvas layout, mode sorting, responsive master-detail |
| Output Management CLI | [`third_party/cosmic-randr`](file:///home/nyl/coding/Penumbra-Shell/third_party/cosmic-randr) | `crates/penumbra-services` | `wlr-output-management-unstable-v1` client bindings |
| Protocols | [`third_party/cosmic-protocols`](file:///home/nyl/coding/Penumbra-Shell/third_party/cosmic-protocols) | `crates/penumbra-comp` | XML specifications for toplevels & workspaces |
| Application Launcher | [`third_party/cosmic-launcher`](file:///home/nyl/coding/Penumbra-Shell/third_party/cosmic-launcher) | `crates/penumbra-desktop-shell` | `.desktop` parsing, app indexing, search ranking |

---

## Phase 1 Execution Summary (Completed: $1\text{ Phone} + N\text{ External Displays}$)

1. **Adapted from `cosmic-comp`**:
   - `crates/penumbra-comp/src/outputs/connector.rs`: DRM connector enumeration, EDID physical extraction, and conflict-free CRTC allocation via `display_configuration`.
   - `crates/penumbra-comp/src/outputs/layout.rs`: Continuous 2D multi-display coordinate layout computation (`compute_default_multi_display_layout`) arranging Phone at `(0, 0)` and $N$ external monitors side-by-side (`X_k = 412 + sum(W_prev)`).
   - `crates/penumbra-comp/src/policy.rs`: `OutputRole::ExternalDesktop(String)` and `AsymmetricOutputManager` supporting arbitrary simultaneous external monitors with partial and total disconnect migration rules.
   - `crates/penumbra-comp/src/server.rs`: `external_outputs: BTreeMap<String, Output>` and `migrate_desktop_surfaces_from_output(...)`.
   - `crates/penumbra-comp/src/native.rs`:
     - Multi-output KMS surface representation (`KmsOutputSurface`) stored in `externals: HashMap<crtc::Handle, KmsOutputSurface>`.
     - Decoupled render loop: independent frame queues, DRM pageflip buffers, and VBlank retirement callbacks for primary and all connected external displays.
     - Real-time udev event source (`UdevBackend`) integrated into the `calloop` event loop to detect cable hotplug events (`UdevEvent::Changed`).
     - Element rendering in `render_external_frame` calculates window locations relative to `output_geo.loc`.
2. **Quality Assurance**:
   - 16 unit tests passed (`cargo test -p penumbra-comp --features native-drm`) verifying multi-display layout, hotplug, partial disconnect migration, and total disconnect fallback.
   - Clean cross-compilation for `aarch64-unknown-linux-musl` inside Docker edge container (31.14s).
   - Deployed release binary to `/usr/local/bin/penumbra-comp` (6.4 MB) on Fairphone 4.
   - Greetd restarted on device and active.

---

## Phase 2 Execution Summary (Completed: Multi-Output Routing & App Migration)

1. **Multi-Output Pointer Motion & Clamping (`native_input.rs`)**:
   - Implemented `clamp_pointer_motion`: computes global layout bounds $[0 \dots 412 + \sum W_{\text{externals}} - 1, 0 \dots \max(H_k) - 1]$ across all active displays. Mouse and touchpad pointers traverse seamlessly across all external monitors.
   - Strict touch containment: internal touch screen digitizer events (`TouchDown`, `TouchMotion`, `TouchUp`) remain strictly clamped to $[0 \dots 412, 0 \dots 915]$ via `clamp_to_canvas`. Phone touches never leak or trigger external desktop surfaces.
2. **Unified Focus Resolution (`server.rs`)**:
   - Implemented `focus_under(pos)`:
     - $pos.x < 412.0$: delegates to `phone_focus_under(pos)` (layer-shell insets, phone task stack, drawer).
     - $pos.x \ge 412.0$: delegates to `desktop_focus_under(pos)`: resolves containing external monitor, queries output's `layer_map` (`Overlay` $\to$ `Top`), then floating desktop windows in `space`, then `Bottom` $\to$ `Background`.
   - On `PointerButton`: clicking any desktop window raises it to top via `space.raise_element(&window, true)` and activates keyboard focus.
3. **Cursor Sprite Rendering (`native.rs`)**:
   - In `render_external_frame`: checks if `pointer_location` intersects the output geometry.
   - Client cursor support: renders `CursorImageStatus::Surface(surf)` using client-supplied hotspot offsets.
   - High-contrast fallback cursor: bundled 64×64 RGBA cursor bitmap rendered via `MemoryRenderBufferRenderElement` when default cursor is active.
4. **Boundary Window Transitions & Dragging (`server.rs`, `native_input.rs`)**:
   - Implemented `handle_window_drag_motion` and `move_request`:
     - Phone $\to$ Desktop: window dragged across $x = 412$ is removed from `phone_tasks`, configured to floating $(960, 640)$, un-maximized, and positioned freely on the external monitor.
     - Desktop $\to$ Phone: window dragged into $x < 412$ enters the phone task stack, maximizes to $(412, 835)$, positioned at $(0, 42)$, and switches shell mode to `App`.
5. **Overview Card Badges**:
   - In `publish_phone_tasks`: desktop tasks migrated upon total disconnect are automatically formatted as `🖥 Desktop Session • {Title}` in `penumbra-windows` and UI.
6. **Quality Assurance**:
   - 18 unit tests passed (`cargo test -p penumbra-comp --features native-drm`).
   - Cross-compiled release binary for `aarch64-unknown-linux-musl` in Docker container (41.65s).
   - Deployed `/usr/local/bin/penumbra-comp` (6.7 MB) to Fairphone 4 at `172.16.42.1`. Greetd verified running.

---

## Phase 3 Execution Summary (Completed: Native Wayland Desktop Shell)

1. **Zero-Winit Layer-Shell Integration (`layer-shell-slint`)**:
   - Enhanced `LayerConfig` with explicit `output_name: Option<String>` and `.with_output(...)`.
   - Binds layer surfaces directly to requested monitor names (`"DP-1"`, `"HDMI-A-1"`, or `"external"`) via SCTK `OutputState` while maintaining full backward-compatibility with phone shell components (`top_bar`, `nav_bar`, `homescreen`).
2. **Desktop Top Panel (`DesktopPanelWindow`)**:
   - Anchors to `Top | Left | Right` at height 36px with `exclusive_zone: 36px` on `Layer::Top`.
   - Branded Application Launcher button ("Apps") with subtle hover pill.
   - Interactive Workspaces Applet (`[ 1 ] [ 2 ] [ 3 ] [ 4 ]`) with active workspace highlight and switching.
   - Focused Window Title telemetry.
   - Center Clock & Date Applet (`HH:MM • EEE, MMM d`).
   - Quick Status Capsule: Wi-Fi connection indicator & SSID, Volume level percentage, Battery status indicator (percentage + charging state via `penumbra-services`).
   - Power / Lock Action button.
3. **Desktop Dock (`DesktopDockWindow`)**:
   - Anchors to `Bottom` at height 64px with `exclusive_zone: 64px` on `Layer::Top`.
   - Floating centered translucent glassmorphism container with specular border rim.
   - Launcher trigger icon, pre-pinned desktop essentials (Terminal, Web Browser, Files, Text Editor, Settings).
   - Running application indicators (active dot underneath open apps).
   - Smooth hover background and scale animations.
4. **Modal Application Launcher (`DesktopLauncherWindow`)**:
   - Fullscreen scrim backdrop on `Layer::Overlay` with floating centered modal chassis (720×520px).
   - Instant search input bar filtering installed `.desktop` applications in real time.
   - Category filter tabs (`All`, `Internet`, `Utilities`, `System`, `Development`, `Office`, `Multimedia`).
   - Application grid with vector icons, app title, category badge, and exec spawn on external display.
   - Zero-overhead dynamic Wayland input region: expands fullscreen when open, collapses to $0 \times 0$ click-through when closed.
5. **Quality Assurance & Target Deployment**:
   - 6/6 unit tests passed in `penumbra-desktop-shell`.
   - All workspace tests passed cleanly.
   - Cross-compiled release binary for `aarch64-unknown-linux-musl` inside Docker Alpine Edge container.
   - Deployed `/usr/local/bin/penumbra-desktop-shell` and updated `/usr/local/bin/penumbra-session` to Fairphone 4 at `172.16.42.1`.

6. **External Display Output Isolation Fix**:
   - **Compositor Guard ([server.rs](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-comp/src/server.rs))**: Updated `new_layer_surface` to strictly reject mapping any `desktop` / `dock` layer surface onto `output_a` (the internal phone panel). If no external monitor is connected, mapping is rejected with a warning.
   - **Client-Side Fail-Fast ([platform.rs](file:///home/nyl/coding/Penumbra-Shell/crates/layer-shell-slint/src/platform.rs))**: In `layer-shell-slint`, if an explicit target output (such as `"external"`) is requested and not found among available Wayland outputs, `create_window_adapter` fails fast with `Err(PlatformError::Other("Target output 'external' not found"))`.
   - **Graceful Hotplug Polling ([main.rs](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-desktop-shell/src/main.rs))**: Handled output discovery cleanly in `penumbra-desktop-shell`; when no external display is plugged in, it exits with code 0 to wait for display hotplug without polluting logs.
   - **Device Verification**: Deployed updated `penumbra-comp` and `penumbra-desktop-shell` to Fairphone 4 (`172.16.42.1`). Verified live that the phone screen shows strictly the mobile environment (`penumbra-homescreen`, `penumbra-shell`, and `penumbra-keyboard`) with zero desktop panels or dock surfaces interfering with mobile UI or touch.

---

## Phase 4 Execution Summary (Completed: Convergent Settings & Displays Management)

1. **Adapted from `cosmic-settings`**:
   - `third_party/cosmic-settings/cosmic-settings/src/pages/display/arrangement.rs`: Multi-display arrangement canvas coordinate layout, drag/reposition math, and aspect ratio normalization.
   - `third_party/cosmic-settings/cosmic-settings/src/pages/display/mod.rs`: Fractional scaling presets (`100%`, `125%`, `150%`, `175%`, `200%`), native mode highlighting, primary display toggle, and display mirroring vs. extended desktop.
   - `third_party/cosmic-settings/cosmic-settings/src/pages/display/night_light.rs`: Night Light temperature slider (warm amber 2700K to daylight 6500K) and sunset-to-sunrise scheduling.
   - `third_party/cosmic-settings/cosmic-settings/src/page.rs`: Two-column Master-Detail responsive split-view navigation architecture.
2. **Multi-Display Schema & TOML Storage ([`penumbra-services`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-services))**:
   - Enhanced [`display.rs`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-services/src/display.rs) with `DisplayOutputInfo`, `DisplayModeInfo`, `NightLightConfig`, `DesktopCustomizationConfig`, and `DisplaysConfig`.
   - Configuration persists to `~/.config/penumbra/displays.toml`.
3. **Slint UI Convergent Components ([`penumbra-settings/ui`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-settings/ui))**:
   - [`display_canvas.slint`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-settings/ui/components/display_canvas.slint): Interactive 2D multi-display preview canvas with aspect ratio geometry, display number badge, primary star indicator, and selection state.
   - [`desktop_sidebar.slint`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-settings/ui/components/desktop_sidebar.slint): Category sidebar with search filter, active selection pill, and 8 top-level categories.
   - [`page_display.slint`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-settings/ui/pages/page_display.slint): Upgraded with Display Arrangement Canvas, Mirroring switch, Primary display toggle, Resolution & Refresh Rate badge, Fractional Scaling presets (`100%`, `125%`, `150%`, `175%`, `200%`), and Night Light controls.
   - [`settings_window.slint`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-settings/ui/settings_window.slint): Responsive adaptive layout (`is-desktop: root.width >= 720px`). Renders two-column Master-Detail window in desktop convergence mode and 1-column touch drilldown in phone mode.
4. **Testing & Target Deployment**:
   - All 16 unit tests in `penumbra-services` passed.
   - All 8 unit tests in `penumbra-settings` passed (including responsive desktop/mobile geometry and displays model binding).
   - All workspace tests passed cleanly.
   - Cross-compiled release binary for `aarch64-unknown-linux-musl` inside Docker Alpine Edge container.
   - Deployed `/usr/local/bin/penumbra-settings` to Fairphone 4 at `172.16.42.1`.

5. **Phone Mode Touch Refinements & Window Decoration Removal**:
   - **Removal of Client-Side Window Frame (`no-frame: true`)**: Configured Slint `Window` with `no-frame: true;` removing SCTK's default client-side decorations (`_ [] X`) on mobile. In phone mode, settings is completely borderless and edge-to-edge.
   - **Desktop-Only CSD Window Controls**: Desktop mode (`is-desktop: true`) renders its own sleek integrated header bar close control (`✕`) wired to clean window exit.
   - **Left-Edge Swipe-to-Go-Back Gesture**: Slint `TouchArea` attached to the left margin (36px width, `z: 999`) detects horizontal rightward swipes past 48px to navigate back to the main menu without interfering with vertical flick scrolling.
   - **Expanded Touch Targets**:
     - `TopBarNav` back button hit area enlarged to 56×56px with a 44px circular tactile pressed feedback state.
     - Display scaling selector pills enlarged to 44px height with tactile pressed states.
   - **COSMIC-Settings Features Implemented**:
     - **Power Profiles**: Performance, Balanced, and Power Saver interactive cards integrated with `powerprofilesctl` and local fallback in [`penumbra-services::battery`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-services/src/battery.rs).
     - **Audio Routing & Mute Controls**: Output device selection (Internal Speaker, Wired Headset, HDMI Audio), Master Mute switch, and Microphone input volume card with toggle mute in [`penumbra-services::sound`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-services/src/sound.rs) and `PageSound`.
     - **Desktop Accent Color Selection**: 5 color swatches (Ocean Blue, Lavender, Mint, Rose, Amber) with active checkmark badge and persistence in `DisplayConfig`.

