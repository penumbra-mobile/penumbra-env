# Convergent Settings & Displays Management Architecture

## Overview

The Penumbra Universal Settings application ([`penumbra-settings`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-settings)) provides a unified, adaptive system configuration interface that operates seamlessly across both mobile and desktop convergence modes. Built using **Slint UI** and adapted from the proven design patterns in [`third_party/cosmic-settings`](file:///home/nyl/coding/Penumbra-Shell/third_party/cosmic-settings), it automatically morphs its layout and interaction model based on screen geometry and output capabilities.

```
                      +-----------------------------+
                      |   penumbra-settings (Slint) |
                      +--------------+--------------+
                                     |
               +---------------------+---------------------+
               |                                           |
      (width < 720px)                             (width >= 720px)
     Mobile Phone Mode                           Desktop Window Mode
               |                                           |
  +------------v------------+                 +------------v------------+
  | Single-Column Drilldown |                 | Two-Column Master-Detail|
  | TopBarNav + Slide Stack |                 | Left 280px Nav Sidebar  |
  | Fullscreen OLED (#111318) |                 | In-place Category Pane  |
  +-------------------------+                 +-------------------------+
```

---

## 1. Convergent Master-Detail Architecture

### 1.1 Responsive Threshold (`root.width >= 720px`)
- **Phone Display**: Handheld portrait mode runs frameless and fullscreen (e.g. 412×915 logical canvas) with tactile thumb-sized touch targets and horizontal sliding page transitions (`subpage-slide`).
- **Desktop Display**: When opened on an external monitor or resized past 720px, it immediately transforms into a floating windowed desktop application:
  - **Left Sidebar Navigation ([`desktop_sidebar.slint`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-settings/ui/components/desktop_sidebar.slint))**: Pinned 280px sidebar containing quick search filtering and a vertical category list with active selection indicator pills.
  - **Right Content Area**: Renders the selected configuration page in-place with zero navigation latency.

---

## 2. Multi-Display Arrangement Canvas ([`display_canvas.slint`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-settings/ui/components/display_canvas.slint))

Adapted directly from COSMIC's `arrangement.rs`:
- **Visual Display Previews**: Renders scaled preview nodes representing the internal Phone panel and all connected external displays (e.g., TV / HDMI / DisplayPort).
- **Aspect Ratio Normalization**: Accurately computes whether each display is portrait (e.g., Phone 1080×2340) or landscape (e.g., External 1920×1080), reflecting physical orientation on the canvas.
- **Selection State**: Clicking any screen highlights it with an accent border, enabling detailed configuration of that specific output in the properties card below.
- **Visual Identifiers**: Displays clear numbering badges (1, 2, ...) and marks the primary display with a star indicator.

---

## 3. Convergence & Display Controls ([`page_display.slint`](file:///home/nyl/coding/Penumbra-Shell/crates/penumbra-settings/ui/pages/page_display.slint))

1. **Mirroring vs. Extended Desktop**:
   - Easily toggle between extending your desktop across multiple screens or mirroring the display output.
2. **Primary Display Selection**:
   - Choose which connected monitor serves as the primary desktop surface for dock placement and default application launching.
3. **Fractional Scaling Presets**:
   - Tactile quick-select scale chips: `100%`, `125%`, `150%`, `175%`, `200%`.
4. **Resolution & Refresh Rate**:
   - Displays real-time hardware mode info (e.g. `1920 × 1080 @ 60Hz`).
5. **Night Light & Color Warmth**:
   - Modeled after `cosmic-settings/src/pages/display/night_light.rs`.
   - Toggle night light on/off with custom color warmth from warm amber (2700K) to daylight (6500K).
   - Supports automated sunset-to-sunrise scheduling.
6. **Tactile Backlight Scrubbing & Timeout**:
   - Smooth touch-scrubbing brightness card and configurable screen idle timeout dialog.

---

## 4. Configuration Schema (`~/.config/penumbra/displays.toml`)

All multi-display arrangements, scaling, and night light preferences are persistently serialized to `~/.config/penumbra/displays.toml`:

```toml
arrangement_mode = "extended"

[[outputs]]
id = "DSI-1"
name = "Internal Phone Display"
connector = "DSI-1"
enabled = true
is_primary = true
pos_x = 0
pos_y = 0
width = 1080
height = 2340
refresh_rate_hz = 60
scale = 2.62
orientation_deg = 0

[[outputs]]
id = "DP-1"
name = "External Monitor / TV"
connector = "DP-1"
enabled = true
is_primary = false
pos_x = 1080
pos_y = 0
width = 1920
height = 1080
refresh_rate_hz = 60
scale = 1.0
orientation_deg = 0

[night_light]
enabled = false
temperature_k = 3800
schedule_mode = "sunset"
start_hour = 20
end_hour = 7

[desktop]
dock_position = "bottom"
dock_size_px = 64
dock_autohide = false
panel_size_px = 36
window_snapping = true
```
