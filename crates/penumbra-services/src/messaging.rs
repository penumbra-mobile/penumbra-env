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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsMessage {
    pub id: String,
    pub peer_number: String,
    pub text: String,
    pub outgoing: bool,
    pub timestamp_secs: u64,
    pub modem_path: String,
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
        if let Some(existing) = threads
            .iter_mut()
            .find(|t| phone_matches(&t.peer_number, &m.peer_number))
        {
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
                unread_count: 0,
            });
        }
    }
    threads.sort_by(|a, b| b.last_timestamp_secs.cmp(&a.last_timestamp_secs));
    threads
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
    };
    let mut store = load_messages();
    store.push(msg.clone());
    save_messages(&store);
    Ok(msg)
}
