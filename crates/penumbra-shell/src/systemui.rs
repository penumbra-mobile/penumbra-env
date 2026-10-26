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

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use chrono::Local;
use slint::{ComponentHandle, Global, Model, ModelRc, Timer, TimerMode, VecModel};
use tracing::{info, warn};

use layer_shell_slint::{Anchor, KeyboardInteractivity, Layer, LayerConfig};
use penumbra_gestures::Spring;
use penumbra_services::{
    apply_backlight_ratio, apply_volume_ratio, read_backlight_ratio, read_volume_ratio,
    BatteryService, NetworkService,
};
use penumbra_shell::apply_monet_palette;
use penumbra_shell::ipc::{runtime_path, PhoneMode, ShellState, StateSocket, SHELL_STATE_SOCK};
use penumbra_shell::quick_settings::{tile, QuickSettingsConfig, TileDefinition, TileState};

use crate::{NavBarWindow, NotificationData, QuickSettingsWindow, Theme, TileData, TopBarWindow};

/// Release/snap rule (offset coordinates): closed -915, compact -565 (350px), full 0.
fn release_target(y_current: f64, v_y: f64) -> f64 {
    // Fast pull up gesture: any upward flick (v_y < -120) immediately closes shade
    if v_y < -120.0 {
        return -915.0;
    }
    // Moderate upward drag from compact shade or lower half closes
    if v_y < -40.0 {
        if y_current < 600.0 {
            return -915.0;
        } else {
            return -565.0;
        }
    }
    // Fast downward flick -> full expansion
    if v_y > 400.0 {
        return 0.0;
    }
    // Downward gesture from top
    if v_y > 60.0 {
        if y_current > 420.0 {
            return 0.0;
        } else {
            return -565.0;
        }
    }
    // Positional snap thresholds
    if y_current < 160.0 {
        -915.0
    } else if y_current < 500.0 {
        -565.0
    } else {
        0.0
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    info!("Initializing Penumbra Consolidated SystemUI (Phase 6C)...");

    let (logical_w, logical_h, _scale) = layer_shell_slint::mobile_output_geometry();

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
        .with_anchor(Anchor::Top | Anchor::Left | Anchor::Right)
        .with_size(logical_w, logical_h)
        .with_exclusive_zone(0)
        .with_keyboard_interactivity(KeyboardInteractivity::None);

    let nav_bar_config = LayerConfig::default()
        .with_namespace("penumbra-nav-bar")
        .with_layer(Layer::Top)
        .with_anchor(Anchor::Bottom | Anchor::Left | Anchor::Right)
        .with_size(logical_w, 32)
        .with_exclusive_zone(32)
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

    // Enqueue NavBar config and instantiate
    layer_shell_slint::set_next_window_config(nav_bar_config);
    let nav_bar = NavBarWindow::new()?;
    let nav_bar_weak = nav_bar.as_weak();

    info!("Instantiated TopBar, QuickSettings, and NavBar layer-shell surfaces");

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
        apply_monet_palette!(t_nav, initial_palette);
    }

    let _theme_timer = {
        let tb = top_bar_weak.clone();
        let q = qs_weak.clone();
        let nb = nav_bar_weak.clone();
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
                apply_monet_palette!(theme, palette);
            }
        })
    };

    // Initially retracted off-screen: y = -915px
    qs.set_slide_offset_y(-915.0);
    qs.set_expansion_progress(0.0);

    // Initial hardware telemetry for sliders
    let init_brightness = read_backlight_ratio().unwrap_or(0.75);
    qs.set_brightness(init_brightness as f32);
    let init_volume = read_volume_ratio();
    qs.set_volume(init_volume as f32);

    // -----------------------------------------------------------------------
    // 4. Spring Physics Engine (Stiffness: 300.0, Damping: 26.0)
    // -----------------------------------------------------------------------
    let spring_state = Rc::new(RefCell::new(
        Spring::from_physics(300.0, 26.0, 1.0)
            .with_position(-915.0)
            .with_target(-915.0),
    ));
    let spring_timer = Rc::new(RefCell::new(Timer::default()));
    let last_spring_time = Rc::new(RefCell::new(Instant::now()));

    let animate_to = {
        let spring_state = Rc::clone(&spring_state);
        let spring_timer = Rc::clone(&spring_timer);
        let last_spring_time = Rc::clone(&last_spring_time);
        let qs_weak = qs_weak.clone();

        move |target: f64, initial_vel: f64| {
            {
                let mut sp = spring_state.borrow_mut();
                sp.target = target;
                sp.velocity = initial_vel;
            }
            *last_spring_time.borrow_mut() = Instant::now();

            let spring_state = Rc::clone(&spring_state);
            let spring_timer_inner = Rc::clone(&spring_timer);
            let last_spring_time = Rc::clone(&last_spring_time);
            let qs_weak = qs_weak.clone();

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

                    let is_settled = sp.snap_if_settled(1.0, 5.0);
                    let mut state = ShellState::load();
                    if is_settled {
                        spring_timer_inner.borrow().stop();
                        let target = sp.target;
                        let final_progress = ((target + 915.0) / 915.0).clamp(0.0, 1.0) as f32;
                        if let Some(ui) = qs_weak.upgrade() {
                            ui.set_slide_offset_y(target as f32);
                            ui.set_expansion_progress(final_progress);
                        }
                        state.drawer_open = target > -900.0;
                        state.drawer_pull_dy = if state.drawer_open { 915.0 } else { 0.0 };
                        state.drawer_progress = if state.drawer_open { 1.0 } else { 0.0 };
                        state.drawer_dragging = false;
                    } else {
                        state.drawer_open = true;
                        state.drawer_pull_dy = (new_y + 915.0).clamp(0.0, 915.0);
                        state.drawer_progress = progress as f64;
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
        let spring_timer = Rc::clone(&spring_timer);
        let spring_state = Rc::clone(&spring_state);
        let top_drag_velocity = Rc::clone(&top_drag_velocity);
        let last_top_pull_y = Rc::clone(&last_top_pull_y);
        let last_top_pull_time = Rc::clone(&last_top_pull_time);

        top_bar.on_top_bar_pulled_down(move |dy| {
            // Stop any running settle animation on QuickSettings
            spring_timer.borrow().stop();

            let pull = dy as f64;
            let dt = last_top_pull_time
                .borrow()
                .elapsed()
                .as_secs_f64()
                .clamp(0.001, 0.1);
            let last_y = *last_top_pull_y.borrow();
            let instant_v = (pull - last_y) / dt;
            *top_drag_velocity.borrow_mut() = *top_drag_velocity.borrow() * 0.6 + instant_v * 0.4;
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
            let _ = state.save();
        });
    }

    {
        let animate_to = Rc::clone(&animate_to);
        let top_drag_velocity = Rc::clone(&top_drag_velocity);
        let spring_state = Rc::clone(&spring_state);

        top_bar.on_top_bar_released(move || {
            let y_current = spring_state.borrow().position + 915.0;
            let v = *top_drag_velocity.borrow();
            let target = release_target(y_current, v);
            info!(
                "TopBar released: y_current={:.1}px, v={:.1}px/s -> target={:.1}",
                y_current, v, target
            );

            let mut state = ShellState::load();
            state.drawer_dragging = false;
            let _ = state.save();

            animate_to(target, v);
        });
    }

    // -----------------------------------------------------------------------
    // 6. QuickSettings Own Drag & Kinetic Handlers
    // -----------------------------------------------------------------------
    {
        let spring_timer = Rc::clone(&spring_timer);
        let spring_state = Rc::clone(&spring_state);
        let qs_weak = qs_weak.clone();

        qs.on_drag_delta(move |dy| {
            spring_timer.borrow().stop();
            if let Some(ui) = qs_weak.upgrade() {
                let cur_offset = ui.get_slide_offset_y() as f64;
                let delta = dy as f64;

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
        qs.on_drag_released(move |cur_y, vel| {
            let y_offset = cur_y as f64;
            let v_y = vel as f64;
            let y_current = y_offset + 915.0;
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

    // -----------------------------------------------------------------------
    // 7. Tile Engine (Registry + Persistent Config)
    // -----------------------------------------------------------------------
    let registry: Rc<RefCell<Vec<TileDefinition>>> = Rc::new(RefCell::new(tile::registry()));
    let qs_config = Rc::new(RefCell::new(QuickSettingsConfig::load()));

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

    {
        let registry = Rc::clone(&registry);
        let refresh = Rc::clone(&refresh_tiles);
        qs.on_tile_clicked(move |id| {
            if let Some(t) = registry
                .borrow_mut()
                .iter_mut()
                .find(|t| t.id == id.as_str())
            {
                t.toggle();
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
    let notifs_model = Rc::new(VecModel::from(vec![NotificationData {
        app: "Penumbra Core".into(),
        title: "Wayland Ready • M3 Expressive".into(),
        body: "Consolidated SystemUI and Dual-Kawase blur active".into(),
    }]));
    qs.set_notifications(ModelRc::from(Rc::clone(&notifs_model)));

    {
        let notifs = Rc::clone(&notifs_model);
        qs.on_notification_dismissed(move |idx| {
            if (idx as usize) < notifs.row_count() {
                info!("QuickSettings: Notification at index {idx} dismissed");
                notifs.remove(idx as usize);
            }
        });
    }
    {
        let notifs = Rc::clone(&notifs_model);
        qs.on_clear_notifications(move || {
            info!("QuickSettings: Cleared all notifications");
            while notifs.row_count() > 0 {
                notifs.remove(0);
            }
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

    qs.on_brightness_changed(|val| {
        info!("QuickSettings: Brightness set to {:.2}", val);
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
    // 11. Unified Telemetry: Single Clock Timer & Single Tokio Runtime
    // -----------------------------------------------------------------------
    let clock_timer = Timer::default();
    {
        let tb = top_bar_weak.clone();
        let q = qs_weak.clone();
        let update_time = move || {
            let now = Local::now();
            let time_str = now.format("%H:%M").to_string();
            let date_str = now.format("%A, %b %-d").to_string();
            if let Some(bar) = tb.upgrade() {
                bar.set_time_text(time_str.clone().into());
            }
            if let Some(shade) = q.upgrade() {
                shade.set_time_text(time_str.into());
                shade.set_date_text(date_str.into());
            }
        };
        update_time();
        clock_timer.start(TimerMode::Repeated, Duration::from_millis(1000), update_time);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let (battery_service, network_service) = rt.block_on(async {
        let bat = BatteryService::new().await;
        let net = NetworkService::new().await;
        (bat, net)
    });

    // Initial Telemetry push to both windows
    {
        let bat_info = battery_service.current();
        let net_info = network_service.current();

        top_bar.set_battery_pct(bat_info.percentage as f32);
        top_bar.set_is_charging(bat_info.is_charging);
        top_bar.set_wifi_connected(net_info.is_connected);
        top_bar.set_wifi_signal(net_info.signal_strength as i32);
        top_bar.set_carrier_text(net_info.ssid.clone().into());

        qs.set_battery_pct(bat_info.percentage as i32);
        qs.set_carrier_text(net_info.ssid.clone().into());

        if let Some(t) = registry
            .borrow_mut()
            .iter_mut()
            .find(|t| t.id == "internet")
        {
            t.state = if net_info.is_connected {
                TileState::Active
            } else {
                TileState::Inactive
            };
            t.subtitle = if net_info.is_connected {
                net_info.ssid.clone()
            } else {
                "Off".into()
            };
        }
        refresh_tiles();
    }

    // Subscribe to async telemetry streams and push simultaneously
    {
        let mut bat_rx = battery_service.subscribe();
        let tb = top_bar_weak.clone();
        let q = qs_weak.clone();
        let _keep_bat = battery_service;
        rt.spawn(async move {
            let _bat = _keep_bat;
            while bat_rx.changed().await.is_ok() {
                let info = bat_rx.borrow().clone();
                let tb = tb.clone();
                let q = q.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(bar) = tb.upgrade() {
                        bar.set_battery_pct(info.percentage as f32);
                        bar.set_is_charging(info.is_charging);
                    }
                    if let Some(shade) = q.upgrade() {
                        shade.set_battery_pct(info.percentage as i32);
                    }
                });
            }
        });
    }

    {
        let mut net_rx = network_service.subscribe();
        let tb = top_bar_weak.clone();
        let q = qs_weak.clone();
        let _keep_net = network_service;
        rt.spawn(async move {
            let _net = _keep_net;
            while net_rx.changed().await.is_ok() {
                let info = net_rx.borrow().clone();
                let tb = tb.clone();
                let q = q.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(bar) = tb.upgrade() {
                        bar.set_wifi_connected(info.is_connected);
                        bar.set_wifi_signal(info.signal_strength as i32);
                        bar.set_carrier_text(info.ssid.clone().into());
                    }
                    if let Some(shade) = q.upgrade() {
                        shade.set_carrier_text(info.ssid.into());
                    }
                });
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
        let last_known_open = Rc::new(RefCell::new(false));
        let last_known_power_dialog = Rc::new(RefCell::new(false));
        let last_known_volume_modal = Rc::new(RefCell::new(false));
        let last_known_volume = Rc::new(RefCell::new(-1.0f32));
        let volume_dismiss_timer_clone = Rc::clone(&volume_dismiss_timer);
        let state_socket = StateSocket::bind(&runtime_path(SHELL_STATE_SOCK)).ok();
        let q = qs_weak.clone();

        ipc_timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            let state = if let Some(ref sock) = state_socket {
                let mut latest = None;
                while let Some((st, _seq)) = sock.try_recv() {
                    latest = Some(st);
                }
                latest.unwrap_or_else(ShellState::load)
            } else {
                ShellState::load()
            };

            let mut last_open = last_known_open.borrow_mut();

            // External open/close triggers (when not dragging)
            if state.drawer_open != *last_open {
                *last_open = state.drawer_open;
                if !state.drawer_dragging {
                    let cur_target = spring_state.borrow().target;
                    if state.drawer_open {
                        if cur_target < -570.0 {
                            info!("External trigger: opening QuickSettings shade (compact)");
                            animate_to(-565.0, 400.0);
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
