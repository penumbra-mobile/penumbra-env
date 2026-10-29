//! Telemetry synchronization for desktop status applets (Battery, Network, Clock).

use chrono::Local;
use penumbra_services::{BatteryService, NetworkService};
use tracing::info;

pub struct TelemetrySnapshot {
    pub time_str: String,
    pub date_str: String,
    pub battery_pct: f32,
    pub is_charging: bool,
    pub wifi_connected: bool,
    pub wifi_ssid: String,
    pub wifi_signal: i32,
}

pub fn current_time_and_date() -> (String, String) {
    let now = Local::now();
    let time_str = now.format("%H:%M").to_string();
    let date_str = now.format("%a, %b %-d").to_string();
    (time_str, date_str)
}

pub struct TelemetryManager {
    battery_service: BatteryService,
    network_service: NetworkService,
}

impl TelemetryManager {
    pub async fn new() -> Self {
        let battery_service = BatteryService::new().await;
        let network_service = NetworkService::new().await;
        info!("Initialized TelemetryManager hardware services");
        Self {
            battery_service,
            network_service,
        }
    }

    pub fn snapshot(&self) -> TelemetrySnapshot {
        let (time_str, date_str) = current_time_and_date();
        let bat = self.battery_service.current();
        let net = self.network_service.current();

        TelemetrySnapshot {
            time_str,
            date_str,
            battery_pct: bat.percentage as f32,
            is_charging: bat.is_charging,
            wifi_connected: net.is_connected,
            wifi_ssid: net.ssid,
            wifi_signal: net.signal_strength as i32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_time_and_date_format() {
        let (time_str, date_str) = current_time_and_date();
        assert_eq!(time_str.len(), 5); // HH:MM
        assert!(time_str.contains(':'));
        assert!(!date_str.is_empty());
    }
}
