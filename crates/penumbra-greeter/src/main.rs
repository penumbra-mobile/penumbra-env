mod buttons;
mod greetd;
mod power;
mod sessions;
mod users;

use clap::Parser;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

slint::include_modules!();

#[derive(Parser, Debug)]
#[command(
    name = "penumbra-greeter",
    about = "Penumbra's mobile-first greetd login frontend",
    version
)]
struct CliArgs {
    /// Run in test/mock mode without requiring a greetd daemon socket
    #[arg(long, default_value_t = false)]
    test: bool,
}

fn read_battery_percentage() -> Option<i32> {
    let power_supply = Path::new("/sys/class/power_supply");
    if let Ok(entries) = fs::read_dir(power_supply) {
        for entry in entries.flatten() {
            let path = entry.path();
            // Check if power supply is a battery (either type=Battery or known battery/gauge controller)
            let is_battery = fs::read_to_string(path.join("type"))
                .map(|t| t.trim().eq_ignore_ascii_case("Battery"))
                .unwrap_or(false);

            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if is_battery || name.contains("battery") || name.contains("qg") || name.contains("bms") || name.contains("BAT") {
                let cap_file = path.join("capacity");
                if let Ok(content) = fs::read_to_string(cap_file) {
                    if let Ok(val) = content.trim().parse::<i32>() {
                        return Some(val);
                    }
                }
            }
        }
    }
    None
}

fn is_wifi_connected() -> bool {
    let net = Path::new("/sys/class/net");
    if let Ok(entries) = fs::read_dir(net) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("wl") {
                let carrier = fs::read_to_string(entry.path().join("carrier"))
                    .map(|c| c.trim() == "1")
                    .unwrap_or(false);
                let operstate = fs::read_to_string(entry.path().join("operstate"))
                    .map(|s| s.trim() == "up")
                    .unwrap_or(false);
                if carrier || operstate {
                    return true;
                }
            }
        }
    }
    false
}

fn detect_and_apply_network_timezone() {
    thread::spawn(|| {
        let mut detected_tz: Option<String> = None;

        // 1. Try detecting from Modem MCC (Mobile Country Code) via mmcli
        if let Ok(output) = std::process::Command::new("mmcli")
            .args(["-m", "any"])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if line.contains("operator id") {
                    let id = line.split(':').nth(1).unwrap_or("").trim();
                    if id.len() >= 3 {
                        let mcc = &id[..3];
                        match mcc {
                            "226" => detected_tz = Some("Europe/Bucharest".into()),
                            "262" => detected_tz = Some("Europe/Berlin".into()),
                            "208" => detected_tz = Some("Europe/Paris".into()),
                            "234" | "235" => detected_tz = Some("Europe/London".into()),
                            "222" => detected_tz = Some("Europe/Rome".into()),
                            "214" => detected_tz = Some("Europe/Madrid".into()),
                            "204" => detected_tz = Some("Europe/Amsterdam".into()),
                            "206" => detected_tz = Some("Europe/Brussels".into()),
                            "260" => detected_tz = Some("Europe/Warsaw".into()),
                            "216" => detected_tz = Some("Europe/Budapest".into()),
                            "284" => detected_tz = Some("Europe/Sofia".into()),
                            "202" => detected_tz = Some("Europe/Athens".into()),
                            _ => {}
                        }
                    }
                    if detected_tz.is_some() {
                        break;
                    }
                }
            }
        }

        // 2. Query IP geolocation via wget (works over WiFi and cellular data)
        if detected_tz.is_none() {
            if let Ok(output) = std::process::Command::new("wget")
                .args(["-qO-", "http://ip-api.com/json"])
                .output()
            {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(pos) = text.find("\"timezone\":\"") {
                    let rest = &text[pos + 12..];
                    if let Some(end) = rest.find('"') {
                        let tz = &rest[..end];
                        if !tz.is_empty() {
                            detected_tz = Some(tz.to_string());
                        }
                    }
                }
            }
        }

        // 3. If a timezone was discovered, update system timezone
        if let Some(tz) = detected_tz {
            info!("Detected local timezone from modem/network: {}", tz);
            let _ = std::process::Command::new("timedatectl")
                .args(["set-timezone", &tz])
                .status();
            let zoneinfo = format!("/usr/share/zoneinfo/{}", tz);
            if Path::new(&zoneinfo).exists() {
                let _ = std::fs::remove_file("/etc/localtime");
                let _ = std::os::unix::fs::symlink(&zoneinfo, "/etc/localtime");
            }
        }
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = CliArgs::parse();

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    info!("Starting Penumbra Greeter (test mode: {})", args.test);

    // Auto-detect and sync local timezone in background
    detect_and_apply_network_timezone();

    // Enumerate users and sessions
    let available_users = users::get_available_users();
    let available_sessions = sessions::get_available_sessions();

    info!(
        "Discovered {} user(s), {} session(s)",
        available_users.len(),
        available_sessions.len()
    );

    // Initialize Slint UI
    let ui = GreeterWindow::new()?;

    // Populate Users Model
    let user_models: Vec<UserInfo> = available_users
        .iter()
        .map(|u| UserInfo {
            username: u.username.clone().into(),
            display_name: u.display_name.clone().into(),
            uid: u.uid,
        })
        .collect();
    ui.set_users(slint::ModelRc::new(slint::VecModel::from(user_models)));

    if let Some(first_user) = available_users.first() {
        ui.set_current_username(first_user.username.clone().into());
        ui.set_current_display_name(first_user.display_name.clone().into());
        ui.set_selected_user_index(0);
    }

    // Populate Sessions Model
    let session_models: Vec<SessionInfo> = available_sessions
        .iter()
        .map(|s| SessionInfo {
            id: s.id.clone().into(),
            name: s.name.clone().into(),
            comment: s.comment.clone().into(),
        })
        .collect();
    ui.set_sessions(slint::ModelRc::new(slint::VecModel::from(session_models)));

    if let Some(first_sess) = available_sessions.first() {
        ui.set_current_session_name(first_sess.name.clone().into());
        ui.set_selected_session_index(0);
    }

    // Dynamic wallpaper resolution
    for candidate in &[
        "/usr/share/penumbra/assets/wallpaper_hires.jpg",
        "/usr/share/penumbra/assets/wallpaper.jpg",
        "assets/wallpaper_hires.jpg",
        "assets/wallpaper.jpg",
    ] {
        if Path::new(candidate).exists() {
            if let Ok(img) = slint::Image::load_from_path(Path::new(candidate)) {
                info!("Loaded wallpaper from {}", candidate);
                ui.set_background_image(img);
                break;
            }
        }
    }

    // Update initial time and status
    let now = chrono::Local::now();
    ui.set_time_text(now.format("%H:%M").to_string().into());
    ui.set_date_text(now.format("%A, %B %-d").to_string().into());

    if let Some(bat) = read_battery_percentage() {
        ui.set_battery_text(format!("{}%", bat).into());
        ui.set_has_battery(true);
    } else {
        ui.set_battery_text("".into());
        ui.set_has_battery(false);
    }
    ui.set_has_wifi(is_wifi_connected());

    // Setup periodic timer for Clock and Telemetry (every 1 second)
    let ui_weak = ui.as_weak();
    let clock_timer = slint::Timer::default();
    clock_timer.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(1),
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                let now = chrono::Local::now();
                ui.set_time_text(now.format("%H:%M").to_string().into());
                ui.set_date_text(now.format("%A, %B %-d").to_string().into());

                if let Some(bat) = read_battery_percentage() {
                    ui.set_battery_text(format!("{}%", bat).into());
                    ui.set_has_battery(true);
                } else {
                    ui.set_has_battery(false);
                }
                ui.set_has_wifi(is_wifi_connected());
            }
        },
    );

    // Hardware physical buttons listener (Power, Vol Up, Vol Down)
    ui.set_volume_level(buttons::get_initial_volume());
    buttons::start_button_listener(ui.as_weak());

    // Callbacks
    // 1. Backspace from virtual keyboard
    let ui_weak = ui.as_weak();
    ui.on_backspace_requested(move || {
        if let Some(ui) = ui_weak.upgrade() {
            if ui.get_focused_field() == "username" {
                let mut u = ui.get_current_username().to_string();
                u.pop();
                ui.set_current_username(u.into());
            } else {
                let mut pw = ui.get_password_input().to_string();
                pw.pop();
                ui.set_password_input(pw.into());
                ui.set_error_message("".into());
            }
        }
    });

    // 2. User selection
    let ui_weak = ui.as_weak();
    let users_for_select = available_users.clone();
    ui.on_user_selected(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            if let Some(user) = users_for_select.get(idx as usize) {
                ui.set_selected_user_index(idx);
                ui.set_current_username(user.username.clone().into());
                ui.set_current_display_name(user.display_name.clone().into());
                ui.set_password_input("".into());
                ui.set_error_message("".into());
            }
        }
    });

    // 3. Session selection
    let ui_weak = ui.as_weak();
    let sess_for_select = available_sessions.clone();
    ui.on_session_selected(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            if let Some(sess) = sess_for_select.get(idx as usize) {
                ui.set_selected_session_index(idx);
                ui.set_current_session_name(sess.name.clone().into());
            }
        }
    });

    // 4. Power actions
    ui.on_poweroff_requested(|| {
        power::poweroff();
    });
    ui.on_reboot_requested(|| {
        power::reboot();
    });
    ui.on_suspend_requested(|| {
        power::suspend();
    });

    // 5. Login action
    let ui_weak = ui.as_weak();
    let sessions_for_login = Arc::new(Mutex::new(available_sessions));
    let test_mode = args.test;

    ui.on_login_requested(move |username, password| {
        let ui_handle = match ui_weak.upgrade() {
            Some(u) => u,
            None => return,
        };

        ui_handle.set_is_authenticating(true);
        ui_handle.set_error_message("".into());

        let username = username.to_string();
        let password = password.to_string();
        let selected_sess_idx = ui_handle.get_selected_session_index() as usize;

        let sess_cmd = {
            let sessions = sessions_for_login.lock().unwrap();
            sessions
                .get(selected_sess_idx)
                .map(|s| s.exec.clone())
                .unwrap_or_else(|| vec!["/usr/local/bin/penumbra-session".to_string()])
        };

        let ui_for_thread = ui_weak.clone();
        let greetd_sock = env::var("GREETD_SOCK").ok();

        thread::spawn(move || {
            let result = greetd::authenticate_and_start(
                greetd_sock.as_deref(),
                &username,
                &password,
                &sess_cmd,
                test_mode,
            );

            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_for_thread.upgrade() {
                    match result {
                        greetd::AuthResult::Success => {
                            info!("Authentication successful!");
                            if test_mode {
                                ui.set_is_authenticating(false);
                                ui.set_error_message("".into());
                                info!("Mock session started successfully!");
                            } else {
                                // greetd will terminate the greeter process once session takes over
                                std::process::exit(0);
                            }
                        }
                        greetd::AuthResult::Failed(err) => {
                            ui.set_is_authenticating(false);
                            ui.set_error_message(err.into());
                            ui.set_password_input("".into());
                        }
                    }
                }
            });
        });
    });

    // Show and run UI event loop
    ui.run()?;
    Ok(())
}
