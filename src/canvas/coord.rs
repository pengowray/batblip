use leptos::prelude::*;
use web_sys::HtmlCanvasElement;
use crate::canvas::spectrogram_renderer;
use crate::state::AppState;

/// Convert a pointer position (client_x, client_y) relative to the canvas
/// into (px_x, px_y, time, freq).
///
/// Works for both mouse and touch events — the caller extracts client_x/client_y
/// from whichever event type they have.
pub fn pointer_to_xtf(
    client_x: f64,
    client_y: f64,
    canvas_ref: &NodeRef<leptos::html::Canvas>,
    state: &AppState,
) -> Option<(f64, f64, f64, f64)> {
    let canvas_el = canvas_ref.get()?;
    let canvas: &HtmlCanvasElement = canvas_el.as_ref();
    let rect = canvas.get_bounding_client_rect();
    let px_x = client_x - rect.left();
    let px_y = client_y - rect.top();
    let cw = canvas.width() as f64;
    let ch = canvas.height() as f64;

    let files = state.files.get_untracked();
    let idx = state.current_file_index.get_untracked()?;
    let file = files.get(idx)?;
    let time_res = file.spectrogram.time_resolution;
    let file_max_freq = file.spectrogram.max_freq;
    let max_freq = state.max_display_freq.get_untracked()
        .unwrap_or(file_max_freq);
    let min_freq = state.min_display_freq.get_untracked()
        .unwrap_or(0.0);
    let scroll = state.scroll_offset.get_untracked();
    let zoom = state.zoom_level.get_untracked();

    let (t, f) = spectrogram_renderer::pixel_to_time_freq(
        px_x, px_y, min_freq, max_freq, scroll, time_res, zoom, cw, ch,
    );
    Some((px_x, px_y, t, f))
}
