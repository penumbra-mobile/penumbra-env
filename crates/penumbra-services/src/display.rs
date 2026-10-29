//! Persistent Multi-Display, Theme, Night Light, and Convergence Configuration.

use std::fs;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DisplayModeInfo {
    pub width: u32,
    pub height: u32,
    pub refresh_rate_hz: u32,
    pub is_preferred: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DisplayOutputInfo {
    pub id: String,
    pub name: String,
    pub connector: String,
    pub enabled: bool,
    pub is_primary: bool,
    pub pos_x: i32,
    pub pos_y: i32,
    pub width: u32,
    pub height: u32,
    pub refresh_rate_hz: u32,
    pub scale: f64,
    pub orientation_deg: u32,
    pub available_modes: Vec<DisplayModeInfo>,
}

impl Default for DisplayOutputInfo {
    fn default() -> Self {
        Self {
            id: "DSI-1".into(),
            name: "Internal Phone Display".into(),
            connector: "DSI-1".into(),
            enabled: true,
            is_primary: true,
            pos_x: 0,
            pos_y: 0,
            width: 1080,
            height: 2340,
            refresh_rate_hz: 60,
            scale: 2.62,
            orientation_deg: 0,
            available_modes: vec![DisplayModeInfo {
                width: 1080,
                height: 2340,
                refresh_rate_hz: 60,
                is_preferred: true,
            }],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct NightLightConfig {
    pub enabled: bool,
    pub temperature_k: u32,
    pub schedule_mode: String, // "sunset" or "manual"
    pub start_hour: u8,
    pub end_hour: u8,
}

impl Default for NightLightConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            temperature_k: 3800,
            schedule_mode: "sunset".into(),
            start_hour: 20,
            end_hour: 7,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DesktopCustomizationConfig {
    pub dock_position: String, // "bottom", "left", "right"
    pub dock_size_px: u32,
    pub dock_autohide: bool,
    pub panel_size_px: u32,
    pub window_snapping: bool,
}

impl Default for DesktopCustomizationConfig {
    fn default() -> Self {
        Self {
            dock_position: "bottom".into(),
            dock_size_px: 64,
            dock_autohide: false,
            panel_size_px: 36,
            window_snapping: true,
        }
    }
}

/// Comprehensive Convergence Displays Configuration (saved in `~/.config/penumbra/displays.toml`)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DisplaysConfig {
    pub arrangement_mode: String, // "extended" or "mirror"
    pub outputs: Vec<DisplayOutputInfo>,
    pub night_light: NightLightConfig,
    pub desktop: DesktopCustomizationConfig,
}

impl Default for DisplaysConfig {
    fn default() -> Self {
        Self {
            arrangement_mode: "extended".into(),
            outputs: vec![DisplayOutputInfo::default()],
            night_light: NightLightConfig::default(),
            desktop: DesktopCustomizationConfig::default(),
        }
    }
}

impl DisplaysConfig {
    pub fn config_path() -> PathBuf {
        if let Ok(cfg_dir) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(cfg_dir).join("penumbra").join("displays.toml")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config").join("penumbra").join("displays.toml")
        } else {
            PathBuf::from("/etc/penumbra/displays.toml")
        }
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(cfg) = toml::from_str(&data) {
                    return cfg;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        fs::write(path, toml_str)
    }

    /// Helper to find or add an external display into the arrangement.
    pub fn ensure_external_output(&mut self, connector: &str, width: u32, height: u32) {
        if !self.outputs.iter().any(|o| o.connector == connector) {
            // Place next to the right of the rightmost display
            let max_x = self.outputs.iter().map(|o| o.pos_x + o.width as i32).max().unwrap_or(0);
            self.outputs.push(DisplayOutputInfo {
                id: connector.into(),
                name: format!("External Display ({})", connector),
                connector: connector.into(),
                enabled: true,
                is_primary: false,
                pos_x: max_x,
                pos_y: 0,
                width,
                height,
                refresh_rate_hz: 60,
                scale: 1.0,
                orientation_deg: 0,
                available_modes: vec![
                    DisplayModeInfo { width, height, refresh_rate_hz: 60, is_preferred: true },
                    DisplayModeInfo { width: 1280, height: 720, refresh_rate_hz: 60, is_preferred: false },
                ],
            });
        }
    }
}

// ==============================================================================
// Legacy Mobile Backlight & Timeout Schema (display.json)
// ==============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    pub brightness: f64,
    pub adaptive_brightness: bool,
    pub dark_theme: bool,
    pub screen_timeout_secs: u32,
    pub accent_color: String,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            brightness: 0.75,
            adaptive_brightness: true,
            dark_theme: true,
            screen_timeout_secs: 120,
            accent_color: "blue".into(),
        }
    }
}

impl DisplayConfig {
    pub fn config_path() -> PathBuf {
        if let Ok(cfg_dir) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(cfg_dir).join("penumbra").join("display.json")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config").join("penumbra").join("display.json")
        } else {
            PathBuf::from("/etc/penumbra/display.json")
        }
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(cfg) = serde_json::from_str(&data) {
                    return cfg;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        fs::write(path, json)
    }

    pub fn apply_gsettings(&self) {
        let color_scheme = if self.dark_theme { "prefer-dark" } else { "prefer-light" };
        let gtk_theme = if self.dark_theme { "Adwaita-dark" } else { "Adwaita" };
        let idle_delay = self.screen_timeout_secs.to_string();

        let _ = std::process::Command::new("gsettings")
            .args(["set", "org.gnome.desktop.interface", "color-scheme", color_scheme])
            .spawn();

        let _ = std::process::Command::new("gsettings")
            .args(["set", "org.gnome.desktop.interface", "gtk-theme", gtk_theme])
            .spawn();

        let _ = std::process::Command::new("gsettings")
            .args(["set", "org.gnome.desktop.session", "idle-delay", &idle_delay])
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_displays_config_toml_roundtrip() {
        let mut cfg = DisplaysConfig::default();
        cfg.ensure_external_output("DP-1", 1920, 1080);
        assert_eq!(cfg.outputs.len(), 2);
        assert_eq!(cfg.outputs[1].pos_x, 1080);

        let toml_str = toml::to_string_pretty(&cfg).expect("serialize toml");
        let parsed: DisplaysConfig = toml::from_str(&toml_str).expect("deserialize toml");
        assert_eq!(cfg, parsed);
    }

    #[test]
    fn test_night_light_defaults() {
        let nl = NightLightConfig::default();
        assert!(!nl.enabled);
        assert_eq!(nl.temperature_k, 3800);
    }
}
