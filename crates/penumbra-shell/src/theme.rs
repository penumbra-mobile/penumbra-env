//! Theme helper functions and Slint color bridge for Material 3 Expressive.

use penumbra_services::MonetPalette;

/// Parses a hex color string (`#RRGGBB` or `#RRGGBBAA`) into a `slint::Color`.
pub fn hex_to_slint_color(hex: &str) -> slint::Color {
    let clean = hex.trim_start_matches('#');
    if clean.len() == 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).unwrap_or(0);
        let g = u8::from_str_radix(&clean[2..4], 16).unwrap_or(0);
        let b = u8::from_str_radix(&clean[4..6], 16).unwrap_or(0);
        slint::Color::from_argb_u8(255, r, g, b)
    } else if clean.len() == 8 {
        let r = u8::from_str_radix(&clean[0..2], 16).unwrap_or(0);
        let g = u8::from_str_radix(&clean[2..4], 16).unwrap_or(0);
        let b = u8::from_str_radix(&clean[4..6], 16).unwrap_or(0);
        let a = u8::from_str_radix(&clean[6..8], 16).unwrap_or(255);
        slint::Color::from_argb_u8(a, r, g, b)
    } else {
        slint::Color::from_argb_u8(255, 0, 0, 0)
    }
}

/// Applies a MonetPalette to a Slint `Theme` global instance.
#[macro_export]
macro_rules! apply_monet_palette {
    ($theme:expr, $palette:expr) => {{
        let p = $palette;
        let t = $theme;
        t.set_primary($crate::theme::hex_to_slint_color(&p.primary));
        t.set_on_primary($crate::theme::hex_to_slint_color(&p.on_primary));
        t.set_primary_container($crate::theme::hex_to_slint_color(&p.primary_container));
        t.set_on_primary_container($crate::theme::hex_to_slint_color(&p.on_primary_container));
        t.set_secondary($crate::theme::hex_to_slint_color(&p.secondary));
        t.set_on_secondary($crate::theme::hex_to_slint_color(&p.on_secondary));
        t.set_secondary_container($crate::theme::hex_to_slint_color(&p.secondary_container));
        t.set_on_secondary_container($crate::theme::hex_to_slint_color(&p.on_secondary_container));
        t.set_tertiary($crate::theme::hex_to_slint_color(&p.tertiary));
        t.set_on_tertiary($crate::theme::hex_to_slint_color(&p.on_tertiary));
        t.set_tertiary_container($crate::theme::hex_to_slint_color(&p.tertiary_container));
        t.set_on_tertiary_container($crate::theme::hex_to_slint_color(&p.on_tertiary_container));
        t.set_surface($crate::theme::hex_to_slint_color(&p.surface));
        t.set_surface_dim($crate::theme::hex_to_slint_color(&p.surface_dim));
        t.set_surface_bright($crate::theme::hex_to_slint_color(&p.surface_bright));
        t.set_surface_container_lowest($crate::theme::hex_to_slint_color(&p.surface_container_lowest));
        t.set_surface_container_low($crate::theme::hex_to_slint_color(&p.surface_container_low));
        t.set_surface_container($crate::theme::hex_to_slint_color(&p.surface_container));
        t.set_surface_container_high($crate::theme::hex_to_slint_color(&p.surface_container_high));
        t.set_surface_container_highest($crate::theme::hex_to_slint_color(&p.surface_container_highest));
        t.set_surface_variant($crate::theme::hex_to_slint_color(&p.surface_variant));
        t.set_on_surface($crate::theme::hex_to_slint_color(&p.on_surface));
        t.set_on_surface_variant($crate::theme::hex_to_slint_color(&p.on_surface_variant));
        t.set_outline($crate::theme::hex_to_slint_color(&p.outline));
        t.set_outline_variant($crate::theme::hex_to_slint_color(&p.outline_variant));
    }};
}

/// Helper to load MonetPalette from the IPC path or fallback to default.
pub fn load_current_theme() -> MonetPalette {
    let path = MonetPalette::default_ipc_path();
    MonetPalette::load_from_ipc_path(&path).unwrap_or_else(|_| MonetPalette::default_dark())
}

/// Spawns a background timer to watch for changes to the Monet IPC theme file and apply them.
pub fn start_theme_watcher<F: Fn(MonetPalette) + 'static>(apply_fn: F) -> slint::Timer {
    let timer = slint::Timer::default();
    let theme_path = MonetPalette::default_ipc_path();
    let last_mod = std::rc::Rc::new(std::cell::RefCell::new(None));

    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(500),
        move || {
            if let Ok(meta) = std::fs::metadata(&theme_path) {
                if let Ok(mod_time) = meta.modified() {
                    let mut last = last_mod.borrow_mut();
                    if last.as_ref() != Some(&mod_time) {
                        *last = Some(mod_time);
                        if let Ok(palette) = MonetPalette::load_from_ipc_path(&theme_path) {
                            apply_fn(palette);
                        }
                    }
                }
            }
        },
    );

    timer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_to_slint_color() {
        let col = hex_to_slint_color("#ADC6FF");
        assert_eq!(col.red(), 173);
        assert_eq!(col.green(), 198);
        assert_eq!(col.blue(), 255);
        assert_eq!(col.alpha(), 255);

        let col_alpha = hex_to_slint_color("#14121880");
        assert_eq!(col_alpha.red(), 20);
        assert_eq!(col_alpha.green(), 18);
        assert_eq!(col_alpha.blue(), 24);
        assert_eq!(col_alpha.alpha(), 128);
    }
}
