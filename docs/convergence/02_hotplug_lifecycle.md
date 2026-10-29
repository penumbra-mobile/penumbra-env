# Penumbra Multi-Output Routing & Lifecycle Policies

This document specifies the routing policies, input containment, spatial focus resolution, and disconnect migration rules implemented in `penumbra-comp` for mobile convergence.

---

## 1. Input Routing: Pointer Unclamping vs. Touch Containment

In convergence mode, input devices attached to the Fairphone 4 exhibit asymmetric behavior depending on their physical modality:

### 1.1 Touchscreen Digitizer (Internal Phone Panel)
- **Modality**: `InputEvent::TouchDown`, `InputEvent::TouchMotion`, `InputEvent::TouchUp` via `libinput`.
- **Constraint**: Strictly clamped to the physical phone canvas `[0..411, 0..914]`.
- **Rationale**: The user tapping, scrolling, or swiping on the phone screen must never accidentally send events to external monitors or click background desktop windows.

### 1.2 Mouse & Touchpad Pointers
- **Modality**: `InputEvent::PointerMotion`, `InputEvent::PointerMotionAbsolute`.
- **Constraint**: Dynamically bounded by `global_layout_bounds()`:
  $$\text{Bounds} = \left[0 \dots 412 + \sum_{k=1}^N W_k - 1, \; 0 \dots \max(H_{\text{phone}}, H_{\text{ext}}) - 1\right]$$
- **Rationale**: Mouse and touchpad pointers can freely traverse across the phone display and any number of connected external displays.

---

## 2. Spatial Focus Resolution (`focus_under`)

When dispatching pointer motion or click events, `PenumbraServerState::focus_under(pos)` resolves the target Wayland surface across multiple displays:

```
pos = (x, y)
 │
 ├── If x < 412.0: phone_focus_under(pos)
 │     ├── 0. Lockscreen Overlay (highest priority when locked)
 │     ├── 1. Drawer / Dialog Overlay
 │     ├── 2. Top Layer (Status bar, OSK, Nav bar)
 │     ├── 3. Phone App Inset (running mobile app inside (0, 42, 412, 835))
 │     ├── 4. Bottom Layer
 │     └── 5. Background Layer (Homescreen wallpaper)
 │
 └── If x >= 412.0: desktop_focus_under(pos)
       ├── Find containing external monitor (x ∈ [output.loc.x, output.loc.x + width])
       ├── 1. Overlay Layer (Popups, screen saver)
       ├── 2. Top Layer (Desktop Top Panel, Dock, Applets)
       ├── 3. Floating XDG Desktop Windows in Smithay Space
       ├── 4. Bottom Layer
       └── 5. Background Layer (Desktop wallpaper)
```

### Click & Window Activation
On `InputEvent::PointerButton` (Press):
1. If the pointer is at $x \ge 412.0$ over an XDG desktop window:
   - `server.space.raise_element(&window, true)` brings the window to the front.
   - `keyboard.set_focus(&mut server, Some(surf), serial)` transfers keyboard focus so keystrokes immediately target the clicked application.

---

## 3. Window Boundary Dragging & Modality Transitions

When moving windows via `XdgShellHandler::move_request` or interactive pointer drags:

```
                       x = 412px
    Mobile Phone Zone    │   External Desktop Zone
    (0 <= x < 412)       │   (x >= 412)
                         │
    ┌────────────────┐   │   ┌───────────────────────────┐
    │ Mobile App     │ ◄──── │ Floating Desktop Window   │
    │ Maximized CSD  │   │   │ Free geometry (960 x 640) │
    │ Inside Inset   │ ────► │ Custom window position    │
    └────────────────┘   │   └───────────────────────────┘
```

1. **Desktop $\to$ Phone Transition**:
   - Triggered when window center crosses $x < 412$.
   - Window is added to `phone_tasks`, configured to maximized $(412, 835)$, mapped at $(0, 42)$, and switches shell mode to `PhoneMode::App`.
2. **Phone $\to$ Desktop Transition**:
   - Triggered when window center crosses $x \ge 412$.
   - Window is removed from `phone_tasks`, un-maximized, configured to floating size $(960, 640)$, and mapped freely in `space`.

---

## 4. Cursor Rendering on External Displays

In `render_external_frame`, cursor sprites are drawn only on the display intersecting `server.pointer_location`:

1. **Client Cursor Surfaces**:
   - When a client supplies a custom cursor via `wl_pointer.set_cursor`, the compositor reads the hotspot offset from `CursorImageAttributes` and paints the client surface.
2. **Fallback / Default Cursor**:
   - Bundled 64×64 RGBA cursor bitmap rendered via `MemoryRenderBufferRenderElement` at 24×24 logical size when default pointer is active.

---

## 5. Disconnect App Migration & Overview Badges

1. **Partial Disconnect ($N > 1 \to N - 1$)**:
   - If monitor 2 is disconnected while monitor 1 remains connected, windows intersecting monitor 2 are moved to monitor 1 without disturbing the phone screen.
2. **Total Disconnect ($N \to 0$)**:
   - When all external displays are unplugged, desktop windows migrate to the Phone Overview (`phone_tasks`), positioned behind the foreground mobile app.
   - Migrated tasks are flagged with `is_migrated_desktop: true`.
   - `publish_phone_tasks()` prefixes their title in `penumbra-windows` with `🖥 Desktop Session • {Title}` so the user can easily identify and resume or dismiss their desktop workflow.
