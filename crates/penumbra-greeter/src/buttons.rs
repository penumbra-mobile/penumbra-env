use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, info};

use crate::GreeterWindow;

const BACKLIGHT_PATH: &str = "/sys/class/backlight/backlight/brightness";

fn read_current_brightness() -> u32 {
    fs::read_to_string(BACKLIGHT_PATH)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1156)
}

fn write_brightness(val: u32) {
    if let Err(e) = fs::write(BACKLIGHT_PATH, val.to_string()) {
        debug!("Failed to write brightness {}: {}", val, e);
    }
}

pub fn get_initial_volume() -> i32 {
    if let Ok(output) = Command::new("amixer").args(["get", "Master"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(pct) = parse_volume_percentage(&stdout) {
            return pct;
        }
    }
    50
}

pub fn adjust_volume(step: &str) -> Option<i32> {
    if let Ok(output) = Command::new("amixer")
        .args(["set", "Master", step])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(pct) = parse_volume_percentage(&stdout) {
            return Some(pct);
        }
    }

    let wp_step = if step.contains('+') { "5%+" } else { "5%-" };
    let _ = Command::new("wpctl")
        .args(["set-volume", "@DEFAULT_AUDIO_SINK@", wp_step])
        .output();
    None
}

fn parse_volume_percentage(output: &str) -> Option<i32> {
    for line in output.lines() {
        if line.contains("Playback") && line.contains('[') && line.contains("%]") {
            if let Some(start) = line.find('[') {
                if let Some(end) = line[start..].find("%]") {
                    let num_str = &line[start + 1..start + end];
                    if let Ok(val) = num_str.trim().parse::<i32>() {
                        return Some(val);
                    }
                }
            }
        }
    }
    None
}

pub fn start_button_listener(ui_weak: slint::Weak<GreeterWindow>) {
    thread::spawn(move || {
        info!("Starting hardware button event listener thread...");

        let mut power_press_start: Option<Instant> = None;
        let mut is_screen_blanked = false;
        let mut saved_brightness = read_current_brightness();
        if saved_brightness == 0 {
            saved_brightness = 1156;
        }

        let volume_hud_epoch = Arc::new(Mutex::new(0u64));

        loop {
            // Find all /dev/input/event* devices
            let mut event_files = Vec::new();
            if let Ok(entries) = fs::read_dir("/dev/input") {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with("event") {
                            event_files.push(path);
                        }
                    }
                }
            }

            if event_files.is_empty() {
                thread::sleep(Duration::from_secs(2));
                continue;
            }

            let mut fds = Vec::new();
            let mut opened_files = Vec::new();

            for path in event_files {
                match OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(&path)
                {
                    Ok(file) => {
                        fds.push(libc::pollfd {
                            fd: file.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        });
                        opened_files.push(file);
                    }
                    Err(_) => {}
                }
            }

            if fds.is_empty() {
                thread::sleep(Duration::from_secs(2));
                continue;
            }

            info!("Monitoring {} input devices for hardware keys", fds.len());

            'poll_loop: loop {
                let ret = unsafe {
                    libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 250)
                };

                if ret < 0 {
                    let err = std::io::Error::last_os_error();
                    if err.raw_os_error() == Some(libc::EINTR) {
                        continue;
                    }
                    break 'poll_loop;
                }

                if ret == 0 {
                    // Check if power button was held down for >= 500ms
                    if let Some(start) = power_press_start {
                        if start.elapsed() >= Duration::from_millis(500) {
                            power_press_start = None;
                            info!("Power button long-press detected -> Toggle Power Dialog");
                            let ui_handle = ui_weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = ui_handle.upgrade() {
                                    let curr = ui.get_is_power_dialog_visible();
                                    ui.set_is_power_dialog_visible(!curr);
                                }
                            });
                        }
                    }
                    continue;
                }

                for (idx, poll_fd) in fds.iter_mut().enumerate() {
                    if (poll_fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
                        // Device disconnected or changed
                        break 'poll_loop;
                    }

                    if (poll_fd.revents & libc::POLLIN) != 0 {
                        let mut buf = [0u8; 24]; // 24 bytes for 64-bit input_event
                        let file = &mut opened_files[idx];

                        while let Ok(n) = file.read(&mut buf) {
                            if n == 24 {
                                // Extract type, code, value
                                let type_ = u16::from_ne_bytes([buf[16], buf[17]]);
                                let code = u16::from_ne_bytes([buf[18], buf[19]]);
                                let value = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);

                                // If screen was blanked and any input event arrives, wake screen!
                                if is_screen_blanked {
                                    info!("Wake event received while screen blanked");
                                    write_brightness(saved_brightness);
                                    is_screen_blanked = false;
                                }

                                if type_ == 1 { // EV_KEY
                                    match code {
                                        115 => { // KEY_VOLUMEUP
                                            if value == 1 || value == 2 {
                                                info!("Physical Volume UP pressed");
                                                let new_vol = adjust_volume("5%+");
                                                let ui_handle = ui_weak.clone();
                                                let epoch_clone = volume_hud_epoch.clone();
                                                let cur_epoch = {
                                                    let mut ep = epoch_clone.lock().unwrap();
                                                    *ep += 1;
                                                    *ep
                                                };

                                                let _ = slint::invoke_from_event_loop(move || {
                                                    if let Some(ui) = ui_handle.upgrade() {
                                                        if let Some(vol) = new_vol {
                                                            ui.set_volume_level(vol);
                                                        } else {
                                                            let v = (ui.get_volume_level() + 5).min(100);
                                                            ui.set_volume_level(v);
                                                        }
                                                        ui.set_is_volume_hud_visible(true);
                                                    }
                                                });

                                                let ui_hide = ui_weak.clone();
                                                let epoch_check = volume_hud_epoch.clone();
                                                thread::spawn(move || {
                                                    thread::sleep(Duration::from_millis(2000));
                                                    let ep = *epoch_check.lock().unwrap();
                                                    if ep == cur_epoch {
                                                        let _ = slint::invoke_from_event_loop(move || {
                                                            if let Some(ui) = ui_hide.upgrade() {
                                                                ui.set_is_volume_hud_visible(false);
                                                            }
                                                        });
                                                    }
                                                });
                                            }
                                        }
                                        114 => { // KEY_VOLUMEDOWN
                                            if value == 1 || value == 2 {
                                                info!("Physical Volume DOWN pressed");
                                                let new_vol = adjust_volume("5%-");
                                                let ui_handle = ui_weak.clone();
                                                let epoch_clone = volume_hud_epoch.clone();
                                                let cur_epoch = {
                                                    let mut ep = epoch_clone.lock().unwrap();
                                                    *ep += 1;
                                                    *ep
                                                };

                                                let _ = slint::invoke_from_event_loop(move || {
                                                    if let Some(ui) = ui_handle.upgrade() {
                                                        if let Some(vol) = new_vol {
                                                            ui.set_volume_level(vol);
                                                        } else {
                                                            let v = (ui.get_volume_level() - 5).max(0);
                                                            ui.set_volume_level(v);
                                                        }
                                                        ui.set_is_volume_hud_visible(true);
                                                    }
                                                });

                                                let ui_hide = ui_weak.clone();
                                                let epoch_check = volume_hud_epoch.clone();
                                                thread::spawn(move || {
                                                    thread::sleep(Duration::from_millis(2000));
                                                    let ep = *epoch_check.lock().unwrap();
                                                    if ep == cur_epoch {
                                                        let _ = slint::invoke_from_event_loop(move || {
                                                            if let Some(ui) = ui_hide.upgrade() {
                                                                ui.set_is_volume_hud_visible(false);
                                                            }
                                                        });
                                                    }
                                                });
                                            }
                                        }
                                        116 | 356 => { // KEY_POWER
                                            if value == 1 {
                                                power_press_start = Some(Instant::now());
                                            } else if value == 0 {
                                                if let Some(start) = power_press_start.take() {
                                                    let elapsed = start.elapsed();
                                                    if elapsed >= Duration::from_millis(500) {
                                                        info!(elapsed_ms = elapsed.as_millis(), "Power button long-press -> Toggle Power Dialog");
                                                        let ui_handle = ui_weak.clone();
                                                        let _ = slint::invoke_from_event_loop(move || {
                                                            if let Some(ui) = ui_handle.upgrade() {
                                                                let curr = ui.get_is_power_dialog_visible();
                                                                ui.set_is_power_dialog_visible(!curr);
                                                            }
                                                        });
                                                    } else {
                                                        info!("Power button short-press -> Toggle Screen Blanking");
                                                        let ui_handle = ui_weak.clone();
                                                        let dialog_dismissed = Arc::new(AtomicBool::new(false));
                                                        let dd = dialog_dismissed.clone();
                                                        let _ = slint::invoke_from_event_loop(move || {
                                                            if let Some(ui) = ui_handle.upgrade() {
                                                                if ui.get_is_power_dialog_visible() {
                                                                    ui.set_is_power_dialog_visible(false);
                                                                    dd.store(true, Ordering::SeqCst);
                                                                }
                                                            }
                                                        });

                                                        thread::sleep(Duration::from_millis(20));
                                                        if !dialog_dismissed.load(Ordering::SeqCst) {
                                                            let curr_b = read_current_brightness();
                                                            if curr_b > 0 {
                                                                saved_brightness = curr_b;
                                                                write_brightness(0);
                                                                is_screen_blanked = true;
                                                            } else {
                                                                write_brightness(saved_brightness);
                                                                is_screen_blanked = false;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            } else {
                                break;
                            }
                        }
                    }
                }
            }

            thread::sleep(Duration::from_millis(500));
        }
    });
}
