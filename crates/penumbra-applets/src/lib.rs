pub mod calendar;

use chrono::{Datelike, Local, NaiveDate};
use penumbra_services::battery::PowerProfile;
use tracing::info;

slint::include_modules!();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppletKind {
    None,
    Time,
    Network,
    Audio,
    Battery,
    Power,
}

impl AppletKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppletKind::None => "",
            AppletKind::Time => "time",
            AppletKind::Network => "network",
            AppletKind::Audio => "audio",
            AppletKind::Battery => "battery",
            AppletKind::Power => "power",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "time" | "calendar" | "clock" => AppletKind::Time,
            "network" | "wifi" => AppletKind::Network,
            "audio" | "volume" | "sound" => AppletKind::Audio,
            "battery" | "power-profile" => AppletKind::Battery,
            "power" | "session" => AppletKind::Power,
            _ => AppletKind::None,
        }
    }
}

/// Builds a 42-cell (6x7, Monday-first) calendar grid for the current local month.
pub fn build_month_calendar() -> (String, Vec<CalendarDay>) {
    let today = Local::now().date_naive();
    let header = today.format("%B %Y").to_string();
    let days = calendar::month_grid(today.year(), today.month(), today);
    let slint_days = days
        .into_iter()
        .map(|d| CalendarDay {
            label: d.day.to_string().into(),
            in_month: d.in_month,
            is_today: d.is_today,
        })
        .collect();
    (header, slint_days)
}

/// Reads current system audio volume ratio (`0.0..=1.0`).
pub fn read_volume_ratio() -> f32 {
    penumbra_services::audio::read_volume_ratio() as f32
}

/// Applies system audio volume ratio (`0.0..=1.0`).
pub fn apply_volume_ratio(ratio: f32) {
    penumbra_services::audio::apply_volume_ratio(ratio.clamp(0.0, 1.0) as f64);
}

/// Applies a power profile (`"power-saver"`, `"balanced"`, `"performance"`).
pub fn apply_power_profile(profile_id: &str) {
    info!("Penumbra Applets: Applying power profile '{}'", profile_id);
    let profile = PowerProfile::from_str_lossy(profile_id);
    profile.apply();
}

/// Opens a specific `penumbra-settings` subpage on the external display.
pub fn open_settings_page(page: &str) {
    info!("Penumbra Applets: Opening settings page '{}'", page);
    let _ = std::process::Command::new("penumbra-settings")
        .arg("--page")
        .arg(page)
        .spawn();
}

/// Executes a desktop session/power action (`"lock"`, `"logout"`, `"reboot"`, `"poweroff"`).
pub fn execute_power_action(action: &str) {
    info!("Penumbra Applets: Executing power action '{}'", action);
    match action {
        "reboot" => {
            let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
        }
        "poweroff" => {
            let _ = std::process::Command::new("systemctl").arg("poweroff").spawn();
        }
        "logout" => {
            let _ = std::process::Command::new("loginctl")
                .args(["terminate-user", &std::env::var("USER").unwrap_or_default()])
                .spawn();
        }
        "lock" => {
            if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
                let _ = std::fs::write(format!("{}/penumbra-shell-cmd", dir), "lock");
            }
        }
        _ => {}
    }
}

pub fn naive_date_today() -> NaiveDate {
    Local::now().date_naive()
}
