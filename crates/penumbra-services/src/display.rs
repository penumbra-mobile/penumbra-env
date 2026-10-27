//! Persistent Display, Theme, and Screen Timeout Configuration.

use std::fs;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    pub brightness: f64,
    pub adaptive_brightness: bool,
    pub dark_theme: bool,
    pub screen_timeout_secs: u32,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            brightness: 0.75,
            adaptive_brightness: true,
            dark_theme: true,
            screen_timeout_secs: 120,
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
