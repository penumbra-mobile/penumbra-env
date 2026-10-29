//! Persistent Sound Channels, DND, and Call Vibration Configuration.

use std::fs;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundConfig {
    pub media_volume: f64,
    pub call_volume: f64,
    pub ring_volume: f64,
    pub mic_volume: f64,
    pub output_muted: bool,
    pub mic_muted: bool,
    pub output_device: String, // "speaker", "headset", "hdmi"
    pub dnd_enabled: bool,
    pub vibrate_for_calls: bool,
}

impl Default for SoundConfig {
    fn default() -> Self {
        Self {
            media_volume: 0.65,
            call_volume: 0.80,
            ring_volume: 0.70,
            mic_volume: 0.85,
            output_muted: false,
            mic_muted: false,
            output_device: "speaker".into(),
            dnd_enabled: false,
            vibrate_for_calls: true,
        }
    }
}

impl SoundConfig {
    pub fn config_path() -> PathBuf {
        if let Ok(cfg_dir) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(cfg_dir).join("penumbra").join("sound.json")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config").join("penumbra").join("sound.json")
        } else {
            PathBuf::from("/etc/penumbra/sound.json")
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
}
