//! COSMIC-inspired Server-Side Window Decorations (SSD) for convergent desktop mode.

use std::collections::HashMap;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::SolidColorRenderElement,
                Kind,
            },
            gles::GlesRenderer, Color32F,
        },
    },
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{Logical, Physical, Point, Rectangle, Size, Transform},
};

pub const SSD_TITLEBAR_H: i32 = 34;
pub const SSD_RESIZE_BORDER: f64 = 7.0;
pub const SSD_RESIZE_CORNER: f64 = 14.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitlebarAction {
    Drag,
    Minimize,
    Maximize,
    Close,
}

/// Hit-tests the SSD titlebar area `[win_x .. win_x + win_w, win_y - SSD_TITLEBAR_H .. win_y)`.
pub fn hit_test_titlebar(
    win_loc: (i32, i32),
    win_w: i32,
    pos: (f64, f64),
) -> Option<TitlebarAction> {
    if win_w <= 0 {
        return None;
    }
    let x0 = win_loc.0 as f64;
    let y0 = (win_loc.1 - SSD_TITLEBAR_H) as f64;
    let x1 = (win_loc.0 + win_w) as f64;
    let y1 = win_loc.1 as f64;
    if pos.0 < x0 || pos.0 >= x1 || pos.1 < y0 || pos.1 >= y1 {
        return None;
    }
    let rel_x = pos.0 - x0;
    let w = win_w as f64;
    if rel_x >= (w - 34.0).max(0.0) {
        Some(TitlebarAction::Close)
    } else if rel_x >= (w - 68.0).max(0.0) {
        Some(TitlebarAction::Maximize)
    } else if rel_x >= (w - 102.0).max(0.0) {
        Some(TitlebarAction::Minimize)
    } else {
        Some(TitlebarAction::Drag)
    }
}

/// Hit-tests the outer resize border/corners around a desktop window.
/// Includes the SSD titlebar top edge when `has_ssd` is true.
pub fn hit_test_resize_border(
    win_loc: (i32, i32),
    win_size: (i32, i32),
    has_ssd: bool,
    pos: (f64, f64),
) -> Option<xdg_toplevel::ResizeEdge> {
    if win_size.0 <= 0 || win_size.1 <= 0 {
        return None;
    }
    let top_pad = if has_ssd { SSD_TITLEBAR_H } else { 0 };
    let x0 = win_loc.0 as f64;
    let y0 = (win_loc.1 - top_pad) as f64;
    let x1 = (win_loc.0 + win_size.0) as f64;
    let y1 = (win_loc.1 + win_size.1) as f64;

    let b = SSD_RESIZE_BORDER;
    // Must be within outer [x0 - b .. x1 + b, y0 - b .. y1 + b]
    if pos.0 < x0 - b || pos.0 >= x1 + b || pos.1 < y0 - b || pos.1 >= y1 + b {
        return None;
    }

    let c = SSD_RESIZE_CORNER;
    let on_left = pos.0 < x0 + b;
    let on_right = pos.0 >= x1 - b;
    let on_top = pos.1 < y0 + b;
    let on_bottom = pos.1 >= y1 - b;

    if !(on_left || on_right || on_top || on_bottom) {
        return None;
    }

    // Expand corner hit zones along edges for easier diagonal grabbing
    let near_left = pos.0 < x0 + c;
    let near_right = pos.0 >= x1 - c;
    let near_top = pos.1 < y0 + c;
    let near_bottom = pos.1 >= y1 - c;

    if (on_top && near_left) || (on_left && near_top) {
        Some(xdg_toplevel::ResizeEdge::TopLeft)
    } else if (on_top && near_right) || (on_right && near_top) {
        Some(xdg_toplevel::ResizeEdge::TopRight)
    } else if (on_bottom && near_left) || (on_left && near_bottom) {
        Some(xdg_toplevel::ResizeEdge::BottomLeft)
    } else if (on_bottom && near_right) || (on_right && near_bottom) {
        Some(xdg_toplevel::ResizeEdge::BottomRight)
    } else if on_top {
        Some(xdg_toplevel::ResizeEdge::Top)
    } else if on_bottom {
        Some(xdg_toplevel::ResizeEdge::Bottom)
    } else if on_left {
        Some(xdg_toplevel::ResizeEdge::Left)
    } else {
        Some(xdg_toplevel::ResizeEdge::Right)
    }
}

#[derive(Hash, PartialEq, Eq, Clone)]
pub struct TitlebarCacheKey {
    pub width: i32,
    pub title: String,
    pub focused: bool,
}

/// Builds or retrieves a cached COSMIC-style titlebar `MemoryRenderBufferRenderElement`.
pub fn render_titlebar_element(
    renderer: &mut GlesRenderer,
    cache: &mut HashMap<TitlebarCacheKey, MemoryRenderBuffer>,
    width: i32,
    title: &str,
    focused: bool,
    ext_loc: (i32, i32),
) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
    let w = width.max(120);
    let key = TitlebarCacheKey {
        width: w,
        title: title.to_string(),
        focused,
    };

    if cache.len() > 64 {
        cache.clear();
    }
    let buffer = cache.entry(key).or_insert_with(|| {
        let rgba = rasterize_titlebar(w, SSD_TITLEBAR_H, title, focused);
        MemoryRenderBuffer::from_slice(
            &rgba,
            Fourcc::Abgr8888,
            (w, SSD_TITLEBAR_H),
            1,
            Transform::Normal,
            None,
        )
    });

    let loc = Point::<f64, Physical>::from((ext_loc.0 as f64, (ext_loc.1 - SSD_TITLEBAR_H) as f64));
    MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        loc,
        buffer,
        None,
        Some(Rectangle::<f64, Logical>::from_size((w as f64, SSD_TITLEBAR_H as f64).into())),
        Some(Size::<i32, Logical>::from((w, SSD_TITLEBAR_H))),
        Kind::Unspecified,
    )
    .ok()
}

/// Renders the left, right, and bottom 2px COSMIC container border elements around an SSD window
/// so the titlebar and window content form one continuous framed container.
pub fn render_window_borders(
    win_size: (i32, i32),
    focused: bool,
    ssd_loc: (i32, i32),
) -> [SolidColorRenderElement; 3] {
    let color = if focused {
        Color32F::new(0.388, 0.816, 0.875, 0.72) // #63d0df @ ~72%
    } else {
        Color32F::new(1.0, 1.0, 1.0, 0.14)
    };
    let b = 2;
    let (w, h) = (win_size.0.max(1), win_size.1.max(1));
    let left = SolidColorRenderElement::new(
        smithay::backend::renderer::element::Id::new(),
        Rectangle::new((ssd_loc.0, ssd_loc.1).into(), (b, h + b).into()),
        0,
        color,
        Kind::Unspecified,
    );
    let right = SolidColorRenderElement::new(
        smithay::backend::renderer::element::Id::new(),
        Rectangle::new((ssd_loc.0 + w - b, ssd_loc.1).into(), (b, h + b).into()),
        0,
        color,
        Kind::Unspecified,
    );
    let bottom = SolidColorRenderElement::new(
        smithay::backend::renderer::element::Id::new(),
        Rectangle::new((ssd_loc.0, ssd_loc.1 + h).into(), (w, b).into()),
        0,
        color,
        Kind::Unspecified,
    );
    [left, right, bottom]
}

/// Fills the top 16px behind an SSD window with the titlebar background color so apps that
/// draw their own rounded top corners (e.g. GTK4/libadwaita, Firefox) never leave triangular gaps.
pub fn render_window_corner_fill(
    win_w: i32,
    focused: bool,
    ssd_loc: (i32, i32),
) -> SolidColorRenderElement {
    let color = if focused {
        Color32F::new(0x22 as f32 / 255.0, 0x24 as f32 / 255.0, 0x2e as f32 / 255.0, 0xf8 as f32 / 255.0)
    } else {
        Color32F::new(0x16 as f32 / 255.0, 0x17 as f32 / 255.0, 0x1e as f32 / 255.0, 0xf0 as f32 / 255.0)
    };
    SolidColorRenderElement::new(
        smithay::backend::renderer::element::Id::new(),
        Rectangle::new((ssd_loc.0, ssd_loc.1).into(), (win_w.max(1), 16).into()),
        0,
        color,
        Kind::Unspecified,
    )
}

fn set_px(buf: &mut [u8], w: i32, h: i32, x: i32, y: i32, rgba: [u8; 4]) {
    if x < 0 || y < 0 || x >= w || y >= h {
        return;
    }
    let idx = ((y * w + x) * 4) as usize;
    buf[idx..idx + 4].copy_from_slice(&rgba);
}

fn blend_px(buf: &mut [u8], w: i32, h: i32, x: i32, y: i32, rgba: [u8; 4]) {
    if x < 0 || y < 0 || x >= w || y >= h {
        return;
    }
    let idx = ((y * w + x) * 4) as usize;
    let a = rgba[3] as u32;
    if a == 255 {
        buf[idx..idx + 4].copy_from_slice(&rgba);
        return;
    }
    let inv = 255 - a;
    for c in 0..3 {
        buf[idx + c] = ((rgba[c] as u32 * a + buf[idx + c] as u32 * inv) / 255) as u8;
    }
    buf[idx + 3] = (a + (buf[idx + 3] as u32 * inv) / 255).min(255) as u8;
}

fn rasterize_titlebar(w: i32, h: i32, title: &str, focused: bool) -> Vec<u8> {
    let mut buf = vec![0u8; (w * h * 4) as usize];
    // COSMIC-inspired dark slate header with subtle accent border when focused
    let bg = if focused { [0x22, 0x24, 0x2e, 0xf8] } else { [0x16, 0x17, 0x1e, 0xf0] };
    let border = if focused { [0x63, 0xd0, 0xdf, 0x88] } else { [0xff, 0xff, 0xff, 0x18] };
    let fg = if focused { [0xf4, 0xf5, 0xf8, 0xff] } else { [0x94, 0x98, 0xa8, 0xff] };
    let r = 10;

    for y in 0..h {
        for x in 0..w {
            // Rounded top-left and top-right corners
            let in_corner = if x < r && y < r {
                let dx = r - x;
                let dy = r - y;
                dx * dx + dy * dy > r * r
            } else if x >= w - r && y < r {
                let dx = x - (w - 1 - r);
                let dy = r - y;
                dx * dx + dy * dy > r * r
            } else {
                false
            };
            if in_corner {
                continue;
            }
            set_px(&mut buf, w, h, x, y, bg);
            // 2px COSMIC container rim on top, left, and right; bottom edge is open to merge seamlessly with window
            if y < 2 || x < 2 || x >= w - 2 {
                blend_px(&mut buf, w, h, x, y, border);
            }
        }
    }

    // Window title (5x7 bitmap font)
    let max_chars = ((w - 120).max(0) / 7) as usize;
    let mut cx = 14;
    let cy = (h - 7) / 2;
    for ch in title.chars().take(max_chars) {
        draw_char(&mut buf, w, h, cx, cy, ch, fg);
        cx += 7;
    }

    // Minimize button (center at w - 85)
    let min_x = w - 85;
    let mid_y = h / 2;
    for dx in -5..=5 {
        set_px(&mut buf, w, h, min_x + dx, mid_y + 2, fg);
    }

    // Maximize button (center at w - 51)
    let max_x = w - 51;
    for dx in -5..=5 {
        set_px(&mut buf, w, h, max_x + dx, mid_y - 5, fg);
        set_px(&mut buf, w, h, max_x + dx, mid_y + 5, fg);
    }
    for dy in -5..=5 {
        set_px(&mut buf, w, h, max_x - 5, mid_y + dy, fg);
        set_px(&mut buf, w, h, max_x + 5, mid_y + dy, fg);
    }

    // Close button (center at w - 17) with subtle circular pill
    let cls_x = w - 17;
    let pill = if focused { [0xd8, 0x4c, 0x5c, 0x55] } else { [0xff, 0xff, 0xff, 0x18] };
    for dy in -10..=10 {
        for dx in -10..=10 {
            if dx * dx + dy * dy <= 100 {
                blend_px(&mut buf, w, h, cls_x + dx, mid_y + dy, pill);
            }
        }
    }
    for d in -4..=4 {
        set_px(&mut buf, w, h, cls_x + d, mid_y + d, fg);
        set_px(&mut buf, w, h, cls_x + d, mid_y - d, fg);
    }

    buf
}

fn draw_char(buf: &mut [u8], w: i32, h: i32, x0: i32, y0: i32, ch: char, color: [u8; 4]) {
    let glyph = glyph_5x7(ch.to_ascii_uppercase());
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..5 {
            if (bits >> (4 - col)) & 1 == 1 {
                set_px(buf, w, h, x0 + col, y0 + row as i32, color);
            }
        }
    }
}

fn glyph_5x7(ch: char) -> [u8; 7] {
    match ch {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F],
        '3' => [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        '/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        '~' => [0x00, 0x00, 0x0A, 0x15, 0x00, 0x00, 0x00],
        ' ' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        _ => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    }
}
