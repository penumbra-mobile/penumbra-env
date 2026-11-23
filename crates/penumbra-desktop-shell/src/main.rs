//! Standalone Native Wayland Desktop Convergence Shell binary.
//!
//! Provides the external monitor Top Desktop Panel, Bottom Dock, and Modal Application
//! Launcher for Penumbra Desktop Convergence Mode. Built with Slint and `layer-shell-slint`.

use std::collections::HashMap;
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

use apps::{
    discover_desktop_apps, filter_apps, load_slint_icon, read_open_desktop_windows,
    spawn_desktop_app, AppInfo,
};
use cli::CliArgs;
use penumbra_applets::{
    apply_power_profile, apply_volume_ratio, build_month_calendar, execute_power_action,
    open_settings_page, read_volume_ratio, AppletPopoverWindow,
};
use telemetry::TelemetryManager;

slint::include_modules!();

fn convert_to_slint_entry(
    app: &AppInfo,
    icon_cache: &HashMap<String, (slint::Image, bool)>,
) -> DesktopAppEntry {
    let (icon, has_icon) = icon_cache
        .get(&app.id)
        .cloned()
        .unwrap_or_else(|| (slint::Image::default(), false));
    DesktopAppEntry {
        id: app.id.clone().into(),
        name: app.name.clone().into(),
        exec: app.exec.clone().into(),
        category: app.category.clone().into(),
        icon_name: app.icon_name.clone().into(),
        icon,
        has_icon,
    }
}

fn eval_inline_calc(query: &str) -> String {
    let q = query.trim();
    let Some(expr) = q.strip_prefix('=') else {
        return String::new();
    };
    let expr = expr.trim();
    if expr.is_empty() {
        return String::new();
    }
    for (op_ch, op_fn) in [
        ('+', (|a: f64, b: f64| a + b) as fn(f64, f64) -> f64),
        ('-', |a, b| a - b),
        ('*', |a, b| a * b),
        ('/', |a, b| if b.abs() > 1e-12 { a / b } else { f64::NAN }),
    ] {
        if let Some((lhs, rhs)) = expr.split_once(op_ch) {
            if let (Ok(a), Ok(b)) = (lhs.trim().parse::<f64>(), rhs.trim().parse::<f64>()) {
                let res = op_fn(a, b);
                if res.is_finite() {
                    return if (res - res.round()).abs() < 1e-9 {
                        format!("{}", res.round() as i64)
                    } else {
                        format!("{:.4}", res).trim_end_matches('0').trim_end_matches('.').to_string()
                    };
                }
            }
        }
    }
    String::new()
}

fn find_app_for_id<'a>(apps: &'a [AppInfo], id_or_app_id: &str) -> Option<&'a AppInfo> {
    let needle = id_or_app_id.trim();
    if needle.is_empty() {
        return None;
    }
    let needle_lower = needle.to_lowercase();
    apps.iter().find(|a| {
        a.id == needle
            || a.id.to_lowercase() == needle_lower
            || a.name.to_lowercase() == needle_lower
            || a.exec
                .split_whitespace()
                .next()
                .map(|bin| bin.rsplit('/').next().unwrap_or(bin).to_lowercase() == needle_lower)
                .unwrap_or(false)
    })
}

fn build_dock_entries(
    favorite_ids: &[String],
    all_apps: &[AppInfo],
    icon_cache: &HashMap<String, (slint::Image, bool)>,
    open_windows: &[(String, String)],
) -> Vec<DesktopDockApp> {
    let mut entries = Vec::new();
    let mut matched_open = vec![false; open_windows.len()];

    for fav_id in favorite_ids {
        let app_opt = find_app_for_id(all_apps, fav_id);
        let (id, name, icon_name, icon, has_icon) = if let Some(a) = app_opt {
            let (img, has) = icon_cache
                .get(&a.id)
                .cloned()
                .unwrap_or_else(|| (slint::Image::default(), false));
            (a.id.clone(), a.name.clone(), a.icon_name.clone(), img, has)
        } else {
            (
                fav_id.clone(),
                fav_id.clone(),
                "grid".to_string(),
                slint::Image::default(),
                false,
            )
        };

        let mut is_running = false;
        for (idx, (win_app_id, win_title)) in open_windows.iter().enumerate() {
            let win_lower = win_app_id.to_lowercase();
            let title_lower = win_title.to_lowercase();
            let id_lower = id.to_lowercase();
            let name_lower = name.to_lowercase();
            let exec_bin = app_opt
                .and_then(|a| a.exec.split_whitespace().next())
                .map(|b| b.rsplit('/').next().unwrap_or(b).to_lowercase())
                .unwrap_or_default();
            if (!win_lower.is_empty()
                && (win_lower == id_lower
                    || id_lower.contains(&win_lower)
                    || win_lower.contains(&id_lower)
                    || win_lower == exec_bin
                    || win_lower == name_lower))
                || (!title_lower.is_empty()
                    && (title_lower.contains(&name_lower)
                        || name_lower.contains(&title_lower)
                        || title_lower.contains(&id_lower)
                        || id_lower.contains(&title_lower)))
            {
                is_running = true;
                matched_open[idx] = true;
            }
        }

        entries.push(DesktopDockApp {
            id: id.into(),
            name: name.into(),
            icon_name: icon_name.into(),
            icon,
            has_icon,
            is_running,
            is_active: is_running,
        });
    }

    // Append open apps that are not already in the phone dock favorites
    let mut seen_extra = std::collections::HashSet::new();
    for (idx, (win_app_id, win_title)) in open_windows.iter().enumerate() {
        if matched_open[idx] {
            continue;
        }
        let key = if !win_app_id.is_empty() {
            win_app_id.to_lowercase()
        } else {
            win_title.to_lowercase()
        };
        if key.is_empty() || !seen_extra.insert(key) {
            continue;
        }

        if let Some(a) = find_app_for_id(all_apps, win_app_id)
            .or_else(|| find_app_for_id(all_apps, win_title))
        {
            let (icon, has_icon) = icon_cache
                .get(&a.id)
                .cloned()
                .unwrap_or_else(|| (slint::Image::default(), false));
            entries.push(DesktopDockApp {
                id: a.id.clone().into(),
                name: a.name.clone().into(),
                icon_name: a.icon_name.clone().into(),
                icon,
                has_icon,
                is_running: true,
                is_active: true,
            });
        } else {
            let label = if !win_app_id.is_empty() {
                win_app_id.clone()
            } else {
                win_title.clone()
            };
            let fallback_path = penumbra_shell::apps::find_icon_fast(&label.to_lowercase());
            let (icon, has_icon) = load_slint_icon(fallback_path.as_deref());
            entries.push(DesktopDockApp {
                id: label.clone().into(),
                name: label.into(),
                icon_name: "terminal".into(),
                icon,
                has_icon,
                is_running: true,
                is_active: true,
            });
        }
    }

    entries
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
    let _ = slint::select_bundled_translation(&penumbra_services::i18n::active_lang_code());
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
        LayerConfig::top_bar(30)
            .with_namespace("penumbra-desktop-panel")
            .with_output(&args.output)
            .with_exclusive_zone(30)
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

    // 2. Discover Applications & Preload Real XDG Icons + Phone Dock Favorites
    let all_apps = Rc::new(discover_desktop_apps());
    let mut cache_map = HashMap::new();
    for app in all_apps.iter() {
        let loaded = load_slint_icon(app.icon_path.as_deref());
        cache_map.insert(app.id.clone(), loaded);
    }
    let icon_cache = Rc::new(cache_map);

    let available_ids: Vec<String> = all_apps.iter().map(|a| a.id.clone()).collect();
    let phone_layout = penumbra_shell::config::load_layout_config(&available_ids);
    let favorite_dock_ids = Rc::new(phone_layout.dock_apps);

    // 2b. External Monitor Wallpaper (Layer::Background) + Right-Click Context Menu
    let wallpaper_opt = if run_panel {
        layer_shell_slint::set_next_window_config(
            LayerConfig::overlay()
                .with_layer(layer_shell_slint::Layer::Background)
                .with_namespace("penumbra-desktop-wallpaper")
                .with_output(&args.output)
                .with_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right)
                .with_exclusive_zone(-1)
                .with_keyboard_interactivity(KeyboardInteractivity::None),
        );
        let wp_win = DesktopWallpaperWindow::new()?;
        if !phone_layout.wallpaper_path.is_empty() {
            if let Ok(img) = slint::Image::load_from_path(std::path::Path::new(&phone_layout.wallpaper_path)) {
                wp_win.set_wallpaper(img);
                wp_win.set_has_wallpaper(true);
            }
        }
        let out_term = args.output.clone();
        wp_win.on_open_terminal(move || {
            let app = find_app_for_id(&[], "terminal").cloned().unwrap_or(AppInfo {
                id: "terminal".into(),
                name: "Terminal".into(),
                exec: "kgx".into(),
                category: "System".into(),
                icon_name: "terminal".into(),
                icon_path: None,
                terminal: false,
            });
            let _ = spawn_desktop_app(&app, Some(&out_term));
        });
        let out_files = args.output.clone();
        wp_win.on_open_files(move || {
            let app = AppInfo {
                id: "files".into(),
                name: "Files".into(),
                exec: "nautilus".into(),
                category: "Utilities".into(),
                icon_name: "folder".into(),
                icon_path: None,
                terminal: false,
            };
            let _ = spawn_desktop_app(&app, Some(&out_files));
        });
        wp_win.on_open_settings(|page| {
            open_settings_page(page.as_str());
        });
        wp_win.show()?;
        Some(wp_win)
    } else {
        None
    };

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
        let initial_slint_apps: Vec<DesktopAppEntry> = all_apps
            .iter()
            .map(|a| convert_to_slint_entry(a, &icon_cache))
            .collect();
        let apps_model = Rc::new(VecModel::from(initial_slint_apps));
        launcher.set_apps(ModelRc::from(apps_model.clone()));

        // Search filtering + inline calculator (`=expr`)
        let all_apps_search = all_apps.clone();
        let icon_cache_search = icon_cache.clone();
        let launcher_weak_search = launcher_weak.clone();
        let apps_model_search = apps_model.clone();
        launcher.on_search_query_changed(move |query| {
            if let Some(l) = launcher_weak_search.upgrade() {
                l.set_calc_result(eval_inline_calc(query.as_str()).into());
                let cat = l.get_active_category();
                let filtered = filter_apps(&all_apps_search, query.as_str(), cat.as_str());
                let new_entries: Vec<DesktopAppEntry> = filtered
                    .iter()
                    .map(|a| convert_to_slint_entry(a, &icon_cache_search))
                    .collect();
                apps_model_search.set_vec(new_entries);
                l.window().request_redraw();
            }
        });

        // Category filter callback
        let all_apps_cat = all_apps.clone();
        let icon_cache_cat = icon_cache.clone();
        let launcher_weak_cat = launcher_weak.clone();
        let apps_model_cat = apps_model.clone();
        launcher.on_category_selected(move |category| {
            if let Some(l) = launcher_weak_cat.upgrade() {
                let q = l.get_search_text();
                let filtered = filter_apps(&all_apps_cat, q.as_str(), category.as_str());
                let new_entries: Vec<DesktopAppEntry> = filtered
                    .iter()
                    .map(|a| convert_to_slint_entry(a, &icon_cache_cat))
                    .collect();
                apps_model_cat.set_vec(new_entries);
                l.window().request_redraw();
            }
        });

        // App launched callback
        let output_name = args.output.clone();
        let launcher_weak_launch = launcher_weak.clone();
        let all_apps_launch = all_apps.clone();
        launcher.on_app_clicked(move |id, exec| {
            info!("Desktop launcher launching app: id={}, exec={}", id, exec);
            let app_info = all_apps_launch
                .iter()
                .find(|a| a.id == id.as_str())
                .cloned()
                .unwrap_or_else(|| AppInfo {
                    id: id.to_string(),
                    name: id.to_string(),
                    exec: exec.to_string(),
                    category: "Utilities".into(),
                    icon_name: "grid".into(),
                    icon_path: None,
                    terminal: false,
                });
            let _ = spawn_desktop_app(&app_info, Some(&output_name));
            if let Some(l) = launcher_weak_launch.upgrade() {
                l.set_is_open(false);
                layer_shell_slint::sync_input_region("penumbra-desktop-launcher", 4096, 4096, 0, false);
            }
        });

        // Dismiss callback
        let launcher_weak_close = launcher_weak.clone();
        launcher.on_close_requested(move || {
            if let Some(l) = launcher_weak_close.upgrade() {
                l.set_is_open(false);
                layer_shell_slint::sync_input_region("penumbra-desktop-launcher", 4096, 4096, 0, false);
            }
        });

        // Initially closed and click-through (call sync_input_region AFTER show() so wl_surface is mapped)
        launcher.set_is_open(false);
        launcher.show()?;
        layer_shell_slint::sync_input_region("penumbra-desktop-launcher", 4096, 4096, 0, false);

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
                    launcher.window().request_redraw();
                    if new_state {
                        layer_shell_slint::sync_input_region(
                            "penumbra-desktop-launcher",
                            4096,
                            4096,
                            4096,
                            true,
                        );
                    } else {
                        layer_shell_slint::sync_input_region(
                            "penumbra-desktop-launcher",
                            4096,
                            4096,
                            0,
                            false,
                        );
                    }
                }
            }
        }
    };

    // Wire wallpaper right-click "Applications" item to toggle_launcher
    if let Some(ref wp) = wallpaper_opt {
        let toggle_wp = toggle_launcher.clone();
        wp.on_open_launcher(move || {
            toggle_wp();
        });
    }

    // 3b. Interactive Panel Applet Popover Overlay (penumbra-applets)
    let popover_opt = if run_panel {
        layer_shell_slint::set_next_window_config(
            LayerConfig::overlay()
                .with_namespace("penumbra-desktop-applet")
                .with_output(&args.output)
                .with_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right)
                .with_exclusive_zone(-1)
                .with_keyboard_interactivity(KeyboardInteractivity::None),
        );

        let popover = AppletPopoverWindow::new()?;
        popover.set_active_applet("".into());
        popover.set_volume(read_volume_ratio());
        let (cal_header, cal_days) = build_month_calendar();
        popover.set_calendar_header(cal_header.into());
        popover.set_calendar_days(ModelRc::from(Rc::new(VecModel::from(cal_days))));
        layer_shell_slint::sync_input_region("penumbra-desktop-applet", 4096, 4096, 0, false);

        let pop_close = popover.as_weak();
        popover.on_close_requested(move || {
            if let Some(p) = pop_close.upgrade() {
                p.set_active_applet("".into());
                layer_shell_slint::sync_input_region("penumbra-desktop-applet", 4096, 4096, 0, false);
            }
        });

        popover.on_volume_changed(|v| {
            apply_volume_ratio(v);
        });
        popover.on_mute_toggled(|muted| {
            apply_volume_ratio(if muted { 0.0 } else { 0.65 });
        });
        popover.on_power_profile_selected(|prof| {
            apply_power_profile(prof.as_str());
        });

        let pop_settings = popover.as_weak();
        popover.on_open_settings(move |page| {
            if let Some(p) = pop_settings.upgrade() {
                p.set_active_applet("".into());
                layer_shell_slint::sync_input_region("penumbra-desktop-applet", 4096, 4096, 0, false);
            }
            open_settings_page(page.as_str());
        });

        let pop_power = popover.as_weak();
        popover.on_power_action(move |action| {
            if let Some(p) = pop_power.upgrade() {
                p.set_active_applet("".into());
                layer_shell_slint::sync_input_region("penumbra-desktop-applet", 4096, 4096, 0, false);
            }
            execute_power_action(action.as_str());
        });

        popover.show()?;
        layer_shell_slint::sync_input_region("penumbra-desktop-applet", 4096, 4096, 0, false);
        Some(popover)
    } else {
        None
    };

    let popover_handle = popover_opt.as_ref().map(|p| p.as_weak());

    // 4. Optional Top Desktop Panel
    let panel_opt = if run_panel {
        layer_shell_slint::set_next_window_config(
            LayerConfig::top_bar(30)
                .with_namespace("penumbra-desktop-panel")
                .with_output(&args.output)
                .with_exclusive_zone(30),
        );

        let panel = DesktopPanelWindow::new()?;
        let panel_weak = panel.as_weak();

        // Wire launcher toggle
        let toggle_l = toggle_launcher.clone();
        panel.on_toggle_launcher(move || {
            toggle_l();
        });

        // Wire workspace switch -> notify penumbra-comp via penumbra-desktop-cmd
        let panel_weak_ws = panel_weak.clone();
        panel.on_switch_workspace(move |ws| {
            info!("Switched to workspace {}", ws);
            if let Some(p) = panel_weak_ws.upgrade() {
                p.set_active_workspace(ws);
            }
            let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
            let _ = std::fs::write(format!("{}/penumbra-desktop-cmd", dir), format!("switch_workspace:{}\n", ws));
        });

        // Wire panel applet popovers ("time", "network", "audio", "battery", "power")
        let pop_open = popover_handle.clone();
        let panel_weak_app = panel_weak.clone();
        panel.on_open_applet(move |kind| {
            if let Some(ref handle) = pop_open {
                if let Some(pop) = handle.upgrade() {
                    let current = pop.get_active_applet();
                    let next = if current == kind { "" } else { kind.as_str() };
                    pop.set_active_applet(next.into());
                    if next.is_empty() {
                        layer_shell_slint::sync_input_region("penumbra-desktop-applet", 4096, 4096, 0, false);
                    } else {
                        if next == "audio" {
                            pop.set_volume(read_volume_ratio());
                        } else if next == "time" {
                            let (hdr, days) = build_month_calendar();
                            pop.set_calendar_header(hdr.into());
                            pop.set_calendar_days(ModelRc::from(Rc::new(VecModel::from(days))));
                        }
                        layer_shell_slint::sync_input_region("penumbra-desktop-applet", 4096, 4096, 4096, true);
                    }
                    if let Some(p) = panel_weak_app.upgrade() {
                        p.set_active_applet(next.into());
                    }
                }
            }
        });

        panel.on_power_clicked(|| {});

        panel.show()?;
        Some(panel)
    } else {
        None
    };

    // 5. Optional Desktop Dock
    let dock_opt = if run_dock {
        layer_shell_slint::set_next_window_config(
            LayerConfig::bottom_panel(64)
                .with_namespace("penumbra-desktop-dock")
                .with_output(&args.output)
                .with_exclusive_zone(64),
        );

        let dock = DesktopDockWindow::new()?;
        let initial_open = read_open_desktop_windows();
        let initial_dock =
            build_dock_entries(&favorite_dock_ids, &all_apps, &icon_cache, &initial_open);
        dock.set_dock_apps(ModelRc::from(Rc::new(VecModel::from(initial_dock))));

        // Wire launcher clicked
        let toggle_l = toggle_launcher.clone();
        dock.on_launcher_clicked(move || {
            toggle_l();
        });

        // Wire dock app clicked: focus/raise if already open, otherwise spawn
        let output_dock = args.output.clone();
        let all_apps_dock = all_apps.clone();
        dock.on_app_clicked(move |app_id| {
            info!("Dock app clicked: {}", app_id);
            let open_wins = read_open_desktop_windows();
            let q = app_id.as_str().to_lowercase();
            let is_open = open_wins.iter().any(|(wid, title)| {
                let wl = wid.to_lowercase();
                let tl = title.to_lowercase();
                (!wl.is_empty() && (wl == q || q.contains(&wl) || wl.contains(&q)))
                    || (!tl.is_empty() && (tl.contains(&q) || q.contains(&tl)))
            });
            if is_open {
                let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
                let _ = std::fs::write(
                    format!("{}/penumbra-desktop-cmd", dir),
                    format!("focus_app:{}\n", app_id),
                );
                return;
            }
            let app_to_run = find_app_for_id(&all_apps_dock, app_id.as_str())
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
                        icon_path: None,
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
    let pop_weak_timer = popover_handle.clone();
    let telemetry_mgr_timer = telemetry_mgr.clone();
    let clock_timer = Timer::default();
    let mut last_lang = penumbra_services::i18n::active_lang_code();
    clock_timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
        let cur_lang = penumbra_services::i18n::active_lang_code();
        if cur_lang != last_lang {
            let _ = slint::select_bundled_translation(if cur_lang == "en" { "" } else { &cur_lang });
            last_lang = cur_lang;
        }
        let snap = telemetry_mgr_timer.snapshot();
        let vol_pct = (read_volume_ratio() * 100.0).round() as i32;
        if let Some(ref weak) = panel_weak_timer {
            if let Some(p) = weak.upgrade() {
                p.set_time_text(snap.time_str.clone().into());
                p.set_date_text(snap.date_str.clone().into());
                p.set_battery_pct(snap.battery_pct);
                p.set_is_charging(snap.is_charging);
                p.set_wifi_connected(snap.wifi_connected);
                p.set_wifi_ssid(snap.wifi_ssid.clone().into());
                p.set_wifi_signal(snap.wifi_signal);
                p.set_volume_pct(vol_pct);
            }
        }
        if let Some(ref pw) = pop_weak_timer {
            if let Some(pop) = pw.upgrade() {
                pop.set_time_text(snap.time_str.into());
                pop.set_date_text(snap.date_str.into());
                pop.set_battery_pct(snap.battery_pct);
                pop.set_is_charging(snap.is_charging);
                pop.set_wifi_connected(snap.wifi_connected);
                pop.set_wifi_ssid(snap.wifi_ssid.into());
                pop.set_wifi_signal(snap.wifi_signal);
                if let Some(ref pweak) = panel_weak_timer {
                    if let Some(p) = pweak.upgrade() {
                        p.set_active_applet(pop.get_active_applet());
                    }
                }
            }
        }
    });

    // Periodic Dock Open-Windows, Workspace & Super-Key Launcher Sync Timer (200ms)
    let dock_weak_timer = dock_opt.as_ref().map(|d| d.as_weak());
    let panel_weak_sync = panel_opt.as_ref().map(|p| p.as_weak());
    let launcher_weak_sync = launcher_handle.clone();
    let toggle_from_super = toggle_launcher.clone();
    let favs_sync = favorite_dock_ids.clone();
    let apps_sync = all_apps.clone();
    let cache_sync = icon_cache.clone();
    let last_open_windows = Rc::new(std::cell::RefCell::new(Vec::<(String, String)>::new()));
    let dock_sync_timer = Timer::default();
    dock_sync_timer.start(TimerMode::Repeated, Duration::from_millis(200), move || {
        let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        let toggle_path = std::path::PathBuf::from(&dir).join("penumbra-launcher-toggle");
        if toggle_path.exists() {
            let _ = std::fs::remove_file(&toggle_path);
            toggle_from_super();
        }
        let ws_path = std::path::PathBuf::from(&dir).join("penumbra-desktop-workspace");
        if let Ok(ws_str) = std::fs::read_to_string(&ws_path) {
            if let Ok(ws) = ws_str.trim().parse::<i32>() {
                if let Some(ref pw) = panel_weak_sync {
                    if let Some(p) = pw.upgrade() {
                        if p.get_active_workspace() != ws {
                            p.set_active_workspace(ws);
                        }
                    }
                }
            }
        }
        let is_open = launcher_weak_sync
            .as_ref()
            .and_then(|w| w.upgrade())
            .map(|l| l.get_is_open())
            .unwrap_or(false);
        if let Some(ref pw) = panel_weak_sync {
            if let Some(p) = pw.upgrade() {
                p.set_launcher_active(is_open);
            }
        }
        if let Some(ref dw) = dock_weak_timer {
            if let Some(d) = dw.upgrade() {
                d.set_launcher_active(is_open);
                let open_wins = read_open_desktop_windows();
                let mut last = last_open_windows.borrow_mut();
                if *last != open_wins {
                    *last = open_wins.clone();
                    let updated =
                        build_dock_entries(&favs_sync, &apps_sync, &cache_sync, &open_wins);
                    d.set_dock_apps(ModelRc::from(Rc::new(VecModel::from(updated))));
                }
            }
        }
    });

    info!("Penumbra Desktop Shell event loop running");
    slint::run_event_loop()?;
    info!("Penumbra Desktop Shell shutting down cleanly");

    Ok(())
}
