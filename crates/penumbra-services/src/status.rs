//! Phosh-inspired system D-Bus status indicators (ModemManager WWAN, BlueZ Bluetooth,
//! NetworkManager VPN, and DRM external display docked state).

use std::collections::HashMap;
use zbus::{zvariant::OwnedObjectPath, zvariant::OwnedValue, Connection, Proxy};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MobileStatusIndicators {
    pub wwan_present: bool,
    pub wwan_signal: u32,
    pub wwan_tech: String,
    pub bt_powered: bool,
    pub bt_connected: bool,
    pub vpn_active: bool,
    pub docked: bool,
}

fn access_tech_label(mask: u32) -> &'static str {
    // MMModemAccessTechnology bitflags from ModemManager
    if mask & (1 << 15) != 0 {
        "5G"
    } else if mask & (1 << 14) != 0 {
        "4G"
    } else if mask & ((1 << 5) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9)) != 0 {
        "3G"
    } else if mask & ((1 << 1) | (1 << 2) | (1 << 3) | (1 << 4)) != 0 {
        "2G"
    } else {
        ""
    }
}

fn is_external_drm_connected() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.contains("-DP-") || name.contains("-HDMI-") {
            if let Ok(status) = std::fs::read_to_string(entry.path().join("status")) {
                if status.trim() == "connected" {
                    return true;
                }
            }
        }
    }
    false
}

/// Polls system D-Bus and DRM sysfs for Phosh-style top bar indicators.
pub async fn poll_mobile_status() -> MobileStatusIndicators {
    let mut out = MobileStatusIndicators {
        docked: is_external_drm_connected(),
        ..Default::default()
    };

    let Ok(conn) = Connection::system().await else {
        return out;
    };

    // 1. ModemManager1 (WWAN signal & access technology)
    if let Ok(mm_proxy) = Proxy::new(
        &conn,
        "org.freedesktop.ModemManager1",
        "/org/freedesktop/ModemManager1",
        "org.freedesktop.DBus.ObjectManager",
    )
    .await
    {
        match mm_proxy
            .call::<_, _, HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>>(
                "GetManagedObjects",
                &(),
            )
            .await
        {
            Ok(objects) => {
                for (_path, ifaces) in objects {
                    if let Some(modem) = ifaces.get("org.freedesktop.ModemManager1.Modem") {
                        out.wwan_present = true;
                        if let Some(sq) = modem.get("SignalQuality") {
                            if let Ok((pct, _recent)) = <(u32, bool)>::try_from(sq.clone()) {
                                out.wwan_signal = pct.min(100);
                            }
                        }
                        if let Some(tech) = modem.get("AccessTechnologies") {
                            if let Ok(mask) = u32::try_from(tech) {
                                out.wwan_tech = access_tech_label(mask).to_string();
                            }
                        }
                        break;
                    }
                }
            }
            Err(e) => tracing::warn!("ModemManager1 GetManagedObjects failed: {e}"),
        }
    }

    // 2. BlueZ (Adapter powered + connected device)
    if let Ok(bz_proxy) = Proxy::new(
        &conn,
        "org.bluez",
        "/",
        "org.freedesktop.DBus.ObjectManager",
    )
    .await
    {
        match bz_proxy
            .call::<_, _, HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>>(
                "GetManagedObjects",
                &(),
            )
            .await
        {
            Ok(objects) => {
                for (_path, ifaces) in objects {
                    if let Some(adapter) = ifaces.get("org.bluez.Adapter1") {
                        if let Some(val) = adapter.get("Powered") {
                            if let Ok(true) = bool::try_from(val) {
                                out.bt_powered = true;
                            }
                        }
                    }
                    if let Some(dev) = ifaces.get("org.bluez.Device1") {
                        if let Some(val) = dev.get("Connected") {
                            if let Ok(true) = bool::try_from(val) {
                                out.bt_connected = true;
                            }
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("BlueZ GetManagedObjects failed: {e}"),
        }
    }

    // 3. NetworkManager active VPN connections
    if let Ok(nm_proxy) = Proxy::new(
        &conn,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .await
    {
        if let Ok(conns) = nm_proxy
            .get_property::<Vec<OwnedObjectPath>>("ActiveConnections")
            .await
        {
            for path in conns {
                if let Ok(ac_proxy) = Proxy::new(
                    &conn,
                    "org.freedesktop.NetworkManager",
                    path,
                    "org.freedesktop.NetworkManager.Connection.Active",
                )
                .await
                {
                    if let Ok(true) = ac_proxy.get_property::<bool>("Vpn").await {
                        out.vpn_active = true;
                        break;
                    }
                }
            }
        }
    }

    out
}

fn find_bluez_adapter(
    conn: &zbus::blocking::Connection,
) -> Option<(OwnedObjectPath, HashMap<String, OwnedValue>)> {
    let om = zbus::blocking::Proxy::new(
        conn,
        "org.bluez",
        "/",
        "org.freedesktop.DBus.ObjectManager",
    )
    .ok()?;
    let objects: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> =
        om.call("GetManagedObjects", &()).ok()?;
    for (path, mut ifaces) in objects {
        if let Some(adapter) = ifaces.remove("org.bluez.Adapter1") {
            return Some((path, adapter));
        }
    }
    None
}

/// Returns whether any BlueZ adapter (`org.bluez.Adapter1`) is currently powered on.
pub fn is_bluetooth_powered() -> bool {
    let Ok(conn) = zbus::blocking::Connection::system() else {
        return false;
    };
    let Some((_path, adapter)) = find_bluez_adapter(&conn) else {
        return false;
    };
    adapter
        .get("Powered")
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}

/// Sets `org.bluez.Adapter1.Powered` directly via D-Bus (avoids `bluetoothctl` async exit race).
pub fn set_bluetooth_powered(powered: bool) -> bool {
    let Ok(conn) = zbus::blocking::Connection::system() else {
        return false;
    };
    let Some((path, _adapter)) = find_bluez_adapter(&conn) else {
        return false;
    };
    let Ok(props) = zbus::blocking::Proxy::new(
        &conn,
        "org.bluez",
        path.as_str(),
        "org.freedesktop.DBus.Properties",
    ) else {
        return false;
    };
    props
        .call::<_, _, ()>(
            "Set",
            &("org.bluez.Adapter1", "Powered", zbus::zvariant::Value::from(powered)),
        )
        .is_ok()
}

/// Reads the primary BlueZ adapter's hardware MAC address (`org.bluez.Adapter1.Address`).
pub fn read_bluetooth_adapter_mac() -> Option<String> {
    let conn = zbus::blocking::Connection::system().ok()?;
    let (_path, adapter) = find_bluez_adapter(&conn)?;
    let addr = adapter.get("Address")?;
    let s = <&str>::try_from(addr).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_uppercase())
    }
}

