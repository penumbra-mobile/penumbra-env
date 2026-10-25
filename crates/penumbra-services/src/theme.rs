//! Dynamic Monet Theming Service for Material 3 Expressive.
//!
//! Provides dominant wallpaper color extraction, color quantization,
//! Material 3 tonal palette generation, and cross-process IPC synchronization.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Material 3 Expressive Tonal Palette extracted or derived from wallpaper/seed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonetPalette {
    /// Extracted dominant seed RGB color.
    pub seed_rgb: [u8; 3],

    // Primary accent roles (Tonal 80 / 20 / 30 / 90)
    pub primary: String,
    pub on_primary: String,
    pub primary_container: String,
    pub on_primary_container: String,

    // Secondary accent roles (Tonal 80 / 20 / 30 / 90)
    pub secondary: String,
    pub on_secondary: String,
    pub secondary_container: String,
    pub on_secondary_container: String,

    // Tertiary accent roles (Tonal 80 / 20 / 30 / 90)
    pub tertiary: String,
    pub on_tertiary: String,
    pub tertiary_container: String,
    pub on_tertiary_container: String,

    // Surface & OLED Container Hierarchy
    pub surface: String,
    pub surface_dim: String,
    pub surface_bright: String,
    pub surface_container_lowest: String,
    pub surface_container_low: String,
    pub surface_container: String,
    pub surface_container_high: String,
    pub surface_container_highest: String,
    pub surface_variant: String,

    // Content & Boundary Roles
    pub on_surface: String,
    pub on_surface_variant: String,
    pub outline: String,
    pub outline_variant: String,
}

impl Default for MonetPalette {
    fn default() -> Self {
        Self::default_dark()
    }
}

impl MonetPalette {
    /// Baseline Material 3 Expressive Iris Periwinkle dark palette.
    pub fn default_dark() -> Self {
        Self {
            seed_rgb: [173, 198, 255],
            primary: "#ADC6FF".into(),
            on_primary: "#002E69".into(),
            primary_container: "#254070".into(),
            on_primary_container: "#D8E2FF".into(),

            secondary: "#BFC6DC".into(),
            on_secondary: "#283141".into(),
            secondary_container: "#3E4758".into(),
            on_secondary_container: "#DBE2F9".into(),

            tertiary: "#EFB8C8".into(),
            on_tertiary: "#492532".into(),
            tertiary_container: "#633B48".into(),
            on_tertiary_container: "#FFD8E4".into(),

            surface: "#141218".into(),
            surface_dim: "#110f15".into(),
            surface_bright: "#38353d".into(),
            surface_container_lowest: "#0b0a0e".into(),
            surface_container_low: "#17151d".into(),
            surface_container: "#1e1b24".into(),
            surface_container_high: "#27232e".into(),
            surface_container_highest: "#36343b".into(),
            surface_variant: "#44474f".into(),

            on_surface: "#E3E2E6".into(),
            on_surface_variant: "#C4C6D0".into(),
            outline: "#8E9099".into(),
            outline_variant: "#44474F".into(),
        }
    }

    /// Generates a complete Material 3 Expressive tonal dark palette from an arbitrary seed RGB color.
    pub fn from_seed_rgb(r: u8, g: u8, b: u8) -> Self {
        let (h, s, _l) = rgb_to_hsl(r, g, b);

        // Enforce vibrant primary saturation
        let primary_sat = s.clamp(0.40, 0.85);
        let secondary_sat = (s * 0.40).clamp(0.12, 0.32);
        let tertiary_hue = (h + 60.0) % 360.0;
        let tertiary_sat = (s * 0.65).clamp(0.28, 0.65);
        let neutral_sat = 0.08;

        Self {
            seed_rgb: [r, g, b],

            // Primary: Tone 80 / 20 / 30 / 90
            primary: hsl_to_hex(h, primary_sat, 0.80),
            on_primary: hsl_to_hex(h, primary_sat, 0.20),
            primary_container: hsl_to_hex(h, primary_sat, 0.30),
            on_primary_container: hsl_to_hex(h, primary_sat, 0.90),

            // Secondary: Tone 80 / 20 / 30 / 90
            secondary: hsl_to_hex(h, secondary_sat, 0.80),
            on_secondary: hsl_to_hex(h, secondary_sat, 0.20),
            secondary_container: hsl_to_hex(h, secondary_sat, 0.30),
            on_secondary_container: hsl_to_hex(h, secondary_sat, 0.90),

            // Tertiary: Tone 80 / 20 / 30 / 90
            tertiary: hsl_to_hex(tertiary_hue, tertiary_sat, 0.80),
            on_tertiary: hsl_to_hex(tertiary_hue, tertiary_sat, 0.20),
            tertiary_container: hsl_to_hex(tertiary_hue, tertiary_sat, 0.30),
            on_tertiary_container: hsl_to_hex(tertiary_hue, tertiary_sat, 0.90),

            // OLED and Neutral Surfaces
            surface: hsl_to_hex(h, neutral_sat, 0.08),
            surface_dim: hsl_to_hex(h, neutral_sat, 0.06),
            surface_bright: hsl_to_hex(h, neutral_sat, 0.22),
            surface_container_lowest: hsl_to_hex(h, neutral_sat, 0.04),
            surface_container_low: hsl_to_hex(h, neutral_sat, 0.09),
            surface_container: hsl_to_hex(h, neutral_sat, 0.12),
            surface_container_high: hsl_to_hex(h, neutral_sat, 0.16),
            surface_container_highest: hsl_to_hex(h, neutral_sat, 0.21),
            surface_variant: hsl_to_hex(h, neutral_sat, 0.28),

            // Content & Boundary
            on_surface: hsl_to_hex(h, neutral_sat, 0.90),
            on_surface_variant: hsl_to_hex(h, neutral_sat, 0.77),
            outline: hsl_to_hex(h, neutral_sat, 0.56),
            outline_variant: hsl_to_hex(h, neutral_sat, 0.30),
        }
    }

    /// Extracts dominant seed color from raw image bytes and builds tonal palette.
    pub fn from_image_bytes(bytes: &[u8]) -> Result<Self, String> {
        let img = image::load_from_memory(bytes).map_err(|e| format!("Failed to decode image: {e}"))?;
        let rgb_img = img.to_rgb8();
        let (width, height) = rgb_img.dimensions();

        if width == 0 || height == 0 {
            return Err("Image dimensions are 0".into());
        }

        // Fast grid sampling (16x16 = 256 sample points)
        let sample_steps = 16u32;
        let step_x = (width / sample_steps).max(1);
        let step_y = (height / sample_steps).max(1);

        // 16 hue bins (each 22.5 degrees)
        let mut bins: [f32; 16] = [0.0; 16];
        let mut bin_colors: [(u64, u64, u64, u32); 16] = [(0, 0, 0, 0); 16];
        let mut total_colorfulness = 0.0f32;
        let mut total_samples = 0u32;

        let mut avg_r = 0u64;
        let mut avg_g = 0u64;
        let mut avg_b = 0u64;

        for y in (0..height).step_by(step_y as usize) {
            for x in (0..width).step_by(step_x as usize) {
                let pixel = rgb_img.get_pixel(x, y);
                let (r, g, b) = (pixel[0], pixel[1], pixel[2]);

                avg_r += r as u64;
                avg_g += g as u64;
                avg_b += b as u64;
                total_samples += 1;

                let (h, s, l) = rgb_to_hsl(r, g, b);

                // Exclude pure black/white highlights
                if l < 0.10 || l > 0.92 {
                    continue;
                }

                // Score: prioritize vibrant, non-washed colors
                let score = s * (1.0 - (l - 0.5).abs() * 1.5).max(0.1);
                let bin_idx = ((h / 22.5).floor() as usize).min(15);

                bins[bin_idx] += score;
                bin_colors[bin_idx].0 += r as u64;
                bin_colors[bin_idx].1 += g as u64;
                bin_colors[bin_idx].2 += b as u64;
                bin_colors[bin_idx].3 += 1;

                total_colorfulness += s;
            }
        }

        if total_samples == 0 {
            return Ok(Self::default_dark());
        }

        // Check if image is substantially monochrome
        let avg_colorfulness = total_colorfulness / total_samples as f32;
        if avg_colorfulness < 0.08 {
            // Low saturation image: use a cool tinted slate seed
            let r = (avg_r / total_samples as u64) as u8;
            let g = (avg_g / total_samples as u64) as u8;
            let b = (avg_b / total_samples as u64) as u8;
            return Ok(Self::from_seed_rgb(r.max(30), g.max(40), b.max(60)));
        }

        // Find the bin with the highest score
        let mut best_bin = 0;
        let mut best_score = -1.0f32;
        for (i, &score) in bins.iter().enumerate() {
            if score > best_score && bin_colors[i].3 > 0 {
                best_score = score;
                best_bin = i;
            }
        }

        if best_score <= 0.0 || bin_colors[best_bin].3 == 0 {
            return Ok(Self::default_dark());
        }

        let count = bin_colors[best_bin].3 as u64;
        let seed_r = (bin_colors[best_bin].0 / count).clamp(0, 255) as u8;
        let seed_g = (bin_colors[best_bin].1 / count).clamp(0, 255) as u8;
        let seed_b = (bin_colors[best_bin].2 / count).clamp(0, 255) as u8;

        debug!(
            "Extracted dominant seed RGB: ({}, {}, {}) with score {}",
            seed_r, seed_g, seed_b, best_score
        );

        Ok(Self::from_seed_rgb(seed_r, seed_g, seed_b))
    }

    /// Extracts dominant seed color from an image file path.
    pub fn from_image_path(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path).map_err(|e| format!("Failed to read image at {:?}: {e}", path))?;
        Self::from_image_bytes(&bytes)
    }

    /// Resolves canonical user runtime IPC theme file path (`$XDG_RUNTIME_DIR/penumbra-theme.json`).
    pub fn default_ipc_path() -> PathBuf {
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            PathBuf::from(runtime_dir).join("penumbra-theme.json")
        } else {
            PathBuf::from("/tmp/penumbra-theme.json")
        }
    }

    /// Writes palette to IPC path atomically via temporary file and atomic rename.
    pub fn save_to_ipc_path(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let serialized = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));
        fs::write(&tmp_path, serialized.as_bytes())?;
        fs::rename(&tmp_path, path)?;
        info!("Synchronized Monet theme to {:?}", path);
        Ok(())
    }

    /// Reads palette from IPC path.
    pub fn load_from_ipc_path(path: &Path) -> io::Result<Self> {
        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

/// Converts RGB [0..255] to HSL (H in [0..360], S in [0..1], L in [0..1]).
fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;

    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let delta = max - min;

    let l = (max + min) / 2.0;

    if delta < 1e-5 {
        return (0.0, 0.0, l);
    }

    let s = if l > 0.5 {
        delta / (2.0 - max - min)
    } else {
        delta / (max + min)
    };

    let h = if (max - rf).abs() < 1e-5 {
        let mut segment = (gf - bf) / delta;
        if segment < 0.0 {
            segment += 6.0;
        }
        segment * 60.0
    } else if (max - gf).abs() < 1e-5 {
        (((bf - rf) / delta) + 2.0) * 60.0
    } else {
        (((rf - gf) / delta) + 4.0) * 60.0
    };

    (h.clamp(0.0, 360.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0))
}

/// Converts HSL to hex string `#RRGGBB`.
fn hsl_to_hex(h: f32, s: f32, l: f32) -> String {
    let (r, g, b) = hsl_to_rgb(h, s, l);
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// Converts HSL (H in [0..360], S in [0..1], L in [0..1]) to RGB [0..255].
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (((h / 60.0) % 2.0) - 1.0).abs());
    let m = l - c / 2.0;

    let (r_prime, g_prime, b_prime) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    let r = ((r_prime + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = ((g_prime + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = ((b_prime + m) * 255.0).round().clamp(0.0, 255.0) as u8;

    (r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_dark_palette() {
        let palette = MonetPalette::default_dark();
        assert_eq!(palette.primary, "#ADC6FF");
        assert_eq!(palette.on_primary, "#002E69");
        assert!(palette.primary.starts_with('#'));
        assert_eq!(palette.primary.len(), 7);
    }

    #[test]
    fn test_from_seed_rgb_tonal_properties() {
        // Test with green seed (Emerald)
        let emerald = MonetPalette::from_seed_rgb(46, 204, 113);
        assert!(emerald.primary.starts_with('#'));
        assert!(emerald.on_primary.starts_with('#'));
        assert!(emerald.surface_container_lowest.starts_with('#'));

        // Verify primary and on_primary have high luminance contrast
        let (_hp, _sp, lp) = {
            let r = u8::from_str_radix(&emerald.primary[1..3], 16).unwrap();
            let g = u8::from_str_radix(&emerald.primary[3..5], 16).unwrap();
            let b = u8::from_str_radix(&emerald.primary[5..7], 16).unwrap();
            rgb_to_hsl(r, g, b)
        };
        let (_ho, _so, lo) = {
            let r = u8::from_str_radix(&emerald.on_primary[1..3], 16).unwrap();
            let g = u8::from_str_radix(&emerald.on_primary[3..5], 16).unwrap();
            let b = u8::from_str_radix(&emerald.on_primary[5..7], 16).unwrap();
            rgb_to_hsl(r, g, b)
        };
        assert!(lp > 0.70, "Primary tone should be light for dark mode");
        assert!(lo < 0.30, "On-primary tone should be dark for high contrast");
    }

    #[test]
    fn test_quantizer_synthetic_image() {
        // Create an in-memory 32x32 image with rich orange sunset color
        let mut img_buf = image::RgbImage::new(32, 32);
        for pixel in img_buf.pixels_mut() {
            *pixel = image::Rgb([255, 140, 0]); // DarkOrange
        }

        let mut bytes = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut bytes);
        img_buf
            .write_to(&mut cursor, image::ImageFormat::Png)
            .expect("Failed to encode synthetic png");

        let palette = MonetPalette::from_image_bytes(&bytes).expect("Failed to extract palette");
        // Orange seed hue should be ~33 degrees
        let (h, _, _) = rgb_to_hsl(palette.seed_rgb[0], palette.seed_rgb[1], palette.seed_rgb[2]);
        assert!((h - 33.0).abs() < 15.0, "Extracted hue should be orange");
    }

    #[test]
    fn test_ipc_save_and_load() {
        let palette = MonetPalette::from_seed_rgb(100, 150, 240);
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join(format!("test-monet-{}.json", std::process::id()));

        palette.save_to_ipc_path(&file_path).unwrap();
        let loaded = MonetPalette::load_from_ipc_path(&file_path).unwrap();

        assert_eq!(palette, loaded);
        let _ = fs::remove_file(file_path);
    }
}
