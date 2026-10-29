//! Multi-output management and dynamic hotplug engine for Penumbra Compositor.

#[cfg(feature = "native-drm")]
pub mod connector;
pub mod layout;

#[cfg(feature = "native-drm")]
pub use connector::{
    display_configuration, enumerate_connectors, find_compatible_crtc, is_internal_connector,
    DiscoveredConnector,
};
pub use layout::{
    compute_default_multi_display_layout, find_display_at_point, LayoutDisplay, PHONE_CANVAS_HEIGHT,
    PHONE_CANVAS_WIDTH,
};
