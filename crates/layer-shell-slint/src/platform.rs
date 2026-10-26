//! Slint [`Platform`] driver for Wayland wlr-layer-shell.
//!
//! Integrates the Wayland event queue into a [`calloop::EventLoop`] alongside
//! Slint internal animation timers, bypassing Winit completely.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use calloop::ping::make_ping;
use calloop_wayland_source::WaylandSource;
use log::{debug, error, info, warn};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::shell::wlr_layer::LayerShell;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::Shm;
use wayland_client::globals::registry_queue_init;
use wayland_client::{Connection, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    Anchor, KeyboardInteractivity,
};

fn to_sctk_layer(layer: Layer) -> smithay_client_toolkit::shell::wlr_layer::Layer {
    match layer {
        Layer::Background => smithay_client_toolkit::shell::wlr_layer::Layer::Background,
        Layer::Bottom => smithay_client_toolkit::shell::wlr_layer::Layer::Bottom,
        Layer::Top => smithay_client_toolkit::shell::wlr_layer::Layer::Top,
        Layer::Overlay => smithay_client_toolkit::shell::wlr_layer::Layer::Overlay,
        _ => smithay_client_toolkit::shell::wlr_layer::Layer::Top,
    }
}

fn to_sctk_anchor(anchor: Anchor) -> smithay_client_toolkit::shell::wlr_layer::Anchor {
    smithay_client_toolkit::shell::wlr_layer::Anchor::from_bits_truncate(anchor.bits())
}

fn to_sctk_keyboard(
    interactivity: KeyboardInteractivity,
) -> smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity {
    match interactivity {
        KeyboardInteractivity::None => {
            smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::None
        }
        KeyboardInteractivity::Exclusive => {
            smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::Exclusive
        }
        KeyboardInteractivity::OnDemand => {
            smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::OnDemand
        }
        _ => smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::None,
    }
}

use crate::error::LayerShellError;
use crate::shm::calculate_buffer_size;
use crate::state::AppState;
use crate::window::LayerWindow;
use crate::LayerConfig;

/// Internal holder of runtime event loop and Wayland connection structures.
struct PlatformRuntime {
    connection: Connection,
    app_state: AppState,
    event_loop: calloop::EventLoop<'static, AppState>,
    qh: QueueHandle<AppState>,
    ping: calloop::ping::Ping,
    default_config: LayerConfig,
    active_windows: Vec<Rc<LayerWindow>>,
}

/// Custom Slint platform backend driver implementing zero-winit layer-shell integration.
pub struct LayerShellPlatform {
    runtime: RefCell<PlatformRuntime>,
}

impl LayerShellPlatform {
    /// Creates and initializes a new `LayerShellPlatform` backend.
    pub fn new(config: LayerConfig) -> Result<Self, LayerShellError> {
        info!(
            "Connecting to Wayland display for layer-shell backend (namespace='{}')",
            config.namespace
        );

        let connection = Connection::connect_to_env()?;
        let (globals, event_queue) = registry_queue_init::<AppState>(&connection).map_err(|e| {
            LayerShellError::ProtocolError(format!("Registry init failed: {:?}", e))
        })?;

        let qh = event_queue.handle();

        // Initialize SCTK registry state
        let registry_state = RegistryState::new(&globals);

        // Bind standard Wayland compositor protocol
        let compositor_state = CompositorState::bind(&globals, &qh).map_err(|e| {
            LayerShellError::ProtocolError(format!("wl_compositor bind failure: {:?}", e))
        })?;

        // Bind output manager protocol
        let output_state = OutputState::new(&globals, &qh);

        // Bind seat input manager protocol
        let seat_state = SeatState::new(&globals, &qh);

        // Bind shared memory protocol
        let shm = Shm::bind(&globals, &qh)
            .map_err(|e| LayerShellError::ProtocolError(format!("wl_shm bind failure: {:?}", e)))?;

        // Pre-allocate a slot pool capable of holding at least one full window frame
        let initial_buf_size = calculate_buffer_size(config.width.max(1), config.height.max(1));
        let pool = SlotPool::new(initial_buf_size, &shm).map_err(|e| {
            LayerShellError::BufferAllocationError(format!("SlotPool::new failed: {:?}", e))
        })?;

        // Bind wlr-layer-shell protocol
        let layer_shell = LayerShell::bind(&globals, &qh).map_err(|e| {
            LayerShellError::ProtocolError(format!("zwlr_layer_shell_v1 bind failure: {:?}", e))
        })?;

        let exit_flag = Arc::new(AtomicBool::new(false));

        let app_state = AppState {
            registry_state,
            compositor_state,
            output_state,
            seat_state,
            shm,
            pool,
            layer_shell,
            pointer: None,
            touch: None,
            windows: Vec::new(),
            touch_targets: std::collections::HashMap::new(),
            exit_flag: exit_flag.clone(),
        };

        // Create the Calloop event loop
        let event_loop = calloop::EventLoop::<AppState>::try_new().map_err(|e| {
            LayerShellError::EventLoopError(format!("Failed to create calloop event loop: {:?}", e))
        })?;

        let loop_handle = event_loop.handle();

        // Create ping mechanism to wake up event loop from Slint request_redraw
        let (ping, ping_source) = make_ping().map_err(|e| {
            LayerShellError::EventLoopError(format!(
                "Failed to create calloop ping source: {:?}",
                e
            ))
        })?;

        loop_handle
            .insert_source(ping_source, |_event, _metadata, _state: &mut AppState| {
                // Ping event woke up loop; no-op payload
            })
            .map_err(|e| {
                LayerShellError::EventLoopError(format!("Failed to insert ping source: {:?}", e))
            })?;

        // Register the Wayland socket event source
        let wayland_source = WaylandSource::new(connection.clone(), event_queue);
        wayland_source.insert(loop_handle).map_err(|e| {
            LayerShellError::EventLoopError(format!("Failed to insert WaylandSource: {:?}", e))
        })?;

        let runtime = PlatformRuntime {
            connection,
            app_state,
            event_loop,
            qh,
            ping,
            default_config: config,
            active_windows: Vec::new(),
        };

        Ok(Self {
            runtime: RefCell::new(runtime),
        })
    }
}

// ---------------------------------------------------------------------------
// Slint Platform Trait Implementation
// ---------------------------------------------------------------------------
impl Platform for LayerShellPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let mut rt = self.runtime.borrow_mut();

        let config = crate::take_next_window_config().unwrap_or_else(|| rt.default_config.clone());

        // 1. Create a base wl_surface from the compositor
        let wl_surface = rt.app_state.compositor_state.create_surface(&rt.qh);

        // 2. Instantiate zwlr_layer_surface_v1 via SCTK
        let sctk_layer = to_sctk_layer(config.layer);
        let layer_surface = rt.app_state.layer_shell.create_layer_surface(
            &rt.qh,
            wl_surface,
            sctk_layer,
            Some(config.namespace.clone()),
            None, // Use default focused compositor output
        );

        // 3. Configure anchors, geometry, margins, and exclusive zone
        let sctk_anchor = to_sctk_anchor(config.anchor);
        let sctk_keyboard = to_sctk_keyboard(config.keyboard_interactivity);
        layer_surface.set_anchor(sctk_anchor);
        layer_surface.set_size(config.width, config.height);
        let (m_top, m_right, m_bottom, m_left) = config.margin;
        layer_surface.set_margin(m_top, m_right, m_bottom, m_left);
        layer_surface.set_exclusive_zone(config.exclusive_zone);
        layer_surface.set_keyboard_interactivity(sctk_keyboard);

        // 4. Initial commit to instruct compositor to compute layer layout and emit configure
        layer_surface.commit();

        // 5. Determine effective renderer strategy with universal CPU fallback
        let is_dri_available = std::path::Path::new("/dev/dri/renderD128").exists()
            || std::path::Path::new("/dev/dri/card0").exists();

        let effective_renderer = match config.renderer {
            crate::RendererType::Software => crate::RendererType::Software,
            crate::RendererType::Gpu => {
                if is_dri_available {
                    crate::RendererType::Gpu
                } else {
                    log::warn!(
                        "GPU acceleration requested for '{}' but no DRI device found; falling back to CPU SoftwareRenderer",
                        config.namespace
                    );
                    crate::RendererType::Software
                }
            }
            crate::RendererType::Auto => {
                if is_dri_available {
                    crate::RendererType::Gpu
                } else {
                    crate::RendererType::Software
                }
            }
        };

        log::info!(
            "LayerWindow '{}' active renderer: {:?} (configured: {:?}, VBlank dynamic refresh: 60Hz/90Hz/120Hz)",
            config.namespace,
            effective_renderer,
            config.renderer
        );

        // 6. Instantiate the Slint window adapter
        let window = LayerWindow::new(
            layer_surface,
            config.width,
            config.height,
            rt.qh.clone(),
            rt.ping.clone(),
            effective_renderer,
        );

        // 7. Connect window reference to SCTK state handlers
        rt.app_state.windows.push(Rc::downgrade(&window));
        rt.active_windows.push(window.clone());

        debug!(
            "Created native LayerWindow adapter for namespace '{}'",
            config.namespace
        );

        Ok(window as Rc<dyn WindowAdapter>)
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        debug!("Entering Slint layer-shell event loop");

        loop {
            let mut rt = self.runtime.borrow_mut();

            // Check if the compositor requested window closure
            if rt.app_state.exit_flag.load(Ordering::SeqCst) {
                info!("Exit flag set; breaking event loop");
                break;
            }

            // Prune dead windows from active_windows
            rt.active_windows.retain(|w| Rc::strong_count(w) > 1);

            // Perform render pass for each active window if ready and dirty
            let active_windows = rt.active_windows.clone();
            for window in &active_windows {
                if window.can_render() {
                    window.render(&mut rt.app_state.pool);
                }
            }

            // Flush pending Wayland outgoing protocol messages
            if let Err(e) = rt.connection.flush() {
                warn!("Wayland connection flush warning: {:?}", e);
            }

            // Calculate duration until next active Slint timer/animation update
            let timer_duration = slint::platform::duration_until_next_timer_update();
            let timeout = timer_duration.unwrap_or(Duration::from_millis(100));

            // Run event loop pass with dispatch
            let PlatformRuntime {
                ref mut event_loop,
                ref mut app_state,
                ..
            } = *rt;

            if let Err(e) = event_loop.dispatch(Some(timeout), app_state) {
                error!("Error in calloop event loop dispatch: {:?}", e);
                return Err(PlatformError::Other(format!(
                    "Event loop dispatch error: {:?}",
                    e
                )));
            }

            // Advance Slint timers and trigger animation step
            slint::platform::update_timers_and_animations();
        }

        Ok(())
    }
}
