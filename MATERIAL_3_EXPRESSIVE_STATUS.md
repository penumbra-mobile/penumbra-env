# Material 3 Expressive (M3E) Implementation Status

This ledger tracks implemented phases, architectural milestones, commit hashes, and verification outcomes for the Material 3 Expressive mobile desktop experience in **Penumbra Mobile DE**. Any agent or contributor working on this repository should inspect this file to understand the current progress and active foundation.

---

## Progress Overview

| Phase | Milestone | Status | Key Targets | Tests / Build |
| :--- | :--- | :--- | :--- | :--- |
| **Foundation 0A** | Progressive Dual-Kawase Compositor Blur |  **Completed** | `crates/penumbra-comp/src/render/*` | DRM scanout GL pass verified |
| **Foundation 0B** | Layer-Shell Slint Alpha Zero-Fill |  **Completed** | `crates/layer-shell-slint/src/*` | True transparency verified on DRM |
| **Phase 1** | M3E 12-Tier Container & Optical Palette |  **Completed** | `crates/penumbra-shell/ui/theme.slint` | 48/48 cargo tests passed |
| **Phase 2** | Expressive QuickSettings & Morphing Pastilles |  **Completed** | `crates/penumbra-shell/ui/quick_settings*` | 48/48 cargo tests passed |
| **Phase 3** | Fluid Elastic Sliders (Logarithmic Bounds) |  **Completed** | `crates/penumbra-shell/ui/quick_settings*` | 48/48 cargo tests passed |
| **Phase 4** | Dynamic Monet Theming (Wallpaper Color Extraction) | ⏳ *In Progress* | `crates/penumbra-services/`, `penumbra-shell/src/config.rs` | Pending |
| **Phase 5** | Homescreen & Floating System Insets Polish | 📅 *Queued* | `crates/penumbra-shell/ui/homescreen.slint`, `app_surface.slint` | Pending |

---

## Detailed Ledger of Completed Work

### 1. Foundation 0A: Compositor Offscreen Capture & Dual-Kawase Blur
- **Commit**: `69d5f1b` (*Add dual-kawase blur rendering pipeline and quick settings backdrop compositor*)
- **Implementation**:
  - [`crates/penumbra-comp/src/render/quad.vert`](crates/penumbra-comp/src/render/quad.vert): Screen-aligned NDC billboard quad vertex shader with UV coordinates.
  - [`crates/penumbra-comp/src/render/kawase_down.frag`](crates/penumbra-comp/src/render/kawase_down.frag): 4-tap diagonal downsample pass scaled to $\frac{1}{4}$ resolution ($103 \times 228$).
  - [`crates/penumbra-comp/src/render/kawase_blur.frag`](crates/penumbra-comp/src/render/kawase_blur.frag): Dual-Kawase dispersion blur shader for Gaussian-approximate diffusion without GPU stalls.
  - [`crates/penumbra-comp/src/render/quicksettings_backdrop.frag`](crates/penumbra-comp/src/render/quicksettings_backdrop.frag): Progressive cross-fade shader combining sharp scene capture and diffuse field via Hermite interpolation:
    $$\alpha_{\text{blend}} = \text{smoothstep}(0.05, 0.95, u\_progress)$$
    with progressive organic pitch-black scrim tinting (`#0b0a0e`, $\alpha = t \cdot 0.40$).
  - [`crates/penumbra-comp/src/render/quicksettings.rs`](crates/penumbra-comp/src/render/quicksettings.rs): `QuickSettingsBackdropPipeline` with lazy FBO allocation, texture parameter clamping/linear filtering, and atomic FBO/viewport state preservation.
  - [`crates/penumbra-comp/src/server.rs`](crates/penumbra-comp/src/server.rs) & [`crates/penumbra-comp/src/native.rs`](crates/penumbra-comp/src/native.rs): Wired scene capture into `scene_tex` before main render pass and blended directly onto DRM scanout framebuffer.

### 2. Foundation 0B: Wayland Layer-Shell Alpha Transparency
- **Commit**: `521a209` (*align navigation gesture touch zone strictly with navbar height*)
- **Implementation**:
  - [`crates/layer-shell-slint/src/shm.rs`](crates/layer-shell-slint/src/shm.rs): Overrode `TargetPixel::background() -> Argb8888Pixel { b: 0, g: 0, r: 0, a: 0 }` so the Slint software renderer clears to fully transparent instead of defaulting to opaque black ($a = 255$).
  - [`crates/layer-shell-slint/src/window.rs`](crates/layer-shell-slint/src/window.rs): Added `pixels.fill(Argb8888Pixel::default())` prior to rendering to guarantee shared memory buffers are zero-filled.
  - [`crates/penumbra-shell/ui/top_bar.slint`](crates/penumbra-shell/ui/top_bar.slint) & [`crates/penumbra-shell/ui/status_bar.slint`](crates/penumbra-shell/ui/status_bar.slint): Set top bar and status bar backgrounds to `transparent`.
  - [`crates/penumbra-shell/ui/quick_settings.slint`](crates/penumbra-shell/ui/quick_settings.slint), [`shell_root.slint`](crates/penumbra-shell/ui/shell_root.slint), and [`quick_settings_slider.slint`](crates/penumbra-shell/ui/quick_settings_slider.slint): Set outer chassis backgrounds to `transparent` so the compositor's progressive blur backdrop shows through.

### 3. Phase 1: M3E Foundations & Optical Hierarchy
- **Commit**: `022a63a` (*feat(ui): implement Material 3 Expressive 12-tier container tokens and specular glass highlights*)
- **Implementation**:
  - [`crates/penumbra-shell/ui/theme.slint`](crates/penumbra-shell/ui/theme.slint):
    - **12-Tier Container Roles**:
      - `surface` (`#141218`), `surface-dim` (`#110f15`), `surface-bright` (`#38353d`)
      - `surface-container-lowest` (`#0b0a0e`, 0dp OLED pitch black)
      - `surface-container-low` (`#17151d`, 1dp card container)
      - `surface-container` (`#1e1b24`, 2dp resting sheet/pastille)
      - `surface-container-high` (`#27232e`, 3dp elevated tile/slider chassis)
      - `surface-container-highest` (`#36343b`, 4dp high-emphasis floating capsule)
      - `surface-variant` (`#44474f`, dividers and chips)
      - Backward-compatible aliases maintained (`base-surface`, `surface-lowest`, `surface-low`, etc.)
    - **Dynamic Tonal Accents**:
      - Primary Iris Periwinkle: `primary` (`#ADC6FF`), `on-primary` (`#002E69`), `primary-container` (`#254070`), `on-primary-container` (`#D8E2FF`)
      - Secondary Slate: `secondary` (`#BFC6DC`), `on-secondary` (`#283141`), `secondary-container` (`#3E4758`), `on-secondary-container` (`#DBE2F9`)
      - Tertiary Wildflower Rose: `tertiary` (`#EFB8C8`), `on-tertiary` (`#492532`), `tertiary-container` (`#633B48`), `on-tertiary-container` (`#FFD8E4`)
      - Error Crimson: `error` (`#FFB4AB`), `on-error` (`#690005`), `error-container` (`#93000A`), `on-error-container` (`#FFDAD6`)
    - **Specular Rim Tokens**:
      - `specular-rim`: `#ffffff14` ($8\%$ white glass specular edge)
      - `specular-rim-active`: `#ffffff33` ($20\%$ active component specular edge)
      - `specular-rim-glow`: `#ffffff50` ($31\%$ focus highlight edge)
      - `glass-scrim-thin/medium/deep`: Organic backdrop scrim overlays
    - **Expressive Radii & Durations**:
      - `radius-pill` (`9999px`), `radius-card` (`32px`), `radius-tile` (`28px`), `radius-tile-squash` (`22px`), `radius-item` (`16px`)
      - Canonical motion durations: `duration-micro` ($100\text{ms}$), `duration-short` ($150\text{ms}$), `duration-medium` ($300\text{ms}$), `duration-long` ($500\text{ms}$)
  - **Component Upgrades**:
    - [`crates/penumbra-shell/ui/dock.slint`](crates/penumbra-shell/ui/dock.slint): Elevated shelf using `surface-container-high`, `error-container` rejection styling, and `specular-rim`.
    - [`crates/penumbra-shell/ui/app_surface.slint`](crates/penumbra-shell/ui/app_surface.slint): Scrim overlay, weather pill, floating search capsule, and folder modal close button updated.
    - [`crates/penumbra-shell/ui/status_bar.slint`](crates/penumbra-shell/ui/status_bar.slint) & [`crates/penumbra-shell/ui/quick_settings_slider.slint`](crates/penumbra-shell/ui/quick_settings_slider.slint): Battery threshold error styling and pull indicator updated.
- **Verification**:
  - `cargo check --workspace`: Passed.
  - `cargo check --workspace --features native-drm`: Passed.
  - `cargo test --workspace`: **All 48 tests passed**.

### 4. Phase 2: Expressive QuickSettings & Morphing Pastilles
- **Commit**: `c998800` (*feat(qs): implement Phase 2 Material 3 Expressive pastilles, touch squash physics, and swipe-to-dismiss notifications*)
- **Implementation**:
  - **Tactile Touch Squash Physics ($0.96\times$)**:
    - [`crates/penumbra-shell/ui/quick_settings_slider.slint`](crates/penumbra-shell/ui/quick_settings_slider.slint) (`TileView`): Inner tile container dynamically insets by $2.5\text{px}$ horizontally and $1.5\text{px}$ vertically on pointer down, squashing from `radius-tile` ($28\text{px}$) to `radius-tile-squash` ($22\text{px}$) with cubic-bezier spring release.
    - [`crates/penumbra-shell/ui/quick_settings.slint`](crates/penumbra-shell/ui/quick_settings.slint) (`QuickToggle`): Added identical $0.96\times$ touch compression and brightness elevation feedback.
  - **Two-Stage Shape Morphing**:
    - Compact QQS ($y \le 300\text{px}$): 4 circular capsules across with centered icon badges.
    - Expanded QS ($y > 300\text{px}$): Smoothly transforms into 2-column pastilles ($185 \times 64\text{px}$) with the icon smoothly gliding to the left margin ($12\text{px}$) and primary title/subtitle cross-fading in.
  - **Stacked Notification Pastilles with Swipe-to-Dismiss**:
    - Added `NotificationCard` with individual elevation card geometry (`surface-container-low`), $16\text{px}$ item radius, and specular rim.
    - Interactive horizontal touch tracking ($dx > 100\text{px}$) with continuous alpha fadeout and dismissal.
    - "Clear all" rounded pill chip with active press ripple feedback.
    - Fully wired `notification-dismissed` and `clear-notifications` callbacks in [`crates/penumbra-shell/src/bin/quick_settings.rs`](crates/penumbra-shell/src/bin/quick_settings.rs) against dynamic `Rc<VecModel<NotificationData>>`.
- **Verification**:
  - `cargo check --workspace`: Passed.
  - `cargo check --workspace --features native-drm`: Passed.
  - `cargo test --workspace`: **All 48 tests passed**.

### 5. Phase 3: Fluid Elastic Sliders with Overscroll Resistance
- **Commit**: `6aecf01` (*feat(sliders): implement Phase 3 fluid elastic sliders with boundary resistance and icon state morphing*)
- **Implementation**:
  - **Boundary Elastic Stretch**:
    - [`crates/penumbra-shell/ui/quick_settings_slider.slint`](crates/penumbra-shell/ui/quick_settings_slider.slint) & [`crates/penumbra-shell/ui/quick_settings.slint`](crates/penumbra-shell/ui/quick_settings.slint): When dragged beyond $0.0$ or $1.0$, sliders apply smooth compressive boundary tension:
      $$\Delta x_{\text{visual}} = \text{sign}(\Delta x) \cdot \sqrt{|\Delta x| \cdot 1.5}$$
    - On touch release, snaps back to boundaries with an underdamped cubic-bezier curve (`Theme.duration-short`).
    - The active fill bar expands continuously into the boundary overscroll space.
  - **Dynamic Inset Text Contrast**:
    - Percentage text flips from `Theme.on-surface-variant` to `Theme.on-primary` when fill exceeds $88\%$.
    - Center label flips from `Theme.on-surface` to `Theme.on-primary` when fill exceeds $38\%$.
    - Icon flips from `Theme.on-surface` to `Theme.on-primary` when fill exceeds $8\%$.
  - **Icon State Morphing**:
    - [`crates/penumbra-shell/ui/icon.slint`](crates/penumbra-shell/ui/icon.slint): Added `brightness-low`, `volume-mute`, `volume-low`, and `bell` vector SVG paths.
    - Brightness slider dynamically switches between low and high radiating ray patterns based on fill ratio ($< 0.35$ vs $\ge 0.35$).
    - Volume slider dynamically animates from `volume-mute` (slashed X) to `volume-low` (single wave) to `volume` (double waves).
- **Verification**:
  - `cargo check --workspace`: Passed.
  - `cargo check --workspace --features native-drm`: Passed.
  - `cargo test --workspace`: **All 48 tests passed**.

---

## Active Plan Roadmap (Next Phases)

### Phase 4: Dynamic Monet Theming
- [ ] Color quantizer service in `penumbra-services` sampling 100 pixels from wallpaper.
- [ ] IPC theme publishing over `/run/user/<uid>/penumbra-theme.json` to update Slint properties live.

### Phase 5: Homescreen & Floating System Insets Polish
- [ ] Floating capsule search bar with Google "G" badge, mic pill, lens search.
- [ ] Squircle folder expansion animation ($32\text{px}$ radius).
- [ ] Uniform $24\text{px}$ app squircle mask alignment.
