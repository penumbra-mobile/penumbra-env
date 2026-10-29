//! Application discovery, categorization, search filtering, and process spawning
//! for the Penumbra Desktop Convergence Shell.

use std::path::Path;
use std::process::Command;
use freedesktop_desktop_entry::{default_paths, DesktopEntry, Iter};
use tracing::info;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub exec: String,
    pub category: String,
    pub icon_name: String,
    pub terminal: bool,
}

/// Discovers all installed XDG `.desktop` applications from system and user directories.
pub fn discover_desktop_apps() -> Vec<AppInfo> {
    let mut apps = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    for entry_path in Iter::new(default_paths()) {
        if let Ok(entry) = DesktopEntry::from_path(&entry_path, None::<&[&str]>) {
            if entry.no_display() {
                continue;
            }

            if let Some(name) = entry.name(&[""]) {
                if let Some(exec_raw) = entry.exec() {
                    let id = entry.id().to_string();
                    if seen_ids.contains(&id) {
                        continue;
                    }
                    seen_ids.insert(id.clone());

                    let exec_clean = clean_exec(exec_raw);
                    let category = classify_categories(&entry.categories().unwrap_or_default());
                    let icon_name = determine_icon_name(&id, &name, &exec_clean, &category);

                    apps.push(AppInfo {
                        id,
                        name: name.to_string(),
                        exec: exec_clean,
                        category,
                        icon_name,
                        terminal: entry.terminal(),
                    });
                }
            }
        }
    }

    // Default fallback applications if system has sparse .desktop entries
    if apps.is_empty() {
        apps = default_fallback_apps();
    }

    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    info!("Discovered {} desktop applications for Convergence Shell", apps.len());
    apps
}

/// Fallback desktop applications ensuring the launcher is always populated
pub fn default_fallback_apps() -> Vec<AppInfo> {
    vec![
        AppInfo {
            id: "org.penumbra.terminal".into(),
            name: "Terminal".into(),
            exec: "kgx".into(),
            category: "System".into(),
            icon_name: "terminal".into(),
            terminal: false,
        },
        AppInfo {
            id: "org.penumbra.browser".into(),
            name: "Web Browser".into(),
            exec: "firefox".into(),
            category: "Internet".into(),
            icon_name: "browser".into(),
            terminal: false,
        },
        AppInfo {
            id: "org.penumbra.files".into(),
            name: "Files".into(),
            exec: "nautilus".into(),
            category: "Utilities".into(),
            icon_name: "folder".into(),
            terminal: false,
        },
        AppInfo {
            id: "org.penumbra.editor".into(),
            name: "Text Editor".into(),
            exec: "gedit".into(),
            category: "Utilities".into(),
            icon_name: "edit".into(),
            terminal: false,
        },
        AppInfo {
            id: "org.penumbra.settings".into(),
            name: "Penumbra Settings".into(),
            exec: "penumbra-settings".into(),
            category: "System".into(),
            icon_name: "settings".into(),
            terminal: false,
        },
    ]
}

/// Classifies raw XDG category strings into primary desktop categories
pub fn classify_categories(categories: &[&str]) -> String {
    for cat in categories {
        let c = cat.to_lowercase();
        if c.contains("network") || c.contains("web") || c.contains("browser") || c.contains("internet") {
            return "Internet".into();
        }
        if c.contains("audio") || c.contains("video") || c.contains("media") || c.contains("player") {
            return "Multimedia".into();
        }
        if c.contains("development") || c.contains("programming") || c == "ide" || c.starts_with("ide;") {
            return "Development".into();
        }
        if c.contains("system") || c.contains("settings") || c.contains("monitor") {
            return "System".into();
        }
        if c.contains("office") || c.contains("document") || c.contains("text") {
            return "Office".into();
        }
        if c.contains("utility") || c.contains("accessories") || c.contains("tools") {
            return "Utilities".into();
        }
    }
    "Utilities".into()
}

/// Resolves a generic icon glyph matching the desktop icon library
pub fn determine_icon_name(id: &str, name: &str, exec: &str, category: &str) -> String {
    let lower = format!("{} {} {} {}", id, name, exec, category).to_lowercase();

    if lower.contains("term") || lower.contains("console") || lower.contains("alacritty") || lower.contains("foot") || lower.contains("kgx") {
        "terminal".into()
    } else if lower.contains("browser") || lower.contains("firefox") || lower.contains("chromium") || lower.contains("web") || lower.contains("chrome") {
        "browser".into()
    } else if lower.contains("file") || lower.contains("nautilus") || lower.contains("thunar") || lower.contains("manager") {
        "folder".into()
    } else if lower.contains("edit") || lower.contains("gedit") || lower.contains("code") || lower.contains("note") || lower.contains("write") {
        "edit".into()
    } else if lower.contains("setting") || lower.contains("config") || lower.contains("control") {
        "settings".into()
    } else {
        "grid".into()
    }
}

/// Strips XDG % field codes from the command
pub fn clean_exec(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|part| !part.starts_with('%'))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Filters applications by search query and category tab
pub fn filter_apps(all_apps: &[AppInfo], query: &str, category: &str) -> Vec<AppInfo> {
    let q = query.trim().to_lowercase();
    all_apps
        .iter()
        .filter(|app| {
            let matches_cat = category == "All" || app.category.eq_ignore_ascii_case(category);
            if !matches_cat {
                return false;
            }
            if q.is_empty() {
                return true;
            }
            app.name.to_lowercase().contains(&q)
                || app.id.to_lowercase().contains(&q)
                || app.exec.to_lowercase().contains(&q)
        })
        .cloned()
        .collect()
}

/// Spawns the application on the external desktop display
pub fn spawn_desktop_app(app: &AppInfo, output: Option<&str>) -> std::io::Result<()> {
    info!("Launching desktop app '{}' (exec: '{}')", app.name, app.exec);

    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    let target_file = Path::new(&runtime_dir).join("penumbra-target-output");
    let target = output.unwrap_or("desktop");
    let _ = std::fs::write(&target_file, target);

    let cmd = if app.terminal {
        format!("foot -e {}", app.exec)
    } else {
        app.exec.clone()
    };

    Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .spawn()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_exec_strips_field_codes() {
        assert_eq!(clean_exec("firefox %u"), "firefox");
        assert_eq!(clean_exec("gimp-2.10 %U %F"), "gimp-2.10");
        assert_eq!(clean_exec("gedit --new-window %f"), "gedit --new-window");
    }

    #[test]
    fn test_classify_categories() {
        assert_eq!(classify_categories(&["Network", "WebBrowser"]), "Internet");
        assert_eq!(classify_categories(&["AudioVideo", "Player"]), "Multimedia");
        assert_eq!(classify_categories(&["System", "Settings"]), "System");
        assert_eq!(classify_categories(&["Development", "IDE"]), "Development");
        assert_eq!(classify_categories(&["Utility"]), "Utilities");
    }

    #[test]
    fn test_filter_apps_by_query_and_category() {
        let apps = vec![
            AppInfo {
                id: "firefox".into(),
                name: "Firefox Web Browser".into(),
                exec: "firefox".into(),
                category: "Internet".into(),
                icon_name: "browser".into(),
                terminal: false,
            },
            AppInfo {
                id: "alacritty".into(),
                name: "Alacritty Terminal".into(),
                exec: "alacritty".into(),
                category: "System".into(),
                icon_name: "terminal".into(),
                terminal: false,
            },
            AppInfo {
                id: "calculator".into(),
                name: "Calculator".into(),
                exec: "gnome-calculator".into(),
                category: "Utilities".into(),
                icon_name: "grid".into(),
                terminal: false,
            },
        ];

        let res = filter_apps(&apps, "fire", "All");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].id, "firefox");

        let res_cat = filter_apps(&apps, "", "System");
        assert_eq!(res_cat.len(), 1);
        assert_eq!(res_cat[0].id, "alacritty");

        let res_none = filter_apps(&apps, "xyz123", "All");
        assert_eq!(res_none.len(), 0);
    }
}
