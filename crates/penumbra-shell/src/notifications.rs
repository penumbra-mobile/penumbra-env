use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

static NOTIFICATION_COUNTER: AtomicU64 = AtomicU64::new(1);

/// A single notification item with metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationItem {
    pub id: String,
    pub app: String,
    pub title: String,
    pub body: String,
    pub time_text: String,
    pub timestamp_secs: u64,
}

impl NotificationItem {
    pub fn new(app: impl Into<String>, title: impl Into<String>, body: impl Into<String>) -> Self {
        let id = format!("notif-{}", NOTIFICATION_COUNTER.fetch_add(1, Ordering::Relaxed));
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            id,
            app: app.into(),
            title: title.into(),
            body: body.into(),
            time_text: "now".to_string(),
            timestamp_secs: secs,
        }
    }
}

/// Persistent notification store (`~/.config/penumbra/notifications.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationStore {
    pub items: Vec<NotificationItem>,
}

impl Default for NotificationStore {
    fn default() -> Self {
        Self {
            items: Vec::new(),
        }
    }
}

pub fn config_path() -> PathBuf {
    crate::config::get_layout_config_path().with_file_name("notifications.json")
}

impl NotificationStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load() -> Self {
        Self::load_from(&config_path())
    }

    pub fn load_from(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<Self>(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&config_path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        crate::ipc::write_atomic(path, &json)
    }

    pub fn add(&mut self, app: impl Into<String>, title: impl Into<String>, body: impl Into<String>) -> String {
        let item = NotificationItem::new(app, title, body);
        let id = item.id.clone();
        self.items.insert(0, item);
        let _ = self.save();
        id
    }

    pub fn dismiss_at(&mut self, index: usize) -> Option<NotificationItem> {
        if index < self.items.len() {
            let item = self.items.remove(index);
            let _ = self.save();
            Some(item)
        } else {
            None
        }
    }

    pub fn dismiss_id(&mut self, id: &str) -> Option<NotificationItem> {
        if let Some(pos) = self.items.iter().position(|i| i.id == id) {
            let item = self.items.remove(pos);
            let _ = self.save();
            Some(item)
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        let _ = self.save();
    }
}

/// Server object implementing `org.freedesktop.Notifications`.
pub struct NotificationServer {
    tx: tokio::sync::mpsc::UnboundedSender<NotificationItem>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl NotificationServer {
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        _replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        _actions: Vec<String>,
        _hints: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
        _expire_timeout: i32,
    ) -> zbus::fdo::Result<u32> {
        let app = if app_name.trim().is_empty() {
            "Application".to_string()
        } else {
            app_name
        };
        let item = NotificationItem::new(app, summary, body);
        let id_num = NOTIFICATION_COUNTER.load(Ordering::Relaxed) as u32;
        let _ = self.tx.send(item);
        Ok(id_num)
    }

    async fn close_notification(&self, _id: u32) -> zbus::fdo::Result<()> {
        Ok(())
    }

    async fn get_capabilities(&self) -> zbus::fdo::Result<Vec<String>> {
        Ok(vec![
            "body".to_string(),
            "actions".to_string(),
            "persistence".to_string(),
        ])
    }

    async fn get_server_information(
        &self,
    ) -> zbus::fdo::Result<(String, String, String, String)> {
        Ok((
            "Penumbra Notification Daemon".to_string(),
            "Penumbra DE".to_string(),
            "1.0".to_string(),
            "1.2".to_string(),
        ))
    }
}

/// Registers the `org.freedesktop.Notifications` service on the session D-Bus.
pub async fn start_notification_service(
    tx: tokio::sync::mpsc::UnboundedSender<NotificationItem>,
) -> Option<zbus::connection::Connection> {
    match zbus::connection::Builder::session() {
        Ok(builder) => {
            let server = NotificationServer { tx };
            match builder
                .name("org.freedesktop.Notifications")
                .and_then(|b| b.serve_at("/org/freedesktop/Notifications", server))
            {
                Ok(b) => match b.build().await {
                    Ok(conn) => {
                        tracing::info!("Registered org.freedesktop.Notifications D-Bus service");
                        Some(conn)
                    }
                    Err(err) => {
                        tracing::warn!("Could not bind org.freedesktop.Notifications: {err}");
                        None
                    }
                },
                Err(err) => {
                    tracing::warn!("Could not configure Notifications D-Bus server: {err}");
                    None
                }
            }
        }
        Err(err) => {
            tracing::warn!("Session D-Bus unavailable: {err}");
            None
        }
    }
}
