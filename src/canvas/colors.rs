/// Map a spectrogram magnitude to a greyscale pixel value (0-255).
/// Uses log scale (dB) for perceptual brightness.
pub fn magnitude_to_greyscale(mag: f32, max_mag: f32) -> u8 {
    if max_mag <= 0.0 || mag <= 0.0 {
        return 0;
    }
    let db = 20.0 * (mag / max_mag).log10();
    // Clamp to [-80, 0] dB dynamic range
    let db_clamped = db.max(-80.0).min(0.0);
    // Map to 0-255
    ((db_clamped + 80.0) / 80.0 * 255.0) as u8
}

/// Resistor color band colors for frequency markers at 10 kHz intervals.
/// Repeats every 10 decades (0=black, 1=brown, ..., 9=white, 10=black, ...).
pub fn freq_marker_color(freq_hz: f64) -> [u8; 3] {
    const BANDS: [[u8; 3]; 10] = [
        [40, 40, 40],      // 0 - black (lightened for visibility)
        [139, 69, 19],     // 1 - brown
        [255, 0, 0],       // 2 - red
        [255, 165, 0],     // 3 - orange
        [255, 255, 0],     // 4 - yellow
        [0, 128, 0],       // 5 - green
        [0, 0, 255],       // 6 - blue
        [148, 0, 211],     // 7 - violet
        [128, 128, 128],   // 8 - grey
        [255, 255, 255],   // 9 - white
    ];
    let digit = (freq_hz / 10_000.0).round() as u32 % 10;
    BANDS[digit as usize]
}

/// Hermite smoothstep: smooth transition from 0 to 1 between edge0 and edge1.
fn smoothstep(x: f32, edge0: f32, edge1: f32) -> f32 {
    if edge1 <= edge0 {
        return if x >= edge0 { 1.0 } else { 0.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Map a greyscale base value and a frequency-shift amount to an RGB triple.
/// `shift` > 0 → energy moving upward in frequency → red tint.
/// `shift` < 0 → energy moving downward in frequency → blue tint.
///
/// Two smooth gates control when color appears:
/// - `intensity_gate` (0.0–1.0): how bright must a pixel be to show color
/// - `movement_gate` (0.0–1.0): how large must the shift be to show color
/// - `opacity` (0.0–1.0): overall color strength multiplier
pub fn movement_rgb(grey: u8, shift: f32, intensity_gate: f32, movement_gate: f32, opacity: f32) -> [u8; 3] {
    let g_norm = grey as f32 / 255.0;

    // Smooth intensity gate: ramp from gate*0.6 to gate*1.4
    let ig_lo = intensity_gate * 0.6;
    let ig_hi = (intensity_gate * 1.4).min(1.0);
    let intensity_factor = smoothstep(g_norm, ig_lo, ig_hi);

    // Smooth movement gate: ramp based on |shift|
    let abs_shift = shift.abs();
    let mg_lo = movement_gate * 0.3;
    let mg_hi = (movement_gate * 2.0).max(0.05);
    let movement_factor = smoothstep(abs_shift, mg_lo, mg_hi);

    let effective = intensity_factor * movement_factor * opacity;
    if effective < 0.001 {
        return [grey, grey, grey];
    }

    let gain: f32 = 3.0;
    let s = (shift * gain * effective).clamp(-1.0, 1.0);
    let g = grey as f32;
    if s > 0.0 {
        // Upward shift → red
        let r = (g + s * (255.0 - g)).min(255.0) as u8;
        let gb = (g * (1.0 - 0.5 * s)).max(0.0) as u8;
        [r, gb, gb]
    } else {
        // Downward shift → blue
        let a = -s;
        let b = (g + a * (255.0 - g)).min(255.0) as u8;
        let rg = (g * (1.0 - 0.5 * a)).max(0.0) as u8;
        [rg, rg, b]
    }
}

/// Label for a frequency marker (number only, e.g. "40").
pub fn freq_marker_label(freq_hz: f64) -> String {
    format!("{}", (freq_hz / 1000.0).round() as u32)
}
