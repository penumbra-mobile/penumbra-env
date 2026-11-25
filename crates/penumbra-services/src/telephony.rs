//! Linux Mobile Voice Telephony (`org.freedesktop.ModemManager1.Modem.Voice` + `Call`)
//! and Call Audio Routing (`org.mobian_project.CallAudio`).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, Value};

use crate::contacts::{normalize_phone, resolve_number};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallState {
    Unknown = 0,
    Dialing = 1,
    RingingOut = 2,
    RingingIn = 3,
    Active = 4,
    Held = 5,
    Waiting = 6,
    Terminated = 7,
}

impl CallState {
    pub fn from_mm(v: i32) -> Self {
        match v {
            1 => Self::Dialing,
            2 => Self::RingingOut,
            3 => Self::RingingIn,
            4 => Self::Active,
            5 => Self::Held,
            6 => Self::Waiting,
            7 => Self::Terminated,
            _ => Self::Unknown,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Dialing => "Dialing...",
            Self::RingingOut => "Ringing...",
            Self::RingingIn => "Incoming call",
            Self::Active => "Active",
            Self::Held => "On hold",
            Self::Waiting => "Waiting",
            Self::Terminated => "Ended",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallDirection {
    Unknown = 0,
    Incoming = 1,
    Outgoing = 2,
}

impl CallDirection {
    pub fn from_mm(v: i32) -> Self {
        match v {
            1 => Self::Incoming,
            2 => Self::Outgoing,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveCall {
    pub path: String,
    pub number: String,
    pub contact_name: String,
    pub state: CallState,
    pub direction: CallDirection,
    pub start_epoch_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHistoryEntry {
    pub id: String,
    pub number: String,
    pub contact_name: String,
    pub direction: String, // "incoming", "outgoing", "missed"
    pub timestamp_secs: u64,
    pub duration_secs: u64,
}

pub fn call_history_path() -> PathBuf {
    if let Ok(data) = std::env::var("XDG_DATA_HOME") {
        PathBuf::from(data).join("penumbra").join("call_history.json")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("penumbra")
            .join("call_history.json")
    } else {
        PathBuf::from("/tmp/penumbra-call-history.json")
    }
}

pub fn load_call_history() -> Vec<CallHistoryEntry> {
    let path = call_history_path();
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Ok(list) = serde_json::from_str::<Vec<CallHistoryEntry>>(&raw) {
            return list;
        }
    }
    Vec::new()
}

pub fn append_call_history(entry: CallHistoryEntry) {
    let mut list = load_call_history();
    list.insert(0, entry);
    list.truncate(200);
    let path = call_history_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&list) {
        let _ = fs::write(path, json);
    }
}

pub fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Discovers the first ModemManager1 modem object path (e.g. `/org/freedesktop/ModemManager1/Modem/0`).
pub fn find_modem_path(conn: &Connection) -> Option<String> {
    let proxy = zbus::blocking::Proxy::new(
        conn,
        "org.freedesktop.ModemManager1",
        "/org/freedesktop/ModemManager1",
        "org.freedesktop.DBus.ObjectManager",
    )
    .ok()?;
    let objs: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>> =
        proxy.call("GetManagedObjects", &()).ok()?;
    let mut paths: Vec<String> = objs.keys().map(|p| p.to_string()).collect();
    paths.sort();
    paths.into_iter().next()
}

fn read_call_properties(conn: &Connection, call_path: &str) -> Option<ActiveCall> {
    let proxy = zbus::blocking::Proxy::new(
        conn,
        "org.freedesktop.ModemManager1",
        call_path,
        "org.freedesktop.ModemManager1.Call",
    )
    .ok()?;

    let number: String = proxy.get_property("Number").unwrap_or_default();
    let state_i: i32 = proxy.get_property("State").unwrap_or(0);
    let dir_i: i32 = proxy.get_property("Direction").unwrap_or(0);
    let state = CallState::from_mm(state_i);
    let direction = CallDirection::from_mm(dir_i);
    let contact_name = resolve_number(&number)
        .map(|c| c.name)
        .unwrap_or_default();

    Some(ActiveCall {
        path: call_path.to_string(),
        number,
        contact_name,
        state,
        direction,
        start_epoch_secs: now_epoch_secs(),
    })
}

/// Lists all current ModemManager calls (excluding already-terminated ones if desired, or all non-deleted).
pub fn list_active_calls() -> Result<Vec<ActiveCall>, String> {
    let conn = Connection::system().map_err(|e| e.to_string())?;
    let Some(modem_path) = find_modem_path(&conn) else {
        return Ok(Vec::new());
    };
    let voice = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.ModemManager1",
        modem_path.as_str(),
        "org.freedesktop.ModemManager1.Modem.Voice",
    )
    .map_err(|e| e.to_string())?;

    let call_paths: Vec<OwnedObjectPath> = voice.call("ListCalls", &()).map_err(|e| e.to_string())?;
    let mut calls = Vec::new();
    for p in call_paths {
        if let Some(c) = read_call_properties(&conn, p.as_str()) {
            if c.state != CallState::Terminated {
                calls.push(c);
            } else {
                // Clean up terminated call object so ModemManager doesn't accumulate stale slots
                let _ = voice.call::<_, _, ()>("DeleteCall", &(p,));
            }
        }
    }
    Ok(calls)
}

/// Places an outgoing cellular voice call via ModemManager1 and switches `callaudiod` to Call mode.
pub fn dial(raw_number: &str) -> Result<ActiveCall, String> {
    let number = normalize_phone(raw_number);
    if number.is_empty() {
        return Err("Empty phone number".to_string());
    }
    let conn = Connection::system().map_err(|e| e.to_string())?;
    let modem_path = find_modem_path(&conn).ok_or_else(|| "No cellular modem found".to_string())?;

    let voice = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.ModemManager1",
        modem_path.as_str(),
        "org.freedesktop.ModemManager1.Modem.Voice",
    )
    .map_err(|e| e.to_string())?;

    let mut props: HashMap<&str, Value<'_>> = HashMap::new();
    props.insert("number", Value::from(number.as_str()));

    let call_path: OwnedObjectPath = voice
        .call("CreateCall", &(props,))
        .map_err(|e| format!("CreateCall failed: {e}"))?;

    let call_proxy = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.ModemManager1",
        call_path.as_str(),
        "org.freedesktop.ModemManager1.Call",
    )
    .map_err(|e| e.to_string())?;

    set_call_audio_mode(true);
    call_proxy
        .call::<_, _, ()>("Start", &())
        .map_err(|e| {
            set_call_audio_mode(false);
            format!("Start call failed: {e}")
        })?;

    let contact_name = resolve_number(&number).map(|c| c.name).unwrap_or_default();
    let now = now_epoch_secs();
    append_call_history(CallHistoryEntry {
        id: format!("out-{now}"),
        number: number.clone(),
        contact_name: contact_name.clone(),
        direction: "outgoing".to_string(),
        timestamp_secs: now,
        duration_secs: 0,
    });

    Ok(ActiveCall {
        path: call_path.to_string(),
        number,
        contact_name,
        state: CallState::Dialing,
        direction: CallDirection::Outgoing,
        start_epoch_secs: now,
    })
}

/// Answers an incoming call via `ModemManager1.Call.Accept` and activates `callaudiod`.
pub fn accept_call(call_path: &str) -> Result<(), String> {
    let conn = Connection::system().map_err(|e| e.to_string())?;
    let call_proxy = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.ModemManager1",
        call_path,
        "org.freedesktop.ModemManager1.Call",
    )
    .map_err(|e| e.to_string())?;

    set_call_audio_mode(true);
    call_proxy
        .call::<_, _, ()>("Accept", &())
        .map_err(|e| format!("Accept failed: {e}"))?;
    Ok(())
}

/// Hangs up or declines a call via `ModemManager1.Call.Hangup` and restores normal audio mode.
pub fn hangup_call(call_path: &str) -> Result<(), String> {
    let conn = Connection::system().map_err(|e| e.to_string())?;
    let call_proxy = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.ModemManager1",
        call_path,
        "org.freedesktop.ModemManager1.Call",
    )
    .map_err(|e| e.to_string())?;

    let res = call_proxy
        .call::<_, _, ()>("Hangup", &())
        .map_err(|e| format!("Hangup failed: {e}"));
    set_call_audio_mode(false);
    res
}

/// Sends DTMF tone(s) (e.g. `"1"`, `"#"`) during an active call.
pub fn send_dtmf(call_path: &str, dtmf: &str) -> Result<(), String> {
    let conn = Connection::system().map_err(|e| e.to_string())?;
    let call_proxy = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.ModemManager1",
        call_path,
        "org.freedesktop.ModemManager1.Call",
    )
    .map_err(|e| e.to_string())?;
    call_proxy
        .call::<_, _, ()>("SendDtmf", &(dtmf,))
        .map_err(|e| format!("SendDtmf failed: {e}"))
}

/// Discovers the PipeWire/PulseAudio card and profile matching `Voice Call` or `HiFi` on ANY Linux device.
fn find_voice_card_and_profiles(want_call: bool, want_speaker: bool) -> Option<(String, String)> {
    let out = std::process::Command::new("pactl")
        .args(["-f", "json", "list", "cards"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let cards: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let arr = cards.as_array()?;

    for card in arr {
        let card_name = card.get("name")?.as_str()?.to_string();
        let Some(profiles) = card.get("profiles").and_then(|p| p.as_object()) else {
            continue;
        };

        let mut voice_profiles: Vec<(&str, i64)> = Vec::new();
        let mut hifi_profiles: Vec<(&str, i64)> = Vec::new();

        for (pname, pobj) in profiles {
            if pname == "off" {
                continue;
            }
            let avail = pobj.get("available").and_then(|v| v.as_bool()).unwrap_or(true);
            if !avail {
                continue;
            }
            let prio = pobj.get("priority").and_then(|v| v.as_i64()).unwrap_or(0);
            let lower = pname.to_lowercase();
            if lower.contains("voice") || lower.contains("call") {
                voice_profiles.push((pname.as_str(), prio));
            } else if lower.contains("hifi") || lower.contains("analog") || lower.contains("default") {
                hifi_profiles.push((pname.as_str(), prio));
            }
        }

        if voice_profiles.is_empty() {
            continue;
        }

        if want_call {
            voice_profiles.sort_by(|a, b| b.1.cmp(&a.1));
            let chosen = if want_speaker {
                voice_profiles
                    .iter()
                    .find(|(n, _)| n.to_lowercase().contains("speaker"))
                    .or_else(|| voice_profiles.first())
            } else {
                voice_profiles
                    .iter()
                    .find(|(n, _)| n.to_lowercase().contains("earpiece") || n.to_lowercase().contains("handset"))
                    .or_else(|| voice_profiles.first())
            };
            if let Some((prof, _)) = chosen {
                return Some((card_name, (*prof).to_string()));
            }
        } else if !hifi_profiles.is_empty() {
            hifi_profiles.sort_by(|a, b| b.1.cmp(&a.1));
            let chosen = hifi_profiles
                .iter()
                .find(|(n, _)| n.to_lowercase().contains("speaker"))
                .or_else(|| hifi_profiles.first());
            if let Some((prof, _)) = chosen {
                return Some((card_name, (*prof).to_string()));
            }
        }
    }
    None
}

fn wait_for_sink_matching(kw: &str) {
    for _ in 0..10 {
        if let Ok(out) = std::process::Command::new("pactl")
            .args(["get-default-sink"])
            .output()
        {
            let sink = String::from_utf8_lossy(&out.stdout).to_lowercase();
            let trimmed = sink.trim();
            if !trimmed.is_empty() && trimmed != "auto_null" && (kw.is_empty() || trimmed.contains(kw)) {
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Switches PipeWire/PulseAudio UCM profile (and sink port if needed) generically across any Linux device.
pub fn set_call_audio_mode(in_call: bool) {
    if let Some((card, profile)) = find_voice_card_and_profiles(in_call, false) {
        let _ = std::process::Command::new("pactl")
            .args(["set-card-profile", &card, &profile])
            .output();
    }
    if in_call {
        // Wait for WirePlumber to create and promote the Voice Call sink before unmuting.
        wait_for_sink_matching("voice");
        let _ = std::process::Command::new("pactl")
            .args(["set-sink-mute", "@DEFAULT_SINK@", "0"])
            .output();
        let _ = std::process::Command::new("pactl")
            .args(["set-sink-volume", "@DEFAULT_SINK@", "100%"])
            .output();
        set_mic_muted(false);
    }
}

fn for_each_alsa_card<F: FnMut(&str)>(mut f: F) {
    let Ok(cards_txt) = std::fs::read_to_string("/proc/asound/cards") else {
        return;
    };
    for line in cards_txt.lines() {
        let trimmed = line.trim_start();
        let Some((idx_str, rest)) = trimmed.split_once(' ') else {
            continue;
        };
        if !rest.trim_start().starts_with('[') || idx_str.parse::<u32>().is_err() {
            continue;
        }
        f(&format!("hw:{idx_str}"));
    }
}

/// Toggles speakerphone during a call generically via PipeWire UCM profiles and sink ports.
pub fn set_speakerphone(enabled: bool) {
    let target_kw = if enabled { "speaker" } else { "earpiece" };
    if let Some((card, profile)) = find_voice_card_and_profiles(true, enabled) {
        let out = std::process::Command::new("pactl")
            .args(["set-card-profile", &card, &profile])
            .output();
        if let Ok(ref res) = out {
            if !res.status.success() {
                tracing::warn!(
                    card = %card,
                    profile = %profile,
                    stderr = %String::from_utf8_lossy(&res.stderr),
                    "pactl set-card-profile failed"
                );
            } else {
                tracing::info!(card = %card, profile = %profile, "Switched call audio profile");
            }
        }
        wait_for_sink_matching(target_kw);
    }
    // Attempt port switch on @DEFAULT_SINK@ for devices using a single Voice Call profile with multiple ports
    let snap = crate::audio::snapshot();
    if let Some(dev) = snap
        .outputs
        .iter()
        .find(|d| d.port_name.to_lowercase().contains(target_kw))
    {
        crate::audio::set_default_sink(&dev.node_name, &dev.port_name);
    }
    let _ = std::process::Command::new("pactl")
        .args(["set-sink-mute", "@DEFAULT_SINK@", "0"])
        .output();
}

/// Toggles microphone mute during a call via PipeWire/PulseAudio default source mute
/// and generic ALSA hardware capture/ADC switches (for DSP voice paths that bypass PipeWire).
pub fn set_mic_muted(muted: bool) {
    crate::audio::set_source_mute("", muted);

    for_each_alsa_card(|hw| {
        let Ok(scontrols) = std::process::Command::new("amixer")
            .args(["-D", hw, "scontrols"])
            .output()
        else {
            return;
        };
        if !scontrols.status.success() {
            return;
        }
        let list = String::from_utf8_lossy(&scontrols.stdout);
        for (ctrl, val_muted, val_unmuted) in [
            ("Capture", "nocap", "cap"),
            ("Mic", "nocap", "cap"),
            ("ADC1", "off", "on"),
            ("ADC1_MIXER", "off", "on"),
            ("ADC2", "off", "on"),
            ("ADC2_MIXER", "off", "on"),
        ] {
            if list.contains(&format!("'{ctrl}'")) {
                let state = if muted { val_muted } else { val_unmuted };
                let _ = std::process::Command::new("amixer")
                    .args(["-D", hw, "sset", ctrl, state])
                    .output();
            }
        }
    });
}

/// Sets the active call output volume percentage (`0..=100`) on PipeWire `@DEFAULT_SINK@`
/// and hardware ALSA RX/codec volume controls (for DSP voice paths that bypass PipeWire PCM).
pub fn set_call_volume(pct: i32) {
    let clamped = pct.clamp(0, 100);
    let _ = std::process::Command::new("pactl")
        .args(["set-sink-mute", "@DEFAULT_SINK@", if clamped == 0 { "1" } else { "0" }])
        .output();
    let _ = std::process::Command::new("pactl")
        .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("{clamped}%")])
        .output();

    for_each_alsa_card(|hw| {
        if let Ok(ctrls) = std::process::Command::new("amixer")
            .args(["-D", hw, "controls"])
            .output()
        {
            if ctrls.status.success() {
                // Generic attenuation-style RX volume controls (0 = 0 dB max, 255 = mute)
                let atten = if clamped == 0 {
                    255
                } else {
                    ((100 - clamped) * 80) / 100
                };
                let atten_str = atten.to_string();
                for line in String::from_utf8_lossy(&ctrls.stdout).lines() {
                    if let Some(start) = line.find("name='") {
                        let rest = &line[start + 6..];
                        if let Some(end) = rest.find('\'') {
                            let name = &rest[..end];
                            let lower = name.to_lowercase();
                            if lower.contains("rx_volume") {
                                let _ = std::process::Command::new("amixer")
                                    .args(["-D", hw, "cset", &format!("name='{name}'"), &atten_str])
                                    .output();
                            }
                        }
                    }
                }
            }
        }

        if let Ok(scontrols) = std::process::Command::new("amixer")
            .args(["-D", hw, "scontrols"])
            .output()
        {
            if scontrols.status.success() {
                let slist = String::from_utf8_lossy(&scontrols.stdout);
                let pct_arg = format!("{clamped}%");
                for ctrl in ["Earpiece", "Speaker", "Voice Rx Volume", "Headphone"] {
                    if slist.contains(&format!("'{ctrl}'")) {
                        let _ = std::process::Command::new("amixer")
                            .args(["-D", hw, "sset", ctrl, &pct_arg])
                            .output();
                    }
                }
            }
        }
    });
}

/// Blocks on the system D-Bus and invokes `on_event` immediately whenever
/// ModemManager1 emits a Voice or Call signal (`CallAdded`, `CallDeleted`,
/// `StateChanged`, or `PropertiesChanged`), eliminating polling delay.
pub fn watch_call_events<F>(mut on_event: F)
where
    F: FnMut(),
{
    loop {
        let Ok(conn) = zbus::blocking::Connection::system() else {
            std::thread::sleep(std::time::Duration::from_secs(2));
            continue;
        };
        if let Ok(dbus) = zbus::blocking::Proxy::new(
            &conn,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        ) {
            for rule in [
                "type='signal',interface='org.freedesktop.ModemManager1.Modem.Voice'",
                "type='signal',interface='org.freedesktop.ModemManager1.Call'",
                "type='signal',interface='org.freedesktop.DBus.Properties',arg0='org.freedesktop.ModemManager1.Call'",
            ] {
                let _ = dbus.call::<_, _, ()>("AddMatch", &(rule,));
            }
        }
        // Trigger an immediate initial sync on connect/reconnect.
        on_event();

        let iter = zbus::blocking::MessageIterator::from(&conn);
        for msg_res in iter {
            let Ok(msg) = msg_res else {
                break;
            };
            if msg.message_type() == zbus::message::Type::Signal {
                let header = msg.header();
                let iface = header.interface().map(|i| i.as_str()).unwrap_or("");
                if iface.starts_with("org.freedesktop.ModemManager1")
                    || iface == "org.freedesktop.DBus.Properties"
                {
                    on_event();
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

