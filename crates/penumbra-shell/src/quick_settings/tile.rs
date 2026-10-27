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

/// Query and control real system hardware subsystems.
pub fn is_torch_active() -> bool {
    for path in ["/sys/class/leds/white:flash/brightness", "/sys/class/leds/torch/brightness"] {
        if let Ok(val) = std::fs::read_to_string(path) {
            if let Ok(num) = val.trim().parse::<u32>() {
                return num > 0;
            }
        }
    }
    false
}

pub fn set_torch(active: bool) {
    let val = if active { "255" } else { "0" };
    for path in ["/sys/class/leds/white:flash/brightness", "/sys/class/leds/torch/brightness"] {
        if std::path::Path::new(path).exists() {
            let _ = std::fs::write(path, val);
            break;
        }
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
    if let Ok(output) = std::process::Command::new("bluetoothctl").args(["show"]).output() {
        if let Ok(text) = String::from_utf8(output.stdout) {
            return text.lines().any(|l| l.trim().eq_ignore_ascii_case("Powered: yes"));
        }
    }
    true
}

pub fn set_bluetooth(active: bool) {
    let arg = if active { "on" } else { "off" };
    let _ = std::process::Command::new("bluetoothctl").args(["power", arg]).spawn();
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
