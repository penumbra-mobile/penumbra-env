//! Multi-display coordinate space layout engine.
//!
//! Arranges the primary mobile phone screen and an arbitrary number of external desktop
//! displays within the Smithay global 2D coordinate space.

use smithay::utils::{Point, Size, Logical};

/// Mobile phone logical canvas width (standard Penumbra geometry).
pub const PHONE_CANVAS_WIDTH: i32 = 412;
/// Mobile phone logical canvas height.
pub const PHONE_CANVAS_HEIGHT: i32 = 915;

/// Descriptor of an active physical or virtual display for spatial layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutDisplay {
    pub id: String,
    pub is_phone: bool,
    pub size: Size<i32, Logical>,
    pub position: Point<i32, Logical>,
}

/// Computes the default side-by-side layout for the phone display and N external monitors.
///
/// Layout rules:
/// - The phone display is anchored at `(0, 0)` with logical dimensions `(412, 915)`.
/// - External monitor 1 is placed immediately to the right at `(412, 0)`.
/// - Additional external monitors (2..N) are placed horizontally adjacent to the previous monitor:
///   `X_k = 412 + sum(W_prev)`.
pub fn compute_default_multi_display_layout(
    external_displays: &[(String, Size<i32, Logical>)],
) -> Vec<LayoutDisplay> {
    let mut result = Vec::with_capacity(1 + external_displays.len());

    // 1. Primary phone canvas always at origin (0, 0)
    result.push(LayoutDisplay {
        id: "Phone-DSI-1".to_string(),
        is_phone: true,
        size: (PHONE_CANVAS_WIDTH, PHONE_CANVAS_HEIGHT).into(),
        position: (0, 0).into(),
    });

    // 2. Arrange N external desktop displays horizontally to the right
    let mut current_x = PHONE_CANVAS_WIDTH;
    for (id, size) in external_displays {
        result.push(LayoutDisplay {
            id: id.clone(),
            is_phone: false,
            size: *size,
            position: (current_x, 0).into(),
        });
        current_x += size.w;
    }

    result
}

/// Finds the target display that contains a given logical coordinate `(x, y)`.
pub fn find_display_at_point<'a>(
    displays: &'a [LayoutDisplay],
    point: Point<f64, Logical>,
) -> Option<&'a LayoutDisplay> {
    displays.iter().find(|d| {
        let x = point.x as i32;
        let y = point.y as i32;
        x >= d.position.x
            && x < d.position.x + d.size.w
            && y >= d.position.y
            && y < d.position.y + d.size.h
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_external_monitor_layout() {
        let externals = vec![("DP-1".to_string(), (1920, 1080).into())];
        let layout = compute_default_multi_display_layout(&externals);

        assert_eq!(layout.len(), 2);
        // Phone
        assert_eq!(layout[0].position, (0, 0).into());
        assert_eq!(layout[0].size, (412, 915).into());
        // External 1
        assert_eq!(layout[1].id, "DP-1");
        assert_eq!(layout[1].position, (412, 0).into());
        assert_eq!(layout[1].size, (1920, 1080).into());
    }

    #[test]
    fn test_multiple_external_monitors_layout() {
        let externals = vec![
            ("DP-1".to_string(), (1920, 1080).into()),
            ("HDMI-A-1".to_string(), (2560, 1440).into()),
            ("DP-2".to_string(), (1920, 1200).into()),
        ];
        let layout = compute_default_multi_display_layout(&externals);

        assert_eq!(layout.len(), 4);
        // Phone
        assert_eq!(layout[0].position, (0, 0).into());
        // External 1: at 412
        assert_eq!(layout[1].id, "DP-1");
        assert_eq!(layout[1].position, (412, 0).into());
        // External 2: at 412 + 1920 = 2332
        assert_eq!(layout[2].id, "HDMI-A-1");
        assert_eq!(layout[2].position, (2332, 0).into());
        // External 3: at 2332 + 2560 = 4892
        assert_eq!(layout[3].id, "DP-2");
        assert_eq!(layout[3].position, (4892, 0).into());

        // Test coordinate hit testing
        assert_eq!(
            find_display_at_point(&layout, (200.0, 500.0).into()).map(|d| d.id.as_str()),
            Some("Phone-DSI-1")
        );
        assert_eq!(
            find_display_at_point(&layout, (1000.0, 500.0).into()).map(|d| d.id.as_str()),
            Some("DP-1")
        );
        assert_eq!(
            find_display_at_point(&layout, (3000.0, 500.0).into()).map(|d| d.id.as_str()),
            Some("HDMI-A-1")
        );
        assert_eq!(
            find_display_at_point(&layout, (5000.0, 500.0).into()).map(|d| d.id.as_str()),
            Some("DP-2")
        );
    }
}
