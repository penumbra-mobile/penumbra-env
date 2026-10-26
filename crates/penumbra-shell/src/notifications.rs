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
            items: vec![
                NotificationItem {
                    id: "notif-1".to_string(),
                    app: "Messages".to_string(),
                    title: "Antigravity Team".to_string(),
                    body: "Android 17 Material 3 Expressive SystemUI is ready!".to_string(),
                    time_text: "now".to_string(),
                    timestamp_secs: 0,
                },
                NotificationItem {
                    id: "notif-2".to_string(),
                    app: "Penumbra Shell".to_string(),
                    title: "Display Server".to_string(),
                    body: "Dual Kawase blur pipeline active at 60 FPS".to_string(),
                    time_text: "2m ago".to_string(),
                    timestamp_secs: 0,
                },
                NotificationItem {
                    id: "notif-3".to_string(),
                    app: "System".to_string(),
                    title: "Battery Full".to_string(),
                    body: "Device is 100% charged and ready".to_string(),
                    time_text: "15m ago".to_string(),
                    timestamp_secs: 0,
                },
            ],
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
