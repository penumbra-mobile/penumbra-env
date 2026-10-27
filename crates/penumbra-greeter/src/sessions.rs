use std::fs;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct WaylandSession {
    pub id: String,
    pub name: String,
    pub comment: String,
    pub exec: Vec<String>,
}

pub fn get_available_sessions() -> Vec<WaylandSession> {
    let mut sessions = Vec::new();
    let dir_path = Path::new("/usr/share/wayland-sessions");

    if dir_path.is_dir() {
        if let Ok(entries) = fs::read_dir(dir_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("desktop") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        let id = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("unknown")
                            .to_string();

                        let mut name = id.clone();
                        let mut comment = String::new();
                        let mut exec_cmd = String::new();
                        let mut in_desktop_entry = false;

                        for line in content.lines() {
                            let line = line.trim();
                            if line.starts_with('[') && line.ends_with(']') {
                                in_desktop_entry = line == "[Desktop Entry]";
                                continue;
                            }
                            if !in_desktop_entry {
                                continue;
                            }

                            if let Some(val) = line.strip_prefix("Name=") {
                                name = val.trim().to_string();
                            } else if let Some(val) = line.strip_prefix("Comment=") {
                                comment = val.trim().to_string();
                            } else if let Some(val) = line.strip_prefix("Exec=") {
                                exec_cmd = val.trim().to_string();
                            }
                        }

                        if !exec_cmd.is_empty() {
                            let exec_parts = shlex_split(&exec_cmd);
                            sessions.push(WaylandSession {
                                id,
                                name,
                                comment,
                                exec: exec_parts,
                            });
                        }
                    }
                }
            }
        }
    }

    // Ensure default Penumbra Mobile session is prioritized
    let default_penumbra = WaylandSession {
        id: "penumbra".to_string(),
        name: "Penumbra Mobile".to_string(),
        comment: "Adaptive Mobile Wayland Compositor".to_string(),
        exec: vec!["/usr/local/bin/penumbra-native-session".to_string()],
    };

    if let Some(pos) = sessions.iter().position(|s| s.id.to_lowercase().contains("penumbra")) {
        let s = sessions.remove(pos);
        sessions.insert(0, s);
    } else {
        sessions.insert(0, default_penumbra);
    }

    sessions
}

fn shlex_split(input: &str) -> Vec<String> {
    // Simple command-line splitting handling basic spaces
    input
        .split_whitespace()
        .map(|s| s.to_string())
        .collect()
}
