//! Standalone Native Wayland Desktop Convergence Shell binary.
//!
//! Provides the external monitor Top Desktop Panel, Bottom Dock, and Modal Application
//! Launcher for Penumbra Desktop Convergence Mode. Built with Slint and `layer-shell-slint`.

use std::rc::Rc;
use std::time::Duration;

use clap::Parser;
use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use layer_shell_slint::{Anchor, KeyboardInteractivity, LayerConfig};

mod apps;
mod cli;
mod telemetry;

use apps::{discover_desktop_apps, filter_apps, spawn_desktop_app, AppInfo};
use cli::CliArgs;
use telemetry::TelemetryManager;

slint::include_modules!();

fn convert_to_slint_entry(app: &AppInfo) -> DesktopAppEntry {
    DesktopAppEntry {
        id: app.id.clone().into(),
        name: app.name.clone().into(),
        exec: app.exec.clone().into(),
        category: app.category.clone().into(),
        icon_name: app.icon_name.clone().into(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let args = CliArgs::parse();
    if let Err(e) = run_shell(args) {
        let msg = e.to_string();
        if msg.contains("not found") {
            info!("Desktop shell waiting for display: {}", msg);
            return Ok(());
        }
        return Err(e);
    }
    Ok(())
}

fn run_shell(args: CliArgs) -> Result<(), Box<dyn std::error::Error>> {
    info!(
        "Starting Penumbra Desktop Convergence Shell on output '{}' (panel={}, dock={}, launcher={})",
        args.output, !args.dock_only && !args.launcher_only,
        !args.panel_only && !args.launcher_only,
        !args.panel_only && !args.dock_only
    );

    let run_panel = !args.dock_only && !args.launcher_only;
    let run_dock = !args.panel_only && !args.launcher_only;
    let run_launcher = !args.panel_only && !args.dock_only;

    // 1. Base LayerConfig for initialization
    let primary_config = if run_panel {
        LayerConfig::top_bar(36)
            .with_namespace("penumbra-desktop-panel")
            .with_output(&args.output)
            .with_exclusive_zone(36)
    } else if run_dock {
        LayerConfig::bottom_panel(64)
            .with_namespace("penumbra-desktop-dock")
            .with_output(&args.output)
            .with_exclusive_zone(64)
    } else {
        LayerConfig::overlay()
            .with_namespace("penumbra-desktop-launcher")
            .with_output(&args.output)
    };

    layer_shell_slint::init(primary_config)?;

    // 2. Discover Applications
    let all_apps = Rc::new(discover_desktop_apps());

    // 3. Optional Centered Launcher Component
    let launcher_opt = if run_launcher {
        layer_shell_slint::set_next_window_config(
            LayerConfig::overlay()
                .with_namespace("penumbra-desktop-launcher")
                .with_output(&args.output)
                .with_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right)
                .with_exclusive_zone(-1)
                .with_keyboard_interactivity(KeyboardInteractivity::OnDemand),
        );

        let launcher = DesktopLauncherWindow::new()?;
        let launcher_weak = launcher.as_weak();

        // Initial app population
        let initial_slint_apps: Vec<DesktopAppEntry> = all_apps.iter().map(convert_to_slint_entry).collect();
        let apps_model = Rc::new(VecModel::from(initial_slint_apps));
        launcher.set_apps(ModelRc::from(apps_model.clone()));

        // Search filtering callback
        let all_apps_search = all_apps.clone();
        let launcher_weak_search = launcher_weak.clone();
        let apps_model_search = apps_model.clone();
        launcher.on_search_query_changed(move |query| {
            if let Some(l) = launcher_weak_search.upgrade() {
                let cat = l.get_active_category();
                let filtered = filter_apps(&all_apps_search, query.as_str(), cat.as_str());
                let new_entries: Vec<DesktopAppEntry> = filtered.iter().map(convert_to_slint_entry).collect();
                apps_model_search.set_vec(new_entries);
            }
        });

        // Category filter callback
        let all_apps_cat = all_apps.clone();
        let launcher_weak_cat = launcher_weak.clone();
        let apps_model_cat = apps_model.clone();
        launcher.on_category_selected(move |category| {
            if let Some(l) = launcher_weak_cat.upgrade() {
                let q = l.get_search_text();
                let filtered = filter_apps(&all_apps_cat, q.as_str(), category.as_str());
                let new_entries: Vec<DesktopAppEntry> = filtered.iter().map(convert_to_slint_entry).collect();
                apps_model_cat.set_vec(new_entries);
            }
        });

        // App launched callback
        let output_name = args.output.clone();
        let launcher_weak_launch = launcher_weak.clone();
        launcher.on_app_clicked(move |id, exec| {
            info!("Desktop launcher launching app: id={}, exec={}", id, exec);
            let app_info = AppInfo {
                id: id.to_string(),
                name: id.to_string(),
                exec: exec.to_string(),
                category: "Utilities".into(),
                icon_name: "grid".into(),
                terminal: false,
            };
            let _ = spawn_desktop_app(&app_info, Some(&output_name));
            if let Some(l) = launcher_weak_launch.upgrade() {
                l.set_is_open(false);
                layer_shell_slint::sync_input_region("penumbra-desktop-launcher", 1920, 1080, 0, false);
            }
        });

        // Dismiss callback
        let launcher_weak_close = launcher_weak.clone();
        launcher.on_close_requested(move || {
            if let Some(l) = launcher_weak_close.upgrade() {
                l.set_is_open(false);
                layer_shell_slint::sync_input_region("penumbra-desktop-launcher", 1920, 1080, 0, false);
            }
        });

        // Initially closed and click-through
        launcher.set_is_open(false);
        layer_shell_slint::sync_input_region("penumbra-desktop-launcher", 1920, 1080, 0, false);
        launcher.show()?;

        Some(launcher)
    } else {
        None
    };

    let launcher_handle = launcher_opt.map(|l| l.as_weak());

    // Helper closure to toggle launcher
    let toggle_launcher = {
        let launcher_weak = launcher_handle.clone();
        move || {
            if let Some(ref handle) = launcher_weak {
                if let Some(launcher) = handle.upgrade() {
                    let new_state = !launcher.get_is_open();
                    launcher.set_is_open(new_state);
                    if new_state {
                        // Expand input region fullscreen
                        layer_shell_slint::sync_input_region(
                            "penumbra-desktop-launcher",
                            1920,
                            1080,
                            1080,
                            true,
                        );
                    } else {
                        // Collapse input region
                        layer_shell_slint::sync_input_region(
                            "penumbra-desktop-launcher",
                            1920,
                            1080,
                            0,
                            false,
                        );
                    }
                }
            }
        }
    };

    // 4. Optional Top Desktop Panel
    let panel_opt = if run_panel {
        layer_shell_slint::set_next_window_config(
            LayerConfig::top_bar(36)
                .with_namespace("penumbra-desktop-panel")
                .with_output(&args.output)
                .with_exclusive_zone(36),
        );

        let panel = DesktopPanelWindow::new()?;
        let panel_weak = panel.as_weak();

        // Wire launcher toggle
        let toggle_l = toggle_launcher.clone();
        panel.on_toggle_launcher(move || {
            toggle_l();
        });

        // Wire workspace switch
        let panel_weak_ws = panel_weak.clone();
        panel.on_switch_workspace(move |ws| {
            info!("Switched to workspace {}", ws);
            if let Some(p) = panel_weak_ws.upgrade() {
                p.set_active_workspace(ws);
            }
        });

        // Wire power clicked
        panel.on_power_clicked(|| {
            info!("Desktop power button clicked -> requesting power menu");
            let _ = std::process::Command::new("sh")
                .arg("-c")
                .arg("penumbra-settings --page system || true")
                .spawn();
        });

        panel.show()?;
        Some(panel)
    } else {
        None
    };

    // 5. Optional Desktop Dock
    let _dock_opt = if run_dock {
        layer_shell_slint::set_next_window_config(
            LayerConfig::bottom_panel(64)
                .with_namespace("penumbra-desktop-dock")
                .with_output(&args.output)
                .with_exclusive_zone(64),
        );

        let dock = DesktopDockWindow::new()?;
        let _dock_weak = dock.as_weak();

        // Wire launcher clicked
        let toggle_l = toggle_launcher.clone();
        dock.on_launcher_clicked(move || {
            toggle_l();
        });

        // Wire dock app clicked
        let output_dock = args.output.clone();
        let all_apps_dock = all_apps.clone();
        dock.on_app_clicked(move |app_id| {
            info!("Dock app clicked: {}", app_id);
            // Search in discovered apps or fallback
            let app_to_run = all_apps_dock
                .iter()
                .find(|a| a.id == app_id.as_str() || a.name.to_lowercase() == app_id.as_str().to_lowercase())
                .cloned()
                .unwrap_or_else(|| {
                    let exec = match app_id.as_str() {
                        "terminal" => "kgx",
                        "browser" => "firefox",
                        "files" => "nautilus",
                        "editor" => "gedit",
                        "settings" => "penumbra-settings",
                        other => other,
                    };
                    AppInfo {
                        id: app_id.to_string(),
                        name: app_id.to_string(),
                        exec: exec.into(),
                        category: "Utilities".into(),
                        icon_name: app_id.to_string(),
                        terminal: false,
                    }
                });

            let _ = spawn_desktop_app(&app_to_run, Some(&output_dock));
        });

        dock.show()?;
        Some(dock)
    } else {
        None
    };

    // 6. Asynchronous Hardware Telemetry & Timers
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let telemetry_mgr = Rc::new(rt.block_on(TelemetryManager::new()));

    // Periodic Clock & Telemetry Timer (1s)
    let panel_weak_timer = panel_opt.as_ref().map(|p| p.as_weak());
    let telemetry_mgr_timer = telemetry_mgr.clone();
    let clock_timer = Timer::default();
    clock_timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
        let snap = telemetry_mgr_timer.snapshot();
        if let Some(ref weak) = panel_weak_timer {
            if let Some(p) = weak.upgrade() {
                p.set_time_text(snap.time_str.into());
                p.set_date_text(snap.date_str.into());
                p.set_battery_pct(snap.battery_pct);
                p.set_is_charging(snap.is_charging);
                p.set_wifi_connected(snap.wifi_connected);
                p.set_wifi_ssid(snap.wifi_ssid.into());
                p.set_wifi_signal(snap.wifi_signal);
            }
        }
    });

    info!("Penumbra Desktop Shell event loop running");
    slint::run_event_loop()?;
    info!("Penumbra Desktop Shell shutting down cleanly");

    Ok(())
}
