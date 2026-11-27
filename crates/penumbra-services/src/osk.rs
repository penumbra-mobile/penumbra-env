//! Active on-screen keyboard (OSK) configuration and visibility control.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Configured default virtual keyboard read from `~/.config/penumbra/keyboard.conf`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveOskConfig {
    pub desktop_id: String,
    pub command: String,
}

impl Default for ActiveOskConfig {
    fn default() -> Self {
        Self {
            desktop_id: "penumbra-keyboard.desktop".to_string(),
            command: "penumbra-keyboard".to_string(),
        }
    }
}

impl ActiveOskConfig {
    pub fn config_path() -> PathBuf {
        if let Ok(cfg_dir) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(cfg_dir).join("penumbra/keyboard.conf")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config/penumbra/keyboard.conf")
        } else {
            PathBuf::from("/tmp/penumbra-keyboard.conf")
        }
    }

    pub fn parse(content: &str) -> Self {
        let mut cfg = Self::default();
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(val) = trimmed.strip_prefix("active_desktop_id=") {
                if !val.trim().is_empty() {
                    cfg.desktop_id = val.trim().to_string();
                }
            } else if let Some(val) = trimmed.strip_prefix("command=") {
                if !val.trim().is_empty() {
                    cfg.command = val.trim().to_string();
                }
            }
        }
        cfg
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        fs::read_to_string(path)
            .map(|c| Self::parse(&c))
            .unwrap_or_default()
    }
}

fn runtime_path(name: &str) -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(dir).join(name)
}

fn ensure_daemon_running(exec_cmd: &str) {
    let bin = exec_cmd.split_whitespace().next().unwrap_or(exec_cmd);
    if bin.is_empty() {
        return;
    }
    let running = Command::new("pgrep")
        .args(["-f", bin])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !running {
        let _ = Command::new(bin).env("SQUEEKBOARD_FORCE", "1").spawn();
    }
}

/// Toggles whichever OSK is configured as default in `penumbra-settings`.
pub fn toggle_configured_osk() {
    set_configured_osk_visible(None);
}

/// Shows (`Some(true)`), hides (`Some(false)`), or toggles (`None`) the configured OSK.
pub fn set_configured_osk_visible(visible: Option<bool>) {
    let cfg = ActiveOskConfig::load();
    let combined = format!(
        "{} {}",
        cfg.desktop_id.to_lowercase(),
        cfg.command.to_lowercase()
    );

    if combined.contains("penumbra-keyboard") {
        let cmd = match visible {
            Some(true) => "show",
            Some(false) => "hide",
            None => "toggle",
        };
        let _ = fs::write(runtime_path("penumbra-keyboard-cmd"), cmd);
        if visible != Some(false) {
            ensure_daemon_running(&cfg.command);
        }
        return;
    }

    // Run third-party OSK D-Bus / signal calls off the compositor or UI thread.
    std::thread::spawn(move || {
        if visible != Some(false) {
            ensure_daemon_running(&cfg.command);
        }

        if combined.contains("squeekboard")
            || combined.contains("phosh-osk")
            || combined.contains("stevia")
        {
            let target = match visible {
                Some(v) => v,
                None => {
                    let out = Command::new("busctl")
                        .args([
                            "--user",
                            "get-property",
                            "sm.puri.OSK0",
                            "/sm/puri/OSK0",
                            "sm.puri.OSK0",
                            "Visible",
                        ])
                        .output();
                    let currently_visible = out
                        .map(|o| String::from_utf8_lossy(&o.stdout).contains("true"))
                        .unwrap_or(false);
                    !currently_visible
                }
            };
            let _ = Command::new("busctl")
                .args([
                    "--user",
                    "call",
                    "sm.puri.OSK0",
                    "/sm/puri/OSK0",
                    "sm.puri.OSK0",
                    "SetVisible",
                    "b",
                    if target { "true" } else { "false" },
                ])
                .status();
        } else if combined.contains("wvkbd") {
            let sig = match visible {
                Some(true) => "-USR2",
                Some(false) => "-USR1",
                None => "-RTMIN",
            };
            let _ = Command::new("pkill").args([sig, "wvkbd"]).status();
        } else if visible != Some(false) {
            let _ = Command::new("gsettings")
                .args([
                    "set",
                    "org.gnome.desktop.a11y.applications",
                    "screen-keyboard-enabled",
                    "true",
                ])
                .status();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_keyboard_conf() {
        let conf = "[Keyboard]\nactive_desktop_id=sm.puri.Squeekboard.desktop\ncommand=squeekboard\nname=Squeekboard\n";
        let parsed = ActiveOskConfig::parse(conf);
        assert_eq!(parsed.desktop_id, "sm.puri.Squeekboard.desktop");
        assert_eq!(parsed.command, "squeekboard");
    }
}
