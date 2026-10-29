//! Penumbra Compositor library.

pub mod args;
#[cfg(feature = "native-drm")]
pub mod native;
#[cfg(feature = "native-drm")]
mod native_input;
pub mod phone;
pub mod policy;
pub mod server;
pub mod wallpaper_blur;
pub mod card_rounding;
pub mod render;
pub mod outputs;

pub use args::CliArgs;
pub use policy::{AsymmetricOutputManager, OutputConfig, OutputRole, SurfaceDisplayMode};
pub use server::PenumbraCompositor;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phone_dsi_policy_invariants() {
        let mut manager = AsymmetricOutputManager::new();
        let config = manager.output_config(&OutputRole::PhoneDsi).unwrap();

        assert_eq!(config.width, 412);
        assert_eq!(config.height, 915);
        assert!(!config.allows_floating);
        assert!(config.enforces_gestures);

        let s1 = manager.register_surface(1, "phone.app", "Phone App");
        assert_eq!(s1.assigned_output, OutputRole::PhoneDsi);
        assert_eq!(s1.display_mode, SurfaceDisplayMode::FullscreenExclusive);
    }

    #[test]
    fn test_convergence_hotplug_and_migration() {
        let mut manager = AsymmetricOutputManager::new();
        manager.register_surface(1, "phone.app", "Phone App");

        // Hotplug external DP-1
        manager.hotplug_external_display();
        assert!(manager.is_convergence_active());

        // Second surface should route to DP-1 as floating window
        let s2 = manager.register_surface(2, "desktop.app", "Desktop App");
        assert_eq!(s2.assigned_output, OutputRole::external("DP-1"));
        assert!(matches!(
            s2.display_mode,
            SurfaceDisplayMode::FloatingWindow { .. }
        ));

        // Unplug external display
        manager.unplug_external_display();
        assert!(!manager.is_convergence_active());

        let s2_migrated = manager
            .surfaces_for_output(&OutputRole::PhoneDsi)
            .into_iter()
            .find(|s| s.id == 2)
            .unwrap();
        assert_eq!(
            s2_migrated.display_mode,
            SurfaceDisplayMode::FullscreenExclusive
        );
    }

    #[test]
    fn test_multi_external_display_hotplug_and_partial_disconnect() {
        let mut manager = AsymmetricOutputManager::new();
        manager.register_surface(1, "phone.app", "Phone App");

        // Hotplug two external displays: DP-1 and HDMI-A-1
        manager.hotplug_external("DP-1", 1920, 1080, 60_000);
        manager.hotplug_external("HDMI-A-1", 2560, 1440, 60_000);
        assert_eq!(manager.external_outputs().len(), 2);
        assert!(manager.is_convergence_active());

        // Surface routes to external desktop
        let s2 = manager.register_surface(2, "browser.desktop", "Web Browser");
        assert!(s2.assigned_output.is_external());

        // Unplug DP-1: HDMI-A-1 remains active! Convergence stays active.
        manager.unplug_external("DP-1");
        assert_eq!(manager.external_outputs().len(), 1);
        assert!(manager.is_convergence_active());

        // Surface s2 should now be on HDMI-A-1, still floating (not on phone)
        let s2_check = manager
            .surfaces_for_output(&OutputRole::external("HDMI-A-1"))
            .into_iter()
            .find(|s| s.id == 2)
            .unwrap();
        assert!(matches!(s2_check.display_mode, SurfaceDisplayMode::FloatingWindow { .. }));

        // Unplug remaining HDMI-A-1: total disconnect -> now migrates to phone fullscreen
        manager.unplug_external("HDMI-A-1");
        assert!(!manager.is_convergence_active());
        let s2_on_phone = manager
            .surfaces_for_output(&OutputRole::PhoneDsi)
            .into_iter()
            .find(|s| s.id == 2)
            .unwrap();
        assert_eq!(s2_on_phone.display_mode, SurfaceDisplayMode::FullscreenExclusive);
    }

    #[test]
    fn test_multi_output_layout_and_pointer_clamping() {
        use crate::outputs::layout::compute_default_multi_display_layout;
        use smithay::utils::Rectangle;

        let externals = vec![
            ("DP-1".to_string(), (1920, 1080).into()),
            ("HDMI-A-1".to_string(), (2560, 1440).into()),
        ];
        let layout = compute_default_multi_display_layout(&externals);
        assert_eq!(layout.len(), 3);
        assert_eq!(layout[0].id, "Phone-DSI-1");
        assert_eq!(layout[0].position, (0, 0).into());
        assert_eq!(layout[1].id, "DP-1");
        assert_eq!(layout[1].position, (412, 0).into());
        assert_eq!(layout[2].id, "HDMI-A-1");
        assert_eq!(layout[2].position, (2332, 0).into());

        let total_w = 412 + 1920 + 2560; // 4892
        let total_h = 1440;
        let bounds: Rectangle<i32, smithay::utils::Logical> =
            Rectangle::new((0, 0).into(), (total_w, total_h).into());

        assert_eq!(bounds.size.w, 4892);
        assert_eq!(bounds.size.h, 1440);

        let clamp = |x: f64, y: f64| -> (f64, f64) {
            let max_x = (bounds.loc.x + bounds.size.w - 1) as f64;
            let max_y = (bounds.loc.y + bounds.size.h - 1) as f64;
            (x.clamp(bounds.loc.x as f64, max_x), y.clamp(bounds.loc.y as f64, max_y))
        };

        // Pointer inside phone canvas
        assert_eq!(clamp(100.0, 500.0), (100.0, 500.0));
        // Pointer on first external monitor
        assert_eq!(clamp(1500.0, 700.0), (1500.0, 700.0));
        // Pointer on second external monitor
        assert_eq!(clamp(3000.0, 1200.0), (3000.0, 1200.0));
        // Pointer overflowing past right edge of 2nd monitor
        assert_eq!(clamp(5000.0, 700.0), (4891.0, 700.0));
        // Pointer overflowing past bottom edge
        assert_eq!(clamp(3000.0, 2000.0), (3000.0, 1439.0));
    }

    #[test]
    fn test_desktop_session_overview_badge_formatting() {
        let title_normal = "Firefox Web Browser";
        let title_migrated = format!("🖥 Desktop Session • {}", title_normal);
        assert!(title_migrated.starts_with("🖥 Desktop Session • "));
        assert_eq!(title_migrated, "🖥 Desktop Session • Firefox Web Browser");

        // Idempotence check
        let rebadged = if title_migrated.starts_with("🖥") {
            title_migrated.clone()
        } else {
            format!("🖥 Desktop Session • {}", title_migrated)
        };
        assert_eq!(rebadged, "🖥 Desktop Session • Firefox Web Browser");
    }
}
