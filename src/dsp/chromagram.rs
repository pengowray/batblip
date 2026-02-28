//! Chromagram computation: maps STFT magnitude columns to 12 pitch classes,
//! each subdivided by octave.

/// Number of pitch classes (C, C#, D, ..., B).
pub const NUM_PITCH_CLASSES: usize = 12;

/// Number of octaves to track (octaves 0–9, covering MIDI notes 0–127).
pub const NUM_OCTAVES: usize = 10;

/// Result of mapping one STFT column to chromagram data.
pub struct ChromagramColumn {
    /// Total intensity per pitch class (sum across all octaves).
    pub pitch_classes: [f32; NUM_PITCH_CLASSES],
    /// Per-octave detail: `octave_detail[pitch_class][octave]`.
    pub octave_detail: [[f32; NUM_OCTAVES]; NUM_PITCH_CLASSES],
}

/// Pitch class names for labelling.
pub const PITCH_CLASS_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Convert an STFT magnitude column to a chromagram column.
///
/// `magnitudes`: FFT magnitude bins (index 0 = DC, index N = Nyquist).
/// `freq_resolution`: Hz per FFT bin (= sample_rate / fft_size).
pub fn stft_to_chromagram(
    magnitudes: &[f32],
    freq_resolution: f64,
) -> ChromagramColumn {
    let mut pitch_classes = [0.0f32; NUM_PITCH_CLASSES];
    let mut octave_detail = [[0.0f32; NUM_OCTAVES]; NUM_PITCH_CLASSES];

    for (bin_idx, &mag) in magnitudes.iter().enumerate() {
        if bin_idx == 0 { continue; } // skip DC
        let freq = bin_idx as f64 * freq_resolution;
        if freq < 16.35 { continue; } // below C0
        if freq > 16744.0 { continue; } // above B9 (practical limit)

        // MIDI note number: 69 = A4 = 440 Hz
        let midi = 69.0 + 12.0 * (freq / 440.0).log2();
        if midi < 0.0 || midi > 127.0 { continue; }

        let midi_rounded = midi.round() as usize;
        let pc = midi_rounded % 12;
        let octave = (midi_rounded / 12).saturating_sub(1).min(NUM_OCTAVES - 1);

        // Use energy (mag²) for better perceptual weighting
        let energy = mag * mag;
        pitch_classes[pc] += energy;
        octave_detail[pc][octave] += energy;
    }

    ChromagramColumn { pitch_classes, octave_detail }
}

/// The total number of rows in a chromagram display.
/// Each pitch class gets `NUM_OCTAVES` sub-rows.
pub const CHROMA_ROWS: usize = NUM_PITCH_CLASSES * NUM_OCTAVES;

/// Pre-render a set of STFT columns as a chromagram tile.
///
/// Returns greyscale RGBA pixels where:
/// - Width = number of columns
/// - Height = `CHROMA_ROWS` (12 pitch classes × 10 octaves = 120 rows)
/// - Row 0 = B9 (top), last row = C0 (bottom)
///
/// Each pixel encodes two values packed into the RGB channels:
/// - R channel: overall pitch class intensity (0–255)
/// - G channel: specific note (octave) intensity (0–255)
/// - B channel: 0 (unused, reserved for future 2D colormap)
/// - A channel: 255
///
/// The 2D colormap is applied during blit (not baked in), so the chromagram
/// view can adjust color mapping without re-rendering tiles.
pub fn pre_render_chromagram_columns(
    stft_columns: &[crate::types::SpectrogramColumn],
    freq_resolution: f64,
) -> crate::canvas::spectrogram_renderer::PreRendered {
    use crate::canvas::spectrogram_renderer::PreRendered;

    if stft_columns.is_empty() {
        return PreRendered { width: 0, height: 0, pixels: Vec::new() };
    }

    let width = stft_columns.len();
    let height = CHROMA_ROWS;
    let mut pixels = vec![0u8; width * height * 4];

    // First pass: compute all chromagram columns and find max values for normalization
    let chromas: Vec<ChromagramColumn> = stft_columns.iter()
        .map(|col| stft_to_chromagram(&col.magnitudes, freq_resolution))
        .collect();

    // Find global max for normalization
    let max_class = chromas.iter()
        .flat_map(|c| c.pitch_classes.iter())
        .copied()
        .fold(0.0f32, f32::max);
    let max_note = chromas.iter()
        .flat_map(|c| c.octave_detail.iter().flat_map(|o| o.iter()))
        .copied()
        .fold(0.0f32, f32::max);

    if max_class <= 0.0 || max_note <= 0.0 {
        return PreRendered { width: width as u32, height: height as u32, pixels };
    }

    // Second pass: render pixels
    for (col_idx, chroma) in chromas.iter().enumerate() {
        for pc in 0..NUM_PITCH_CLASSES {
            let class_norm = (chroma.pitch_classes[pc] / max_class).sqrt().min(1.0);
            let class_byte = (class_norm * 255.0) as u8;

            for oct in 0..NUM_OCTAVES {
                let note_norm = (chroma.octave_detail[pc][oct] / max_note).sqrt().min(1.0);
                let note_byte = (note_norm * 255.0) as u8;

                // Row layout: pitch class 0 (C) at bottom, B at top
                // Within each pitch class: octave 0 at bottom, highest at top
                let row_from_bottom = pc * NUM_OCTAVES + oct;
                let y = height - 1 - row_from_bottom;
                let pixel_idx = (y * width + col_idx) * 4;
                pixels[pixel_idx] = class_byte;     // R = pitch class intensity
                pixels[pixel_idx + 1] = note_byte;  // G = note intensity
                pixels[pixel_idx + 2] = 0;           // B = reserved
                pixels[pixel_idx + 3] = 255;
            }
        }
    }

    PreRendered { width: width as u32, height: height as u32, pixels }
}
