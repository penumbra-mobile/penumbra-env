//! Data model and built-in registry for Quick Settings tiles.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TileState {
    /// High-contrast accent color.
    Active,
    /// Dark surface container.
    Inactive,
    /// Dimmed / disabled.
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileDefinition {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub icon_name: String,
    pub state: TileState,
    pub is_enabled: bool,
}

impl TileDefinition {
    fn new(id: &str, title: &str, icon: &str, on: &str, off: &str, active: bool) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            subtitle: if active { on } else { off }.into(),
            icon_name: icon.into(),
            state: if active {
                TileState::Active
            } else {
                TileState::Inactive
            },
            is_enabled: true,
        }
    }

    /// Flips Active/Inactive and refreshes the subtitle. Unavailable tiles are untouched.
    pub fn toggle(&mut self) {
        let (on, off) = subtitles(&self.id);
        match self.state {
            TileState::Active => {
                self.state = TileState::Inactive;
                self.subtitle = off.into();
            }
            TileState::Inactive => {
                self.state = TileState::Active;
                self.subtitle = on.into();
            }
            TileState::Unavailable => {}
        }
    }
}

fn subtitles(id: &str) -> (&'static str, &'static str) {
    match id {
        "internet" => ("Connected", "Off"),
        "bluetooth" => ("On", "Off"),
        "flashlight" => ("On", "Off"),
        "dnd" => ("Priority only", "Off"),
        "rotation" => ("Auto-rotate", "Portrait"),
        "battery_saver" => ("On", "Off"),
        "airplane" => ("On", "Off"),
        _ => ("On", "Off"),
    }
}

fn find_torch_leds() -> Vec<(std::path::PathBuf, String, u32)> {
    let mut leds = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/leds") else {
        return leds;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_lowercase();
        if lower.contains("torch") || lower.contains("flash") {
            let dir = entry.path();
            let brightness = dir.join("brightness");
            if brightness.exists() {
                let max_val = std::fs::read_to_string(dir.join("max_brightness"))
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok())
                    .filter(|&v| v > 0)
                    .unwrap_or(255);
                leds.push((brightness, name, max_val));
            }
        }
    }
    // Prefer white/primary flash LED over warm/yellow secondary flash when both exist.
    leds.sort_by_key(|(_, name, _)| {
        let l = name.to_lowercase();
        if l.contains("white") || l == "torch" {
            0
        } else {
            1
        }
    });
    leds
}

pub fn is_torch_active() -> bool {
    find_torch_leds().iter().any(|(path, _, _)| {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .is_some_and(|n| n > 0)
    })
}

pub fn set_torch(active: bool) {
    let leds = find_torch_leds();
    let Some((path, led_name, max_val)) = leds.into_iter().next() else {
        tracing::warn!("No torch/flash LED found under /sys/class/leds");
        return;
    };
    let target = if active { max_val } else { 0 };
    if let Err(err) = std::fs::write(&path, target.to_string()) {
        // Sysfs LED nodes are often 0644 root:root; fall back to systemd-logind SetBrightness.
        tracing::debug!(%led_name, %err, "Direct sysfs torch write failed, trying logind SetBrightness");
        let _ = std::process::Command::new("busctl")
            .args([
                "call",
                "org.freedesktop.login1",
                "/org/freedesktop/login1/session/auto",
                "org.freedesktop.login1.Session",
                "SetBrightness",
                "ssu",
                "leds",
                &led_name,
                &target.to_string(),
            ])
            .status();
    }
}

pub fn is_wifi_active() -> bool {
    if let Ok(output) = std::process::Command::new("nmcli").args(["radio", "wifi"]).output() {
        if let Ok(text) = String::from_utf8(output.stdout) {
            return text.trim().eq_ignore_ascii_case("enabled");
        }
    }
    true
}

pub fn set_wifi(active: bool) {
    let arg = if active { "on" } else { "off" };
    let _ = std::process::Command::new("nmcli").args(["radio", "wifi", arg]).spawn();
}

pub fn is_bluetooth_active() -> bool {
    penumbra_services::status::is_bluetooth_powered()
}

pub fn set_bluetooth(active: bool) {
    // Run blocking D-Bus property write off the UI thread.
    std::thread::spawn(move || {
        penumbra_services::status::set_bluetooth_powered(active);
    });
}

pub fn is_battery_saver_active() -> bool {
    if let Ok(gov) = std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor") {
        return gov.trim() == "powersave";
    }
    false
}

pub fn set_battery_saver(active: bool) {
    let gov = if active { "powersave" } else { "schedutil" };
    if let Ok(entries) = std::fs::read_dir("/sys/devices/system/cpu") {
        for entry in entries.flatten() {
            let gov_path = entry.path().join("cpufreq/scaling_governor");
            if gov_path.exists() {
                let _ = std::fs::write(&gov_path, gov);
            }
        }
    }
}

pub fn is_airplane_active() -> bool {
    if let Ok(output) = std::process::Command::new("rfkill").args(["list"]).output() {
        if let Ok(text) = String::from_utf8(output.stdout) {
            let mut has_rfkill = false;
            let mut all_blocked = true;
            for line in text.lines() {
                if line.contains("Soft blocked:") {
                    has_rfkill = true;
                    if line.contains("no") {
                        all_blocked = false;
                    }
                }
            }
            return has_rfkill && all_blocked;
        }
    }
    false
}

pub fn set_airplane(active: bool) {
    let action = if active { "block" } else { "unblock" };
    let _ = std::process::Command::new("rfkill").args([action, "all"]).spawn();
}

/// Every tile the shell knows about, in default catalogue order.
pub fn registry() -> Vec<TileDefinition> {
    let wifi_on = is_wifi_active();
    let bt_on = is_bluetooth_active();
    let torch_on = is_torch_active();
    let saver_on = is_battery_saver_active();
    let airplane_on = is_airplane_active();
    let state = crate::ipc::ShellState::load();

    vec![
        TileDefinition::new("internet", "Internet", "wifi", "Connected", "Off", wifi_on),
        TileDefinition::new("bluetooth", "Bluetooth", "bluetooth", "On", "Off", bt_on),
        TileDefinition::new("flashlight", "Flashlight", "flashlight", "On", "Off", torch_on),
        TileDefinition::new("battery_saver", "Battery Saver", "battery", "On", "Off", saver_on),
        TileDefinition::new("dnd", "Do Not Disturb", "dnd", "Priority only", "Off", state.dnd_active),
        TileDefinition::new("rotation", "Auto-rotate", "rotation", "Auto-rotate", "Portrait", state.auto_rotate),
        TileDefinition::new("airplane", "Airplane mode", "airplane", "On", "Off", airplane_on),
    ]
}

/// Fast, non-blocking tile registry initialization (pure sysfs + saved state, no subprocesses).
/// Hardware radio states (`internet`, `bluetooth`, `airplane`) are refreshed asynchronously after startup.
pub fn registry_fast() -> Vec<TileDefinition> {
    let torch_on = is_torch_active();
    let saver_on = is_battery_saver_active();
    let state = crate::ipc::ShellState::load();

    vec![
        TileDefinition::new("internet", "Internet", "wifi", "Connected", "Off", true),
        TileDefinition::new("bluetooth", "Bluetooth", "bluetooth", "On", "Off", false),
        TileDefinition::new("flashlight", "Flashlight", "flashlight", "On", "Off", torch_on),
        TileDefinition::new("battery_saver", "Battery Saver", "battery", "On", "Off", saver_on),
        TileDefinition::new("dnd", "Do Not Disturb", "dnd", "Priority only", "Off", state.dnd_active),
        TileDefinition::new("rotation", "Auto-rotate", "rotation", "Auto-rotate", "Portrait", state.auto_rotate),
        TileDefinition::new("airplane", "Airplane mode", "airplane", "On", "Off", false),
    ]
}

pub fn default_active_ids() -> Vec<String> {
    [
        "internet",
        "bluetooth",
        "flashlight",
        "battery_saver",
        "dnd",
        "rotation",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}
