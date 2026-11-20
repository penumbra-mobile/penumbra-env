//! ModemManager1 & NetworkManager WWAN / Cellular / APN D-Bus Backend.
//!
//! Modeled after GNOME Settings (`third_party/gnome-control-center/panels/wwan`):
//! - Discovers modems via `org.freedesktop.ModemManager1` (`GetManagedObjects`).
//! - Reads SIM state, lock status (`UnlockRequired`), operator name, signal quality,
//!   access technology (5G/4G/3G/2G), IMEI, and ICCID.
//! - Supports SIM PIN unlock (`Sim.SendPin`), preferred network mode (`Modem.SetCurrentModes`),
//!   and NetworkManager GSM/APN profile listing, creation, editing, deletion, and activation.

use std::collections::HashMap;
use tracing::{info, warn};
use zbus::{
    zvariant::{OwnedObjectPath, OwnedValue, Value},
    Connection, Proxy,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApnProfile {
    pub path: String,
    pub uuid: String,
    pub name: String,
    pub apn: String,
    pub username: String,
    pub password: String,
    pub roaming_allowed: bool,
    pub is_active: bool,
}

#[derive(Debug, Clone, Default)]
pub struct WwanState {
    pub modem_present: bool,
    pub modem_path: String,
    pub sim_path: String,
    pub sim_present: bool,
    pub sim_locked: bool,
    pub lock_reason: String,
    pub model: String,
    pub imei: String,
    pub iccid: String,
    pub operator_name: String,
    pub access_tech: String,
    pub signal_percent: u32,
    pub mobile_data_enabled: bool,
    pub data_roaming_enabled: bool,
    pub network_mode: String, // "auto", "4g_3g", "4g_only", "3g_2g"
    pub apns: Vec<ApnProfile>,
    pub status_message: String,
}

fn access_tech_label(mask: u32) -> &'static str {
    if mask & (1 << 15) != 0 {
        "5G"
    } else if mask & (1 << 14) != 0 {
        "4G LTE"
    } else if mask & ((1 << 5) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9)) != 0 {
        "3G"
    } else if mask & ((1 << 1) | (1 << 2) | (1 << 3) | (1 << 4)) != 0 {
        "2G"
    } else {
        ""
    }
}

fn lock_reason_label(code: u32) -> &'static str {
    match code {
        0 => "",
        1 => "Unlocked",
        2 => "SIM PIN required",
        3 => "SIM PUK required",
        4 => "SIM PIN2 required",
        5 => "SIM PUK2 required",
        _ => "SIM Locked",
    }
}

fn mode_tuple_to_label(allowed: u32, preferred: u32) -> &'static str {
    // MMModemMode flags: 2G=2, 3G=4, 4G=8, 5G=16
    if allowed == 8 && preferred == 0 {
        "4g_only"
    } else if allowed == (8 | 4) && preferred == 8 {
        "4g_3g"
    } else if allowed == (4 | 2) && preferred == 4 {
        "3g_2g"
    } else {
        "auto"
    }
}

pub struct WwanBackend;

impl WwanBackend {
    pub async fn fetch_state() -> WwanState {
        let mut state = WwanState {
            network_mode: "auto".to_string(),
            ..Default::default()
        };

        let Ok(conn) = Connection::system().await else {
            state.status_message = "System D-Bus unavailable".to_string();
            return state;
        };

        // 1. Query ModemManager1
        if let Ok(mm_proxy) = Proxy::new(
            &conn,
            "org.freedesktop.ModemManager1",
            "/org/freedesktop/ModemManager1",
            "org.freedesktop.DBus.ObjectManager",
        )
        .await
        {
            if let Ok(objects) = mm_proxy
                .call::<_, _, HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>>(
                    "GetManagedObjects",
                    &(),
                )
                .await
            {
                let mut sim_map: HashMap<String, HashMap<String, OwnedValue>> = HashMap::new();
                for (path, ifaces) in &objects {
                    if let Some(sim_props) = ifaces.get("org.freedesktop.ModemManager1.Sim") {
                        sim_map.insert(path.to_string(), sim_props.clone());
                    }
                }

                for (path, ifaces) in objects {
                    if let Some(modem) = ifaces.get("org.freedesktop.ModemManager1.Modem") {
                        state.modem_present = true;
                        state.modem_path = path.to_string();

                        if let Some(m) = modem.get("Model").and_then(|v| <&str>::try_from(v).ok()) {
                            state.model = m.to_string();
                        }
                        if let Some(eq) = modem.get("EquipmentIdentifier").and_then(|v| <&str>::try_from(v).ok()) {
                            state.imei = eq.to_string();
                        }
                        if let Some(sq) = modem.get("SignalQuality") {
                            if let Ok((pct, _)) = <(u32, bool)>::try_from(sq.clone()) {
                                state.signal_percent = pct.min(100);
                            }
                        }
                        if let Some(tech) = modem.get("AccessTechnologies").and_then(|v| u32::try_from(v).ok()) {
                            state.access_tech = access_tech_label(tech).to_string();
                        }
                        if let Some(cm) = modem.get("CurrentModes") {
                            if let Ok((allowed, preferred)) = <(u32, u32)>::try_from(cm.clone()) {
                                state.network_mode = mode_tuple_to_label(allowed, preferred).to_string();
                            }
                        }
                        if let Some(lock_val) = modem.get("UnlockRequired").and_then(|v| u32::try_from(v).ok()) {
                            // MM_MODEM_LOCK_NONE == 1, UNKNOWN == 0
                            state.sim_locked = lock_val >= 2;
                            state.lock_reason = lock_reason_label(lock_val).to_string();
                        }
                        if let Some(sim_obj) = modem.get("Sim").and_then(|v| OwnedObjectPath::try_from(v.clone()).ok()) {
                            let sim_str = sim_obj.to_string();
                            if sim_str != "/" {
                                state.sim_present = true;
                                state.sim_path = sim_str.clone();
                                if let Some(sim_props) = sim_map.get(&sim_str) {
                                    if let Some(iccid) = sim_props.get("SimIdentifier").and_then(|v| <&str>::try_from(v).ok()) {
                                        state.iccid = iccid.to_string();
                                    }
                                    if let Some(op) = sim_props.get("OperatorName").and_then(|v| <&str>::try_from(v).ok()) {
                                        if !op.is_empty() {
                                            state.operator_name = op.to_string();
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(m3gpp) = ifaces.get("org.freedesktop.ModemManager1.Modem.Modem3gpp") {
                            if let Some(op) = m3gpp.get("OperatorName").and_then(|v| <&str>::try_from(v).ok()) {
                                if !op.is_empty() {
                                    state.operator_name = op.to_string();
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }

        // 2. Query NetworkManager WWAN enabled & GSM APN connections
        if let Ok(nm_proxy) = Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
        )
        .await
        {
            if let Ok(enabled) = nm_proxy.get_property::<bool>("WwanEnabled").await {
                state.mobile_data_enabled = enabled;
            }

            let mut active_uuids = std::collections::HashSet::new();
            if let Ok(active_conns) = nm_proxy
                .get_property::<Vec<OwnedObjectPath>>("ActiveConnections")
                .await
            {
                for ac_path in active_conns {
                    if let Ok(ac_proxy) = Proxy::new(
                        &conn,
                        "org.freedesktop.NetworkManager",
                        ac_path,
                        "org.freedesktop.NetworkManager.Connection.Active",
                    )
                    .await
                    {
                        if let Ok(uuid) = ac_proxy.get_property::<String>("Uuid").await {
                            active_uuids.insert(uuid);
                        }
                    }
                }
            }

            if let Ok(settings_proxy) = Proxy::new(
                &conn,
                "org.freedesktop.NetworkManager",
                "/org/freedesktop/NetworkManager/Settings",
                "org.freedesktop.NetworkManager.Settings",
            )
            .await
            {
                if let Ok(conns) = settings_proxy
                    .call::<_, _, Vec<OwnedObjectPath>>("ListConnections", &())
                    .await
                {
                    for cpath in conns {
                        let Ok(c_proxy) = Proxy::new(
                            &conn,
                            "org.freedesktop.NetworkManager",
                            cpath.clone(),
                            "org.freedesktop.NetworkManager.Settings.Connection",
                        )
                        .await
                        else {
                            continue;
                        };
                        let Ok(cfg) = c_proxy
                            .call::<_, _, HashMap<String, HashMap<String, OwnedValue>>>("GetSettings", &())
                            .await
                        else {
                            continue;
                        };
                        let Some(conn_sec) = cfg.get("connection") else {
                            continue;
                        };
                        let ctype = conn_sec
                            .get("type")
                            .and_then(|v| <&str>::try_from(v).ok())
                            .unwrap_or("");
                        if ctype != "gsm" {
                            continue;
                        }
                        let name = conn_sec
                            .get("id")
                            .and_then(|v| <&str>::try_from(v).ok())
                            .unwrap_or("Mobile")
                            .to_string();
                        let uuid = conn_sec
                            .get("uuid")
                            .and_then(|v| <&str>::try_from(v).ok())
                            .unwrap_or("")
                            .to_string();
                        let gsm = cfg.get("gsm");
                        let apn = gsm
                            .and_then(|g| g.get("apn"))
                            .and_then(|v| <&str>::try_from(v).ok())
                            .unwrap_or("")
                            .to_string();
                        let username = gsm
                            .and_then(|g| g.get("username"))
                            .and_then(|v| <&str>::try_from(v).ok())
                            .unwrap_or("")
                            .to_string();
                        let password = gsm
                            .and_then(|g| g.get("password"))
                            .and_then(|v| <&str>::try_from(v).ok())
                            .unwrap_or("")
                            .to_string();
                        let home_only = gsm
                            .and_then(|g| g.get("home-only"))
                            .and_then(|v| bool::try_from(v).ok())
                            .unwrap_or(true);
                        let is_active = active_uuids.contains(&uuid);
                        if is_active {
                            state.data_roaming_enabled = !home_only;
                        }
                        state.apns.push(ApnProfile {
                            path: cpath.to_string(),
                            uuid,
                            name,
                            apn,
                            username,
                            password,
                            roaming_allowed: !home_only,
                            is_active,
                        });
                    }
                }
            }
        }

        state
    }

    pub async fn set_mobile_data_enabled(enabled: bool) -> Result<(), String> {
        let conn = Connection::system().await.map_err(|e| e.to_string())?;
        let nm_proxy = Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
        )
        .await
        .map_err(|e| e.to_string())?;
        nm_proxy
            .set_property("WwanEnabled", enabled)
            .await
            .map_err(|e| e.to_string())?;
        info!(enabled, "Updated NetworkManager WwanEnabled");
        Ok(())
    }

    pub async fn set_data_roaming(roaming: bool) -> Result<(), String> {
        let state = Self::fetch_state().await;
        let conn = Connection::system().await.map_err(|e| e.to_string())?;
        for apn in state.apns {
            let Ok(c_proxy) = Proxy::new(
                &conn,
                "org.freedesktop.NetworkManager",
                apn.path.as_str(),
                "org.freedesktop.NetworkManager.Settings.Connection",
            )
            .await
            else {
                continue;
            };
            if let Ok(mut cfg) = c_proxy
                .call::<_, _, HashMap<String, HashMap<String, OwnedValue>>>("GetSettings", &())
                .await
            {
                let gsm = cfg.entry("gsm".to_string()).or_default();
                gsm.insert("home-only".to_string(), OwnedValue::from(!roaming));
                let _ = c_proxy.call::<_, _, ()>("Update", &(cfg,)).await;
            }
        }
        info!(roaming, "Updated GSM home-only roaming setting");
        Ok(())
    }

    pub async fn unlock_sim(sim_path: &str, pin: &str) -> Result<(), String> {
        if sim_path.is_empty() {
            return Err("No SIM object path".to_string());
        }
        let conn = Connection::system().await.map_err(|e| e.to_string())?;
        let sim_proxy = Proxy::new(
            &conn,
            "org.freedesktop.ModemManager1",
            sim_path,
            "org.freedesktop.ModemManager1.Sim",
        )
        .await
        .map_err(|e| e.to_string())?;
        sim_proxy
            .call::<_, _, ()>("SendPin", &(pin,))
            .await
            .map_err(|e| format!("PIN unlock failed: {}", e))?;
        info!("SIM PIN unlocked successfully");
        Ok(())
    }

    pub async fn set_network_mode(modem_path: &str, mode: &str) -> Result<(), String> {
        if modem_path.is_empty() {
            return Err("No modem present".to_string());
        }
        let (allowed, preferred): (u32, u32) = match mode {
            "4g_only" => (8, 0),
            "4g_3g" => (8 | 4, 8),
            "3g_2g" => (4 | 2, 4),
            _ => (16 | 8 | 4 | 2, 0), // auto (5G/4G/3G/2G)
        };
        let conn = Connection::system().await.map_err(|e| e.to_string())?;
        let modem_proxy = Proxy::new(
            &conn,
            "org.freedesktop.ModemManager1",
            modem_path,
            "org.freedesktop.ModemManager1.Modem",
        )
        .await
        .map_err(|e| e.to_string())?;
        modem_proxy
            .call::<_, _, ()>("SetCurrentModes", &((allowed, preferred),))
            .await
            .map_err(|e| {
                warn!("SetCurrentModes failed: {}", e);
                format!("Failed to set network mode: {}", e)
            })?;
        info!(mode, "Updated ModemManager preferred network mode");
        Ok(())
    }

    pub async fn save_apn(
        existing_path: Option<String>,
        name: &str,
        apn: &str,
        username: &str,
        password: &str,
        roaming_allowed: bool,
    ) -> Result<(), String> {
        let conn = Connection::system().await.map_err(|e| e.to_string())?;
        if let Some(path) = existing_path.filter(|p| !p.is_empty()) {
            let c_proxy = Proxy::new(
                &conn,
                "org.freedesktop.NetworkManager",
                path.as_str(),
                "org.freedesktop.NetworkManager.Settings.Connection",
            )
            .await
            .map_err(|e| e.to_string())?;
            let mut cfg = c_proxy
                .call::<_, _, HashMap<String, HashMap<String, OwnedValue>>>("GetSettings", &())
                .await
                .map_err(|e| e.to_string())?;
            let conn_sec = cfg.entry("connection".to_string()).or_default();
            conn_sec.insert("id".to_string(), OwnedValue::try_from(Value::from(name)).unwrap());
            let gsm = cfg.entry("gsm".to_string()).or_default();
            gsm.insert("apn".to_string(), OwnedValue::try_from(Value::from(apn)).unwrap());
            if username.is_empty() {
                gsm.remove("username");
            } else {
                gsm.insert("username".to_string(), OwnedValue::try_from(Value::from(username)).unwrap());
            }
            if password.is_empty() {
                gsm.remove("password");
            } else {
                gsm.insert("password".to_string(), OwnedValue::try_from(Value::from(password)).unwrap());
            }
            gsm.insert("home-only".to_string(), OwnedValue::from(!roaming_allowed));
            c_proxy
                .call::<_, _, ()>("Update", &(cfg,))
                .await
                .map_err(|e| e.to_string())?;
            info!(name, apn, "Updated GSM APN connection");
        } else {
            let settings_proxy = Proxy::new(
                &conn,
                "org.freedesktop.NetworkManager",
                "/org/freedesktop/NetworkManager/Settings",
                "org.freedesktop.NetworkManager.Settings",
            )
            .await
            .map_err(|e| e.to_string())?;

            let mut conn_sec: HashMap<String, Value> = HashMap::new();
            conn_sec.insert("id".to_string(), Value::from(name));
            conn_sec.insert("type".to_string(), Value::from("gsm"));
            conn_sec.insert("autoconnect".to_string(), Value::from(true));

            let mut gsm_sec: HashMap<String, Value> = HashMap::new();
            gsm_sec.insert("apn".to_string(), Value::from(apn));
            gsm_sec.insert("home-only".to_string(), Value::from(!roaming_allowed));
            if !username.is_empty() {
                gsm_sec.insert("username".to_string(), Value::from(username));
            }
            if !password.is_empty() {
                gsm_sec.insert("password".to_string(), Value::from(password));
            }

            let mut new_cfg: HashMap<String, HashMap<String, Value>> = HashMap::new();
            new_cfg.insert("connection".to_string(), conn_sec);
            new_cfg.insert("gsm".to_string(), gsm_sec);

            settings_proxy
                .call::<_, _, OwnedObjectPath>("AddConnection", &(new_cfg,))
                .await
                .map_err(|e| e.to_string())?;
            info!(name, apn, "Created new GSM APN connection");
        }
        Ok(())
    }

    pub async fn activate_apn(conn_path: &str) -> Result<(), String> {
        let conn = Connection::system().await.map_err(|e| e.to_string())?;
        let nm_proxy = Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
        )
        .await
        .map_err(|e| e.to_string())?;

        // Find WWAN device (DeviceType == 8 -> MODEM)
        let devices = nm_proxy
            .call::<_, _, Vec<OwnedObjectPath>>("GetDevices", &())
            .await
            .map_err(|e| e.to_string())?;
        let mut modem_dev: Option<OwnedObjectPath> = None;
        for d in devices {
            if let Ok(dp) = Proxy::new(
                &conn,
                "org.freedesktop.NetworkManager",
                d.clone(),
                "org.freedesktop.NetworkManager.Device",
            )
            .await
            {
                if let Ok(dtype) = dp.get_property::<u32>("DeviceType").await {
                    if dtype == 8 {
                        modem_dev = Some(d);
                        break;
                    }
                }
            }
        }
        let dev = modem_dev.unwrap_or_else(|| OwnedObjectPath::try_from("/").unwrap());
        let root = OwnedObjectPath::try_from("/").unwrap();
        let cpath = OwnedObjectPath::try_from(conn_path).map_err(|e| e.to_string())?;
        nm_proxy
            .call::<_, _, OwnedObjectPath>("ActivateConnection", &(cpath, dev, root))
            .await
            .map_err(|e| e.to_string())?;
        info!(conn_path, "Activated GSM APN connection");
        Ok(())
    }

    pub async fn delete_apn(conn_path: &str) -> Result<(), String> {
        let conn = Connection::system().await.map_err(|e| e.to_string())?;
        let c_proxy = Proxy::new(
            &conn,
            "org.freedesktop.NetworkManager",
            conn_path,
            "org.freedesktop.NetworkManager.Settings.Connection",
        )
        .await
        .map_err(|e| e.to_string())?;
        c_proxy
            .call::<_, _, ()>("Delete", &())
            .await
            .map_err(|e| e.to_string())?;
        info!(conn_path, "Deleted GSM APN connection");
        Ok(())
    }
}

