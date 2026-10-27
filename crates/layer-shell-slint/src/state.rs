//! SCTK AppState handling seat, shm, layer-shell, and compositor delegates.

use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use log::{debug, info, trace};
use slint::platform::WindowAdapter;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, delegate_subcompositor, delegate_touch,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{PointerEvent, PointerHandler},
        touch::TouchHandler,
        Capability, SeatHandler, SeatState,
    },
    shell::wlr_layer::{LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
    shell::WaylandSurface,
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use wayland_client::protocol::{
    wl_keyboard::WlKeyboard,
    wl_output::{self, WlOutput},
    wl_pointer::WlPointer,
    wl_seat::WlSeat,
    wl_surface::WlSurface,
    wl_touch::WlTouch,
};
use wayland_client::{Connection, QueueHandle};

use crate::window::LayerWindow;

/// Application state carrying SCTK protocol delegates and managing the Wayland connection.
pub struct AppState {
    /// Wayland global registry state tracker.
    pub registry_state: RegistryState,
    /// Wayland compositor manager state.
    pub compositor_state: CompositorState,
    /// Connected outputs state tracker.
    pub output_state: OutputState,
    /// Seat and capability tracking state.
    pub seat_state: SeatState,
    /// Shared memory manager state.
    pub shm: Shm,
    /// Shared memory buffer allocation pool.
    pub pool: SlotPool,
    /// wlr-layer-shell protocol manager.
    pub layer_shell: LayerShell,

    /// Currently bound mouse pointer device, if available on seat.
    pub pointer: Option<WlPointer>,
    /// Currently bound touchscreen device, if available on seat.
    pub touch: Option<WlTouch>,
    /// Currently bound keyboard device, if available on seat.
    pub keyboard: Option<WlKeyboard>,
    /// Window currently holding keyboard focus.
    pub keyboard_focus: Option<Weak<LayerWindow>>,

    /// Weak references to all active Slint layer window adapters.
    pub windows: Vec<Weak<LayerWindow>>,
    /// Active touch contact mappings (slot ID -> target window).
    pub touch_targets: HashMap<i32, Weak<LayerWindow>>,

    /// Flag set to true when all window surfaces are closed or the event loop should terminate.
    pub exit_flag: Arc<AtomicBool>,
}

impl AppState {
    /// Finds the active layer window whose underlying `wl_surface` matches `surface`.
    pub fn find_window_by_surface(&self, surface: &WlSurface) -> Option<Rc<LayerWindow>> {
        for weak in &self.windows {
            if let Some(w) = weak.upgrade() {
                if w.layer_surface().wl_surface() == surface {
                    return Some(w);
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Compositor Handler
// ---------------------------------------------------------------------------
impl CompositorHandler for AppState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &WlSurface,
        new_factor: i32,
    ) {
        debug!("Wayland surface scale factor changed: {}", new_factor);
        if let Some(w) = self.find_window_by_surface(surface) {
            w.set_scale_factor(new_factor as f32);
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        // Transformations are handled transparently by the compositor.
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &WlSurface,
        _time: u32,
    ) {
        trace!("Wayland frame callback received; ready for next frame presentation");
        if let Some(w) = self.find_window_by_surface(surface) {
            w.on_frame_rendered();
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &WlOutput,
    ) {
    }
}

// ---------------------------------------------------------------------------
// Output Handler
// ---------------------------------------------------------------------------
impl OutputHandler for AppState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: WlOutput) {}
    fn update_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: WlOutput) {}
    fn output_destroyed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: WlOutput) {
    }
}

// ---------------------------------------------------------------------------
// Shared Memory Handler
// ---------------------------------------------------------------------------
impl ShmHandler for AppState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

// ---------------------------------------------------------------------------
// Seat Handler
// ---------------------------------------------------------------------------
impl SeatHandler for AppState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Pointer if self.pointer.is_none() => {
                debug!("Acquiring wl_pointer capability on seat");
                let pointer = self
                    .seat_state
                    .get_pointer(qh, &seat)
                    .expect("Failed to get pointer");
                self.pointer = Some(pointer);
            }
            Capability::Touch if self.touch.is_none() => {
                debug!("Acquiring wl_touch capability on seat");
                let touch = self
                    .seat_state
                    .get_touch(qh, &seat)
                    .expect("Failed to get touch");
                self.touch = Some(touch);
            }
            Capability::Keyboard if self.keyboard.is_none() => {
                debug!("Acquiring wl_keyboard capability on seat");
                match self.seat_state.get_keyboard(qh, &seat, None) {
                    Ok(keyboard) => {
                        self.keyboard = Some(keyboard);
                    }
                    Err(e) => {
                        log::error!("Failed to get keyboard from seat: {:?}", e);
                    }
                }
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Pointer => {
                debug!("Releasing wl_pointer capability");
                if let Some(pointer) = self.pointer.take() {
                    pointer.release();
                }
            }
            Capability::Touch => {
                debug!("Releasing wl_touch capability");
                if let Some(touch) = self.touch.take() {
                    touch.release();
                }
            }
            Capability::Keyboard => {
                debug!("Releasing wl_keyboard capability");
                if let Some(keyboard) = self.keyboard.take() {
                    keyboard.release();
                }
                self.keyboard_focus = None;
            }
            _ => {}
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: WlSeat) {}
}

// ---------------------------------------------------------------------------
// Pointer Handler
// ---------------------------------------------------------------------------
impl PointerHandler for AppState {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if let Some(w) = self.find_window_by_surface(&event.surface) {
                w.process_pointer_event(event);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Touch Handler
// ---------------------------------------------------------------------------
impl TouchHandler for AppState {
    fn down(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &WlTouch,
        _serial: u32,
        _time: u32,
        surface: WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        if let Some(w) = self.find_window_by_surface(&surface) {
            self.touch_targets.insert(id, Rc::downgrade(&w));
            w.process_touch_down(id, position);
        }
    }

    fn up(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &WlTouch,
        _serial: u32,
        _time: u32,
        id: i32,
    ) {
        if let Some(weak) = self.touch_targets.remove(&id) {
            if let Some(w) = weak.upgrade() {
                w.process_touch_up(id);
            }
        } else {
            for weak in self.touch_targets.values() {
                if let Some(w) = weak.upgrade() {
                    w.process_touch_up(id);
                }
            }
            self.touch_targets.clear();
        }
    }

    fn motion(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &WlTouch,
        _time: u32,
        id: i32,
        position: (f64, f64),
    ) {
        if let Some(weak) = self.touch_targets.get(&id) {
            if let Some(w) = weak.upgrade() {
                w.process_touch_motion(id, position);
            }
        }
    }

    fn cancel(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _touch: &WlTouch) {
        for weak in self.touch_targets.values() {
            if let Some(w) = weak.upgrade() {
                w.process_touch_cancel();
            }
        }
        self.touch_targets.clear();
    }

    fn shape(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &WlTouch,
        _id: i32,
        _major: f64,
        _minor: f64,
    ) {
    }

    fn orientation(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &WlTouch,
        _id: i32,
        _orientation: f64,
    ) {
    }
}

// ---------------------------------------------------------------------------
// Layer Shell Handler
// ---------------------------------------------------------------------------
impl LayerShellHandler for AppState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        info!("Wayland layer surface closed by compositor");
        self.windows.retain(|w| {
            if let Some(window) = w.upgrade() {
                window.layer_surface().wl_surface() != layer.wl_surface()
            } else {
                false
            }
        });
        if self.windows.is_empty() {
            info!("All layer surfaces closed; terminating event loop");
            self.exit_flag.store(true, Ordering::SeqCst);
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        serial: u32,
    ) {
        debug!(
            "Wayland layer surface configure event: size={:?}, serial={}",
            configure.new_size, serial
        );

        if let Some(w) = self.find_window_by_surface(layer.wl_surface()) {
            let (w_width, w_height) = configure.new_size;
            w.on_configure(w_width, w_height);
        }
    }
}

// ---------------------------------------------------------------------------
// Keyboard Handler
// ---------------------------------------------------------------------------
impl KeyboardHandler for AppState {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        surface: &WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
        debug!("Keyboard entered surface {:?}", surface);
        self.keyboard_focus = self.find_window_by_surface(surface).map(|w| Rc::downgrade(&w));
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _surface: &WlSurface,
        _serial: u32,
    ) {
        debug!("Keyboard left surface");
        self.keyboard_focus = None;
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        if let Some(window) = self.keyboard_focus.as_ref().and_then(|w| w.upgrade()) {
            let text: slint::SharedString = if event.keysym == Keysym::BackSpace {
                slint::SharedString::from(slint::platform::Key::Backspace)
            } else if event.keysym == Keysym::Return {
                slint::SharedString::from(slint::platform::Key::Return)
            } else if event.keysym == Keysym::Tab {
                slint::SharedString::from(slint::platform::Key::Tab)
            } else if event.keysym == Keysym::Escape {
                slint::SharedString::from(slint::platform::Key::Escape)
            } else if event.keysym == Keysym::Delete {
                slint::SharedString::from(slint::platform::Key::Delete)
            } else if event.keysym == Keysym::Left {
                slint::SharedString::from(slint::platform::Key::LeftArrow)
            } else if event.keysym == Keysym::Right {
                slint::SharedString::from(slint::platform::Key::RightArrow)
            } else if let Some(ref s) = event.utf8 {
                slint::SharedString::from(s.as_str())
            } else if let Some(ch) = event.keysym.key_char() {
                slint::SharedString::from(ch.to_string())
            } else {
                return;
            };
            window.window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
        }
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        if let Some(window) = self.keyboard_focus.as_ref().and_then(|w| w.upgrade()) {
            let text: slint::SharedString = if event.keysym == Keysym::BackSpace {
                slint::SharedString::from(slint::platform::Key::Backspace)
            } else if event.keysym == Keysym::Return {
                slint::SharedString::from(slint::platform::Key::Return)
            } else if event.keysym == Keysym::Tab {
                slint::SharedString::from(slint::platform::Key::Tab)
            } else if event.keysym == Keysym::Escape {
                slint::SharedString::from(slint::platform::Key::Escape)
            } else if event.keysym == Keysym::Delete {
                slint::SharedString::from(slint::platform::Key::Delete)
            } else if event.keysym == Keysym::Left {
                slint::SharedString::from(slint::platform::Key::LeftArrow)
            } else if event.keysym == Keysym::Right {
                slint::SharedString::from(slint::platform::Key::RightArrow)
            } else if let Some(ref s) = event.utf8 {
                slint::SharedString::from(s.as_str())
            } else if let Some(ch) = event.keysym.key_char() {
                slint::SharedString::from(ch.to_string())
            } else {
                return;
            };
            window.window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
        }
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &WlKeyboard,
        _serial: u32,
        _modifiers: Modifiers,
        _layout: u32,
    ) {
    }
}

// Registry handling
impl ProvidesRegistryState for AppState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(AppState);
delegate_compositor!(AppState);
delegate_subcompositor!(AppState);
delegate_output!(AppState);
delegate_shm!(AppState);
delegate_seat!(AppState);
delegate_pointer!(AppState);
delegate_touch!(AppState);
delegate_keyboard!(AppState);
delegate_layer!(AppState);

impl wayland_client::Dispatch<wayland_client::protocol::wl_region::WlRegion, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &wayland_client::protocol::wl_region::WlRegion,
        _event: <wayland_client::protocol::wl_region::WlRegion as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}
