//! Linux Mobile SMS Messaging (`org.freedesktop.ModemManager1.Modem.Messaging` + `Sms`)
//! with persistent conversation thread storage (`~/.local/share/penumbra/messages.json`).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, Value};

use crate::contacts::{normalize_phone, phone_matches, resolve_number};
use crate::telephony::{find_modem_path, now_epoch_secs};

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsMessage {
    pub id: String,
    pub peer_number: String,
    pub text: String,
    pub outgoing: bool,
    pub timestamp_secs: u64,
    pub modem_path: String,
    #[serde(default = "default_true")]
    pub read: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsThread {
    pub peer_number: String,
    pub contact_name: String,
    pub last_message: String,
    pub last_timestamp_secs: u64,
    pub unread_count: usize,
}

pub fn messages_file_path() -> PathBuf {
    if let Ok(data) = std::env::var("XDG_DATA_HOME") {
        PathBuf::from(data).join("penumbra").join("messages.json")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("penumbra")
            .join("messages.json")
    } else {
        PathBuf::from("/tmp/penumbra-messages.json")
    }
}

pub fn load_messages() -> Vec<SmsMessage> {
    let path = messages_file_path();
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Ok(list) = serde_json::from_str::<Vec<SmsMessage>>(&raw) {
            return list;
        }
    }
    Vec::new()
}

pub fn save_messages(msgs: &[SmsMessage]) {
    let path = messages_file_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(msgs) {
        let _ = fs::write(path, json);
    }
}

/// Syncs any received SMS messages from `ModemManager1.Modem.Messaging` into local persistent store.
pub fn sync_modem_sms() -> Vec<SmsMessage> {
    let mut store = load_messages();
    let Ok(conn) = Connection::system() else {
        return store;
    };
    let Some(modem_path) = find_modem_path(&conn) else {
        return store;
    };
    let Ok(messaging) = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.ModemManager1",
        modem_path.as_str(),
        "org.freedesktop.ModemManager1.Modem.Messaging",
    ) else {
        return store;
    };

    let Ok(sms_paths): Result<Vec<OwnedObjectPath>, _> = messaging.call("List", &()) else {
        return store;
    };

    let mut changed = false;
    for p in sms_paths {
        let path_str = p.to_string();
        if store.iter().any(|m| m.modem_path == path_str) {
            continue;
        }
        let Ok(sms_proxy) = zbus::blocking::Proxy::new(
            &conn,
            "org.freedesktop.ModemManager1",
            path_str.as_str(),
            "org.freedesktop.ModemManager1.Sms",
        ) else {
            continue;
        };
        let number: String = sms_proxy.get_property("Number").unwrap_or_default();
        let text: String = sms_proxy.get_property("Text").unwrap_or_default();
        let pdu_type: u32 = sms_proxy.get_property("PduType").unwrap_or(1);
        // MM_SMS_PDU_TYPE_SUBMIT = 2 (outgoing), DELIVER = 1 (incoming)
        let outgoing = pdu_type == 2;
        if number.is_empty() && text.is_empty() {
            continue;
        }
        let now = now_epoch_secs();
        store.push(SmsMessage {
            id: format!("mm-{now}-{}", store.len()),
            peer_number: normalize_phone(&number),
            text,
            outgoing,
            timestamp_secs: now,
            modem_path: path_str.clone(),
            read: outgoing,
        });
        changed = true;
    }

    if changed {
        save_messages(&store);
    }
    store
}

/// Groups messages into conversation threads sorted by most recent message.
pub fn list_threads() -> Vec<SmsThread> {
    let msgs = sync_modem_sms();
    let mut threads: Vec<SmsThread> = Vec::new();
    for m in &msgs {
        let is_unread = !m.outgoing && !m.read;
        if let Some(existing) = threads
            .iter_mut()
            .find(|t| phone_matches(&t.peer_number, &m.peer_number))
        {
            if is_unread {
                existing.unread_count += 1;
            }
            if m.timestamp_secs >= existing.last_timestamp_secs {
                existing.last_message = m.text.clone();
                existing.last_timestamp_secs = m.timestamp_secs;
            }
        } else {
            let contact_name = resolve_number(&m.peer_number)
                .map(|c| c.name)
                .unwrap_or_default();
            threads.push(SmsThread {
                peer_number: m.peer_number.clone(),
                contact_name,
                last_message: m.text.clone(),
                last_timestamp_secs: m.timestamp_secs,
                unread_count: usize::from(is_unread),
            });
        }
    }
    threads.sort_by(|a, b| b.last_timestamp_secs.cmp(&a.last_timestamp_secs));
    threads
}

/// Marks all incoming messages in `peer_number`'s thread as read.
pub fn mark_thread_read(peer_number: &str) {
    let mut store = load_messages();
    let mut changed = false;
    for m in &mut store {
        if phone_matches(&m.peer_number, peer_number) && !m.read {
            m.read = true;
            changed = true;
        }
    }
    if changed {
        save_messages(&store);
    }
}

fn delete_modem_sms_paths(paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    let Ok(conn) = Connection::system() else {
        return;
    };
    let Some(modem_path) = find_modem_path(&conn) else {
        return;
    };
    let Ok(messaging) = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.ModemManager1",
        modem_path.as_str(),
        "org.freedesktop.ModemManager1.Modem.Messaging",
    ) else {
        return;
    };
    for p in paths {
        if p.starts_with('/') {
            if let Ok(obj_path) = zbus::zvariant::ObjectPath::try_from(p.as_str()) {
                let _ = messaging.call::<_, _, ()>("Delete", &(obj_path,));
            }
        }
    }
}

/// Deletes a single SMS message by `id` from local storage and ModemManager.
pub fn delete_message(id: &str) {
    let mut store = load_messages();
    let modem_paths: Vec<String> = store
        .iter()
        .filter(|m| m.id == id && !m.modem_path.is_empty())
        .map(|m| m.modem_path.clone())
        .collect();
    let before = store.len();
    store.retain(|m| m.id != id);
    if store.len() != before {
        save_messages(&store);
        delete_modem_sms_paths(&modem_paths);
    }
}

/// Deletes all messages belonging to `peer_number` from local storage and ModemManager.
pub fn delete_thread(peer_number: &str) {
    let mut store = load_messages();
    let modem_paths: Vec<String> = store
        .iter()
        .filter(|m| phone_matches(&m.peer_number, peer_number) && !m.modem_path.is_empty())
        .map(|m| m.modem_path.clone())
        .collect();
    let before = store.len();
    store.retain(|m| !phone_matches(&m.peer_number, peer_number));
    if store.len() != before {
        save_messages(&store);
        delete_modem_sms_paths(&modem_paths);
    }
}

/// Returns all messages belonging to a conversation with `peer_number`.
pub fn messages_for_peer(peer_number: &str) -> Vec<SmsMessage> {
    sync_modem_sms()
        .into_iter()
        .filter(|m| phone_matches(&m.peer_number, peer_number))
        .collect()
}

/// Sends an SMS via `ModemManager1.Modem.Messaging` and persists it locally.
pub fn send_sms(raw_number: &str, text: &str) -> Result<SmsMessage, String> {
    let number = normalize_phone(raw_number);
    let body = text.trim().to_string();
    if number.is_empty() || body.is_empty() {
        return Err("Recipient number and message text are required".to_string());
    }

    let conn = Connection::system().map_err(|e| e.to_string())?;
    let modem_path = find_modem_path(&conn).ok_or_else(|| "No cellular modem found".to_string())?;
    let messaging = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.ModemManager1",
        modem_path.as_str(),
        "org.freedesktop.ModemManager1.Modem.Messaging",
    )
    .map_err(|e| e.to_string())?;

    let mut props: HashMap<&str, Value<'_>> = HashMap::new();
    props.insert("number", Value::from(number.as_str()));
    props.insert("text", Value::from(body.as_str()));

    let sms_path: OwnedObjectPath = messaging
        .call("Create", &(props,))
        .map_err(|e| format!("SMS Create failed: {e}"))?;

    let sms_proxy = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.ModemManager1",
        sms_path.as_str(),
        "org.freedesktop.ModemManager1.Sms",
    )
    .map_err(|e| e.to_string())?;

    sms_proxy
        .call::<_, _, ()>("Send", &())
        .map_err(|e| format!("SMS Send failed: {e}"))?;

    let now = now_epoch_secs();
    let msg = SmsMessage {
        id: format!("tx-{now}"),
        peer_number: number,
        text: body,
        outgoing: true,
        timestamp_secs: now,
        modem_path: sms_path.to_string(),
        read: true,
    };
    let mut store = load_messages();
    store.push(msg.clone());
    save_messages(&store);
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sms_message_read_defaults_true_on_legacy_json() {
        let legacy = r#"{"id":"1","peer_number":"+15551234567","text":"hi","outgoing":false,"timestamp_secs":100,"modem_path":""}"#;
        let parsed: SmsMessage = serde_json::from_str(legacy).expect("parse legacy sms");
        assert!(parsed.read);
    }
}
