//! Consolidated Penumbra SystemUI runtime.
//!
//! Integrates the Top Status Bar, Quick Settings & Notification Shade, and
//! Navigation Bar into a single, cohesive Wayland layer-shell process.
//!
//! # Architecture (Material 3 Expressive Phase 6C)
//! - Single process hosting multiple `zwlr_layer_surface_v1` surfaces.
//! - Direct, in-memory drag handoff between TopBar and QuickSettings with zero IPC latency.
//! - Unified hardware telemetry (battery, network, clock) pushed simultaneously.
//! - Unified dynamic Monet theme synchronization.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use chrono::Local;
use slint::{ComponentHandle, Global, Model, ModelRc, Timer, TimerMode, VecModel};
use tracing::{info, warn};

use layer_shell_slint::{Anchor, KeyboardInteractivity, Layer, LayerConfig};
use penumbra_gestures::Spring;
use penumbra_services::{
    apply_backlight_ratio, apply_volume_ratio, read_backlight_ratio, read_volume_ratio,
    BatteryService, FingerprintManager, LockType, NetworkService, SecurityConfig,
};
use penumbra_shell::apply_monet_palette;
use penumbra_shell::ipc::{runtime_path, PhoneMode, ShellState, StateSocket, SHELL_STATE_SOCK};
use penumbra_shell::quick_settings::{tile, QuickSettingsConfig, TileDefinition, TileState};

use crate::{
    LockscreenWindow, NavBarWindow, NotificationData, QuickSettingsWindow, Theme, TileData,
    TopBarWindow,
};

/// Release/snap rule (offset coordinates): closed -915, compact -565 (350px), full 0.
fn release_target(y_current: f64, v_y: f64) -> f64 {
    // Fast upward flick anywhere on the drawer closes it immediately
    if v_y < -140.0 {
        return -915.0;
    }
    // Fast downward flick opens it
    if v_y > 180.0 {
        return 0.0;
    }
    // Gentle upward drag in upper 60% closes it
    if v_y < -30.0 && y_current < 550.0 {
        return -915.0;
    }
    // Slow release: magnetize to nearest edge (midpoint = 457.5px)
    if y_current >= 457.5 {
        0.0
    } else {
        -915.0
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    info!("Initializing Penumbra Consolidated SystemUI (Phase 6C)...");
    let active_lang = penumbra_services::i18n::active_lang_code();
    let _ = slint::select_bundled_translation(if active_lang == "en" { "" } else { &active_lang });

    let (logical_w, logical_h, _scale) = layer_shell_slint::mobile_output_geometry();
    let lw = logical_w as i32;
    let lh = logical_h as i32;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // -----------------------------------------------------------------------
    // 1. Layer-Shell Configurations
    // -----------------------------------------------------------------------
    let top_bar_config = LayerConfig::default()
        .with_namespace("penumbra-top-bar")
        .with_layer(Layer::Top)
        .with_anchor(Anchor::Top | Anchor::Left | Anchor::Right)
        .with_size(0, 42)
        .with_exclusive_zone(42)
        .with_keyboard_interactivity(KeyboardInteractivity::None);

    let qs_config_layer = LayerConfig::default()
        .with_namespace("penumbra-quick-settings")
        .with_layer(Layer::Overlay)
        .with_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right)
        .with_size(0, 0)
        .with_exclusive_zone(-1)
        .with_keyboard_interactivity(KeyboardInteractivity::None);

    let nav_bar_config = LayerConfig::default()
        .with_namespace("penumbra-nav-bar")
        .with_layer(Layer::Top)
        .with_anchor(Anchor::Bottom | Anchor::Left | Anchor::Right)
        .with_size(logical_w, 32)
        .with_exclusive_zone(32)
        .with_keyboard_interactivity(KeyboardInteractivity::None);

    let lock_config_layer = LayerConfig::default()
        .with_namespace("penumbra-lockscreen")
        .with_layer(Layer::Overlay)
        .with_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right)
        .with_size(logical_w, logical_h)
        .with_exclusive_zone(-1)
        .with_keyboard_interactivity(KeyboardInteractivity::None);

    // -----------------------------------------------------------------------
    // 2. Multi-Window Instantiation on Shared Platform
    // -----------------------------------------------------------------------
    // Initialize platform with TopBar config
    layer_shell_slint::init(top_bar_config)?;
    let top_bar = TopBarWindow::new()?;
    let top_bar_weak = top_bar.as_weak();

    // Enqueue QuickSettings config and instantiate
    layer_shell_slint::set_next_window_config(qs_config_layer);
    let qs = QuickSettingsWindow::new()?;
    let qs_weak = qs.as_weak();

    // Default collapsed: input region is 0 so status bar touches fall through to native gesture & top bar
    layer_shell_slint::sync_input_region(
        "penumbra-quick-settings",
        logical_w as i32,
        logical_h as i32,
        0,
        false,
    );

    // Enqueue NavBar config and instantiate
    layer_shell_slint::set_next_window_config(nav_bar_config);
    let nav_bar = NavBarWindow::new()?;
    let nav_bar_weak = nav_bar.as_weak();

    // Enqueue Lockscreen config and instantiate
    layer_shell_slint::set_next_window_config(lock_config_layer);
    let lockscreen = LockscreenWindow::new()?;
    let lockscreen_weak = lockscreen.as_weak();

    // Default start: unlocked (input region = 0, touches pass through to apps/homescreen)
    let sec_cfg = SecurityConfig::load();
    lockscreen.set_lock_type(sec_cfg.lock_type.as_str().into());
    lockscreen.set_has_fingerprint(FingerprintManager::has_sensor());
    lockscreen.set_fingerprint_enabled(sec_cfg.fingerprint_enabled && FingerprintManager::has_sensor());
    lockscreen.set_is_locked(false);
    layer_shell_slint::sync_input_region(
        "penumbra-lockscreen",
        logical_w as i32,
        logical_h as i32,
        0,
        false,
    );

    info!("Instantiated TopBar, QuickSettings, NavBar, and Lockscreen layer-shell surfaces");

    // -----------------------------------------------------------------------
    // 3. Unified Monet Dynamic Theming
    // -----------------------------------------------------------------------
    let initial_palette = penumbra_shell::theme::load_current_theme();
    {
        let t_bar = Theme::get(&top_bar);
        apply_monet_palette!(t_bar, initial_palette.clone());
        let t_qs = Theme::get(&qs);
        apply_monet_palette!(t_qs, initial_palette.clone());
        let t_nav = Theme::get(&nav_bar);
        apply_monet_palette!(t_nav, initial_palette.clone());
        let t_lock = Theme::get(&lockscreen);
        apply_monet_palette!(t_lock, initial_palette);
    }

    let _theme_timer = {
        let tb = top_bar_weak.clone();
        let q = qs_weak.clone();
        let nb = nav_bar_weak.clone();
        let lk = lockscreen_weak.clone();
        penumbra_shell::theme::start_theme_watcher(move |palette| {
            if let Some(bar) = tb.upgrade() {
                let theme = Theme::get(&bar);
                apply_monet_palette!(theme, palette.clone());
            }
            if let Some(shade) = q.upgrade() {
                let theme = Theme::get(&shade);
                apply_monet_palette!(theme, palette.clone());
            }
            if let Some(nav) = nb.upgrade() {
                let theme = Theme::get(&nav);
                apply_monet_palette!(theme, palette.clone());
            }
            if let Some(ls) = lk.upgrade() {
                let theme = Theme::get(&ls);
                apply_monet_palette!(theme, palette);
            }
        })
    };
    tracing::debug!("SystemUI init: theme watcher ready");

    // Initially retracted off-screen: y = -915px
    qs.set_slide_offset_y(-915.0);
    qs.set_expansion_progress(0.0);

    // Initial hardware telemetry for sliders (restore saved brightness if available)
    let init_brightness = if let Some(saved) = penumbra_shell::display::load_saved_brightness() {
        if let Err(error) = apply_backlight_ratio(saved) {
            warn!(?error, "Failed to restore saved brightness to system backlight");
        }
        saved
    } else {
        read_backlight_ratio().unwrap_or(0.75)
    };
    qs.set_brightness(init_brightness as f32);
    qs.set_volume(0.5);
    tracing::debug!("SystemUI init: brightness + volume ready");

    // layer-shell-slint does not implement EventLoopProxy, so background threads dispatch
    // UI updates through this mpsc channel drained on the Slint main thread every 80ms.
    let (ui_tx, ui_rx) = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send>>();
    let ui_dispatch_timer = Timer::default();
    ui_dispatch_timer.start(TimerMode::Repeated, Duration::from_millis(80), move || {
        while let Ok(task) = ui_rx.try_recv() {
            task();
        }
    });

    {
        let qs_weak_vol = qs_weak.clone();
        let tx = ui_tx.clone();
        rt.spawn(async move {
            if let Ok(init_volume) = tokio::task::spawn_blocking(read_volume_ratio).await {
                let _ = tx.send(Box::new(move || {
                    if let Some(ui) = qs_weak_vol.upgrade() {
                        ui.set_volume(init_volume as f32);
                    }
                }));
            }
        });
    }

    // -----------------------------------------------------------------------
    // 4. Spring Physics Engine (Stiffness: 300.0, Damping: 26.0)
    // -----------------------------------------------------------------------
    let spring_state = Rc::new(RefCell::new(
        Spring::from_physics(700.0, 44.0, 1.0)
            .with_position(-915.0)
            .with_target(-915.0),
    ));
    let spring_timer = Rc::new(RefCell::new(Timer::default()));
    let last_spring_time = Rc::new(RefCell::new(Instant::now()));
    let is_animating = Rc::new(std::cell::Cell::new(false));

    let animate_to = {
        let spring_state = Rc::clone(&spring_state);
        let spring_timer = Rc::clone(&spring_timer);
        let last_spring_time = Rc::clone(&last_spring_time);
        let is_animating = Rc::clone(&is_animating);
        let qs_weak = qs_weak.clone();
        let top_bar_weak = top_bar_weak.clone();

        move |target: f64, initial_vel: f64| {
            is_animating.set(true);
            if target > -900.0 {
                // Expanding or opened: capture fullscreen touch events
                layer_shell_slint::sync_input_region("penumbra-quick-settings", lw, lh, 42, true);
            }
            {
                let mut sp = spring_state.borrow_mut();
                sp.target = target;
                sp.velocity = initial_vel;
            }
            *last_spring_time.borrow_mut() = Instant::now();

            let spring_state = Rc::clone(&spring_state);
            let spring_timer_inner = Rc::clone(&spring_timer);
            let last_spring_time = Rc::clone(&last_spring_time);
            let is_animating_inner = Rc::clone(&is_animating);
            let qs_weak = qs_weak.clone();
            let top_bar_weak_inner = top_bar_weak.clone();

            spring_timer.borrow().start(
                TimerMode::Repeated,
                Duration::from_millis(16),
                move || {
                    let dt = {
                        let mut t = last_spring_time.borrow_mut();
                        let elapsed = t.elapsed().as_secs_f64().clamp(0.001, 0.05);
                        *t = Instant::now();
                        elapsed
                    };

                    let mut sp = spring_state.borrow_mut();
                    let new_y = sp.step(dt);
                    let progress = ((new_y + 915.0) / 915.0).clamp(0.0, 1.0) as f32;

                    if let Some(ui) = qs_weak.upgrade() {
                        ui.set_slide_offset_y(new_y as f32);
                        ui.set_expansion_progress(progress);
                    }
                    if let Some(tb) = top_bar_weak_inner.upgrade() {
                        tb.set_shade_progress(progress);
                    }

                    let is_settled =
                        sp.snap_if_settled(1.0, 5.0) || (sp.target <= -900.0 && new_y <= -914.0);
                    let mut state = ShellState::load();
                    if is_settled {
                        is_animating_inner.set(false);
                        spring_timer_inner.borrow().stop();
                        let target = sp.target;
                        let final_progress = ((target + 915.0) / 915.0).clamp(0.0, 1.0) as f32;
                        if let Some(ui) = qs_weak.upgrade() {
                            ui.set_slide_offset_y(target as f32);
                            ui.set_expansion_progress(final_progress);
                        }
                        if let Some(tb) = top_bar_weak_inner.upgrade() {
                            tb.set_shade_progress(final_progress);
                        }
                        state.drawer_open = target > -900.0;
                        state.drawer_pull_dy = if state.drawer_open { (target + 915.0).max(350.0) } else { 0.0 };
                        state.drawer_progress = if state.drawer_open { final_progress as f64 } else { 0.0 };
                        state.drawer_dragging = false;

                        // When settled retracted, collapse input region to 0 so top bar receives gestures
                        if target <= -900.0 {
                            layer_shell_slint::sync_input_region("penumbra-quick-settings", lw, lh, 0, false);
                        }
                    } else {
                        state.drawer_open = sp.target > -900.0 || progress > 0.001;
                        state.drawer_pull_dy = (new_y + 915.0).clamp(0.0, 915.0);
                        state.drawer_progress = progress as f64;
                        state.drawer_dragging = false;
                    }
                    let _ = state.save();
                },
            );
        }
    };
    let animate_to = Rc::new(animate_to);

    // -----------------------------------------------------------------------
    // 5. In-Memory Direct TopBar -> QuickSettings Drag Handoff (Zero Latency)
    // -----------------------------------------------------------------------
    let top_drag_velocity = Rc::new(RefCell::new(0.0f64));
    let last_top_pull_y = Rc::new(RefCell::new(0.0f64));
    let last_top_pull_time = Rc::new(RefCell::new(Instant::now()));

    {
        let qs_weak = qs_weak.clone();
        let top_bar_weak_drag = top_bar_weak.clone();
        let spring_timer = Rc::clone(&spring_timer);
        let spring_state = Rc::clone(&spring_state);
        let top_drag_velocity = Rc::clone(&top_drag_velocity);
        let last_top_pull_y = Rc::clone(&last_top_pull_y);
        let last_top_pull_time = Rc::clone(&last_top_pull_time);

        top_bar.on_top_bar_pulled_down(move |dy, is_qs| {
            // Stop any running settle animation on QuickSettings
            spring_timer.borrow().stop();

            // Expand input region fullscreen to capture drag gesture
            layer_shell_slint::sync_input_region("penumbra-quick-settings", lw, lh, 42, true);

            let pull = dy as f64;
            let dt = last_top_pull_time
                .borrow()
                .elapsed()
                .as_secs_f64()
                .clamp(0.001, 0.1);
            let last_y = *last_top_pull_y.borrow();
            let instant_v = (pull - last_y) / dt;
            let current_v = *top_drag_velocity.borrow();
            *top_drag_velocity.borrow_mut() = current_v * 0.6 + instant_v * 0.4;
            *last_top_pull_y.borrow_mut() = pull;
            *last_top_pull_time.borrow_mut() = Instant::now();

            // Android 17 progressive boundary resistance
            let shade_disp = if pull <= 320.0 {
                pull
            } else {
                320.0 + (pull - 320.0) * 0.85
            }
            .clamp(0.0, 915.0);

            let new_y = shade_disp - 915.0;
            let progress = (shade_disp / 915.0).clamp(0.0, 1.0) as f32;

            // Direct in-memory zero-latency Slint update
            if let Some(shade) = qs_weak.upgrade() {
                shade.set_slide_offset_y(new_y as f32);
                shade.set_expansion_progress(progress);
                shade.set_is_full_qs(is_qs);
            }
            if let Some(tb) = top_bar_weak_drag.upgrade() {
                tb.set_shade_progress(progress);
            }

            {
                let mut sp = spring_state.borrow_mut();
                sp.position = new_y;
                sp.velocity = 0.0;
            }

            // Sync compositor for wallpaper depth zoom (0.92x) & Dual-Kawase progressive blur
            let mut state = ShellState::load();
            state.drawer_open = true;
            state.drawer_dragging = true;
            state.drawer_pull_dy = shade_disp;
            state.drawer_progress = progress as f64;
            state.is_full_qs = is_qs;
            let _ = state.save();
        });
    }

    {
        let animate_to = Rc::clone(&animate_to);
        let top_drag_velocity = Rc::clone(&top_drag_velocity);
        let spring_state = Rc::clone(&spring_state);

        top_bar.on_top_bar_released(move |is_qs| {
            let y_current = spring_state.borrow().position + 915.0;
            let v = *top_drag_velocity.borrow();
            let target = release_target(y_current, v);
            info!(
                "TopBar released (is_qs={}): y_current={:.1}px, v={:.1}px/s -> target={:.1}",
                is_qs, y_current, v, target
            );

            let mut state = ShellState::load();
            state.drawer_dragging = false;
            state.is_full_qs = is_qs;
            let _ = state.save();

            animate_to(target, v);
        });
    }

    // -----------------------------------------------------------------------
    // 6. QuickSettings Own Drag & Kinetic Handlers
    // -----------------------------------------------------------------------
    let qs_drag_velocity = Rc::new(RefCell::new(0.0f64));
    let last_qs_drag_time = Rc::new(RefCell::new(Instant::now()));

    {
        let spring_timer = Rc::clone(&spring_timer);
        let spring_state = Rc::clone(&spring_state);
        let qs_weak = qs_weak.clone();
        let top_bar_weak = top_bar_weak.clone();
        let qs_drag_velocity = Rc::clone(&qs_drag_velocity);
        let last_qs_drag_time = Rc::clone(&last_qs_drag_time);

        qs.on_drag_delta(move |dy| {
            spring_timer.borrow().stop();
            layer_shell_slint::sync_input_region("penumbra-quick-settings", lw, lh, 42, true);
            if let Some(ui) = qs_weak.upgrade() {
                let cur_offset = ui.get_slide_offset_y() as f64;
                let delta = dy as f64;

                let dt = last_qs_drag_time
                    .borrow()
                    .elapsed()
                    .as_secs_f64()
                    .clamp(0.001, 0.1);
                let instant_v = delta / dt;
                let prev_v = *qs_drag_velocity.borrow();
                *qs_drag_velocity.borrow_mut() = prev_v * 0.5 + instant_v * 0.5;
                *last_qs_drag_time.borrow_mut() = Instant::now();

                let cur_disp = cur_offset + 915.0;
                let cur_touch = if cur_disp <= 320.0 {
                    cur_disp
                } else {
                    320.0 + (cur_disp - 320.0) / 0.85
                };

                let new_touch = (cur_touch + delta).clamp(0.0, 915.0 / 0.85);

                let new_disp = if new_touch <= 320.0 {
                    new_touch
                } else {
                    320.0 + (new_touch - 320.0) * 0.85
                }
                .clamp(0.0, 915.0);

                let new_y = new_disp - 915.0;
                let progress = (new_disp / 915.0).clamp(0.0, 1.0) as f32;

                ui.set_slide_offset_y(new_y as f32);
                ui.set_expansion_progress(progress);

                if let Some(tb) = top_bar_weak.upgrade() {
                    tb.set_shade_progress(progress);
                }

                let mut sp = spring_state.borrow_mut();
                sp.position = new_y;
                sp.velocity = 0.0;

                let mut state = ShellState::load();
                state.drawer_open = true;
                state.drawer_dragging = true;
                state.drawer_pull_dy = new_disp;
                state.drawer_progress = progress as f64;
                let _ = state.save();
            }
        });
    }

    {
        let animate_to = Rc::clone(&animate_to);
        let spring_state = Rc::clone(&spring_state);
        let qs_drag_velocity = Rc::clone(&qs_drag_velocity);
        qs.on_drag_released(move |cur_y, vel| {
            let y_current = if cur_y.abs() > 1.0 {
                (cur_y as f64) + 915.0
            } else {
                spring_state.borrow().position + 915.0
            };
            let v_y = if vel.abs() > 1.0 {
                vel as f64
            } else {
                *qs_drag_velocity.borrow()
            };
            *qs_drag_velocity.borrow_mut() = 0.0;
            let target_offset = release_target(y_current, v_y);
            info!(
                "QuickSettings drag released: y_current={:.1}px, v={:.1}px/s -> target={:.1}",
                y_current, v_y, target_offset
            );
            animate_to(target_offset, v_y);
        });
    }

    {
        let animate_to = Rc::clone(&animate_to);
        qs.on_close_requested(move || {
            info!("QuickSettings: close requested -> retracting to -915px");
            animate_to(-915.0, -600.0);
        });
    }

    {
        let animate_to = Rc::clone(&animate_to);
        qs.on_fast_pull_up(move || {
            info!("QuickSettings: fast pull up detected -> snapping closed");
            animate_to(-915.0, -850.0);
        });
    }

    {
        qs.on_mode_changed(move |is_full_qs| {
            info!("QuickSettings: mode changed to is_full_qs={}", is_full_qs);
            let mut state = ShellState::load();
            state.is_full_qs = is_full_qs;
            let _ = state.save();
        });
    }

    // -----------------------------------------------------------------------
    // 7. Tile Engine (Registry + Persistent Config)
    // -----------------------------------------------------------------------
    let registry: Rc<RefCell<Vec<TileDefinition>>> = Rc::new(RefCell::new(tile::registry_fast()));
    let qs_config = Rc::new(RefCell::new(QuickSettingsConfig::load()));
    tracing::debug!("SystemUI init: tile registry loaded");

    let refresh_tiles: Rc<dyn Fn()> = {
        let registry = Rc::clone(&registry);
        let qs_config = Rc::clone(&qs_config);
        let qs_weak = qs_weak.clone();
        Rc::new(move || {
            let reg = registry.borrow();
            let cfg = qs_config.borrow();
            let to_data = |id: &String| {
                reg.iter().find(|t| &t.id == id).map(|t| TileData {
                    id: t.id.clone().into(),
                    title: t.title.clone().into(),
                    subtitle: t.subtitle.clone().into(),
                    icon_name: t.icon_name.as_str().into(),
                    active: t.state == TileState::Active,
                    enabled: t.is_enabled && t.state != TileState::Unavailable,
                })
            };
            let active: Vec<TileData> = cfg.active_tiles.iter().filter_map(to_data).collect();
            let available: Vec<TileData> = cfg.available_tiles.iter().filter_map(to_data).collect();
            if let Some(ui) = qs_weak.upgrade() {
                ui.set_active_tiles(ModelRc::from(Rc::new(VecModel::from(active))));
                ui.set_available_tiles(ModelRc::from(Rc::new(VecModel::from(available))));
            }
        })
    };
    refresh_tiles();
    tracing::debug!("SystemUI init: tiles ready");

    {
        let registry = Rc::clone(&registry);
        let refresh = Rc::clone(&refresh_tiles);
        let top_bar_weak_tile = top_bar_weak.clone();
        qs.on_tile_clicked(move |id| {
            let mut is_active = false;
            if let Some(t) = registry
                .borrow_mut()
                .iter_mut()
                .find(|t| t.id == id.as_str())
            {
                t.toggle();
                is_active = t.state == TileState::Active;
            }
            match id.as_str() {
                "internet" => tile::set_wifi(is_active),
                "bluetooth" => tile::set_bluetooth(is_active),
                "flashlight" => tile::set_torch(is_active),
                "battery_saver" => tile::set_battery_saver(is_active),
                "dnd" => {
                    let mut s = ShellState::load();
                    s.dnd_active = is_active;
                    let _ = s.save();
                    if let Some(tb) = top_bar_weak_tile.upgrade() {
                        tb.set_dnd_active(is_active);
                    }
                }
                "rotation" => {
                    let mut s = ShellState::load();
                    s.auto_rotate = is_active;
                    let _ = s.save();
                }
                "airplane" => tile::set_airplane(is_active),
                _ => {}
            }
            refresh();
        });
    }

    {
        let animate_to = Rc::clone(&animate_to);
        qs.on_tile_long_pressed(move |id| {
            info!("QuickSettings: tile long-pressed: {id}");
            let page = match id.as_str() {
                "internet" => "wifi",
                "bluetooth" => "bluetooth",
                "flashlight" | "brightness" => "display",
                "battery_saver" => "battery",
                "dnd" => "sound",
                "rotation" => "display",
                "airplane" => "wifi",
                _ => "main",
            };
            animate_to(-915.0, 0.0);
            let _ = std::process::Command::new("penumbra-settings")
                .arg("--page")
                .arg(page)
                .spawn();
        });
    }

    {
        let animate_to = Rc::clone(&animate_to);
        let qs_weak = qs_weak.clone();
        qs.on_enter_edit_mode(move || {
            if let Some(ui) = qs_weak.upgrade() {
                ui.set_edit_mode(true);
            }
            animate_to(0.0, 0.0);
        });
    }

    {
        let qs_config = Rc::clone(&qs_config);
        let qs_weak = qs_weak.clone();
        qs.on_exit_edit_mode(move || {
            if let Err(e) = qs_config.borrow().save() {
                warn!("Failed to save quicksettings.json: {e}");
            }
            if let Some(ui) = qs_weak.upgrade() {
                ui.set_edit_mode(false);
            }
        });
    }

    {
        let qs_config = Rc::clone(&qs_config);
        let refresh = Rc::clone(&refresh_tiles);
        qs.on_reset_tiles(move || {
            *qs_config.borrow_mut() = QuickSettingsConfig::default();
            refresh();
        });
    }

    {
        let qs_config = Rc::clone(&qs_config);
        let refresh = Rc::clone(&refresh_tiles);
        qs.on_reorder_tiles(move |from, to| {
            if from >= 0 && to >= 0 {
                qs_config.borrow_mut().reorder(from as usize, to as usize);
            }
            refresh();
        });
    }

    {
        let qs_config = Rc::clone(&qs_config);
        let refresh = Rc::clone(&refresh_tiles);
        qs.on_remove_tile(move |id| {
            qs_config.borrow_mut().remove(id.as_str());
            refresh();
        });
    }

    {
        let qs_config = Rc::clone(&qs_config);
        let refresh = Rc::clone(&refresh_tiles);
        qs.on_add_tile(move |id| {
            qs_config.borrow_mut().add(id.as_str());
            refresh();
        });
    }

    // -----------------------------------------------------------------------
    // 8. Notifications Stack
    // -----------------------------------------------------------------------
    let notifs_store = Rc::new(std::cell::RefCell::new(penumbra_shell::notifications::NotificationStore::load()));
    let notifs_items = notifs_store.borrow().items.iter().map(|n| NotificationData {
        id: n.id.clone().into(),
        app: n.app.clone().into(),
        title: n.title.clone().into(),
        body: n.body.clone().into(),
        time_text: n.time_text.clone().into(),
    }).collect::<Vec<_>>();
    let notifs_model = Rc::new(VecModel::from(notifs_items));
    qs.set_notifications(ModelRc::from(Rc::clone(&notifs_model)));
    tracing::debug!("SystemUI init: notifications model ready");

    // Connect real org.freedesktop.Notifications service
    let (notif_sync_tx, notif_sync_rx) = std::sync::mpsc::channel::<penumbra_shell::notifications::NotificationItem>();
    let (async_tx, mut async_rx) = tokio::sync::mpsc::unbounded_channel::<penumbra_shell::notifications::NotificationItem>();
    rt.spawn(async move {
        let _conn = penumbra_shell::notifications::start_notification_service(async_tx).await;
        while let Some(item) = async_rx.recv().await {
            let _ = notif_sync_tx.send(item);
        }
    });

    let _notif_timer = Timer::default();
    {
        let notifs = Rc::clone(&notifs_model);
        let store = Rc::clone(&notifs_store);
        _notif_timer.start(TimerMode::Repeated, Duration::from_millis(50), move || {
            while let Ok(item) = notif_sync_rx.try_recv() {
                let data = NotificationData {
                    id: item.id.clone().into(),
                    app: item.app.clone().into(),
                    title: item.title.clone().into(),
                    body: item.body.clone().into(),
                    time_text: item.time_text.clone().into(),
                };
                notifs.insert(0, data);
                store.borrow_mut().items.insert(0, item);
                let _ = store.borrow().save();
            }
        });
    }

    {
        let notifs = Rc::clone(&notifs_model);
        let store = Rc::clone(&notifs_store);
        qs.on_notification_dismissed(move |idx| {
            let i = idx as usize;
            if i < notifs.row_count() {
                info!("QuickSettings: Notification at index {idx} dismissed");
                notifs.remove(i);
                store.borrow_mut().dismiss_at(i);
            }
        });
    }
    {
        let notifs = Rc::clone(&notifs_model);
        let store = Rc::clone(&notifs_store);
        qs.on_clear_notifications(move || {
            info!("QuickSettings: Cleared all notifications");
            while notifs.row_count() > 0 {
                notifs.remove(0);
            }
            store.borrow_mut().clear();
        });
    }

    // -----------------------------------------------------------------------
    // 9. Hardware Controls & System Dialogs
    // -----------------------------------------------------------------------
    {
        let qs_weak = qs_weak.clone();
        qs.on_power_action(move || {
            info!("QuickSettings: Power button tapped -> showing Power Dialog");
            if let Some(ui) = qs_weak.upgrade() {
                ui.set_show_power_dialog(true);
            }
            let mut s = ShellState::load();
            s.power_dialog_open = true;
            let _ = s.save();
        });
    }

    qs.on_power_off(|| {
        info!("Power Dialog: Turn OFF requested");
        let _ = std::process::Command::new("systemctl").arg("poweroff").spawn();
    });

    qs.on_reboot(|| {
        info!("Power Dialog: Reboot requested");
        let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
    });

    qs.on_logout(|| {
        info!("Power Dialog: Logout requested");
        if let Ok(user) = std::env::var("USER") {
            let _ = std::process::Command::new("loginctl")
                .args(["terminate-user", &user])
                .spawn();
        }
        let _ = std::process::Command::new("pkill")
            .args(["-u", &format!("{}", unsafe { libc::getuid() })])
            .spawn();
    });

    {
        let qs_weak = qs_weak.clone();
        qs.on_dismiss_power_dialog(move || {
            info!("Power Dialog dismissed");
            if let Some(ui) = qs_weak.upgrade() {
                ui.set_show_power_dialog(false);
            }
            let mut s = ShellState::load();
            s.power_dialog_open = false;
            let _ = s.save();
        });
    }

    {
        let animate_to = Rc::clone(&animate_to);
        qs.on_settings_action(move || {
            info!("QuickSettings: Settings tapped -> retracting & launching penumbra-settings");
            animate_to(-915.0, 0.0);
            let _ = std::process::Command::new("penumbra-settings")
                .arg("--page")
                .arg("main")
                .spawn();
        });
    }

    // Phosh-inspired system status indicators (ModemManager WWAN, BlueZ, VPN, Docked)
    {
        let top_bar_weak_status = top_bar_weak.clone();
        let tx = ui_tx.clone();
        rt.spawn(async move {
            let mut last_st: Option<penumbra_services::status::MobileStatusIndicators> = None;
            loop {
                let st = penumbra_services::status::poll_mobile_status().await;
                if last_st.as_ref() != Some(&st) {
                    info!(?st, "TopBar: mobile status indicators updated");
                    last_st = Some(st.clone());
                }
                let weak = top_bar_weak_status.clone();
                let _ = tx.send(Box::new(move || {
                    if let Some(bar) = weak.upgrade() {
                        bar.set_wwan_present(st.wwan_present);
                        bar.set_wwan_signal(st.wwan_signal as i32);
                        bar.set_wwan_tech(st.wwan_tech.into());
                        bar.set_bt_powered(st.bt_powered);
                        bar.set_bt_connected(st.bt_connected);
                        bar.set_vpn_active(st.vpn_active);
                        bar.set_docked(st.docked);
                    }
                }));
                tokio::time::sleep(Duration::from_millis(3000)).await;
            }
        });
    }

    // -----------------------------------------------------------------------
    // 7b. Live MPRIS2 Media Applet (Firefox YouTube, Spotify, VLC, MPV, etc.)
    // -----------------------------------------------------------------------
    let active_mpris_bus = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    {
        let bus_ref = std::sync::Arc::clone(&active_mpris_bus);
        let qs_weak_mpris = qs_weak.clone();
        let tx = ui_tx.clone();
        rt.spawn(async move {
            loop {
                let state = penumbra_services::mpris::poll_active_media().await;
                if let Ok(mut guard) = bus_ref.lock() {
                    *guard = state.as_ref().map(|s| s.bus_name.clone());
                }
                let weak = qs_weak_mpris.clone();
                let _ = tx.send(Box::new(move || {
                    if let Some(ui) = weak.upgrade() {
                        if let Some(s) = state {
                            ui.set_media_active(true);
                            ui.set_media_title(s.title.into());
                            ui.set_media_artist(s.artist.into());
                            ui.set_media_player(s.player_name.into());
                            ui.set_media_progress(s.progress);
                            ui.set_media_playing(s.is_playing);
                        } else {
                            ui.set_media_active(false);
                        }
                    }
                }));
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        });
    }
    {
        let bus_ref = std::sync::Arc::clone(&active_mpris_bus);
        let rt_handle = rt.handle().clone();
        let qs_weak_pp = qs_weak.clone();
        qs.on_media_play_pause(move || {
            if let Some(bus) = bus_ref.lock().ok().and_then(|g| g.clone()) {
                if let Some(ui) = qs_weak_pp.upgrade() {
                    ui.set_media_playing(!ui.get_media_playing());
                }
                rt_handle.spawn(async move {
                    penumbra_services::mpris::send_player_command(&bus, "PlayPause").await;
                });
            }
        });
    }
    {
        let bus_ref = std::sync::Arc::clone(&active_mpris_bus);
        let rt_handle = rt.handle().clone();
        qs.on_media_prev(move || {
            if let Some(bus) = bus_ref.lock().ok().and_then(|g| g.clone()) {
                rt_handle.spawn(async move {
                    penumbra_services::mpris::send_player_command(&bus, "Previous").await;
                });
            }
        });
    }
    {
        let bus_ref = std::sync::Arc::clone(&active_mpris_bus);
        let rt_handle = rt.handle().clone();
        qs.on_media_next(move || {
            if let Some(bus) = bus_ref.lock().ok().and_then(|g| g.clone()) {
                rt_handle.spawn(async move {
                    penumbra_services::mpris::send_player_command(&bus, "Next").await;
                });
            }
        });
    }
    {
        let bus_ref = std::sync::Arc::clone(&active_mpris_bus);
        let rt_handle = rt.handle().clone();
        qs.on_media_seek_back(move || {
            if let Some(bus) = bus_ref.lock().ok().and_then(|g| g.clone()) {
                rt_handle.spawn(async move {
                    penumbra_services::mpris::send_player_seek(&bus, -10_000_000).await;
                });
            }
        });
    }
    {
        let bus_ref = std::sync::Arc::clone(&active_mpris_bus);
        let rt_handle = rt.handle().clone();
        qs.on_media_seek_forward(move || {
            if let Some(bus) = bus_ref.lock().ok().and_then(|g| g.clone()) {
                rt_handle.spawn(async move {
                    penumbra_services::mpris::send_player_seek(&bus, 10_000_000).await;
                });
            }
        });
    }

    qs.on_brightness_changed(|val| {
        info!("QuickSettings: Brightness set to {:.2}", val);
        penumbra_shell::display::save_brightness(val as f64);
        if let Err(error) = apply_backlight_ratio(val as f64) {
            warn!(?error, "Failed to apply system backlight level");
        }
    });

    let volume_dismiss_timer = Rc::new(RefCell::new(Timer::default()));
    {
        let volume_dismiss_timer = Rc::clone(&volume_dismiss_timer);
        let qs_weak = qs_weak.clone();
        qs.on_volume_changed(move |val| {
            info!("QuickSettings: Volume set to {:.2}", val);
            apply_volume_ratio(val as f64);
            let mut s = ShellState::load();
            s.volume_level = val;
            let _ = s.save();

            let weak = qs_weak.clone();
            volume_dismiss_timer.borrow().start(
                TimerMode::SingleShot,
                Duration::from_millis(2500),
                move || {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_show_volume_modal(false);
                    }
                    let mut s = ShellState::load();
                    s.volume_modal_open = false;
                    let _ = s.save();
                },
            );
        });
    }

    {
        let qs_weak = qs_weak.clone();
        qs.on_dismiss_volume_modal(move || {
            info!("Volume Modal dismissed");
            if let Some(ui) = qs_weak.upgrade() {
                ui.set_show_volume_modal(false);
            }
            let mut s = ShellState::load();
            s.volume_modal_open = false;
            let _ = s.save();
        });
    }

    // -----------------------------------------------------------------------
    // 10. Navigation Bar Callbacks
    // -----------------------------------------------------------------------
    nav_bar.on_keyboard_toggle_clicked(|| {
        let path = runtime_path("penumbra-keyboard-cmd");
        let _ = std::fs::write(path, "toggle");
        info!("Navigation: Toggled virtual keyboard");
    });

    nav_bar.on_touch_down(|_, _| {});
    nav_bar.on_touch_move(|_, _| {});
    nav_bar.on_touch_up(|_, _| {});

    nav_bar.on_nav_home(|| {
        info!("Navigation: Home button invoked");
        let mut state = ShellState::load();
        state.mode = PhoneMode::Home;
        state.drawer_open = false;
        let _ = state.save();
    });

    nav_bar.on_nav_recents(|| {
        info!("Navigation: Recents button invoked");
        let mut state = ShellState::load();
        state.mode = PhoneMode::Overview;
        state.drawer_open = false;
        state.overview_index = 0;
        let _ = state.save();
    });

    nav_bar.on_nav_next_app(|| {});
    nav_bar.on_nav_prev_app(|| {});

    // -----------------------------------------------------------------------
    // 10. Mobile Lockscreen Controller (In-House Cross-Distro PAM/Hash Engine)
    // -----------------------------------------------------------------------
    let pin_buffer = Rc::new(RefCell::new(String::new()));
    let last_known_locked = Rc::new(Cell::new(false));
    let do_unlock = {
        let lock_weak = lockscreen_weak.clone();
        let pin_buf = Rc::clone(&pin_buffer);
        let locked_ref = Rc::clone(&last_known_locked);
        Rc::new(move || {
            info!("Mobile lockscreen: unlocked successfully.");
            locked_ref.set(false);
            if let Some(ls) = lock_weak.upgrade() {
                ls.set_is_locked(false);
                ls.set_in_challenge(false);
                ls.set_pin_length(0);
                ls.set_is_error(false);
                ls.set_status_message("".into());
                ls.window().request_redraw();
            }
            if let Ok(mut b) = pin_buf.try_borrow_mut() {
                b.clear();
            }
            layer_shell_slint::sync_input_region(
                "penumbra-lockscreen",
                logical_w as i32,
                logical_h as i32,
                0,
                false,
            );
            let mut s = ShellState::load();
            s.is_locked = false;
            let _ = s.save();
            let _ = std::fs::write("/tmp/penumbra-lock.state", "locked=0\n");
            let _ = std::process::Command::new("penumbra-keyboard-cmd")
                .arg("hide")
                .spawn();
        })
    };

    let do_lock = {
        let lock_weak = lockscreen_weak.clone();
        let pin_buf = Rc::clone(&pin_buffer);
        let locked_ref = Rc::clone(&last_known_locked);
        Rc::new(move || {
            let sec_cfg = SecurityConfig::load();
            if sec_cfg.lock_type == LockType::None {
                info!("Mobile lockscreen: lock_type is None (no lockscreen), skipping lock.");
                return;
            }
            info!("Mobile lockscreen: activated (mode={:?}).", sec_cfg.lock_type);
            locked_ref.set(true);
            if let Ok(mut b) = pin_buf.try_borrow_mut() {
                b.clear();
            }
            if let Some(ls) = lock_weak.upgrade() {
                ls.set_lock_type(sec_cfg.lock_type.as_str().into());
                ls.set_has_fingerprint(FingerprintManager::has_sensor());
                ls.set_fingerprint_enabled(sec_cfg.fingerprint_enabled && FingerprintManager::has_sensor());
                ls.set_is_locked(true);
                ls.set_in_challenge(false);
                ls.set_pin_length(0);
                ls.set_is_error(false);
                ls.set_status_message("".into());
                ls.window().request_redraw();
            }
            layer_shell_slint::sync_input_region(
                "penumbra-lockscreen",
                logical_w as i32,
                logical_h as i32,
                logical_h as i32,
                true,
            );
            let mut s = ShellState::load();
            s.is_locked = true;
            let _ = s.save();
            let _ = std::fs::write("/tmp/penumbra-lock.state", "locked=1\n");
        })
    };

    {
        let unlock = Rc::clone(&do_unlock);
        lockscreen.on_unlock_succeeded(move || {
            unlock();
        });
    }

    {
        let lock_weak = lockscreen_weak.clone();
        let pin_buf = Rc::clone(&pin_buffer);
        let unlock = Rc::clone(&do_unlock);
        lockscreen.on_append_pin(move |digit| {
            if let Some(ls) = lock_weak.upgrade() {
                ls.set_is_error(false);
                ls.set_status_message("".into());
                let (current_pin, len) = {
                    let mut buf = pin_buf.borrow_mut();
                    if buf.len() < 12 {
                        buf.push_str(digit.as_str());
                    }
                    (buf.clone(), buf.len())
                };
                ls.set_pin_length(len as i32);
                let sec_cfg = SecurityConfig::load();
                if sec_cfg.verify_secret(&current_pin) {
                    unlock();
                }
            }
        });
    }

    {
        let lock_weak = lockscreen_weak.clone();
        let pin_buf = Rc::clone(&pin_buffer);
        lockscreen.on_backspace_pin(move || {
            if let Some(ls) = lock_weak.upgrade() {
                ls.set_is_error(false);
                ls.set_status_message("".into());
                let len = {
                    let mut buf = pin_buf.borrow_mut();
                    buf.pop();
                    buf.len()
                };
                ls.set_pin_length(len as i32);
            }
        });
    }

    {
        let lock_weak = lockscreen_weak.clone();
        let pin_buf = Rc::clone(&pin_buffer);
        let unlock = Rc::clone(&do_unlock);
        lockscreen.on_submit_pin(move || {
            if let Some(ls) = lock_weak.upgrade() {
                let current_pin = pin_buf.borrow().clone();
                let sec_cfg = SecurityConfig::load();
                if sec_cfg.verify_secret(&current_pin) {
                    unlock();
                } else {
                    ls.set_is_error(true);
                    ls.set_status_message("Wrong PIN".into());
                    if let Ok(mut b) = pin_buf.try_borrow_mut() {
                        b.clear();
                    }
                    ls.set_pin_length(0);
                }
            }
        });
    }

    {
        let lock_weak = lockscreen_weak.clone();
        let unlock = Rc::clone(&do_unlock);
        lockscreen.on_submit_password(move |pw| {
            if let Some(ls) = lock_weak.upgrade() {
                let sec_cfg = SecurityConfig::load();
                if sec_cfg.verify_secret(pw.as_str()) {
                    unlock();
                } else {
                    ls.set_is_error(true);
                    ls.set_status_message("Wrong password".into());
                }
            }
        });
    }

    {
        let lock_weak = lockscreen_weak.clone();
        let unlock = Rc::clone(&do_unlock);
        lockscreen.on_fingerprint_auth_requested(move || -> bool {
            if let Ok(true) = FingerprintManager::verify_fingerprint() {
                unlock();
                true
            } else {
                if let Some(ls) = lock_weak.upgrade() {
                    ls.set_is_error(true);
                    ls.set_status_message("Fingerprint not recognized".into());
                }
                false
            }
        });
    }

    lockscreen.on_keyboard_show_requested(move || {
        let _ = std::process::Command::new("penumbra-keyboard-cmd")
            .arg("show")
            .spawn();
    });

    lockscreen.on_keyboard_hide_requested(move || {
        let _ = std::process::Command::new("penumbra-keyboard-cmd")
            .arg("hide")
            .spawn();
    });

    {
        let lock_weak = lockscreen_weak.clone();
        lockscreen.on_emergency_clicked(move || {
            if let Some(ls) = lock_weak.upgrade() {
                ls.set_status_message("Emergency dialer".into());
            }
        });
    }

    // -----------------------------------------------------------------------
    // 11. Unified Telemetry: Single Clock Timer & Single Tokio Runtime
    // -----------------------------------------------------------------------
    let clock_timer = Timer::default();
    {
        let tb = top_bar_weak.clone();
        let q = qs_weak.clone();
        let lk = lockscreen_weak.clone();
        let mut last_lang = penumbra_services::i18n::active_lang_code();
        let mut update_time = move || {
            let cur_lang = penumbra_services::i18n::active_lang_code();
            if cur_lang != last_lang {
                let _ = slint::select_bundled_translation(if cur_lang == "en" { "" } else { &cur_lang });
                last_lang = cur_lang;
            }
            let now = Local::now();
            let time_str = now.format("%H:%M").to_string();
            let date_str = now.format("%A, %b %-d").to_string();
            if let Some(bar) = tb.upgrade() {
                bar.set_time_text(time_str.clone().into());
            }
            if let Some(shade) = q.upgrade() {
                shade.set_time_text(time_str.clone().into());
                shade.set_date_text(date_str.clone().into());
            }
            if let Some(ls) = lk.upgrade() {
                ls.set_time_str(time_str.into());
                ls.set_date_str(date_str.into());
            }
        };
        update_time();
        clock_timer.start(TimerMode::Repeated, Duration::from_millis(1000), update_time);
    }

    top_bar.set_dnd_active(ShellState::load().dnd_active);
    refresh_tiles();
    tracing::debug!("SystemUI init: telemetry tasks spawning");

    // Incoming cellular call monitor: event-driven D-Bus listener launches `penumbra-calls` immediately on RingingIn
    std::thread::spawn(|| {
        let mut seen_ringing = std::collections::HashSet::<String>::new();
        penumbra_services::telephony::watch_call_events(move || {
            if let Ok(calls) = penumbra_services::telephony::list_active_calls() {
                let mut current_paths = std::collections::HashSet::<String>::new();
                for c in calls {
                    current_paths.insert(c.path.clone());
                    if c.state == penumbra_services::telephony::CallState::RingingIn
                        && !seen_ringing.contains(&c.path)
                    {
                        seen_ringing.insert(c.path.clone());
                        let already_open = std::process::Command::new("pgrep")
                            .args(["-x", "penumbra-calls"])
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false);
                        if !already_open {
                            info!("Incoming call from {} ({}): launching penumbra-calls", c.number, c.path);
                            if let Ok(mut child) = std::process::Command::new("penumbra-calls").spawn() {
                                std::thread::spawn(move || {
                                    let _ = child.wait();
                                });
                            }
                        }
                    }
                }
                seen_ringing.retain(|p| current_paths.contains(p));
            }
        });
    });

    enum TileSyncMsg {
        Radios { wifi: bool, bt: bool, airplane: bool },
        Internet { connected: bool, ssid: String },
    }
    let (tile_sync_tx, tile_sync_rx) = std::sync::mpsc::channel::<TileSyncMsg>();
    let tile_sync_timer = Timer::default();
    {
        let registry = Rc::clone(&registry);
        let refresh = Rc::clone(&refresh_tiles);
        tile_sync_timer.start(TimerMode::Repeated, Duration::from_millis(500), move || {
            let mut changed = false;
            while let Ok(msg) = tile_sync_rx.try_recv() {
                let mut reg = registry.borrow_mut();
                match msg {
                    TileSyncMsg::Radios { wifi, bt, airplane } => {
                        for (id, active, on_sub, off_sub) in [
                            ("internet", wifi, "Connected", "Off"),
                            ("bluetooth", bt, "On", "Off"),
                            ("airplane", airplane, "On", "Off"),
                        ] {
                            if let Some(t) = reg.iter_mut().find(|t| t.id == id) {
                                t.state = if active { TileState::Active } else { TileState::Inactive };
                                t.subtitle = if active { on_sub } else { off_sub }.into();
                            }
                        }
                        changed = true;
                    }
                    TileSyncMsg::Internet { connected, ssid } => {
                        if let Some(t) = reg.iter_mut().find(|t| t.id == "internet") {
                            t.state = if connected { TileState::Active } else { TileState::Inactive };
                            t.subtitle = if connected { ssid } else { "Off".into() };
                            changed = true;
                        }
                    }
                }
            }
            if changed {
                refresh();
            }
        });
    }

    // Probe initial radio states in background so startup is never blocked by D-Bus/nmcli/bluetoothctl
    {
        let tx = tile_sync_tx.clone();
        rt.spawn(async move {
            if let Ok((wifi, bt, airplane)) = tokio::task::spawn_blocking(|| {
                (
                    tile::is_wifi_active(),
                    tile::is_bluetooth_active(),
                    tile::is_airplane_active(),
                )
            })
            .await
            {
                let _ = tx.send(TileSyncMsg::Radios { wifi, bt, airplane });
            }
        });
    }

    // Initialize BatteryService asynchronously and subscribe to updates
    {
        let tb = top_bar_weak.clone();
        let q = qs_weak.clone();
        let lk = lockscreen_weak.clone();
        let tx = ui_tx.clone();
        rt.spawn(async move {
            let battery_service = BatteryService::new().await;
            let push_bat = |info: penumbra_services::battery::BatteryInfo,
                            tb: &slint::Weak<TopBarWindow>,
                            q: &slint::Weak<QuickSettingsWindow>,
                            lk: &slint::Weak<LockscreenWindow>,
                            tx: &std::sync::mpsc::Sender<Box<dyn FnOnce() + Send>>| {
                let tb = tb.clone();
                let q = q.clone();
                let lk = lk.clone();
                let _ = tx.send(Box::new(move || {
                    if let Some(bar) = tb.upgrade() {
                        bar.set_battery_pct(info.percentage as f32);
                        bar.set_is_charging(info.is_charging);
                    }
                    if let Some(shade) = q.upgrade() {
                        shade.set_battery_pct(info.percentage as i32);
                    }
                    if let Some(ls) = lk.upgrade() {
                        ls.set_battery_pct(info.percentage as i32);
                        ls.set_is_charging(info.is_charging);
                    }
                }));
            };
            push_bat(battery_service.current(), &tb, &q, &lk, &tx);
            let mut bat_rx = battery_service.subscribe();
            while bat_rx.changed().await.is_ok() {
                push_bat(bat_rx.borrow().clone(), &tb, &q, &lk, &tx);
            }
        });
    }

    // Initialize NetworkService asynchronously and subscribe to updates
    {
        let tb = top_bar_weak.clone();
        let q = qs_weak.clone();
        let lk = lockscreen_weak.clone();
        let tx = tile_sync_tx;
        let ui_tx = ui_tx.clone();
        rt.spawn(async move {
            let network_service = NetworkService::new().await;
            let push_net = |info: penumbra_services::network::NetworkInfo,
                            tb: &slint::Weak<TopBarWindow>,
                            q: &slint::Weak<QuickSettingsWindow>,
                            lk: &slint::Weak<LockscreenWindow>,
                            tx: &std::sync::mpsc::Sender<TileSyncMsg>,
                            ui_tx: &std::sync::mpsc::Sender<Box<dyn FnOnce() + Send>>| {
                let _ = tx.send(TileSyncMsg::Internet {
                    connected: info.is_connected,
                    ssid: info.ssid.clone(),
                });
                let tb = tb.clone();
                let q = q.clone();
                let lk = lk.clone();
                let _ = ui_tx.send(Box::new(move || {
                    if let Some(bar) = tb.upgrade() {
                        bar.set_wifi_connected(info.is_connected);
                        bar.set_wifi_signal(info.signal_strength as i32);
                        bar.set_carrier_text(info.ssid.clone().into());
                    }
                    if let Some(shade) = q.upgrade() {
                        shade.set_carrier_text(info.ssid.clone().into());
                    }
                    if let Some(ls) = lk.upgrade() {
                        ls.set_wifi_connected(info.is_connected);
                        ls.set_wifi_signal(info.signal_strength as i32);
                        ls.set_carrier_str(info.ssid.into());
                    }
                }));
            };
            push_net(network_service.current(), &tb, &q, &lk, &tx, &ui_tx);
            let mut net_rx = network_service.subscribe();
            while net_rx.changed().await.is_ok() {
                push_net(net_rx.borrow().clone(), &tb, &q, &lk, &tx, &ui_tx);
            }
        });
    }

    // -----------------------------------------------------------------------
    // 12. External IPC Watcher (Volume Modal & External Commands)
    // -----------------------------------------------------------------------
    let ipc_timer = Timer::default();
    {
        let animate_to = Rc::clone(&animate_to);
        let spring_state = Rc::clone(&spring_state);
        let spring_timer_ipc = Rc::clone(&spring_timer);
        let is_animating_ipc = Rc::clone(&is_animating);
        let tb = top_bar_weak.clone();
        let last_known_open = Rc::new(RefCell::new(false));
        let last_known_dragging = Rc::new(RefCell::new(false));
        let last_known_qs = Rc::new(RefCell::new(false));
        let last_known_power_dialog = Rc::new(RefCell::new(false));
        let last_known_volume_modal = Rc::new(RefCell::new(false));
        let last_known_volume = Rc::new(RefCell::new(-1.0f32));
        let volume_dismiss_timer_clone = Rc::clone(&volume_dismiss_timer);
        let state_socket = StateSocket::bind(&runtime_path(SHELL_STATE_SOCK)).ok();
        let q = qs_weak.clone();
        let do_lock_ipc = Rc::clone(&do_lock);
        let do_unlock_ipc = Rc::clone(&do_unlock);
        let last_known_locked_ipc = Rc::clone(&last_known_locked);

        ipc_timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            // Check for lock / unlock external commands
            if std::path::Path::new("/tmp/penumbra-lock.cmd").exists() {
                if let Ok(cmd) = std::fs::read_to_string("/tmp/penumbra-lock.cmd") {
                    let _ = std::fs::remove_file("/tmp/penumbra-lock.cmd");
                    match cmd.trim() {
                        "lock" => do_lock_ipc(),
                        "unlock" => do_unlock_ipc(),
                        _ => {}
                    }
                }
            }
            if is_animating_ipc.get() {
                // If SystemUI's spring is currently animating the shade, don't let stale loopback packets cancel it
                return;
            }

            let state = if let Some(ref sock) = state_socket {
                let mut latest = None;
                while let Some((st, _seq)) = sock.try_recv() {
                    latest = Some(st);
                }
                latest.unwrap_or_else(ShellState::load)
            } else {
                ShellState::load()
            };

            // Direct sync of lockscreen state from compositor
            if state.is_locked != last_known_locked_ipc.get() {
                last_known_locked_ipc.set(state.is_locked);
                if state.is_locked {
                    do_lock_ipc();
                } else {
                    do_unlock_ipc();
                }
            }

            let mut last_open = last_known_open.borrow_mut();
            let mut was_dragging = last_known_dragging.borrow_mut();
            let mut last_qs = last_known_qs.borrow_mut();

            // Direct sync of partition mode
            if state.is_full_qs != *last_qs {
                *last_qs = state.is_full_qs;
                if let Some(shade) = q.upgrade() {
                    shade.set_is_full_qs(state.is_full_qs);
                }
            }

            if state.drawer_dragging {
                *was_dragging = true;
                spring_timer_ipc.borrow().stop();
                let disp = state.drawer_pull_dy;
                let new_y = disp - 915.0;
                let progress = (disp / 915.0).clamp(0.0, 1.0) as f32;
                if let Some(shade) = q.upgrade() {
                    shade.set_slide_offset_y(new_y as f32);
                    shade.set_expansion_progress(progress);
                    shade.set_is_full_qs(state.is_full_qs);
                }
                if let Some(bar) = tb.upgrade() {
                    bar.set_shade_progress(progress);
                }
                let mut sp = spring_state.borrow_mut();
                let now = Instant::now();
                let mut last_t = last_spring_time.borrow_mut();
                let dt = now.duration_since(*last_t).as_secs_f64().clamp(0.001, 0.05);
                *last_t = now;
                let v = (new_y - sp.position) / dt;
                sp.velocity = sp.velocity * 0.3 + v * 0.7;
                sp.position = new_y;
            } else {
                if *was_dragging {
                    *was_dragging = false;
                    // Native drag just completed -> settle spring to target
                    let target_y = if state.drawer_open {
                        0.0
                    } else {
                        -915.0
                    };
                    let initial_vel = spring_state.borrow().velocity.clamp(-2500.0, 2500.0);
                    info!("Native drag completed -> spring settling to {:.1} with vel {:.1} (is_full_qs={})", target_y, initial_vel, state.is_full_qs);
                    if let Some(shade) = q.upgrade() {
                        shade.set_is_full_qs(state.is_full_qs);
                    }
                    animate_to(target_y, initial_vel);
                } else if state.drawer_open != *last_open {
                    *last_open = state.drawer_open;
                    let cur_target = spring_state.borrow().target;
                    if state.drawer_open {
                        if cur_target < -570.0 {
                            let target_y = 0.0;
                            info!("External trigger: opening QuickSettings shade (target={:.1}, is_full_qs={})", target_y, state.is_full_qs);
                            if let Some(shade) = q.upgrade() {
                                shade.set_is_full_qs(state.is_full_qs);
                            }
                            animate_to(target_y, 400.0);
                        }
                    } else if cur_target > -900.0 {
                        info!("External trigger: closing QuickSettings shade");
                        animate_to(-915.0, -400.0);
                    }
                }
            }

            // Power Dialog sync
            let mut last_power = last_known_power_dialog.borrow_mut();
            if state.power_dialog_open != *last_power {
                *last_power = state.power_dialog_open;
                if let Some(ui) = q.upgrade() {
                    ui.set_show_power_dialog(state.power_dialog_open);
                }
            }

            if std::path::Path::new("/tmp/open_shade").exists() {
                let _ = std::fs::remove_file("/tmp/open_shade");
                animate_to(0.0, 500.0);
            }
            if std::path::Path::new("/tmp/close_shade").exists() {
                let _ = std::fs::remove_file("/tmp/close_shade");
                animate_to(-915.0, -500.0);
            }
            if std::path::Path::new("/tmp/open_full_qs").exists() {
                let _ = std::fs::remove_file("/tmp/open_full_qs");
                if let Some(shade) = q.upgrade() {
                    shade.set_is_full_qs(true);
                }
            }
            if std::path::Path::new("/tmp/close_full_qs").exists() {
                let _ = std::fs::remove_file("/tmp/close_full_qs");
                if let Some(shade) = q.upgrade() {
                    shade.set_is_full_qs(false);
                }
            }

            // Volume Modal sync
            let mut last_vol_modal = last_known_volume_modal.borrow_mut();
            let mut last_vol = last_known_volume.borrow_mut();
            let vol_diff = (state.volume_level - *last_vol).abs();
            if state.volume_modal_open != *last_vol_modal
                || (state.volume_modal_open && vol_diff > 0.01)
            {
                *last_vol_modal = state.volume_modal_open;
                *last_vol = state.volume_level;
                if let Some(ui) = q.upgrade() {
                    ui.set_show_volume_modal(state.volume_modal_open);
                    ui.set_volume(state.volume_level);
                }
                if state.volume_modal_open {
                    let weak = q.clone();
                    volume_dismiss_timer_clone.borrow().start(
                        TimerMode::SingleShot,
                        Duration::from_millis(2500),
                        move || {
                            if let Some(ui) = weak.upgrade() {
                                ui.set_show_volume_modal(false);
                            }
                            let mut s = ShellState::load();
                            s.volume_modal_open = false;
                            let _ = s.save();
                        },
                    );
                }
            } else if !state.volume_modal_open && vol_diff > 0.01 {
                *last_vol = state.volume_level;
                if let Some(ui) = q.upgrade() {
                    ui.set_volume(state.volume_level);
                }
            }
        });
    }

    info!("Consolidated Penumbra SystemUI running layer-shell event loop...");
    top_bar.run()?;

    Ok(())
}
