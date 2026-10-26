# Android 17 Material 3 Expressive Quick Settings Architecture (Phase 7)
## Penumbra Mobile DE — Pixel Experience Architectural Blueprint

---

## 1. Executive Summary

This document specifies the authoritative, clean-slate re-implementation of the **Quick Settings & Notification Center** for the Penumbra Mobile Desktop Environment, aligning with **Android 17 Material 3 Expressive (Pixel Experience)** design guidelines and system architecture.

Prior iterations suffered from fragmented gesture arbitration, where multiple independent processes (`penumbra-homescreen`, `penumbra-top-bar`, and `penumbra-comp`) competed for touch ownership, resulting in:
1. **RefCell Double-Borrow Panics**: Crashing the consolidated `penumbra-shell` process and orphaning the screen without a top status bar or bottom navigation bar.
2. **Gesture Crosstalk**: Homescreen drag-and-drop / page scrolling handlers intercepting vertical pulls and driving divergent drawer state.
3. **Wayland Surface Boundary Clipping**: Layer-shell surfaces ($42\text{px}$) losing pointer focus whenever touch coordinates crossed window borders.

Phase 7 establishes a single-authority, compositor-driven gesture architecture paired with an expressive dual-state morphing UI and an automated session supervisor.

---

## 2. System Architecture & Component Roles

```
┌────────────────────────────────────────────────────────────────────────┐
│                        Hardware Touchscreen (DSI)                      │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │ 120Hz libinput events
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│                 penumbra-comp (Wayland Compositor)                     │
│                                                                        │
│  • Single-Authority Input Routing (native_input.rs)                   │
│    - Status Bar Pull-Down Tracker (ShadeTouchTracker)                  │
│    - Dual-Stage Progressive Boundary Resistance (0..320..915px)        │
│    - Android 17 Velocity-Sensitive Snap Decision Engine               │
│                                                                        │
│  • Visual Scene Engine (render/ & native.rs)                           │
│    - Dual-Kawase Gaussian Progressive Backdrop Blur (4-tap downsample) │
│    - Organic Wallpaper Depth Zoom (1.0x -> 0.92x centered)             │
│    - Hermite Dimming Scrim Cross-Fade (alpha: 0.0 -> 0.25)             │
└───────────────────┬────────────────────────────────┬───────────────────┘
                    │                                │
    Zero-Copy UDS   │ state datagrams (120Hz)        │ wl_surface commit
                    ▼                                ▼
┌───────────────────────────────────────┐ ┌──────────────────────────────┐
│       penumbra-shell (SystemUI)       │ │     penumbra-homescreen      │
│                                       │ │                              │
│  • TopBarWindow (Layer::Top, 42px)    │ │  • Pure Application Grid    │
│    - Shared-element clock dissolve    │ │  • Horizontal Page Carousel  │
│    - Hardware telemetry badges        │ │  • Dock & Folder System      │
│                                       │ │                              │
│  • QuickSettingsWindow (Overlay, 915) │ │  (NO drawer gesture hooks;   │
│    - Stage 1: Compact QQS (0..350px)  │ │   zero state interference)   │
│    - Stage 2: Expanded QS (350..915px)│ └──────────────────────────────┘
│    - Morphing pastille tile grid      │
│    - Fluid logarithmic sliders        │
│    - Swipe-to-dismiss notifications   │
│                                       │
│  • NavBarWindow (Layer::Top, 32px)    │
│    - Home pill, Back, Keyboard toggle │
└───────────────────────────────────────┘
```

---

## 3. The Five Architectural Pillars

### Pillar 1: Single-Authority Compositor Gesture Pipeline
In Android SystemUI and WindowManager, system-wide edge gestures are intercepted before client surfaces receive them.
- **Top Edge Capture**: Swipes originating at $y \le 42.0\text{px}$ are intercepted exclusively by `ShadeTouchTracker` in `native_input.rs`. Client surfaces never receive partial touch events, preventing window boundary clipping.
- **Progressive Resistance Formula**:
  $$\text{disp}(dy) = \begin{cases} dy & \text{if } dy \le 320\text{px} \quad (\text{QQS Compact threshold}) \\ 320 + (dy - 320) \times 0.85 & \text{if } dy > 320\text{px} \quad (\text{QS Full expansion}) \end{cases}$$
- **Velocity-Sensitive Snap Decisions**:
  - $v_y > 400\text{px/s} \implies \text{Full Expansion } (0\text{px offset, } 915\text{px visible})$.
  - $v_y < -120\text{px/s} \implies \text{Immediate Close } (-915\text{px offset, } 0\text{px visible})$.
  - Downward flick from top ($v_y > 60\text{px/s}$): if $\text{disp} > 420\text{px} \implies \text{Full}$, else $\implies \text{Compact QQS } (350\text{px})$.
  - Positional thresholds: $\text{disp} < 160\text{px} \implies \text{Closed}$, $\text{disp} < 500\text{px} \implies \text{Compact QQS}$, $\text{disp} \ge 500\text{px} \implies \text{Full QS}$.

### Pillar 2: Clean Separation from Homescreen
- **Removal of Legacy Hooks**: All pull-down callbacks (`on_homescreen_pulled_down`, `on_homescreen_pull_released`) in `crates/penumbra-shell/src/bin/homescreen.rs` and `app_surface.slint` are purged.
- **No Side-Push Artifacts**: The homescreen horizontal page carousel operates independently without conflicting with drawer drag states.

### Pillar 3: Dual-Stage Morphological Pastille UI (Slint)
The shade UI continuously morphs based on the expansion parameter $t = \frac{\text{disp}}{915\text{px}} \in [0.0, 1.0]$:
- **Stage 1: Quick Quick Settings (QQS, $0 \le \text{disp} \le 350\text{px}$)**:
  - Header: Time (expanded body font), Date, Carrier chip, Battery percentage pastille, Edit, Settings, and Power buttons.
  - Active Pastilles: 4 primary toggles (Internet, Bluetooth, Flashlight, DND) in compact capsule form.
  - Stacked Notifications: Dismissible cards with horizontal swipe gesture physics.
- **Stage 2: Full Quick Settings (QS, $350 < \text{disp} \le 915\text{px}$)**:
  - Full $2 \times 4$ Pastille Grid: Large touch surfaces with $0.96\times$ touch compression, active accent color fill (`#cbe6ff`), specular rims, title + dynamic state subtitle.
  - Dual Fluid Logarithmic Sliders: Brightness and Media Volume sliders with tactile drag scrub and haptic feedback triggers.
  - Media Player Card: Album art container, track metadata, play/pause pastille.
  - System Footer: User avatar, build/uptime chip, clear-all notifications action.

### Pillar 4: Memory Safety & RefCell Elimination
- Replaces nested runtime borrows with primitive copies (`f64`, `bool`) and scope-isolated blocks.
- Eliminates any possibility of `AlreadyBorrowed` panics crashing the SystemUI process.

### Pillar 5: Fault-Tolerant Session Supervisor
- `scripts/penumbra-session` wraps `penumbra-shell` in an automated watchdog loop.
- If an unhandled signal or panic occurs, `penumbra-shell` automatically relaunches within $< 100\text{ms}$, guaranteeing the user never loses status bar telemetry or navigation controls.

---

## 4. Implementation Checklist

- [x] Phase 7 Architectural Blueprint committed.
- [ ] Remove conflicting pull-down hooks from `homescreen.rs` and `app_surface.slint`.
- [ ] Refactor `systemui.rs` gesture velocity tracker to eliminate `RefCell` borrow hazards.
- [ ] Add session watchdog loop for `penumbra-shell` in `scripts/penumbra-session`.
- [ ] Verify clean compilation and 100% test pass on host.
- [ ] Cross-compile ARM64 release binaries via Docker.
- [ ] Deploy and verify live on Fairphone 4.
