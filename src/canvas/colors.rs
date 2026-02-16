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
pub fn freq_marker_color(freq_hz: f64) -> [u8; 3] {
    match (freq_hz / 10_000.0).round() as u32 {
        1 => [139, 69, 19],    // brown  - 10 kHz
        2 => [255, 0, 0],      // red    - 20 kHz
        3 => [255, 165, 0],    // orange - 30 kHz
        4 => [255, 255, 0],    // yellow - 40 kHz
        5 => [0, 128, 0],      // green  - 50 kHz
        6 => [0, 0, 255],      // blue   - 60 kHz
        7 => [148, 0, 211],    // violet - 70 kHz
        8 => [128, 128, 128],  // grey   - 80 kHz
        9 => [255, 255, 255],  // white  - 90 kHz
        10 => [40, 40, 40],    // dark   - 100 kHz (lightened for visibility on black)
        _ => [64, 64, 64],
    }
}

/// Label for a frequency marker.
pub fn freq_marker_label(freq_hz: f64) -> String {
    format!("{} kHz", (freq_hz / 1000.0).round() as u32)
}
