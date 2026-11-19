//! MPRIS2 (`org.mpris.MediaPlayer2.*`) session D-Bus client for real media playback
//! control and metadata (Firefox, Spotify, Chromium, VLC, MPV, etc.).

use std::collections::HashMap;

use zbus::{zvariant::OwnedValue, Connection, Proxy};

#[derive(Debug, Clone, PartialEq)]
pub struct MprisMediaState {
    pub bus_name: String,
    pub player_name: String,
    pub title: String,
    pub artist: String,
    pub is_playing: bool,
    pub progress: f32,
}

fn friendly_player_name(bus_name: &str, identity: Option<String>) -> String {
    if let Some(id) = identity {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let short = bus_name
        .strip_prefix("org.mpris.MediaPlayer2.")
        .unwrap_or(bus_name)
        .split('.')
        .next()
        .unwrap_or("Media");
    let mut chars = short.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Media".to_string(),
    }
}

fn value_to_string(val: &OwnedValue) -> Option<String> {
    if let Ok(s) = <&str>::try_from(val) {
        return Some(s.to_string());
    }
    if let Ok(arr) = <Vec<String>>::try_from(val.clone()) {
        let joined = arr.join(", ");
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    None
}

fn value_to_i64(val: &OwnedValue) -> Option<i64> {
    i64::try_from(val)
        .ok()
        .or_else(|| u64::try_from(val).ok().map(|v| v as i64))
}

async fn read_player_state(conn: &Connection, bus_name: &str) -> Option<MprisMediaState> {
    let player_proxy = Proxy::new(
        conn,
        bus_name,
        "/org/mpris/MediaPlayer2",
        "org.mpris.MediaPlayer2.Player",
    )
    .await
    .ok()?;

    let status: String = player_proxy
        .get_property("PlaybackStatus")
        .await
        .unwrap_or_else(|_| "Stopped".to_string());
    let is_playing = status.eq_ignore_ascii_case("Playing");

    let metadata: HashMap<String, OwnedValue> = player_proxy
        .get_property("Metadata")
        .await
        .unwrap_or_default();

    let title = metadata
        .get("xesam:title")
        .and_then(value_to_string)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let artist = metadata
        .get("xesam:artist")
        .and_then(value_to_string)
        .or_else(|| metadata.get("xesam:album").and_then(value_to_string))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    if status.eq_ignore_ascii_case("Stopped") && title.is_empty() {
        return None;
    }

    let length_us = metadata.get("mpris:length").and_then(value_to_i64).unwrap_or(0);
    let position_us: i64 = player_proxy.get_property("Position").await.unwrap_or(0);
    let progress = if length_us > 0 && position_us >= 0 {
        (position_us as f64 / length_us as f64).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };

    let root_proxy = Proxy::new(
        conn,
        bus_name,
        "/org/mpris/MediaPlayer2",
        "org.mpris.MediaPlayer2",
    )
    .await
    .ok();
    let identity: Option<String> = match root_proxy {
        Some(p) => p.get_property("Identity").await.ok(),
        None => None,
    };

    let player_name = friendly_player_name(bus_name, identity);
    let display_title = if title.is_empty() {
        player_name.clone()
    } else {
        title
    };
    let display_artist = if artist.is_empty() {
        player_name.clone()
    } else {
        artist
    };

    Some(MprisMediaState {
        bus_name: bus_name.to_string(),
        player_name,
        title: display_title,
        artist: display_artist,
        is_playing,
        progress,
    })
}

/// Polls session D-Bus for an active `org.mpris.MediaPlayer2.*` player,
/// prioritizing any player currently in `Playing` state.
pub async fn poll_active_media() -> Option<MprisMediaState> {
    let conn = Connection::session().await.ok()?;
    let dbus_proxy = Proxy::new(
        &conn,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await
    .ok()?;
    let names: Vec<String> = dbus_proxy.call("ListNames", &()).await.ok()?;

    let mut fallback: Option<MprisMediaState> = None;
    for name in names {
        if !name.starts_with("org.mpris.MediaPlayer2.") {
            continue;
        }
        if let Some(state) = read_player_state(&conn, &name).await {
            if state.is_playing {
                return Some(state);
            }
            if fallback.is_none() {
                fallback = Some(state);
            }
        }
    }
    fallback
}

/// Calls an MPRIS2 `org.mpris.MediaPlayer2.Player` method (`PlayPause`, `Next`, `Previous`).
pub async fn send_player_command(bus_name: &str, method: &str) {
    let Ok(conn) = Connection::session().await else {
        return;
    };
    let Ok(proxy) = Proxy::new(
        &conn,
        bus_name,
        "/org/mpris/MediaPlayer2",
        "org.mpris.MediaPlayer2.Player",
    )
    .await
    else {
        return;
    };
    let _: Result<(), _> = proxy.call(method, &()).await;
}

/// Seeks the active MPRIS2 player by `offset_us` microseconds (e.g. `-10_000_000` or `+10_000_000`).
pub async fn send_player_seek(bus_name: &str, offset_us: i64) {
    let Ok(conn) = Connection::session().await else {
        return;
    };
    let Ok(proxy) = Proxy::new(
        &conn,
        bus_name,
        "/org/mpris/MediaPlayer2",
        "org.mpris.MediaPlayer2.Player",
    )
    .await
    else {
        return;
    };
    let _: Result<(), _> = proxy.call("Seek", &(offset_us,)).await;
}

