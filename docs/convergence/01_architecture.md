# Penumbra Multi-Output & Convergence Architecture

## 1. Overview & Smithay KMS Pipeline

Penumbra's compositor (`penumbra-comp`) uses a dual-personality output architecture built on **Smithay** and adapted from the battle-tested KMS output management in **COSMIC Epoch (`cosmic-comp`)**.

### Display Roles & Spaces ($N$ External Monitors)

```
┌───────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                   Smithay Space Coordinate Canvas                                     │
├───────────────────┬───────────────────────────────────┬───────────────────────────────────────────────┤
│ Phone (DSI-1)     │ External Monitor 1 (DP-1)         │ External Monitor 2 (HDMI-A-1) / Miracast      │
│ Loc: (0, 0)       │ Loc: (412, 0)                     │ Loc: (412 + W1, 0)                            │
│ Canvas: 412 × 915 │ Resolution: e.g. 1920×1080 @ 60Hz │ Resolution: e.g. 2560×1440 @ 60Hz             │
│ Touch pinned      │ Pointer & Mouse cursor enabled    │ Pointer & Mouse cursor enabled                │
│ Mobile shell      │ Desktop Shell (Top Panel + Dock)  │ Desktop Shell (Secondary Panel / Wallpapers)  │
└───────────────────┴───────────────────────────────────┴───────────────────────────────────────────────┘
```

The compositor treats external displays as an extensible collection of desktop workspaces (`BTreeMap<crtc::Handle, KmsOutputSurface>`). Any number of external displays supported by the GPU/dock (DisplayPort Alt Mode, DisplayPort MST daisy-chaining, HDMI multi-port adapters, DisplayLink USB adapters) can be hotplugged or unmapped independently.

---

## 2. Dynamic DRM Connector Discovery & Hotplugging

Instead of hardcoding a single display at startup or relying on buggy software scrapers, `penumbra-comp` interfaces directly with the Linux kernel via DRM/KMS and `udev`:

1. **Connector Scanning (`crates/penumbra-comp/src/outputs/connector.rs`)**:
   - Queries `drm_fd.resource_handles()`.
   - Distinguishes internal panels (`connector::Interface::DSI`, `EmbeddedDisplayPort`, `DPI`) from hotpluggable external monitors (`HDMIA`, `DisplayPort`).
   - Reads EDID to determine monitor make, model, serial, and native preferred video mode.
2. **CRTC Allocation (`display_configuration`)**:
   - Implements `cosmic-comp`'s conflict-free CRTC allocation algorithm.
   - Preserves established encoder/CRTC pairings to prevent display flickering.
   - Assigns idle CRTCs to newly connected external monitors.
3. **Udev Hotplug Notification**:
   - The compositor's `calloop` event loop monitors `smithay::backend::udev::UdevBackend`.
   - When a kernel DRM hotplug event fires (`UdevEvent::Changed`), `NativeState::rescan_external_connector()` automatically:
     - Detects newly connected displays $\to$ creates KMS surface, maps `Output B` into `Space` at `(412, 0)`, and activates convergence mode.
     - Detects disconnected displays $\to$ unmaps `Output B`, releases the CRTC, and gracefully migrates all open desktop windows into the **Phone Overview (Recent Apps switcher)** without disturbing the active mobile foreground app.

---

## 3. Decoupled Frame Loops & Zero VNC Overhead

1. **Independent VBlank Loops**:
   - Output A (Phone) and Output B (External) maintain their own `GbmBufferedSurface` and independent DRM page flip queues.
   - VBlank events on `state.primary.crtc` and `state.external.crtc` retire independently. An external 60 Hz or 4K monitor never blocks or drops frames on the phone's 60/120 Hz DSI panel.
2. **Zero In-Process VNC**:
   - All external display connectivity is driven by native hardware KMS (USB-C DP Alt Mode, HDMI multiport docks, DisplayLink `evdi`).
   - Eliminates synchronous `glReadPixels` on the render thread and socket buffer deadlocks that previously caused compositor freezes on the Adreno 619 Freedreno driver.
