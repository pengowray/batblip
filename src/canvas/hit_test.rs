use leptos::prelude::*;
use crate::canvas::spectrogram_renderer;
use crate::state::{AppState, PlaybackMode, SpectrogramHandle};

/// Half-width of the FF handle interaction zone (pixels from center).
pub const FF_HANDLE_HALF_WIDTH: f64 = 50.0;

/// Hit-test all spectrogram overlay handles (FF + HET).
/// Returns the closest handle within `threshold` pixels, or None.
/// HET handles take priority over FF when they overlap and HET is manual.
/// FF handles only respond in a limited horizontal zone around the canvas center.
pub fn hit_test_spec_handles(
    state: &AppState,
    mouse_x: f64,
    mouse_y: f64,
    min_freq: f64,
    max_freq: f64,
    canvas_width: f64,
    canvas_height: f64,
    threshold: f64,
) -> Option<SpectrogramHandle> {
    let mut candidates: Vec<(SpectrogramHandle, f64)> = Vec::new();

    // FF handles — only respond when mouse is within the center handle zone
    let ff_lo = state.ff_freq_lo.get_untracked();
    let ff_hi = state.ff_freq_hi.get_untracked();
    let center_x = canvas_width / 2.0;
    let in_ff_zone = (mouse_x - center_x).abs() <= FF_HANDLE_HALF_WIDTH;
    if ff_hi > ff_lo && in_ff_zone {
        let y_upper = spectrogram_renderer::freq_to_y(ff_hi.min(max_freq), min_freq, max_freq, canvas_height);
        let y_lower = spectrogram_renderer::freq_to_y(ff_lo.max(min_freq), min_freq, max_freq, canvas_height);
        let d_upper = (mouse_y - y_upper).abs();
        let d_lower = (mouse_y - y_lower).abs();
        if d_upper <= threshold { candidates.push((SpectrogramHandle::FfUpper, d_upper)); }
        if d_lower <= threshold { candidates.push((SpectrogramHandle::FfLower, d_lower)); }
        // Middle handle (midpoint between boundaries)
        let mid_freq = (ff_lo + ff_hi) / 2.0;
        let y_mid = spectrogram_renderer::freq_to_y(mid_freq.clamp(min_freq, max_freq), min_freq, max_freq, canvas_height);
        let d_mid = (mouse_y - y_mid).abs();
        if d_mid <= threshold { candidates.push((SpectrogramHandle::FfMiddle, d_mid)); }
    }

    // HET handles (only when in HET mode and parameter is manual)
    if state.playback_mode.get_untracked() == PlaybackMode::Heterodyne {
        let het_freq = state.het_frequency.get_untracked();
        let het_cutoff = state.het_cutoff.get_untracked();

        if !state.het_freq_auto.get_untracked() {
            let y_center = spectrogram_renderer::freq_to_y(het_freq, min_freq, max_freq, canvas_height);
            let d = (mouse_y - y_center).abs();
            if d <= threshold { candidates.push((SpectrogramHandle::HetCenter, d)); }
        }
        if !state.het_cutoff_auto.get_untracked() {
            let y_upper = spectrogram_renderer::freq_to_y(
                (het_freq + het_cutoff).min(max_freq), min_freq, max_freq, canvas_height,
            );
            let y_lower = spectrogram_renderer::freq_to_y(
                (het_freq - het_cutoff).max(min_freq), min_freq, max_freq, canvas_height,
            );
            let d_upper = (mouse_y - y_upper).abs();
            let d_lower = (mouse_y - y_lower).abs();
            if d_upper <= threshold { candidates.push((SpectrogramHandle::HetBandUpper, d_upper)); }
            if d_lower <= threshold { candidates.push((SpectrogramHandle::HetBandLower, d_lower)); }
        }
    }

    if candidates.is_empty() { return None; }

    // Sort by distance, then prefer HET over FF when tied
    candidates.sort_by(|a, b| {
        a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                let a_het = matches!(a.0, SpectrogramHandle::HetCenter | SpectrogramHandle::HetBandUpper | SpectrogramHandle::HetBandLower);
                let b_het = matches!(b.0, SpectrogramHandle::HetCenter | SpectrogramHandle::HetBandUpper | SpectrogramHandle::HetBandLower);
                b_het.cmp(&a_het) // HET first
            })
    });

    Some(candidates[0].0)
}
