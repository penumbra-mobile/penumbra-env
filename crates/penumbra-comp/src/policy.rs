//! Multi-Output Policies: Asymmetric routing for mobile phone (DSI) and arbitrary external desktop monitors.

use std::collections::HashMap;

use crate::phone::{PHONE_H, PHONE_W};

/// Physical output role in the Penumbra convergence model.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OutputRole {
    /// Output A: Primary Phone Display (DSI, 412x915 logical canvas @ 20:9).
    /// Enforces strict single-window fullscreen layout, gesture interception, and layer-shell overlay.
    PhoneDsi,
    /// Output B..N: Secondary External Desktop Displays (DP-1, HDMI-A-1, etc.).
    /// Enforces desktop convergence policy with floating windows, desktop dock, and mouse pointer.
    ExternalDesktop(String),
}

impl OutputRole {
    pub fn external(name: impl Into<String>) -> Self {
        Self::ExternalDesktop(name.into())
    }

    pub fn is_phone(&self) -> bool {
        matches!(self, Self::PhoneDsi)
    }

    pub fn is_external(&self) -> bool {
        matches!(self, Self::ExternalDesktop(_))
    }

    pub fn name(&self) -> &str {
        match self {
            Self::PhoneDsi => "DSI-1",
            Self::ExternalDesktop(ref name) => name.as_str(),
        }
    }
}

/// Surface display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceDisplayMode {
    FullscreenExclusive,
    FloatingWindow {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    LayerShellOverlay,
}

/// Output display configuration and enforced constraints.
#[derive(Debug, Clone)]
pub struct OutputConfig {
    pub role: OutputRole,
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub refresh_rate_mhz: u32,
    pub scale: f64,
    pub allows_floating: bool,
    pub enforces_gestures: bool,
    pub pointer_cursor_enabled: bool,
}

impl OutputConfig {
    /// Mobile Phone DSI output policy: 412x915 logical canvas @ 120Hz.
    pub fn phone_dsi() -> Self {
        Self {
            role: OutputRole::PhoneDsi,
            name: "DSI-1 (Pixel Mobile)".to_string(),
            width: PHONE_W,
            height: PHONE_H,
            refresh_rate_mhz: 120_000,
            scale: 1.0,
            allows_floating: false,
            enforces_gestures: true,
            pointer_cursor_enabled: false,
        }
    }

    /// External Desktop output policy for a given connector name and mode.
    pub fn external(name: &str, width: i32, height: i32, refresh_rate_mhz: u32) -> Self {
        Self {
            role: OutputRole::ExternalDesktop(name.to_string()),
            name: format!("{name} (Convergence Desktop)"),
            width,
            height,
            refresh_rate_mhz,
            scale: 1.0,
            allows_floating: true,
            enforces_gestures: false,
            pointer_cursor_enabled: true,
        }
    }

    /// Default external monitor policy (DP-1 at 1920x1080 @ 60Hz).
    pub fn external_default() -> Self {
        Self::external("DP-1", 1920, 1080, 60_000)
    }
}

/// Surface metadata tracked by the compositor policy manager.
#[derive(Debug, Clone)]
pub struct ManagedSurface {
    pub id: u64,
    pub app_id: String,
    pub title: String,
    pub assigned_output: OutputRole,
    pub display_mode: SurfaceDisplayMode,
    pub is_focused: bool,
}

/// Asymmetric output router managing convergence, hotplug, and surface layouts across N displays.
#[derive(Debug, Default)]
pub struct AsymmetricOutputManager {
    outputs: HashMap<OutputRole, OutputConfig>,
    surfaces: Vec<ManagedSurface>,
    active_dsi_surface_id: Option<u64>,
}

impl AsymmetricOutputManager {
    pub fn new() -> Self {
        let mut manager = Self::default();
        // Phone DSI is always present as the primary mobile display
        manager
            .outputs
            .insert(OutputRole::PhoneDsi, OutputConfig::phone_dsi());
        manager
    }

    /// Hotplugs an external monitor with the specified connector name and resolution.
    pub fn hotplug_external(
        &mut self,
        name: &str,
        width: i32,
        height: i32,
        refresh_rate_mhz: u32,
    ) -> &OutputConfig {
        let role = OutputRole::ExternalDesktop(name.to_string());
        self.outputs
            .entry(role.clone())
            .or_insert_with(|| OutputConfig::external(name, width, height, refresh_rate_mhz))
    }

    /// Hotplugs a default external monitor ("DP-1").
    pub fn hotplug_external_display(&mut self) -> &OutputConfig {
        self.hotplug_external("DP-1", 1920, 1080, 60_000)
    }

    /// Unplugs an external monitor by name.
    ///
    /// - If other external desktop monitors remain connected, surfaces on this unplugged
    ///   display migrate to the first remaining external monitor (retaining floating mode).
    /// - If all external monitors are disconnected, surfaces migrate to the phone DSI in fullscreen mode.
    pub fn unplug_external(&mut self, name: &str) {
        let role = OutputRole::ExternalDesktop(name.to_string());
        self.outputs.remove(&role);

        // Find remaining external desktop output if any
        let remaining_desktop = self.outputs.keys().find(|r| r.is_external()).cloned();

        for surface in &mut self.surfaces {
            if surface.assigned_output == role {
                if let Some(ref target) = remaining_desktop {
                    // Migrate to remaining desktop monitor
                    surface.assigned_output = target.clone();
                } else {
                    // Total disconnect: migrate to phone
                    surface.assigned_output = OutputRole::PhoneDsi;
                    surface.display_mode = SurfaceDisplayMode::FullscreenExclusive;
                }
            }
        }
    }

    /// Unplugs default external monitor.
    pub fn unplug_external_display(&mut self) {
        self.unplug_external("DP-1");
    }

    /// Checks whether any external desktop monitor is currently connected.
    pub fn is_convergence_active(&self) -> bool {
        self.outputs.keys().any(|role| role.is_external())
    }

    /// Returns all currently active external desktop outputs.
    pub fn external_outputs(&self) -> Vec<&OutputConfig> {
        self.outputs
            .iter()
            .filter(|(r, _)| r.is_external())
            .map(|(_, cfg)| cfg)
            .collect()
    }

    /// Returns configuration for an output role if active.
    pub fn output_config(&self, role: &OutputRole) -> Option<&OutputConfig> {
        self.outputs.get(role)
    }

    /// Registers a new surface and computes its layout according to output policy.
    pub fn register_surface(&mut self, id: u64, app_id: &str, title: &str) -> &ManagedSurface {
        // If external desktop is connected and app is desktop-oriented (or second surface), route to external
        let (assigned_output, display_mode) = if self.is_convergence_active()
            && self.active_dsi_surface_id.is_some()
        {
            let target_output = self
                .outputs
                .keys()
                .find(|r| r.is_external())
                .cloned()
                .unwrap_or(OutputRole::PhoneDsi);

            (
                target_output,
                SurfaceDisplayMode::FloatingWindow {
                    x: 100 + ((id as i32 * 40) % 600),
                    y: 80 + ((id as i32 * 30) % 400),
                    width: 900,
                    height: 600,
                },
            )
        } else {
            // Strict single-window fullscreen layout for mobile phone DSI
            self.active_dsi_surface_id = Some(id);
            (
                OutputRole::PhoneDsi,
                SurfaceDisplayMode::FullscreenExclusive,
            )
        };

        let surface = ManagedSurface {
            id,
            app_id: app_id.to_string(),
            title: title.to_string(),
            assigned_output,
            display_mode,
            is_focused: true,
        };

        self.surfaces.push(surface);
        self.surfaces.last().unwrap()
    }

    /// Switches the active fullscreen app on Phone DSI.
    pub fn switch_active_dsi_surface(&mut self, surface_id: u64) -> bool {
        if self
            .surfaces
            .iter()
            .any(|s| s.id == surface_id && s.assigned_output == OutputRole::PhoneDsi)
        {
            self.active_dsi_surface_id = Some(surface_id);
            for s in &mut self.surfaces {
                if s.assigned_output == OutputRole::PhoneDsi {
                    s.is_focused = s.id == surface_id;
                }
            }
            true
        } else {
            false
        }
    }

    /// Returns surfaces currently routed to the specified output role.
    pub fn surfaces_for_output(&self, role: &OutputRole) -> Vec<&ManagedSurface> {
        self.surfaces
            .iter()
            .filter(|s| &s.assigned_output == role)
            .collect()
    }
}
