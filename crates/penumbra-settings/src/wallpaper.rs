//! GNOME-compatible wallpaper discovery and multi-folder configuration.
//!
//! Reads and updates `wallpaper_path` and `wallpaper_folders` in
//! `~/.config/penumbra/layout.json` so changes stay synchronized with
//! `penumbra-homescreen`, and scans GNOME background directories as well as
//! `/usr/share/gnome-background-properties/*.xml`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct DiscoveredWallpaper {
    pub path: String,
    pub name: String,
    pub folder: String,
}

pub fn layout_config_path() -> PathBuf {
    if let Ok(cfg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(cfg).join("penumbra").join("layout.json")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config").join("penumbra").join("layout.json")
    } else {
        PathBuf::from("/etc/penumbra/layout.json")
    }
}

pub fn default_wallpaper_folders() -> Vec<String> {
    let mut folders = Vec::new();
    let mut push_if_exists = |p: PathBuf| {
        let s = p.to_string_lossy().to_string();
        if p.exists() && p.is_dir() && !folders.contains(&s) {
            folders.push(s);
        }
    };

    push_if_exists(PathBuf::from("/usr/share/penumbra/assets"));
    push_if_exists(PathBuf::from("/usr/share/backgrounds"));
    push_if_exists(PathBuf::from("/usr/local/share/backgrounds"));
    push_if_exists(PathBuf::from("/usr/share/wallpapers"));

    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        push_if_exists(PathBuf::from(data_home).join("backgrounds"));
    } else if let Ok(home) = std::env::var("HOME") {
        push_if_exists(PathBuf::from(&home).join(".local").join("share").join("backgrounds"));
    }

    if let Ok(home) = std::env::var("HOME") {
        let p_wall = PathBuf::from(&home).join("Pictures").join("Wallpapers");
        if p_wall.exists() {
            push_if_exists(p_wall);
        } else {
            push_if_exists(PathBuf::from(&home).join("Pictures"));
        }
    }

    folders
}

pub fn load_wallpaper_state() -> (String, Vec<String>) {
    let path = layout_config_path();
    if let Ok(raw) = std::fs::read_to_string(&path) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
            let wp = val
                .get("wallpaper_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let folders: Vec<String> = val
                .get("wallpaper_folders")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            if !folders.is_empty() {
                return (wp, folders);
            }
            return (wp, default_wallpaper_folders());
        }
    }
    (String::new(), default_wallpaper_folders())
}

pub fn save_wallpaper_path(wallpaper_path: &str) {
    let path = layout_config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut val = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    if let Some(obj) = val.as_object_mut() {
        obj.insert(
            "wallpaper_path".to_string(),
            serde_json::Value::String(wallpaper_path.to_string()),
        );
        if !obj.contains_key("wallpaper_folders") {
            let defaults: Vec<serde_json::Value> = default_wallpaper_folders()
                .into_iter()
                .map(serde_json::Value::String)
                .collect();
            obj.insert("wallpaper_folders".to_string(), serde_json::Value::Array(defaults));
        }
    }
    if let Ok(json) = serde_json::to_string_pretty(&val) {
        let _ = std::fs::write(&path, json);
    }
}

pub fn save_wallpaper_folders(folders: &[String]) {
    let path = layout_config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut val = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    if let Some(obj) = val.as_object_mut() {
        let arr: Vec<serde_json::Value> = folders
            .iter()
            .map(|s| serde_json::Value::String(s.clone()))
            .collect();
        obj.insert("wallpaper_folders".to_string(), serde_json::Value::Array(arr));
    }
    if let Ok(json) = serde_json::to_string_pretty(&val) {
        let _ = std::fs::write(&path, json);
    }
}

/// Expands `~/` prefix to `$HOME/`.
pub fn expand_tilde(input: &str) -> String {
    let trimmed = input.trim();
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest).to_string_lossy().to_string();
        }
    } else if trimmed == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return home;
        }
    }
    trimmed.to_string()
}

/// Scans all configured folders + GNOME background XML properties.
pub fn scan_wallpapers(folders: &[String]) -> Vec<DiscoveredWallpaper> {
    let mut items = Vec::new();
    let mut seen_paths = HashSet::new();
    let supported_exts = ["png", "jpg", "jpeg", "webp", "bmp"];

    // 1. Scan configured folders (depth <= 2, skip hidden and Screenshots)
    for folder in folders {
        let root_dir = Path::new(folder);
        if !root_dir.exists() || !root_dir.is_dir() {
            continue;
        }
        let folder_name_lower = root_dir
            .file_name()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if folder_name_lower == "screenshots" || folder_name_lower == "screenshot" {
            continue;
        }

        let mut dirs_to_visit = vec![(root_dir.to_path_buf(), 0usize)];
        while let Some((dir, depth)) = dirs_to_visit.pop() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let file_name = entry.file_name().to_string_lossy().to_string();
                    let file_name_lower = file_name.to_lowercase();

                    if path.is_dir() {
                        if file_name.starts_with('.')
                            || file_name_lower == "screenshots"
                            || file_name_lower == "screenshot"
                            || file_name_lower.starts_with("screenshot")
                        {
                            continue;
                        }
                        if depth < 2 {
                            dirs_to_visit.push((path, depth + 1));
                        }
                    } else if path.is_file() {
                        if file_name.starts_with('.') || file_name_lower.starts_with("screenshot") {
                            continue;
                        }
                        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                            if supported_exts.iter().any(|&s| s.eq_ignore_ascii_case(ext)) {
                                let lossy = path.to_string_lossy().to_string();
                                if seen_paths.insert(lossy.clone()) {
                                    let name = path
                                        .file_stem()
                                        .map(|s| s.to_string_lossy().to_string())
                                        .unwrap_or_else(|| "Wallpaper".to_string());
                                    let folder_name = path
                                        .parent()
                                        .and_then(|p| p.file_name())
                                        .map(|s| s.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    items.push(DiscoveredWallpaper {
                                        path: lossy,
                                        name,
                                        folder: folder_name,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Also discover GNOME-packaged wallpapers from gnome-background-properties/*.xml
    let mut xml_dirs = vec![
        PathBuf::from("/usr/share/gnome-background-properties"),
        PathBuf::from("/usr/local/share/gnome-background-properties"),
    ];
    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        xml_dirs.push(PathBuf::from(data_home).join("gnome-background-properties"));
    } else if let Ok(home) = std::env::var("HOME") {
        xml_dirs.push(
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("gnome-background-properties"),
        );
    }

    for xdir in xml_dirs {
        if let Ok(entries) = std::fs::read_dir(&xdir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("xml") {
                    if let Ok(content) = std::fs::read_to_string(&p) {
                        for tag in ["filename", "filename-dark"] {
                            let open = format!("<{tag}>");
                            let close = format!("</{tag}>");
                            let mut rest = content.as_str();
                            while let Some(start) = rest.find(&open) {
                                let after = &rest[start + open.len()..];
                                if let Some(end) = after.find(&close) {
                                    let candidate = after[..end].trim();
                                    let cp = Path::new(candidate);
                                    let ext_ok = cp
                                        .extension()
                                        .and_then(|e| e.to_str())
                                        .map(|ext| supported_exts.iter().any(|&s| s.eq_ignore_ascii_case(ext)))
                                        .unwrap_or(false);
                                    if ext_ok && cp.is_file() && seen_paths.insert(candidate.to_string()) {
                                        let name = cp
                                            .file_stem()
                                            .map(|s| s.to_string_lossy().to_string())
                                            .unwrap_or_else(|| "GNOME Wallpaper".to_string());
                                        items.push(DiscoveredWallpaper {
                                            path: candidate.to_string(),
                                            name,
                                            folder: "GNOME".to_string(),
                                        });
                                    }
                                    rest = &after[end + close.len()..];
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    items
}

