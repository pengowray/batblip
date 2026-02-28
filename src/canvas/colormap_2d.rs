//! 2D colormap lookup tables.
//!
//! A 2D colormap maps two byte values (primary, secondary) to an RGB triple.
//! Used for visualizations that encode two dimensions of data in color:
//! - Movement: primary = intensity, secondary = shift direction
//! - Chromagram: primary = pitch class intensity, secondary = note intensity

use crate::canvas::colors::movement_rgb;

/// A 2D colormap: 256 × 256 → RGB lookup table (192 KB).
pub struct Colormap2D {
    /// Row-major: `lut[secondary * 256 + primary]`.
    lut: Vec<[u8; 3]>,
}

impl Colormap2D {
    /// Look up the color for given (primary, secondary) byte values.
    #[inline]
    pub fn apply(&self, primary: u8, secondary: u8) -> [u8; 3] {
        self.lut[secondary as usize * 256 + primary as usize]
    }
}

/// Build a movement colormap.
///
/// - Primary axis (0–255): pixel intensity (greyscale magnitude).
/// - Secondary axis (0–255): shift direction — 128 = neutral,
///   0 = max downward (blue), 255 = max upward (red).
///
/// The `intensity_gate`, `movement_gate`, and `opacity` parameters control
/// thresholds and color strength, matching the existing `movement_rgb` logic.
pub fn build_movement_colormap(
    intensity_gate: f32,
    movement_gate: f32,
    opacity: f32,
) -> Colormap2D {
    let mut lut = vec![[0u8; 3]; 256 * 256];

    for sec in 0..256u16 {
        // Map secondary byte to shift in [-1, 1]
        let shift = (sec as f32 - 128.0) / 128.0;

        for pri in 0..256u16 {
            let grey = pri as u8;
            let rgb = movement_rgb(grey, shift, intensity_gate, movement_gate, opacity);
            lut[sec as usize * 256 + pri as usize] = rgb;
        }
    }

    Colormap2D { lut }
}

/// Build a chromagram colormap.
///
/// - Primary axis (0–255): overall pitch class intensity.
/// - Secondary axis (0–255): specific note (octave) intensity.
///
/// When both are high: bright white/yellow. When class is high but note is low:
/// dim warm color (energy in the pitch class, but not this specific octave).
/// When note is high but class is low: shouldn't happen (note ⊆ class).
/// When both are low: black.
pub fn build_chromagram_colormap() -> Colormap2D {
    let mut lut = vec![[0u8; 3]; 256 * 256];

    for sec in 0..256u16 {
        let note = sec as f32 / 255.0; // specific note intensity

        for pri in 0..256u16 {
            let class = pri as f32 / 255.0; // overall pitch class intensity

            // Base brightness from class intensity
            // Note intensity adds contrast within the band
            let brightness = class * 0.4 + note * 0.6;
            // Saturation: high class + low note → warm desaturated; high note → vivid
            let saturation = if class > 0.01 {
                (note / class).min(1.0)
            } else {
                0.0
            };

            // HSL-ish mapping: warm orange-to-white
            // Low saturation: grey/white. High saturation: orange/yellow.
            let r = (brightness * (0.6 + 0.4 * saturation) * 255.0).min(255.0) as u8;
            let g = (brightness * (0.3 + 0.5 * saturation) * 255.0).min(255.0) as u8;
            let b = (brightness * (0.1 + 0.1 * saturation) * 255.0).min(255.0) as u8;

            lut[sec as usize * 256 + pri as usize] = [r, g, b];
        }
    }

    Colormap2D { lut }
}
