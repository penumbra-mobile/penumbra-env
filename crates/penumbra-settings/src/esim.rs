//! eSIM (SGP.22 LPA) Manager Backend powered by `lpac`.
//!
//! Modeled after `third_party/lpa-gtk/src/esim.py`:
//! - Auto-detects operational `lpac` APDU backend (`qmi_qrtr`, `qmi`, `mbim`, or env override).
//! - Reads eUICC chip info (EID) via `lpac chip info`.
//! - Lists, enables, disables, deletes, nicknames, and downloads eSIM profiles (`LPA:1$...`).

use serde::Deserialize;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use tracing::{info, warn};

#[derive(Debug, Clone, Default)]
pub struct ESimProfileInfo {
    pub iccid: String,
    pub nickname: String,
    pub spn: String,
    pub name: String,
    pub state: String,
    pub is_enabled: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ESimState {
    pub available: bool,
    pub eid: String,
    pub backend_label: String,
    pub error_message: String,
    pub profiles: Vec<ESimProfileInfo>,
}

#[derive(Deserialize)]
struct LpacEnvelope {
    #[serde(rename = "type")]
    data_type: String,
    payload: LpacPayload,
}

#[derive(Deserialize)]
struct LpacPayload {
    code: i32,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: serde_json::Value,
}

fn find_lpac_exe() -> Option<String> {
    for candidate in ["/usr/bin/lpac", "/usr/local/bin/lpac"] {
        if std::path::Path::new(candidate).is_file() {
            return Some(candidate.to_string());
        }
    }
    if let Ok(out) = Command::new("which").arg("lpac").output() {
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    None
}

static DETECTED_BACKEND: OnceLock<Mutex<Option<(String, Vec<(String, String)>)>>> = OnceLock::new();

fn parse_lpac_lpa_payload(stdout: &str) -> Option<Result<serde_json::Value, String>> {
    // lpac may emit multiple JSON lines (progress + final lpa)
    for line in stdout.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(env) = serde_json::from_str::<LpacEnvelope>(trimmed) {
            if env.data_type == "lpa" {
                if env.payload.code != 0 {
                    return Some(Err(format!("lpac error {}: {}", env.payload.code, env.payload.message)));
                }
                return Some(Ok(env.payload.data));
            }
        }
    }
    None
}

fn candidate_backends() -> Vec<(String, Vec<(String, String)>)> {
    let mut candidates = Vec::new();
    for slot in ["1", "2", "3", "4"] {
        candidates.push((
            format!("qmi_qrtr (slot {})", slot),
            vec![
                ("LPAC_APDU".to_string(), "qmi_qrtr".to_string()),
                ("LPAC_APDU_QMI_UIM_SLOT".to_string(), slot.to_string()),
            ],
        ));
    }
    for dev in ["/dev/cdc-wdm0", "/dev/wwan0qmi0"] {
        if std::path::Path::new(dev).exists() {
            for slot in ["1", "2"] {
                candidates.push((
                    format!("qmi {} (slot {})", dev, slot),
                    vec![
                        ("LPAC_APDU".to_string(), "qmi".to_string()),
                        ("LPAC_APDU_QMI_DEVICE".to_string(), dev.to_string()),
                        ("LPAC_APDU_QMI_UIM_SLOT".to_string(), slot.to_string()),
                    ],
                ));
            }
        }
    }
    if std::path::Path::new("/dev/wwan0mbim0").exists() {
        candidates.push((
            "mbim".to_string(),
            vec![
                ("LPAC_APDU".to_string(), "mbim".to_string()),
                ("LPAC_APDU_MBIM_DEVICE".to_string(), "/dev/wwan0mbim0".to_string()),
                ("LPAC_APDU_MBIM_USE_PROXY".to_string(), "1".to_string()),
            ],
        ));
    }
    candidates
}

fn detect_apdu_backend(lpac_exe: &str) -> (String, Vec<(String, String)>) {
    if let Ok(existing) = std::env::var("LPAC_APDU") {
        let slot = std::env::var("LPAC_APDU_QMI_UIM_SLOT").ok();
        let label = match slot {
            Some(s) if !s.is_empty() => format!("{} (slot {})", existing, s),
            _ => existing,
        };
        return (label, vec![]);
    }

    let cache = DETECTED_BACKEND.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.as_ref() {
            return cached.clone();
        }
    }

    for (label, envs) in candidate_backends() {
        let mut cmd = Command::new(lpac_exe);
        cmd.args(["chip", "info"]);
        for (k, v) in &envs {
            cmd.env(k, v);
        }
        if let Ok(out) = cmd.output() {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if matches!(parse_lpac_lpa_payload(&stdout), Some(Ok(_))) {
                    if let Ok(mut guard) = cache.lock() {
                        *guard = Some((label.clone(), envs.clone()));
                    }
                    return (label, envs);
                }
            }
        }
    }

    (
        "qmi_qrtr (slot 1)".to_string(),
        vec![
            ("LPAC_APDU".to_string(), "qmi_qrtr".to_string()),
            ("LPAC_APDU_QMI_UIM_SLOT".to_string(), "1".to_string()),
        ],
    )
}

fn run_lpac(args: &[&str]) -> Result<(String, serde_json::Value), String> {
    let Some(lpac_exe) = find_lpac_exe() else {
        return Err("lpac CLI not installed (install lpac for eSIM support)".to_string());
    };
    let (backend_label, envs) = detect_apdu_backend(&lpac_exe);
    let mut cmd = Command::new(&lpac_exe);
    cmd.args(args);
    for (k, v) in &envs {
        cmd.env(k, v);
    }
    if std::env::var_os("LPAC_HTTP").is_none() {
        cmd.env("LPAC_HTTP", "curl");
    }
    let output = cmd.output().map_err(|e| format!("Failed to run lpac: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(res) = parse_lpac_lpa_payload(&stdout) {
        return match res {
            Ok(data) => Ok((backend_label, data)),
            Err(err) => {
                if let Some(cache) = DETECTED_BACKEND.get() {
                    if let Ok(mut guard) = cache.lock() {
                        *guard = None;
                    }
                }
                Err(err)
            }
        };
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("lpac exited with status {}", output.status)
        } else {
            stderr
        });
    }
    Err("Unexpected lpac output".to_string())
}

pub fn fetch_esim_state() -> ESimState {
    let mut state = ESimState::default();
    let (backend, chip_data) = match run_lpac(&["chip", "info"]) {
        Ok(res) => res,
        Err(err) => {
            state.error_message = err;
            return state;
        }
    };
    state.available = true;
    state.backend_label = backend;
    if let Some(eid) = chip_data.get("eidValue").and_then(|v| v.as_str()) {
        state.eid = eid.to_string();
    }

    if let Ok((_, prof_data)) = run_lpac(&["profile", "list"]) {
        if let Some(arr) = prof_data.as_array() {
            for item in arr {
                let iccid = item.get("iccid").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let nickname = item.get("profileNickname").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let spn = item.get("serviceProviderName").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = item.get("profileName").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let pstate = item.get("profileState").and_then(|v| v.as_str()).unwrap_or("disabled").to_string();
                let is_enabled = pstate.eq_ignore_ascii_case("enabled");
                state.profiles.push(ESimProfileInfo {
                    iccid,
                    nickname,
                    spn,
                    name,
                    state: pstate,
                    is_enabled,
                });
            }
        }
    }
    state
}

pub fn enable_profile(iccid: &str) -> Result<(), String> {
    run_lpac(&["profile", "enable", iccid]).map(|_| ())?;
    info!(iccid, "Enabled eSIM profile");
    Ok(())
}

pub fn disable_profile(iccid: &str) -> Result<(), String> {
    run_lpac(&["profile", "disable", iccid]).map(|_| ())?;
    info!(iccid, "Disabled eSIM profile");
    Ok(())
}

pub fn delete_profile(iccid: &str) -> Result<(), String> {
    run_lpac(&["profile", "delete", iccid]).map(|_| ())?;
    info!(iccid, "Deleted eSIM profile");
    Ok(())
}

pub fn download_profile(activation_code: &str) -> Result<(), String> {
    let code = activation_code.trim();
    if code.is_empty() {
        return Err("Activation code cannot be empty".to_string());
    }
    let res = run_lpac(&["profile", "download", "-a", code]);
    if let Err(ref e) = res {
        warn!(error = %e, "eSIM profile download failed");
    } else {
        info!("Downloaded eSIM profile");
    }
    res.map(|_| ())
}

