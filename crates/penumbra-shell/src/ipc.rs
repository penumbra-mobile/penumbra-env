//! IPC client helpers for communicating with the Penumbra Compositor.
//!
//! Provides state serialization, command submission, and window tracking
//! matching the protocol defined by `penumbra-comp`.

use std::path::{Path, PathBuf};

pub const SHELL_STATE_FILE: &str = "penumbra-shell-state";
pub const SHELL_STATE_BIN: &str = "penumbra-shell-state.bin";
pub const SHELL_STATE_SOCK: &str = "penumbra-shell-state.sock";
pub const COMP_STATE_SOCK: &str = "penumbra-comp-state.sock";
pub const SHELL_CMD_FILE: &str = "penumbra-shell-cmd";
pub const WINDOWS_FILE: &str = "penumbra-windows";

/// Returns `$XDG_RUNTIME_DIR/<name>` (falls back to `/tmp`).
pub fn runtime_path(name: &str) -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(dir).join(name)
}

/// Navigation mode of the phone output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PhoneMode {
    #[default]
    Home,
    App,
    Overview,
}

/// Snapshot of shell UI state relevant to compositing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShellState {
    pub mode: PhoneMode,
    pub drawer_open: bool,
    pub drawer_pull_dy: f64,
    pub drawer_dragging: bool,
    pub drawer_progress: f64,
    pub overview_index: usize,
    pub overview_scroll: f64,
    pub is_dragging: bool,
    pub active_card_dy: f64,
    pub power_dialog_open: bool,
    pub volume_modal_open: bool,
    pub volume_level: f32,
    pub nav_gesture_active: bool,
    pub nav_gesture_dy: f64,
    pub nav_gesture_progress: f64,
    pub nav_gesture_vy: f64,
    pub nav_gesture_dx: f64,
    pub nav_gesture_vx: f64,
    pub nav_switch_active: bool,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            mode: PhoneMode::Home,
            drawer_open: false,
            drawer_pull_dy: 0.0,
            drawer_dragging: false,
            drawer_progress: 0.0,
            overview_index: 0,
            overview_scroll: 0.0,
            is_dragging: false,
            active_card_dy: 0.0,
            power_dialog_open: false,
            volume_modal_open: false,
            volume_level: 0.65,
            nav_gesture_active: false,
            nav_gesture_dy: 0.0,
            nav_gesture_progress: 0.0,
            nav_gesture_vy: 0.0,
            nav_gesture_dx: 0.0,
            nav_gesture_vx: 0.0,
            nav_switch_active: false,
        }
    }
}

impl ShellState {
    pub fn parse(text: &str) -> Self {
        let mut s = ShellState::default();
        let mut has_scroll = false;
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match (k.trim(), v.trim()) {
                ("mode", "app") => s.mode = PhoneMode::App,
                ("mode", "overview") => s.mode = PhoneMode::Overview,
                ("mode", _) => s.mode = PhoneMode::Home,
                ("drawer", v) => s.drawer_open = v == "1" || v == "true",
                ("drawer_pull_dy", v) => s.drawer_pull_dy = v.parse().unwrap_or(0.0),
                ("drawer_dragging", v) => s.drawer_dragging = v == "1" || v == "true",
                ("drawer_progress", v) => s.drawer_progress = v.parse().unwrap_or(0.0),
                ("power_dialog", v) => s.power_dialog_open = v == "1" || v == "true",
                ("volume_modal", v) => s.volume_modal_open = v == "1" || v == "true",
                ("volume_level", v) => s.volume_level = v.parse().unwrap_or(0.65),
                ("nav_gesture_active", v) => s.nav_gesture_active = v == "1" || v == "true",
                ("nav_gesture_dy", v) => s.nav_gesture_dy = v.parse().unwrap_or(0.0),
                ("nav_gesture_progress", v) => s.nav_gesture_progress = v.parse().unwrap_or(0.0),
                ("nav_gesture_vy", v) => s.nav_gesture_vy = v.parse().unwrap_or(0.0),
                ("nav_gesture_dx", v) => s.nav_gesture_dx = v.parse().unwrap_or(0.0),
                ("nav_gesture_vx", v) => s.nav_gesture_vx = v.parse().unwrap_or(0.0),
                ("nav_switch_active", v) => s.nav_switch_active = v == "1" || v == "true",
                ("overview_index", v) => s.overview_index = v.parse().unwrap_or(0),
                ("overview_scroll", v) => {
                    s.overview_scroll = v.parse().unwrap_or(0.0);
                    has_scroll = true;
                }
                ("is_dragging", v) => s.is_dragging = v == "1" || v == "true",
                ("active_card_dy", v) => s.active_card_dy = v.parse().unwrap_or(0.0),
                _ => {}
            }
        }
        if !has_scroll {
            s.overview_scroll = s.overview_index as f64;
        }
        s
    }

    pub fn serialize(&self) -> String {
        let mode = match self.mode {
            PhoneMode::Home => "home",
            PhoneMode::App => "app",
            PhoneMode::Overview => "overview",
        };
        format!(
            "mode={}\ndrawer={}\ndrawer_pull_dy={:.1}\ndrawer_dragging={}\ndrawer_progress={:.3}\npower_dialog={}\nvolume_modal={}\nvolume_level={:.2}\nnav_gesture_active={}\nnav_gesture_dy={:.1}\nnav_gesture_progress={:.3}\nnav_gesture_vy={:.1}\nnav_gesture_dx={:.1}\nnav_gesture_vx={:.1}\nnav_switch_active={}\noverview_index={}\noverview_scroll={:.3}\nis_dragging={}\nactive_card_dy={:.1}\n",
            mode,
            if self.drawer_open { 1 } else { 0 },
            self.drawer_pull_dy,
            if self.drawer_dragging { 1 } else { 0 },
            self.drawer_progress,
            if self.power_dialog_open { 1 } else { 0 },
            if self.volume_modal_open { 1 } else { 0 },
            self.volume_level,
            if self.nav_gesture_active { 1 } else { 0 },
            self.nav_gesture_dy,
            self.nav_gesture_progress,
            self.nav_gesture_vy,
            self.nav_gesture_dx,
            self.nav_gesture_vx,
            if self.nav_switch_active { 1 } else { 0 },
            self.overview_index,
            self.overview_scroll,
            if self.is_dragging { 1 } else { 0 },
            self.active_card_dy
        )
    }

    pub const MAGIC: [u8; 4] = *b"PS01";
    pub const BINARY_SIZE: usize = 104;

    /// Encodes state into a zero-allocation, 104-byte little-endian binary buffer.
    pub fn to_bytes(&self, seq: u32) -> [u8; Self::BINARY_SIZE] {
        let mut buf = [0u8; Self::BINARY_SIZE];
        buf[0..4].copy_from_slice(&Self::MAGIC);
        buf[4..8].copy_from_slice(&seq.to_le_bytes());
        buf[8] = match self.mode {
            PhoneMode::Home => 0,
            PhoneMode::App => 1,
            PhoneMode::Overview => 2,
        };
        buf[9] = if self.drawer_open { 1 } else { 0 };
        buf[10] = if self.drawer_dragging { 1 } else { 0 };
        buf[11] = if self.power_dialog_open { 1 } else { 0 };
        buf[12] = if self.volume_modal_open { 1 } else { 0 };
        buf[13] = if self.nav_gesture_active { 1 } else { 0 };
        buf[14] = if self.nav_switch_active { 1 } else { 0 };
        buf[15] = if self.is_dragging { 1 } else { 0 };
        buf[16..20].copy_from_slice(&self.volume_level.to_le_bytes());
        // 20..24 unused alignment padding
        buf[24..32].copy_from_slice(&self.drawer_pull_dy.to_le_bytes());
        buf[32..40].copy_from_slice(&self.drawer_progress.to_le_bytes());
        buf[40..48].copy_from_slice(&(self.overview_index as u64).to_le_bytes());
        buf[48..56].copy_from_slice(&self.overview_scroll.to_le_bytes());
        buf[56..64].copy_from_slice(&self.active_card_dy.to_le_bytes());
        buf[64..72].copy_from_slice(&self.nav_gesture_dy.to_le_bytes());
        buf[72..80].copy_from_slice(&self.nav_gesture_progress.to_le_bytes());
        buf[80..88].copy_from_slice(&self.nav_gesture_vy.to_le_bytes());
        buf[88..96].copy_from_slice(&self.nav_gesture_dx.to_le_bytes());
        buf[96..104].copy_from_slice(&self.nav_gesture_vx.to_le_bytes());
        buf
    }

    /// Decodes state from a 104-byte binary buffer.
    pub fn from_bytes(buf: &[u8]) -> Option<(Self, u32)> {
        if buf.len() < Self::BINARY_SIZE || &buf[0..4] != &Self::MAGIC {
            return None;
        }
        let seq = u32::from_le_bytes(buf[4..8].try_into().ok()?);
        let mode = match buf[8] {
            1 => PhoneMode::App,
            2 => PhoneMode::Overview,
            _ => PhoneMode::Home,
        };
        let s = Self {
            mode,
            drawer_open: buf[9] != 0,
            drawer_dragging: buf[10] != 0,
            power_dialog_open: buf[11] != 0,
            volume_modal_open: buf[12] != 0,
            nav_gesture_active: buf[13] != 0,
            nav_switch_active: buf[14] != 0,
            is_dragging: buf[15] != 0,
            volume_level: f32::from_le_bytes(buf[16..20].try_into().ok()?),
            drawer_pull_dy: f64::from_le_bytes(buf[24..32].try_into().ok()?),
            drawer_progress: f64::from_le_bytes(buf[32..40].try_into().ok()?),
            overview_index: u64::from_le_bytes(buf[40..48].try_into().ok()?) as usize,
            overview_scroll: f64::from_le_bytes(buf[48..56].try_into().ok()?),
            active_card_dy: f64::from_le_bytes(buf[56..64].try_into().ok()?),
            nav_gesture_dy: f64::from_le_bytes(buf[64..72].try_into().ok()?),
            nav_gesture_progress: f64::from_le_bytes(buf[72..80].try_into().ok()?),
            nav_gesture_vy: f64::from_le_bytes(buf[80..88].try_into().ok()?),
            nav_gesture_dx: f64::from_le_bytes(buf[88..96].try_into().ok()?),
            nav_gesture_vx: f64::from_le_bytes(buf[96..104].try_into().ok()?),
        };
        Some((s, seq))
    }

    /// Loads the current state, checking the microsecond binary cache first with legacy fallback.
    pub fn load() -> Self {
        let bin_path = runtime_path(SHELL_STATE_BIN);
        if let Ok(bytes) = std::fs::read(&bin_path) {
            if let Some((s, _seq)) = Self::from_bytes(&bytes) {
                return s;
            }
        }
        let path = runtime_path(SHELL_STATE_FILE);
        std::fs::read_to_string(&path)
            .ok()
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }

    /// Fast non-blocking save: updates binary packet in tmpfs, emits UDS socket datagrams,
    /// and maintains legacy ASCII state file for backwards compatibility.
    pub fn save(&self) -> std::io::Result<()> {
        self.save_with_seq(0)
    }

    pub fn save_with_seq(&self, seq: u32) -> std::io::Result<()> {
        let bin_path = runtime_path(SHELL_STATE_BIN);
        let bytes = self.to_bytes(seq);
        let _ = std::fs::write(&bin_path, &bytes);

        // Notify socket listeners (non-blocking, ignores ENOENT / ECONNREFUSED)
        if let Ok(sock) = std::os::unix::net::UnixDatagram::unbound() {
            let comp_sock = runtime_path(COMP_STATE_SOCK);
            let shell_sock = runtime_path(SHELL_STATE_SOCK);
            let _ = sock.send_to(&bytes, &comp_sock);
            let _ = sock.send_to(&bytes, &shell_sock);
        }

        let path = runtime_path(SHELL_STATE_FILE);
        write_atomic(&path, &self.serialize())
    }
}

/// High-performance non-blocking UnixDatagram state socket
#[derive(Debug)]
pub struct StateSocket {
    socket: std::os::unix::net::UnixDatagram,
}

impl StateSocket {
    /// Binds an IPC socket at `path` (unlinking stale socket if needed) and marks it non-blocking
    pub fn bind(path: &std::path::Path) -> std::io::Result<Self> {
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
        let socket = std::os::unix::net::UnixDatagram::bind(path)?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket })
    }

    /// Creates an unbound socket suitable for zero-allocation sending
    pub fn unbound() -> std::io::Result<Self> {
        let socket = std::os::unix::net::UnixDatagram::unbound()?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket })
    }

    /// Sends a binary ShellState packet (<1µs, in-memory kernel buffer)
    pub fn send_state(&self, target_path: &std::path::Path, state: &ShellState, seq: u32) -> std::io::Result<()> {
        let bytes = state.to_bytes(seq);
        self.socket.send_to(&bytes, target_path)?;
        Ok(())
    }

    /// Tries to receive an incoming state packet without blocking
    pub fn try_recv(&self) -> Option<(ShellState, u32)> {
        let mut buf = [0u8; 256];
        match self.socket.recv(&mut buf) {
            Ok(n) if n >= ShellState::BINARY_SIZE => {
                ShellState::from_bytes(&buf[..n])
            }
            _ => None,
        }
    }
}

/// Atomically writes `contents` to `path` via a temporary file.
pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

/// Appends a one-shot command line to `penumbra-shell-cmd`.
pub fn send_command(cmd: &str) -> std::io::Result<()> {
    use std::io::Write;
    let path = runtime_path(SHELL_CMD_FILE);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", cmd)
}

/// Represents a running task listed in `penumbra-windows`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: u32,
    pub app_id: String,
    pub title: String,
}

/// Reads the current list of running tasks from `penumbra-windows`.
pub fn read_windows() -> Vec<WindowInfo> {
    let path = runtime_path(WINDOWS_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let id = parts.next()?.parse().ok()?;
            let app_id = parts.next()?.to_string();
            let title = parts.next()?.to_string();
            Some(WindowInfo { id, app_id, title })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_state_binary_roundtrip() {
        let s = ShellState {
            mode: PhoneMode::Overview,
            drawer_open: true,
            drawer_dragging: false,
            drawer_pull_dy: 412.5,
            drawer_progress: 0.85,
            overview_index: 3,
            overview_scroll: 2.75,
            is_dragging: true,
            active_card_dy: -12.5,
            power_dialog_open: false,
            volume_modal_open: true,
            volume_level: 0.82,
            nav_gesture_active: true,
            nav_gesture_progress: 0.612,
            nav_gesture_dy: -180.5,
            nav_gesture_vy: -420.0,
            nav_gesture_dx: 55.0,
            nav_gesture_vx: 210.0,
            nav_switch_active: true,
        };
        let bytes = s.to_bytes(42);
        assert_eq!(bytes.len(), ShellState::BINARY_SIZE);
        assert_eq!(&bytes[0..4], &ShellState::MAGIC);
        let (decoded, seq) = ShellState::from_bytes(&bytes).expect("Failed to decode binary state");
        assert_eq!(seq, 42);
        assert_eq!(decoded, s);
    }

    #[test]
    fn test_state_socket_transmission() {
        let temp_dir = std::env::temp_dir();
        let sock_path = temp_dir.join(format!("penumbra-shell-test-{}.sock", std::process::id()));
        let receiver = StateSocket::bind(&sock_path).expect("Failed to bind socket");
        let sender = StateSocket::unbound().expect("Failed to create unbound sender");

        let s = ShellState {
            mode: PhoneMode::App,
            drawer_open: true,
            drawer_progress: 0.92,
            ..Default::default()
        };
        sender.send_state(&sock_path, &s, 101).expect("Failed to send state");

        let (received, seq) = receiver.try_recv().expect("Failed to receive state");
        assert_eq!(seq, 101);
        assert_eq!(received.mode, PhoneMode::App);
        assert_eq!(received.drawer_open, true);
        assert!((received.drawer_progress - 0.92).abs() < 1e-6);

        let _ = std::fs::remove_file(&sock_path);
    }
}
