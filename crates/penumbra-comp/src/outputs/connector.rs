//! DRM connector inspection, EDID extraction, and CRTC matching for multi-monitor convergence.

use smithay::{
    backend::drm::DrmDeviceFd,
    output::{Mode, PhysicalProperties, Subpixel},
    reexports::drm::control::{
        connector, crtc, Device as DrmControlDevice, ResourceHandles,
    },
};

/// Information about a discovered DRM connector.
#[derive(Debug, Clone)]
pub struct DiscoveredConnector {
    pub handle: connector::Handle,
    pub name: String,
    pub is_internal: bool,
    pub is_connected: bool,
    pub preferred_mode: Option<Mode>,
    pub physical: PhysicalProperties,
}

/// Identifies whether a DRM connector is typically an internal mobile/embedded panel.
pub fn is_internal_connector(interface: connector::Interface) -> bool {
    matches!(
        interface,
        connector::Interface::DSI | connector::Interface::EmbeddedDisplayPort | connector::Interface::DPI
    )
}

/// Returns a friendly connector name like `DSI-1` or `DP-1`.
pub fn connector_name(info: &connector::Info) -> String {
    format!("{}-{}", info.interface().as_str(), info.interface_id())
}

/// Discovers all DRM connectors and their connection states on a DRM device.
pub fn enumerate_connectors(
    drm_fd: &DrmDeviceFd,
    resources: &ResourceHandles,
) -> Vec<DiscoveredConnector> {
    let mut discovered = Vec::new();

    for &handle in resources.connectors() {
        let Ok(info) = drm_fd.get_connector(handle, true) else {
            continue;
        };

        let name = connector_name(&info);
        let is_internal = is_internal_connector(info.interface());
        let is_connected = info.state() == connector::State::Connected && !info.modes().is_empty();

        let preferred_mode = if is_connected {
            // Find preferred mode or highest resolution mode
            info.modes()
                .iter()
                .find(|m| m.mode_type().contains(smithay::reexports::drm::control::ModeTypeFlags::PREFERRED))
                .or_else(|| info.modes().iter().max_by_key(|m| m.size().0 as u32 * m.size().1 as u32))
                .map(|m| Mode {
                    size: (m.size().0 as i32, m.size().1 as i32).into(),
                    refresh: (m.vrefresh() as i32) * 1000,
                })
        } else {
            None
        };

        let (phys_w, phys_h) = info.size().unwrap_or((0, 0));
        let physical = PhysicalProperties {
            size: (phys_w as i32, phys_h as i32).into(),
            subpixel: Subpixel::Unknown,
            make: if is_internal {
                "Penumbra".to_string()
            } else {
                "External Display".to_string()
            },
            model: name.clone(),
        };

        discovered.push(DiscoveredConnector {
            handle,
            name,
            is_internal,
            is_connected,
            preferred_mode,
            physical,
        });
    }

    discovered
}

/// Finds a compatible CRTC for a connector that is not already occupied by another active output.
pub fn find_compatible_crtc(
    drm_fd: &DrmDeviceFd,
    connector_info: &connector::Info,
    resources: &ResourceHandles,
    occupied_crtcs: &[crtc::Handle],
) -> Option<crtc::Handle> {
    let compatible_crtcs = connector_info
        .encoders()
        .iter()
        .filter_map(|encoder_handle| drm_fd.get_encoder(*encoder_handle).ok())
        .flat_map(|encoder_info| resources.filter_crtcs(encoder_info.possible_crtcs()))
        .collect::<Vec<_>>();

    // 1. Try currently assigned CRTC if not occupied
    let current_crtc = connector_info
        .current_encoder()
        .and_then(|encoder_handle| drm_fd.get_encoder(encoder_handle).ok())
        .and_then(|encoder_info| encoder_info.crtc())
        .filter(|crtc_handle| compatible_crtcs.contains(crtc_handle) && !occupied_crtcs.contains(crtc_handle));

    if let Some(crtc) = current_crtc {
        return Some(crtc);
    }

    // 2. Try an idle compatible CRTC
    let idle_crtc = compatible_crtcs.iter().copied().find(|crtc_handle| {
        !occupied_crtcs.contains(crtc_handle)
            && drm_fd
                .get_crtc(*crtc_handle)
                .map(|info| info.mode().is_none())
                .unwrap_or(false)
    });

    if let Some(crtc) = idle_crtc {
        return Some(crtc);
    }

    // 3. Fallback to any compatible CRTC not in occupied list
    compatible_crtcs
        .into_iter()
        .find(|crtc_handle| !occupied_crtcs.contains(crtc_handle))
}

/// Matches connected connectors to CRTCs using cosmic-comp's display configuration algorithm.
pub fn display_configuration(
    drm_fd: &DrmDeviceFd,
    resources: &ResourceHandles,
) -> std::collections::HashMap<connector::Handle, Option<crtc::Handle>> {
    let mut map = std::collections::HashMap::new();

    // 1. Keep established mappings from current encoders to reduce flicker
    for &conn_handle in resources.connectors() {
        if let Ok(conn) = drm_fd.get_connector(conn_handle, true) {
            if let Some(enc_handle) = conn.current_encoder() {
                if let Ok(enc) = drm_fd.get_encoder(enc_handle) {
                    if let Some(crtc) = enc.crtc() {
                        if conn.state() == connector::State::Connected {
                            map.insert(conn_handle, Some(crtc));
                        }
                    }
                }
            }
        }
    }

    // 2. Match all remaining connected connectors to available CRTCs
    for &conn_handle in resources.connectors() {
        if map.contains_key(&conn_handle) {
            continue;
        }
        if let Ok(conn) = drm_fd.get_connector(conn_handle, false) {
            if conn.state() == connector::State::Connected {
                let mut matched_crtc = None;
                'outer: for &enc_handle in conn.encoders() {
                    if let Ok(enc) = drm_fd.get_encoder(enc_handle) {
                        for crtc in resources.filter_crtcs(enc.possible_crtcs()) {
                            if !map.values().any(|v| *v == Some(crtc)) {
                                matched_crtc = Some(crtc);
                                break 'outer;
                            }
                        }
                    }
                }
                map.insert(conn_handle, matched_crtc);
            }
        }
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_internal_connector_identification() {
        assert!(is_internal_connector(connector::Interface::DSI));
        assert!(is_internal_connector(connector::Interface::EmbeddedDisplayPort));
        assert!(is_internal_connector(connector::Interface::DPI));

        assert!(!is_internal_connector(connector::Interface::HDMIA));
        assert!(!is_internal_connector(connector::Interface::DisplayPort));
        assert!(!is_internal_connector(connector::Interface::VGA));
    }
}

