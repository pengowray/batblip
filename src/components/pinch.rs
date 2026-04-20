//! Pinch-to-zoom gesture helpers shared across all canvas components.

use crate::viewport;

/// Snapshot of display-freq state at the moment a 2-finger touch begins
/// on the band gutter — drives vertical pinch-to-zoom + two-finger pan
/// of `min_display_freq` / `max_display_freq` on the host view.
#[derive(Clone, Copy, Debug)]
pub struct FreqPinchState {
    /// Pixel y-distance between the two fingers at gesture start.
    pub initial_dist_y: f64,
    /// Resolved min_display_freq at gesture start.
    pub initial_min_freq: f64,
    /// Resolved max_display_freq at gesture start.
    pub initial_max_freq: f64,
    /// Gutter-canvas-local y of the midpoint at gesture start.
    pub initial_mid_canvas_y: f64,
    /// File Nyquist. new_min/new_max are clamped to [0, nyquist].
    pub nyquist: f64,
}

/// Two-finger y-geometry on a touch list — (midpoint_client_y, |y1 - y0|).
pub fn two_finger_y_geometry(touches: &web_sys::TouchList) -> Option<(f64, f64)> {
    if touches.length() != 2 {
        return None;
    }
    let t0 = touches.get(0)?;
    let t1 = touches.get(1)?;
    let y0 = t0.client_y() as f64;
    let y1 = t1.client_y() as f64;
    let mid_y = (y0 + y1) / 2.0;
    let dist = (y1 - y0).abs();
    Some((mid_y, dist))
}

/// Given a freq-pinch snapshot and current gesture geometry, compute
/// (new_min, new_max) display frequencies. The frequency under the
/// initial midpoint stays pinned to the current midpoint y, which
/// combines anchor-zoom + two-finger vertical pan in one formula.
pub fn apply_freq_pinch(
    ps: &FreqPinchState,
    current_dist_y: f64,
    current_mid_canvas_y: f64,
    canvas_h: f64,
) -> (f64, f64) {
    if canvas_h <= 0.0 || ps.initial_dist_y < 5.0 {
        return (ps.initial_min_freq, ps.initial_max_freq);
    }
    let initial_range = (ps.initial_max_freq - ps.initial_min_freq).max(1.0);

    // Larger finger-gap → narrower visible range (zoom in).
    let scale = ps.initial_dist_y / current_dist_y.max(1.0);
    let min_range_hz = 500.0_f64.min(ps.nyquist.max(500.0));
    let new_range = (initial_range * scale).clamp(min_range_hz, ps.nyquist.max(min_range_hz));

    // Anchor: freq under the initial midpoint y.
    let initial_mid_frac = (ps.initial_mid_canvas_y / canvas_h).clamp(0.0, 1.0);
    let anchor_freq = ps.initial_max_freq - initial_mid_frac * initial_range;

    // Place that freq at the CURRENT midpoint y — this handles both zoom
    // (scale change) and two-finger pan (midpoint shift) simultaneously.
    let current_mid_frac = (current_mid_canvas_y / canvas_h).clamp(0.0, 1.0);
    let mut new_max = anchor_freq + current_mid_frac * new_range;
    let mut new_min = new_max - new_range;

    if new_min < 0.0 {
        new_min = 0.0;
        new_max = new_range.min(ps.nyquist);
    }
    if new_max > ps.nyquist {
        new_max = ps.nyquist;
        new_min = (new_max - new_range).max(0.0);
    }
    (new_min, new_max)
}

/// Snapshot of state at the moment a 2-finger touch begins.
#[derive(Clone, Copy, Debug)]
pub struct PinchState {
    /// Pixel distance between the two fingers at gesture start.
    pub initial_dist: f64,
    /// zoom_level at gesture start.
    pub initial_zoom: f64,
    /// scroll_offset (seconds) at gesture start.
    pub initial_scroll: f64,
    /// Midpoint X in client coordinates at gesture start.
    pub initial_mid_client_x: f64,
    /// Seconds per FFT column.
    pub time_res: f64,
    /// File duration in seconds (for scroll clamping).
    pub duration: f64,
    /// Whether FromHere viewport bounds should be used.
    pub from_here_mode: bool,
}

/// Returns (midpoint_client_x, distance) for exactly 2 touches.
pub fn two_finger_geometry(touches: &web_sys::TouchList) -> Option<(f64, f64)> {
    if touches.length() != 2 {
        return None;
    }
    let t0 = touches.get(0)?;
    let t1 = touches.get(1)?;
    let x0 = t0.client_x() as f64;
    let x1 = t1.client_x() as f64;
    let y0 = t0.client_y() as f64;
    let y1 = t1.client_y() as f64;
    let mid_x = (x0 + x1) / 2.0;
    let dist = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    Some((mid_x, dist))
}

/// Given a pinch state snapshot and current gesture geometry, compute (new_zoom, new_scroll).
///
/// Anchor-point zoom: the time under the initial midpoint stays fixed as fingers spread/contract.
/// Two-finger pan: horizontal midpoint movement also translates scroll_offset.
pub fn apply_pinch(
    pinch: &PinchState,
    current_dist: f64,
    current_mid_client_x: f64,
    canvas_left: f64,
    canvas_width: f64,
) -> (f64, f64) {
    if canvas_width == 0.0 || pinch.initial_dist < 10.0 {
        return (pinch.initial_zoom, pinch.initial_scroll);
    }

    // Zoom proportional to finger distance ratio
    let scale = current_dist / pinch.initial_dist;
    let new_zoom = (pinch.initial_zoom * scale).clamp(viewport::MIN_ZOOM, viewport::MAX_ZOOM);

    // What time was under the initial midpoint?
    let initial_visible_time = viewport::visible_time(canvas_width, pinch.initial_zoom, pinch.time_res);
    let initial_mid_canvas_x = pinch.initial_mid_client_x - canvas_left;
    let mid_frac = (initial_mid_canvas_x / canvas_width).clamp(0.0, 1.0);
    let anchor_time = pinch.initial_scroll + mid_frac * initial_visible_time;

    // New visible time at new zoom
    let new_visible_time = viewport::visible_time(canvas_width, new_zoom, pinch.time_res);

    // Scroll so anchor_time stays at the same screen fraction
    let scroll_from_anchor = anchor_time - mid_frac * new_visible_time;

    // Two-finger pan: midpoint shift → time shift
    let mid_shift_px = current_mid_client_x - pinch.initial_mid_client_x;
    let pan_dt = -(mid_shift_px / canvas_width) * new_visible_time;

    let raw_scroll = scroll_from_anchor + pan_dt;
    let new_scroll = viewport::clamp_scroll_for_mode(
        raw_scroll,
        pinch.duration,
        new_visible_time,
        pinch.from_here_mode,
    );

    (new_zoom, new_scroll)
}
