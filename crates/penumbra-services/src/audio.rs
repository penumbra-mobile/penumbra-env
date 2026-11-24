//! Real Linux Audio integration via PulseAudio / PipeWire (`pactl -f json`).
//!
//! Provides GNOME Control Center 1:1 audio management:
//! - Output & Input devices (sinks, sources, and port selection)
//! - Card configuration profiles
//! - Master & per-device volume, mute, and left/right channel balance
//! - Per-application output (`sink-inputs`) and input (`source-outputs`) streams
//! - Speaker test tone playback (`paplay` / `pw-play` / synthesized WAV)

use std::collections::HashMap;
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A selectable audio output or input device (sink/source or port).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioDevice {
    /// Unique identifier: `sink_or_source_name::port_name` (or `sink_or_source_name`).
    pub id: String,
    /// Underlying PulseAudio/PipeWire sink or source name.
    pub node_name: String,
    /// Active/target port name on the node (if ports exist).
    pub port_name: String,
    /// Human-readable device label (e.g. "Speakers - Built-in Audio").
    pub label: String,
    /// Underlying ALSA/PipeWire card name (`device.name` property) for profile lookup.
    pub card_name: String,
    /// Whether this device/port is currently the system default.
    pub is_default: bool,
    /// Volume percentage (0..=150, typically 0..=100).
    pub volume_pct: i32,
    /// Channel balance (-1.0 left .. 0.0 center .. +1.0 right).
    pub balance: f64,
    /// Whether the node is muted.
    pub muted: bool,
    /// True if this is an input (source), false if output (sink).
    pub is_input: bool,
}

/// A hardware card configuration profile (e.g. "Analog Stereo Duplex", "HiFi").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioProfile {
    pub card_name: String,
    pub name: String,
    pub description: String,
    pub available: bool,
    pub active: bool,
}

/// An active per-application audio stream (`sink-input` or `source-output`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioStream {
    pub index: u32,
    pub app_name: String,
    pub media_name: String,
    pub icon_name: String,
    pub volume_pct: i32,
    pub muted: bool,
    pub is_input: bool,
}

/// Complete real-time audio state snapshot matching GNOME Sound Panel.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AudioSnapshot {
    pub default_sink_name: String,
    pub default_source_name: String,
    pub outputs: Vec<AudioDevice>,
    pub inputs: Vec<AudioDevice>,
    pub output_profiles: Vec<AudioProfile>,
    pub input_profiles: Vec<AudioProfile>,
    pub output_streams: Vec<AudioStream>,
    pub input_streams: Vec<AudioStream>,
}

fn run_pactl_json(args: &[&str]) -> Option<Value> {
    let mut full_args = vec!["-f", "json"];
    full_args.extend_from_slice(args);
    let out = Command::new("pactl").args(&full_args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

fn parse_pct(val: &Value) -> Option<i32> {
    let s = val.get("value_percent")?.as_str()?;
    s.trim().trim_end_matches('%').parse::<i32>().ok()
}

fn parse_volume_and_balance(obj: &Value) -> (i32, f64) {
    let balance = obj
        .get("balance")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        .clamp(-1.0, 1.0);

    if let Some(vol_map) = obj.get("volume").and_then(|v| v.as_object()) {
        let mut total = 0i32;
        let mut count = 0i32;
        let mut max_pct = 0i32;
        for ch in vol_map.values() {
            if let Some(p) = parse_pct(ch) {
                total += p;
                count += 1;
                if p > max_pct {
                    max_pct = p;
                }
            }
        }
        if count > 0 {
            // GNOME displays the max channel volume as the master volume when balanced/unbalanced.
            return (max_pct.clamp(0, 150), balance);
        }
        let _ = total;
    }
    (65, balance)
}

fn parse_card_profiles(cards_json: &Value) -> HashMap<String, Vec<AudioProfile>> {
    let mut map: HashMap<String, Vec<AudioProfile>> = HashMap::new();
    let Some(cards) = cards_json.as_array() else {
        return map;
    };
    for card in cards {
        let card_name = card
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if card_name.is_empty() {
            continue;
        }
        let active_profile = card
            .get("active_profile")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mut list = Vec::new();
        if let Some(profiles) = card.get("profiles").and_then(|v| v.as_object()) {
            for (prof_name, prof_obj) in profiles {
                if prof_name == "off" {
                    continue;
                }
                let available = prof_obj
                    .get("available")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                if !available {
                    continue;
                }
                let desc = prof_obj
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or(prof_name)
                    .to_string();
                list.push(AudioProfile {
                    card_name: card_name.clone(),
                    name: prof_name.clone(),
                    description: desc,
                    available,
                    active: prof_name == active_profile,
                });
            }
        }
        list.sort_by(|a, b| a.description.cmp(&b.description));
        map.insert(card_name, list);
    }
    map
}

fn parse_nodes(
    nodes_json: &Value,
    default_name: &str,
    is_input: bool,
) -> Vec<AudioDevice> {
    let mut devices = Vec::new();
    let Some(arr) = nodes_json.as_array() else {
        return devices;
    };
    for node in arr {
        let node_name = node
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if node_name.is_empty() {
            continue;
        }
        // Skip monitor sources for Input Devices (matching GNOME Sound panel).
        // Note: `pactl -f json list sources` emits `"monitor_source": ""` on real
        // microphones and `"monitor_source": "<sink>"` on monitor sources.
        if is_input
            && (node_name.ends_with(".monitor")
                || node
                    .get("monitor_source")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.is_empty())
                || node
                    .get("properties")
                    .and_then(|p| p.get("device.class"))
                    .and_then(|v| v.as_str())
                    == Some("monitor"))
        {
            continue;
        }

        let node_desc = node
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or(&node_name)
            .to_string();
        let card_name = node
            .get("properties")
            .and_then(|p| p.get("device.name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let muted = node.get("mute").and_then(|v| v.as_bool()).unwrap_or(false);
        let (volume_pct, balance) = parse_volume_and_balance(node);
        let active_port = node
            .get("active_port")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let is_node_default = node_name == default_name;

        let ports = node.get("ports").and_then(|v| v.as_array());
        let mut added_port = false;
        if let Some(ports_arr) = ports {
            for port in ports_arr {
                let availability = port
                    .get("availability")
                    .and_then(|v| v.as_str())
                    .unwrap_or("available");
                if availability == "not available" {
                    continue;
                }
                let pname = port
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let pdesc = port
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&pname)
                    .to_string();
                let is_default = is_node_default && (active_port.is_empty() || pname == active_port);
                let label = if node_desc.contains(&pdesc) {
                    node_desc.clone()
                } else {
                    format!("{pdesc} - {node_desc}")
                };
                devices.push(AudioDevice {
                    id: format!("{node_name}::{pname}"),
                    node_name: node_name.clone(),
                    port_name: pname,
                    label,
                    card_name: card_name.clone(),
                    is_default,
                    volume_pct,
                    balance,
                    muted,
                    is_input,
                });
                added_port = true;
            }
        }

        if !added_port {
            devices.push(AudioDevice {
                id: node_name.clone(),
                node_name: node_name.clone(),
                port_name: String::new(),
                label: node_desc,
                card_name,
                is_default: is_node_default,
                volume_pct,
                balance,
                muted,
                is_input,
            });
        }
    }
    devices
}

fn parse_streams(streams_json: &Value, is_input: bool) -> Vec<AudioStream> {
    let mut streams = Vec::new();
    let Some(arr) = streams_json.as_array() else {
        return streams;
    };
    for st in arr {
        let Some(index) = st.get("index").and_then(|v| v.as_u64()).map(|i| i as u32) else {
            continue;
        };
        let props = st.get("properties");
        let app_name = props
            .and_then(|p| p.get("application.name"))
            .and_then(|v| v.as_str())
            .unwrap_or("Application")
            .to_string();
        // Skip our own test tones / pavucontrol level monitors
        if app_name.eq_ignore_ascii_case("PulseAudio Volume Control")
            || app_name.eq_ignore_ascii_case("GNOME Settings")
        {
            continue;
        }
        let media_name = props
            .and_then(|p| p.get("media.name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let icon_name = props
            .and_then(|p| p.get("application.icon_name"))
            .and_then(|v| v.as_str())
            .unwrap_or("audio-x-generic")
            .to_string();
        let muted = st.get("mute").and_then(|v| v.as_bool()).unwrap_or(false);
        let (volume_pct, _) = parse_volume_and_balance(st);
        streams.push(AudioStream {
            index,
            app_name,
            media_name,
            icon_name,
            volume_pct,
            muted,
            is_input,
        });
    }
    streams
}

/// Captures a complete live PipeWire/PulseAudio snapshot for GNOME-style Sound Settings.
pub fn snapshot() -> AudioSnapshot {
    let info = run_pactl_json(&["info"]).unwrap_or(Value::Null);
    let default_sink_name = info
        .get("default_sink_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let default_source_name = info
        .get("default_source_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let cards_json = run_pactl_json(&["list", "cards"]).unwrap_or(Value::Null);
    let card_profiles = parse_card_profiles(&cards_json);

    let sinks_json = run_pactl_json(&["list", "sinks"]).unwrap_or(Value::Null);
    let outputs = parse_nodes(&sinks_json, &default_sink_name, false);

    let sources_json = run_pactl_json(&["list", "sources"]).unwrap_or(Value::Null);
    let inputs = parse_nodes(&sources_json, &default_source_name, true);

    let fallback_profiles = card_profiles
        .values()
        .find(|p| !p.is_empty())
        .cloned()
        .unwrap_or_default();

    let output_profiles = outputs
        .iter()
        .find(|d| d.is_default && !d.card_name.is_empty())
        .or_else(|| outputs.iter().find(|d| !d.card_name.is_empty()))
        .and_then(|d| card_profiles.get(&d.card_name))
        .cloned()
        .unwrap_or_else(|| fallback_profiles.clone());

    let input_profiles = inputs
        .iter()
        .find(|d| d.is_default && !d.card_name.is_empty())
        .or_else(|| inputs.iter().find(|d| !d.card_name.is_empty()))
        .and_then(|d| card_profiles.get(&d.card_name))
        .cloned()
        .unwrap_or(fallback_profiles);

    let sink_inputs_json = run_pactl_json(&["list", "sink-inputs"]).unwrap_or(Value::Null);
    let output_streams = parse_streams(&sink_inputs_json, false);

    let source_outputs_json = run_pactl_json(&["list", "source-outputs"]).unwrap_or(Value::Null);
    let input_streams = parse_streams(&source_outputs_json, true);

    AudioSnapshot {
        default_sink_name,
        default_source_name,
        outputs,
        inputs,
        output_profiles,
        input_profiles,
        output_streams,
        input_streams,
    }
}

/// Switches the default output sink and optionally its active port.
pub fn set_default_sink(node_name: &str, port_name: &str) {
    let _ = Command::new("pactl")
        .args(["set-default-sink", node_name])
        .status();
    if !port_name.is_empty() {
        let _ = Command::new("pactl")
            .args(["set-sink-port", node_name, port_name])
            .status();
    }
}

/// Switches the default input source and optionally its active port.
pub fn set_default_source(node_name: &str, port_name: &str) {
    let _ = Command::new("pactl")
        .args(["set-default-source", node_name])
        .status();
    if !port_name.is_empty() {
        let _ = Command::new("pactl")
            .args(["set-source-port", node_name, port_name])
            .status();
    }
}

/// Sets a card's hardware profile.
pub fn set_card_profile(card_name: &str, profile_name: &str) {
    let _ = Command::new("pactl")
        .args(["set-card-profile", card_name, profile_name])
        .status();
}

/// Sets sink volume (`0..=100` or up to `150`) and L/R channel balance (`-1.0..=1.0`).
pub fn set_sink_volume_balance(node_name: &str, volume_pct: i32, balance: f64) {
    let vol = volume_pct.clamp(0, 150) as f64;
    let bal = balance.clamp(-1.0, 1.0);
    let left = if bal > 0.0 {
        (vol * (1.0 - bal)).round() as i32
    } else {
        vol.round() as i32
    };
    let right = if bal < 0.0 {
        (vol * (1.0 + bal)).round() as i32
    } else {
        vol.round() as i32
    };
    let target = if node_name.is_empty() {
        "@DEFAULT_SINK@"
    } else {
        node_name
    };
    let _ = Command::new("pactl")
        .args([
            "set-sink-volume",
            target,
            &format!("{left}%"),
            &format!("{right}%"),
        ])
        .spawn();
}

/// Sets sink mute state.
pub fn set_sink_mute(node_name: &str, muted: bool) {
    let target = if node_name.is_empty() {
        "@DEFAULT_SINK@"
    } else {
        node_name
    };
    let _ = Command::new("pactl")
        .args(["set-sink-mute", target, if muted { "1" } else { "0" }])
        .spawn();
}

/// Sets source (microphone) volume percentage (`0..=150`).
pub fn set_source_volume(node_name: &str, volume_pct: i32) {
    let target = if node_name.is_empty() {
        "@DEFAULT_SOURCE@"
    } else {
        node_name
    };
    let pct = volume_pct.clamp(0, 150);
    let _ = Command::new("pactl")
        .args(["set-source-volume", target, &format!("{pct}%")])
        .spawn();
}

/// Sets source (microphone) mute state.
pub fn set_source_mute(node_name: &str, muted: bool) {
    let target = if node_name.is_empty() {
        "@DEFAULT_SOURCE@"
    } else {
        node_name
    };
    let _ = Command::new("pactl")
        .args(["set-source-mute", target, if muted { "1" } else { "0" }])
        .spawn();
}

/// Sets per-app stream volume (`sink-input` or `source-output`).
pub fn set_stream_volume(index: u32, is_input: bool, volume_pct: i32) {
    let cmd = if is_input {
        "set-source-output-volume"
    } else {
        "set-sink-input-volume"
    };
    let pct = volume_pct.clamp(0, 150);
    let _ = Command::new("pactl")
        .args([cmd, &index.to_string(), &format!("{pct}%")])
        .spawn();
}

/// Sets per-app stream mute state.
pub fn set_stream_mute(index: u32, is_input: bool, muted: bool) {
    let cmd = if is_input {
        "set-source-output-mute"
    } else {
        "set-sink-input-mute"
    };
    let _ = Command::new("pactl")
        .args([cmd, &index.to_string(), if muted { "1" } else { "0" }])
        .spawn();
}

/// Plays a short synthesized speaker test tone on `"left"`, `"right"`, or `"both"` channels.
pub fn play_test_tone(channel: &str) {
    let ch = channel.to_string();
    std::thread::spawn(move || {
        // Generate a 440Hz / 523Hz stereo 16-bit PCM WAV in memory and pipe to paplay/pw-play
        let sample_rate = 44100u32;
        let duration_samples = (sample_rate as f32 * 0.45) as usize;
        let num_channels = 2u16;
        let data_bytes = (duration_samples * num_channels as usize * 2) as u32;
        let mut wav = Vec::with_capacity(44 + data_bytes as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&num_channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_bytes.to_le_bytes());

        let freq = if ch == "right" { 523.25 } else { 440.0 };
        for i in 0..duration_samples {
            let t = i as f32 / sample_rate as f32;
            let env = if t < 0.03 {
                t / 0.03
            } else if t > 0.38 {
                ((0.45 - t) / 0.07).max(0.0)
            } else {
                1.0
            };
            let s = ((2.0 * std::f32::consts::PI * freq * t).sin() * env * 12000.0) as i16;
            let l = if ch == "right" { 0i16 } else { s };
            let r = if ch == "left" { 0i16 } else { s };
            wav.extend_from_slice(&l.to_le_bytes());
            wav.extend_from_slice(&r.to_le_bytes());
        }

        let tmp = std::env::temp_dir().join("penumbra_test_tone.wav");
        if std::fs::write(&tmp, &wav).is_ok() {
            let status = Command::new("paplay").arg(&tmp).status();
            if status.is_err() {
                let _ = Command::new("pw-play").arg(&tmp).status();
            }
        }
    });
}

/// Plays a short preview of GNOME's alert sound presets ("click", "string", "swing", "hum").
pub fn play_alert_sound(name: &str) {
    let preset = name.to_lowercase();
    if preset == "none" || preset.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        let sample_rate = 44100u32;
        let dur_secs = match preset.as_str() {
            "click" => 0.08_f32,
            "hum" => 0.45_f32,
            _ => 0.30_f32,
        };
        let duration_samples = (sample_rate as f32 * dur_secs) as usize;
        let data_bytes = (duration_samples * 4) as u32;
        let mut wav = Vec::with_capacity(44 + data_bytes as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_bytes.to_le_bytes());

        for i in 0..duration_samples {
            let t = i as f32 / sample_rate as f32;
            let (freq, env) = match preset.as_str() {
                "click" => (1100.0, (1.0 - t / dur_secs).powi(3)),
                "string" => (659.25, (1.0 - t / dur_secs).powi(2)),
                "swing" => (523.25 + 220.0 * (t / dur_secs), (1.0 - t / dur_secs).powi(1)),
                _ => (220.0, if t < 0.05 { t / 0.05 } else { (1.0 - t / dur_secs).max(0.0) }),
            };
            let s = ((2.0 * std::f32::consts::PI * freq * t).sin() * env * 11000.0) as i16;
            wav.extend_from_slice(&s.to_le_bytes());
            wav.extend_from_slice(&s.to_le_bytes());
        }

        let tmp = std::env::temp_dir().join("penumbra_alert_preview.wav");
        if std::fs::write(&tmp, &wav).is_ok() {
            let status = Command::new("paplay").arg(&tmp).status();
            if status.is_err() {
                let _ = Command::new("pw-play").arg(&tmp).status();
            }
        }
    });
}

/// Reads the current default sink volume as a ratio in `0.0..=1.0`.
pub fn read_volume_ratio() -> f64 {
    let output = Command::new("pactl")
        .args(["get-sink-volume", "@DEFAULT_SINK@"])
        .output();

    if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(pct_idx) = text.find('%') {
            let prefix = &text[..pct_idx];
            let start = prefix
                .rfind(|c: char| !c.is_ascii_digit())
                .map(|i| i + 1)
                .unwrap_or(0);
            if let Ok(pct) = prefix[start..].trim().parse::<f64>() {
                return (pct / 100.0).clamp(0.0, 1.0);
            }
        }
    }
    0.65
}

/// Sets the volume of the default sink to `ratio` in `0.0..=1.0`.
pub fn apply_volume_ratio(ratio: f64) {
    let pct = (ratio.clamp(0.0, 1.0) * 100.0).round() as u32;
    let _ = Command::new("pactl")
        .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("{pct}%")])
        .spawn();
}

/// Changes the default sink volume by relative step (e.g. `+5` or `-5`).
pub fn adjust_volume_step(step_pct: i32) {
    let arg = if step_pct >= 0 {
        format!("+{step_pct}%")
    } else {
        format!("{step_pct}%")
    };
    let _ = Command::new("pactl")
        .args(["set-sink-volume", "@DEFAULT_SINK@", &arg])
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sources_keeps_real_mic_with_empty_monitor_source() {
        let json: Value = serde_json::json!([
            {
                "name": "alsa_input.platform-sound.HiFi__Mic__source",
                "description": "Built-in Audio Microphone",
                "mute": false,
                "monitor_source": "",
                "volume": { "front-left": { "value_percent": "100%" } },
                "balance": 0.0,
                "properties": { "device.name": "alsa_card.platform-sound" },
                "ports": [
                    { "name": "[In] Mic", "description": "Microphone", "availability": "availability unknown" }
                ],
                "active_port": "[In] Mic"
            },
            {
                "name": "auto_null.monitor",
                "description": "Monitor of Dummy Output",
                "mute": false,
                "monitor_source": "auto_null",
                "volume": { "front-left": { "value_percent": "100%" } },
                "balance": 0.0,
                "properties": { "device.class": "monitor" },
                "ports": [],
                "active_port": null
            }
        ]);

        let inputs = parse_nodes(&json, "alsa_input.platform-sound.HiFi__Mic__source", true);
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].node_name, "alsa_input.platform-sound.HiFi__Mic__source");
        assert_eq!(inputs[0].card_name, "alsa_card.platform-sound");
        assert!(inputs[0].is_default);
    }
}
