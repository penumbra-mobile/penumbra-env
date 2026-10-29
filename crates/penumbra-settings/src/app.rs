//! # Application Controller & Reactive UI Coordinator
//!
//! Synchronizes asynchronous D-Bus hardware backends with the declarative Slint UI.
//!
//! ## Threading & Asynchronous Architecture
//! * **UI Thread Separation:** Slint runs synchronously inside its own rendering and windowing
//!   event loop on the main thread.
//! * **Tokio Background Runtime:** All network queries, BlueZ ObjectManager scans, and UPower
//!   telemetry run asynchronously on a multi-threaded Tokio runtime.
//! * **Event Handover:** Data is bridged from Tokio worker threads into the Slint event loop
//!   using [`slint::invoke_from_event_loop`].
//! * **Lifecycle Management:** Navigation events trigger background hardware actions (e.g.,
//!   entering the Bluetooth subpage starts discovery scanning, leaving halts discovery).

use std::rc::Rc;
use std::sync::Arc;
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use tokio::runtime::Handle;
use tracing::{info, warn};

use penumbra_services::{
    apply_backlight_ratio, apply_volume_ratio, read_backlight_ratio, read_volume_ratio,
    DisplayConfig, DisplaysConfig, FingerprintManager, LockType, PowerProfile, SecurityConfig, SoundConfig,
};

use crate::dbus::{BluetoothBackend, NetworkBackend, PowerBackend};
use crate::{
    BtDeviceData, DisplayItemData, PhysicalKeyboardData, SettingsWindow, VirtualKeyboardData,
    WifiApData,
};

/// Sets up reactive state bindings and user interaction callbacks for `SettingsWindow`.
///
/// # Arguments
/// * `window` - Reference to the instantiated Slint root window component.
/// * `net_backend` - Shared controller for NetworkManager wireless integration.
/// * `bt_backend` - Shared controller for BlueZ adapter and peripheral integration.
/// * `power_backend` - Shared controller for UPower battery telemetry.
/// * `rt_handle` - Handle to the Tokio runtime executing background tasks.
pub fn setup_app(
    window: &SettingsWindow,
    net_backend: Arc<NetworkBackend>,
    bt_backend: Arc<BluetoothBackend>,
    power_backend: Arc<PowerBackend>,
    rt_handle: Handle,
) {
    let ui_weak = window.as_weak();

    // 1. Initial State Population from real D-Bus backends
    {
        let net = net_backend.current();
        window.set_wifi_enabled(net.wifi_enabled);
        window.set_connected_ssid(net.connected_ssid.into());
        window.set_signal_strength(net.signal_strength as i32);
        let ap_models: Vec<WifiApData> = net
            .access_points
            .iter()
            .map(|ap| WifiApData {
                ssid: ap.ssid.clone().into(),
                signal: ap.signal_strength as i32,
                security: ap.security.clone().into(),
                connected: ap.is_connected,
            })
            .collect();
        window.set_wifi_access_points(ModelRc::from(Rc::new(VecModel::from(ap_models))));

        let bt = bt_backend.current();
        window.set_bt_enabled(bt.powered);
        window.set_adapter_name(bt.adapter_name.into());
        window.set_is_discovering(bt.discovering);
        let bt_models: Vec<BtDeviceData> = bt
            .devices
            .iter()
            .map(|dev| BtDeviceData {
                name: dev.name.clone().into(),
                address: dev.address.clone().into(),
                connected: dev.is_connected,
                paired: dev.is_paired,
                battery: dev.battery_level,
            })
            .collect();
        window.set_bt_devices(ModelRc::from(Rc::new(VecModel::from(bt_models))));

        let power = power_backend.current();
        window.set_battery_pct(power.percentage as i32);
        window.set_is_charging(power.is_charging);
        window.set_battery_state_text(power.state_text.into());
        window.set_battery_saver(power.power_saver);
        window.set_power_profile(PowerProfile::read_current().as_str().into());

        // Populate display & backlight settings
        let disp_cfg = DisplayConfig::load();
        let init_b = read_backlight_ratio().unwrap_or(disp_cfg.brightness);
        window.set_brightness(init_b as f32);
        window.set_adaptive_brightness(disp_cfg.adaptive_brightness);
        window.set_dark_theme(disp_cfg.dark_theme);
        window.set_accent_color(disp_cfg.accent_color.into());

        let multi_disp_cfg = DisplaysConfig::load();
        let display_items: Vec<DisplayItemData> = multi_disp_cfg
            .outputs
            .iter()
            .map(|o| DisplayItemData {
                id: o.id.clone().into(),
                name: o.name.clone().into(),
                connector: o.connector.clone().into(),
                width: o.width as i32,
                height: o.height as i32,
                refresh_rate: o.refresh_rate_hz as i32,
                scale_pct: (o.scale * 100.0) as i32,
                pos_x: o.pos_x,
                pos_y: o.pos_y,
                is_primary: o.is_primary,
                enabled: o.enabled,
            })
            .collect();
        window.set_displays(ModelRc::from(Rc::new(VecModel::from(display_items))));
        window.set_is_mirroring(multi_disp_cfg.arrangement_mode == "mirror");
        window.set_night_light_enabled(multi_disp_cfg.night_light.enabled);
        let temp_norm = (multi_disp_cfg.night_light.temperature_k.clamp(2700, 6500) as f32 - 2700.0) / (6500.0 - 2700.0);
        window.set_night_light_temp(temp_norm);

        // Populate sound channels & preferences
        let snd_cfg = SoundConfig::load();
        let init_v = read_volume_ratio();
        window.set_media_volume(init_v as f32);
        window.set_call_volume(snd_cfg.call_volume as f32);
        window.set_ring_volume(snd_cfg.ring_volume as f32);
        window.set_mic_volume(snd_cfg.mic_volume as f32);
        window.set_output_muted(snd_cfg.output_muted);
        window.set_mic_muted(snd_cfg.mic_muted);
        window.set_output_device(snd_cfg.output_device.into());
        window.set_dnd_enabled(snd_cfg.dnd_enabled);
        window.set_vibrate_for_calls(snd_cfg.vibrate_for_calls);

        // Pre-populate search catalog
        window.set_search_results(crate::search::filter_search(""));

        // Populate genuine hardware and OS telemetry
        let sys = crate::system_info::SystemInfo::read_current();
        window.set_device_name(sys.device_name.into());
        window.set_device_model(sys.device_model.into());
        window.set_os_version(sys.os_version.into());
        window.set_kernel_version(sys.kernel_version.into());
        window.set_cpu_model(sys.cpu_model.into());
        window.set_memory_info(sys.memory_info.into());
        window.set_storage_info(sys.storage_info.into());
        window.set_uptime_info(sys.uptime_info.into());
        window.set_compositor_info(sys.compositor_info.into());
        window.set_wifi_mac(sys.wifi_mac.into());
        window.set_bluetooth_mac(sys.bluetooth_mac.into());
        window.set_display_resolution(sys.display_resolution.into());
        window.set_battery_health(sys.battery_health.into());
        window.set_battery_model(sys.battery_model.into());

        // Live telemetry summaries for Overview Page
        window.set_storage_summary(sys.storage_summary.into());
        window.set_apps_summary(sys.apps_summary.into());

        // Populate virtual and physical keyboard telemetry
        reload_keyboards(window);

        // Populate Security & Lockscreen configuration
        let sec_cfg = SecurityConfig::load();
        window.set_active_lock_type(sec_cfg.lock_type.as_str().into());
        let has_fp = FingerprintManager::has_sensor();
        window.set_has_fingerprint(has_fp);
        window.set_fingerprint_enabled(sec_cfg.fingerprint_enabled && has_fp);
        window.set_auto_lock_timeout(sec_cfg.auto_lock_timeout_secs as i32);
        window.set_hide_sensitive_notifications(sec_cfg.hide_sensitive_notifications);

        let timeout_secs = sec_cfg.auto_lock_timeout_secs;
        let timeout_text = match timeout_secs {
            15 => "15 seconds".to_string(),
            30 => "30 seconds".to_string(),
            60 => "1 minute".to_string(),
            120 => "2 minutes".to_string(),
            300 => "5 minutes".to_string(),
            600 => "10 minutes".to_string(),
            0 => "Never".to_string(),
            s => format!("{s} seconds"),
        };
        window.set_screen_timeout(timeout_text.into());
    }

    // Search Query & Virtual Keyboard IPC Callbacks
    {
        let weak = ui_weak.clone();
        window.on_search_query_changed(move |query| {
            let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
            let cmd_file = std::path::PathBuf::from(runtime_dir).join("penumbra-keyboard-cmd");
            let _ = std::fs::write(cmd_file, "show");
            if let Some(ui) = weak.upgrade() {
                ui.set_search_results(crate::search::filter_search(query.as_str()));
            }
        });
    }

    window.on_search_focused(|| {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        let cmd_file = std::path::PathBuf::from(runtime_dir).join("penumbra-keyboard-cmd");
        let _ = std::fs::write(cmd_file, "show");
        info!("Search focused in Settings -> showing on-screen virtual keyboard");
    });

    window.on_search_unfocused(|| {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        let cmd_file = std::path::PathBuf::from(runtime_dir).join("penumbra-keyboard-cmd");
        let _ = std::fs::write(cmd_file, "hide");
        info!("Search dismissed in Settings -> hiding on-screen virtual keyboard");
    });

    // 2. Reactive Network Subscription (dynamic scans from NetworkManager)
    {
        let mut rx = net_backend.subscribe();
        let weak = ui_weak.clone();
        rt_handle.spawn(async move {
            while rx.changed().await.is_ok() {
                let net = rx.borrow().clone();
                let w = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = w.upgrade() {
                        ui.set_wifi_enabled(net.wifi_enabled);
                        ui.set_connected_ssid(net.connected_ssid.into());
                        ui.set_signal_strength(net.signal_strength as i32);
                        let ap_models: Vec<WifiApData> = net
                            .access_points
                            .iter()
                            .map(|ap| WifiApData {
                                ssid: ap.ssid.clone().into(),
                                signal: ap.signal_strength as i32,
                                security: ap.security.clone().into(),
                                connected: ap.is_connected,
                            })
                            .collect();
                        ui.set_wifi_access_points(ModelRc::from(Rc::new(VecModel::from(ap_models))));
                    }
                });
            }
        });
    }

    // 3. Reactive Bluetooth Subscription (dynamic ObjectManager updates from BlueZ)
    {
        let mut rx = bt_backend.subscribe();
        let weak = ui_weak.clone();
        rt_handle.spawn(async move {
            while rx.changed().await.is_ok() {
                let bt = rx.borrow().clone();
                let w = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = w.upgrade() {
                        ui.set_bt_enabled(bt.powered);
                        ui.set_adapter_name(bt.adapter_name.into());
                        ui.set_is_discovering(bt.discovering);
                        let bt_models: Vec<BtDeviceData> = bt
                            .devices
                            .iter()
                            .map(|dev| BtDeviceData {
                                name: dev.name.clone().into(),
                                address: dev.address.clone().into(),
                                connected: dev.is_connected,
                                paired: dev.is_paired,
                                battery: dev.battery_level,
                            })
                            .collect();
                        ui.set_bt_devices(ModelRc::from(Rc::new(VecModel::from(bt_models))));
                    }
                });
            }
        });
    }

    // 4. Reactive Power Subscription (battery telemetry from UPower)
    {
        let mut rx = power_backend.subscribe();
        let weak = ui_weak.clone();
        rt_handle.spawn(async move {
            while rx.changed().await.is_ok() {
                let power = rx.borrow().clone();
                let w = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = w.upgrade() {
                        ui.set_battery_pct(power.percentage as i32);
                        ui.set_is_charging(power.is_charging);
                        ui.set_battery_state_text(power.state_text.into());
                        ui.set_battery_saver(power.power_saver);
                    }
                });
            }
        });
    }

    // 5. Connect User Action Callbacks to Background Tasks
    {
        let net = net_backend.clone();
        let rt = rt_handle.clone();
        window.on_toggle_wifi(move |val| {
            info!("Settings: Toggle Wi-Fi -> {val}");
            let net = net.clone();
            rt.spawn(async move {
                net.set_wifi_enabled(val).await;
            });
        });
    }

    {
        let net = net_backend.clone();
        let rt = rt_handle.clone();
        window.on_connect_ap(move |ssid| {
            info!("Settings: Connect to Wi-Fi SSID -> {ssid}");
            let net = net.clone();
            let ssid = ssid.to_string();
            rt.spawn(async move {
                net.connect_to_network(&ssid, None).await;
            });
        });
    }

    {
        let net = net_backend.clone();
        let rt = rt_handle.clone();
        window.on_connect_ap_with_password(move |ssid, password| {
            info!("Settings: Connect to Wi-Fi SSID '{ssid}' with password");
            let net = net.clone();
            let ssid = ssid.to_string();
            let pw = password.to_string();
            rt.spawn(async move {
                net.connect_to_network(&ssid, Some(&pw)).await;
            });
        });
    }

    window.on_password_input_focused(|| {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        let cmd_file = std::path::PathBuf::from(runtime_dir).join("penumbra-keyboard-cmd");
        let _ = std::fs::write(cmd_file, "show");
        info!("Wi-Fi password input focused -> showing on-screen virtual keyboard");
    });

    window.on_password_input_dismissed(|| {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        let cmd_file = std::path::PathBuf::from(runtime_dir).join("penumbra-keyboard-cmd");
        let _ = std::fs::write(cmd_file, "hide");
        info!("Wi-Fi password input dismissed -> hiding on-screen virtual keyboard");
    });

    {
        let net = net_backend.clone();
        let rt = rt_handle.clone();
        window.on_refresh_wifi_scan(move || {
            info!("Settings: User requested Wi-Fi rescan");
            let net = net.clone();
            rt.spawn(async move {
                net.request_scan().await;
            });
        });
    }

    {
        let bt = bt_backend.clone();
        let rt = rt_handle.clone();
        window.on_toggle_bt(move |val| {
            info!("Settings: Toggle Bluetooth -> {val}");
            let bt = bt.clone();
            rt.spawn(async move {
                if !val {
                    let _ = bt.stop_discovery().await;
                }
                bt.set_powered(val).await;
            });
        });
    }

    {
        let bt = bt_backend.clone();
        let rt = rt_handle.clone();
        window.on_toggle_bt_device(move |addr| {
            info!("Settings: Toggle Bluetooth device -> {addr}");
            let bt = bt.clone();
            let addr = addr.to_string();
            rt.spawn(async move {
                bt.toggle_device(&addr).await;
            });
        });
    }

    {
        let bt = bt_backend.clone();
        let rt = rt_handle.clone();
        window.on_start_bt_discovery(move || {
            info!("Settings: User started Bluetooth discovery");
            let bt = bt.clone();
            rt.spawn(async move {
                bt.start_discovery().await;
            });
        });
    }

    {
        let power = power_backend.clone();
        let rt = rt_handle.clone();
        window.on_toggle_battery_saver(move |val| {
            info!("Settings: Toggle battery saver -> {val}");
            let power = power.clone();
            rt.spawn(async move {
                power.set_power_saver(val).await;
            });
        });
    }

    window.on_brightness_changed(|v| {
        info!("Settings: Brightness changed -> {:.2}", v);
        if let Err(error) = apply_backlight_ratio(v as f64) {
            tracing::warn!(?error, "Failed to apply system backlight in settings");
        }
        let mut cfg = DisplayConfig::load();
        cfg.brightness = v as f64;
        let _ = cfg.save();
    });

    window.on_toggle_adaptive_brightness(|val| {
        info!("Settings: Toggle adaptive brightness -> {val}");
        let mut cfg = DisplayConfig::load();
        cfg.adaptive_brightness = val;
        let _ = cfg.save();
    });

    window.on_toggle_dark_theme(|val| {
        info!("Settings: Toggle dark theme -> {val}");
        let mut cfg = DisplayConfig::load();
        cfg.dark_theme = val;
        let _ = cfg.save();
        cfg.apply_gsettings();
    });

    {
        let weak = ui_weak.clone();
        window.on_screen_timeout_selected(move |seconds, label| {
            info!("Settings: Screen timeout selected -> {seconds}s ({label})");
            let mut sec_cfg = SecurityConfig::load();
            sec_cfg.auto_lock_timeout_secs = seconds.max(0) as u32;
            let _ = sec_cfg.save();

            let mut disp_cfg = DisplayConfig::load();
            disp_cfg.screen_timeout_secs = seconds.max(0) as u32;
            let _ = disp_cfg.save();
            disp_cfg.apply_gsettings();

            if let Some(ui) = weak.upgrade() {
                ui.set_screen_timeout(label);
                ui.set_auto_lock_timeout(seconds);
            }
        });
    }

    window.on_display_selected(|idx| {
        info!("Settings: Display selected at index {idx}");
    });

    window.on_toggle_mirroring(|val| {
        info!("Settings: Toggle display mirroring -> {val}");
        let mut cfg = DisplaysConfig::load();
        cfg.arrangement_mode = if val { "mirror".into() } else { "extended".into() };
        let _ = cfg.save();
    });

    {
        let weak = ui_weak.clone();
        window.on_display_scale_selected(move |idx, scale_pct| {
            info!("Settings: Display {idx} scale changed -> {scale_pct}%");
            let mut cfg = DisplaysConfig::load();
            if let Some(output) = cfg.outputs.get_mut(idx as usize) {
                output.scale = scale_pct as f64 / 100.0;
                let _ = cfg.save();
            }
            if let Some(ui) = weak.upgrade() {
                let display_items: Vec<DisplayItemData> = cfg
                    .outputs
                    .iter()
                    .map(|o| DisplayItemData {
                        id: o.id.clone().into(),
                        name: o.name.clone().into(),
                        connector: o.connector.clone().into(),
                        width: o.width as i32,
                        height: o.height as i32,
                        refresh_rate: o.refresh_rate_hz as i32,
                        scale_pct: (o.scale * 100.0) as i32,
                        pos_x: o.pos_x,
                        pos_y: o.pos_y,
                        is_primary: o.is_primary,
                        enabled: o.enabled,
                    })
                    .collect();
                ui.set_displays(ModelRc::from(Rc::new(VecModel::from(display_items))));
            }
        });
    }

    {
        let weak = ui_weak.clone();
        window.on_display_primary_toggled(move |idx, val| {
            info!("Settings: Display {idx} primary set -> {val}");
            let mut cfg = DisplaysConfig::load();
            for (i, output) in cfg.outputs.iter_mut().enumerate() {
                if i == idx as usize {
                    output.is_primary = val;
                } else if val {
                    output.is_primary = false;
                }
            }
            let _ = cfg.save();
            if let Some(ui) = weak.upgrade() {
                let display_items: Vec<DisplayItemData> = cfg
                    .outputs
                    .iter()
                    .map(|o| DisplayItemData {
                        id: o.id.clone().into(),
                        name: o.name.clone().into(),
                        connector: o.connector.clone().into(),
                        width: o.width as i32,
                        height: o.height as i32,
                        refresh_rate: o.refresh_rate_hz as i32,
                        scale_pct: (o.scale * 100.0) as i32,
                        pos_x: o.pos_x,
                        pos_y: o.pos_y,
                        is_primary: o.is_primary,
                        enabled: o.enabled,
                    })
                    .collect();
                ui.set_displays(ModelRc::from(Rc::new(VecModel::from(display_items))));
            }
        });
    }

    window.on_toggle_night_light(|val| {
        info!("Settings: Toggle night light -> {val}");
        let mut cfg = DisplaysConfig::load();
        cfg.night_light.enabled = val;
        let _ = cfg.save();
    });

    window.on_night_light_temp_changed(|v| {
        let temp = (2700.0 + v * (6500.0 - 2700.0)) as u32;
        info!("Settings: Night light temperature changed -> {temp}K");
        let mut cfg = DisplaysConfig::load();
        cfg.night_light.temperature_k = temp;
        let _ = cfg.save();
    });

    window.on_media_volume_changed(|v| {
        info!("Settings: Media volume changed -> {:.2}", v);
        apply_volume_ratio(v as f64);
        let mut cfg = SoundConfig::load();
        cfg.media_volume = v as f64;
        let _ = cfg.save();
    });

    window.on_call_volume_changed(|v| {
        info!("Settings: Call volume changed -> {:.2}", v);
        let mut cfg = SoundConfig::load();
        cfg.call_volume = v as f64;
        let _ = cfg.save();
    });

    window.on_ring_volume_changed(|v| {
        info!("Settings: Ring volume changed -> {:.2}", v);
        let mut cfg = SoundConfig::load();
        cfg.ring_volume = v as f64;
        let _ = cfg.save();
    });

    window.on_toggle_dnd(|val| {
        info!("Settings: Toggle DND -> {val}");
        let mut cfg = SoundConfig::load();
        cfg.dnd_enabled = val;
        let _ = cfg.save();
    });

    window.on_toggle_vibrate_calls(|val| {
        info!("Settings: Toggle vibrate for calls -> {val}");
        let mut cfg = SoundConfig::load();
        cfg.vibrate_for_calls = val;
        let _ = cfg.save();
    });

    // COSMIC Sound Routing & Microphone
    window.on_output_device_changed(|dev| {
        info!("Settings: Audio output device changed -> {dev}");
        let mut cfg = SoundConfig::load();
        cfg.output_device = dev.into();
        let _ = cfg.save();
    });

    window.on_toggle_output_muted(|muted| {
        info!("Settings: Audio master output mute -> {muted}");
        let mut cfg = SoundConfig::load();
        cfg.output_muted = muted;
        let _ = cfg.save();
    });

    window.on_mic_volume_changed(|v| {
        info!("Settings: Microphone input volume -> {:.2}", v);
        let mut cfg = SoundConfig::load();
        cfg.mic_volume = v as f64;
        let _ = cfg.save();
    });

    window.on_toggle_mic_muted(|muted| {
        info!("Settings: Microphone mute -> {muted}");
        let mut cfg = SoundConfig::load();
        cfg.mic_muted = muted;
        let _ = cfg.save();
    });

    // COSMIC Power Profiles
    window.on_power_profile_changed(|profile_str| {
        info!("Settings: Power profile changed -> {profile_str}");
        let profile = PowerProfile::from_str_lossy(&profile_str);
        profile.apply();
    });

    // COSMIC Desktop Appearance Accent Color
    window.on_accent_color_changed(|color_str| {
        info!("Settings: Accent color changed -> {color_str}");
        let mut cfg = DisplayConfig::load();
        cfg.accent_color = color_str.into();
        let _ = cfg.save();
    });

    // Desktop Window Controls (Close)
    {
        let weak = ui_weak.clone();
        window.on_close_window(move || {
            info!("Settings: Desktop close window clicked -> exiting");
            if let Some(ui) = weak.upgrade() {
                let _ = ui.window().hide();
            }
            std::process::exit(0);
        });
    }

    // Keyboard Action Callbacks
    window.on_toggle_hide_on_physical(|val| {
        info!("Settings: Hide keyboard on physical connection set to {val}");
    });

    {
        let weak = ui_weak.clone();
        window.on_select_virtual_keyboard(move |id| {
            info!("Settings: User selected virtual keyboard: {id}");
            crate::keyboard_backend::switch_active_virtual_keyboard(&id);
            if let Some(ui) = weak.upgrade() {
                let model = ui.get_virtual_keyboards();
                let mut list: Vec<VirtualKeyboardData> = model.iter().collect();
                for item in &mut list {
                    item.is_active = item.desktop_id == id;
                }
                ui.set_virtual_keyboards(ModelRc::from(Rc::new(VecModel::from(list))));
            }
        });
    }

    {
        let weak = ui_weak.clone();
        window.on_refresh_keyboard_scan(move || {
            info!("Settings: Refreshing keyboard hardware and virtual entries");
            if let Some(ui) = weak.upgrade() {
                reload_keyboards(&ui);
            }
        });
    }

    // Subpage routing hooks for discovery & scanning lifecycle
    {
        let bt = bt_backend.clone();
        let net = net_backend.clone();
        let rt = rt_handle.clone();
        let weak = ui_weak.clone();
        window.on_navigate_to(move |page| {
            info!("Settings: Navigated to subpage '{page}'");
            let bt = bt.clone();
            let net = net.clone();
            let page_str = page.to_string();
            let w = weak.clone();
            rt.spawn(async move {
                if page_str == "bluetooth" {
                    bt.start_discovery().await;
                } else if page_str == "wifi" {
                    net.request_scan().await;
                } else if page_str == "keyboard" {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = w.upgrade() {
                            reload_keyboards(&ui);
                        }
                    });
                } else {
                    bt.stop_discovery().await;
                }
            });
        });
    }

    {
        let bt = bt_backend.clone();
        let rt = rt_handle.clone();
        window.on_navigate_back(move || {
            info!("Settings: Navigated back to main catalog");
            let bt = bt.clone();
            rt.spawn(async move {
                bt.stop_discovery().await;
            });
        });
    }

    // Security & Privacy Callbacks
    {
        let weak = ui_weak.clone();
        window.on_set_lock_swipe(move || {
            let mut sec_cfg = SecurityConfig::load();
            sec_cfg.set_secret("", LockType::Swipe);
            if let Err(e) = sec_cfg.save() {
                warn!("Failed to save security.json: {e}");
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_active_lock_type("swipe".into());
            }
            info!("Settings: Screen lock set to Swipe to unlock");
        });
    }

    {
        let weak = ui_weak.clone();
        window.on_set_lock_none(move || {
            let mut sec_cfg = SecurityConfig::load();
            sec_cfg.set_secret("", LockType::None);
            if let Err(e) = sec_cfg.save() {
                warn!("Failed to save security.json: {e}");
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_active_lock_type("none".into());
            }
            info!("Settings: Screen lock set to None (No lock screen)");
        });
    }

    {
        let weak = ui_weak.clone();
        window.on_set_lock_pin(move |new_pin| {
            let mut sec_cfg = SecurityConfig::load();
            sec_cfg.set_secret(new_pin.as_str(), LockType::Pin);
            if let Err(e) = sec_cfg.save() {
                warn!("Failed to save security.json: {e}");
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_active_lock_type("pin".into());
            }
            info!("Settings: Screen lock PIN updated successfully");
        });
    }

    {
        let weak = ui_weak.clone();
        window.on_set_lock_password(move |new_pw| {
            let mut sec_cfg = SecurityConfig::load();
            sec_cfg.set_secret(new_pw.as_str(), LockType::Password);
            if let Err(e) = sec_cfg.save() {
                warn!("Failed to save security.json: {e}");
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_active_lock_type("password".into());
            }
            info!("Settings: Screen lock Password updated successfully");
        });
    }

    {
        let weak = ui_weak.clone();
        window.on_toggle_fingerprint(move |enabled| {
            let mut sec_cfg = SecurityConfig::load();
            sec_cfg.fingerprint_enabled = enabled;
            if let Err(e) = sec_cfg.save() {
                warn!("Failed to save security.json: {e}");
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_fingerprint_enabled(enabled);
            }
            info!("Settings: Biometric fingerprint toggled to {enabled}");
        });
    }

    {
        let weak = ui_weak.clone();
        window.on_set_auto_lock_timeout(move |timeout| {
            let mut sec_cfg = SecurityConfig::load();
            sec_cfg.auto_lock_timeout_secs = timeout.max(0) as u32;
            let _ = sec_cfg.save();
            if let Some(ui) = weak.upgrade() {
                ui.set_auto_lock_timeout(timeout);
            }
        });
    }

    {
        let weak = ui_weak.clone();
        window.on_toggle_sensitive_notifications(move |hide| {
            let mut sec_cfg = SecurityConfig::load();
            sec_cfg.hide_sensitive_notifications = hide;
            let _ = sec_cfg.save();
            if let Some(ui) = weak.upgrade() {
                ui.set_hide_sensitive_notifications(hide);
            }
        });
    }
}

/// Discovers installed KDE-compliant virtual keyboards and connected physical keyboards,
/// populating the Slint UI models.
fn reload_keyboards(window: &SettingsWindow) {
    let vks = crate::keyboard_backend::detect_virtual_keyboards();
    let vk_models: Vec<VirtualKeyboardData> = vks
        .into_iter()
        .map(|v| VirtualKeyboardData {
            name: v.name.into(),
            comment: v.comment.into(),
            desktop_id: v.desktop_id.into(),
            is_active: v.is_active,
        })
        .collect();
    window.set_virtual_keyboards(ModelRc::from(Rc::new(VecModel::from(vk_models))));

    let pks = crate::keyboard_backend::detect_physical_keyboards();
    let pk_models: Vec<PhysicalKeyboardData> = pks
        .into_iter()
        .map(|p| PhysicalKeyboardData {
            name: p.name.into(),
            bus: p.bus.into(),
        })
        .collect();
    window.set_physical_keyboards(ModelRc::from(Rc::new(VecModel::from(pk_models))));
}
