use leptos::prelude::*;
use leptos::ev::MouseEvent;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};
use crate::dsp::filters::{apply_eq_filter, apply_eq_filter_fast};
use crate::dsp::zc_divide::zc_rate_per_bin;
use crate::state::{AppState, CanvasTool, FilterQuality};

const ZC_BIN_DURATION: f64 = 0.001; // 1ms bins
const TAU: f64 = std::f64::consts::TAU;
const LABEL_AREA_WIDTH: f64 = 60.0;

/// Convert a canvas-local Y coordinate to a frequency (Hz).
fn y_to_freq(y: f64, min_freq: f64, max_freq: f64, canvas_height: f64) -> f64 {
    min_freq + (1.0 - y / canvas_height) * (max_freq - min_freq)
}

/// Convert a frequency (Hz) to a canvas-local Y coordinate.
fn freq_to_y(freq: f64, min_freq: f64, max_freq: f64, canvas_height: f64) -> f64 {
    let range = max_freq - min_freq;
    if range <= 0.0 { return canvas_height / 2.0; }
    canvas_height * (1.0 - (freq - min_freq) / range)
}

/// Pick a nice grid interval (in kHz) for the visible frequency range.
fn grid_interval_khz(range_khz: f64) -> f64 {
    if range_khz <= 10.0 { 2.0 }
    else if range_khz <= 25.0 { 5.0 }
    else if range_khz <= 60.0 { 10.0 }
    else if range_khz <= 150.0 { 20.0 }
    else { 50.0 }
}

#[component]
pub fn ZcDotChart() -> impl IntoView {
    let state = expect_context::<AppState>();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();
    let hand_drag_start = RwSignal::new((0.0f64, 0.0f64));
    let pinch_state: RwSignal<Option<crate::components::pinch::PinchState>> = RwSignal::new(None);
    let axis_drag_raw_start = RwSignal::new(0.0f64);

    // Cache ZC bins — recompute when the file or EQ settings change.
    let zc_bins = Memo::new(move |_| {
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let filter_enabled = state.filter_enabled.get();
        let freq_low = state.filter_freq_low.get();
        let freq_high = state.filter_freq_high.get();
        let db_below = state.filter_db_below.get();
        let db_selected = state.filter_db_selected.get();
        let db_harmonics = state.filter_db_harmonics.get();
        let db_above = state.filter_db_above.get();
        let band_mode = state.filter_band_mode.get();
        let quality = state.filter_quality.get();

        idx.and_then(|i| files.get(i).cloned()).map(|file| {
            let sr = file.audio.sample_rate;
            let samples = if filter_enabled {
                match quality {
                    FilterQuality::Fast => apply_eq_filter_fast(&file.audio.samples, sr, freq_low, freq_high, db_below, db_selected, db_harmonics, db_above, band_mode),
                    FilterQuality::HQ => apply_eq_filter(&file.audio.samples, sr, freq_low, freq_high, db_below, db_selected, db_harmonics, db_above, band_mode),
                }
            } else {
                file.audio.samples.to_vec()
            };
            zc_rate_per_bin(&samples, sr, ZC_BIN_DURATION, filter_enabled)
        })
    });

    // Main render effect
    Effect::new(move || {
        let scroll = state.scroll_offset.get();
        let zoom = state.zoom_level.get();
        let selection = state.selection.get();
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let is_playing = state.is_playing.get();
        let canvas_tool = state.canvas_tool.get();
        let display_min_freq = state.min_display_freq.get();
        let display_max_freq = state.max_display_freq.get();
        let ff_lo = state.ff_freq_lo.get();
        let ff_hi = state.ff_freq_hi.get();
        let _axis_drag_s = state.axis_drag_start_freq.get();
        let _axis_drag_c = state.axis_drag_current_freq.get();

        let Some(canvas_el) = canvas_ref.get() else { return };
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();

        let rect = canvas.get_bounding_client_rect();
        let display_w = rect.width() as u32;
        let display_h = rect.height() as u32;
        if display_w == 0 || display_h == 0 { return; }
        if canvas.width() != display_w || canvas.height() != display_h {
            canvas.set_width(display_w);
            canvas.set_height(display_h);
        }

        let ctx = canvas
            .get_context("2d").unwrap().unwrap()
            .dyn_into::<CanvasRenderingContext2d>().unwrap();

        let cw = display_w as f64;
        let ch = display_h as f64;

        // Clear
        ctx.set_fill_style_str("#0a0a0a");
        ctx.fill_rect(0.0, 0.0, cw, ch);

        let Some(file) = idx.and_then(|i| files.get(i)) else { return };
        let Some(bins) = zc_bins.get().as_ref().cloned() else { return };
        if bins.is_empty() { return; }

        let time_res = file.spectrogram.time_resolution;
        let total_duration = file.audio.duration_secs;
        let file_max_freq = file.spectrogram.max_freq;

        // Display frequency range (respects zoom/focus)
        let min_freq = display_min_freq.unwrap_or(0.0);
        let max_freq = display_max_freq.unwrap_or(file_max_freq);
        let freq_range = max_freq - min_freq;
        if freq_range <= 0.0 { return; }

        // Dot area is to the right of the label area
        let dot_area_w = (cw - LABEL_AREA_WIDTH).max(0.0);

        let visible_time = (dot_area_w / zoom) * time_res;
        let start_time = scroll.max(0.0).min((total_duration - visible_time).max(0.0));
        let px_per_sec = if visible_time > 0.0 { dot_area_w / visible_time } else { 0.0 };

        // Clip to dot area for drawing dots and selection
        ctx.save();
        ctx.begin_path();
        ctx.rect(LABEL_AREA_WIDTH, 0.0, dot_area_w, ch);
        ctx.clip();

        // Selection highlight
        if let Some(sel) = selection {
            let x0 = LABEL_AREA_WIDTH + ((sel.time_start - start_time) * px_per_sec).max(0.0);
            let x1 = LABEL_AREA_WIDTH + ((sel.time_end - start_time) * px_per_sec).min(dot_area_w);
            if x1 > x0 {
                ctx.set_fill_style_str("rgba(50, 120, 200, 0.2)");
                ctx.fill_rect(x0, 0.0, x1 - x0, ch);
            }
        }

        // FF range overlay
        if ff_hi > ff_lo {
            let y_top = freq_to_y(ff_hi.min(max_freq), min_freq, max_freq, ch);
            let y_bot = freq_to_y(ff_lo.max(min_freq), min_freq, max_freq, ch);
            if y_bot > y_top {
                ctx.set_fill_style_str("rgba(80, 160, 255, 0.08)");
                ctx.fill_rect(LABEL_AREA_WIDTH, y_top, dot_area_w, y_bot - y_top);
                // FF boundary lines
                ctx.set_stroke_style_str("rgba(80, 160, 255, 0.3)");
                ctx.set_line_width(1.0);
                for &yy in &[y_top, y_bot] {
                    ctx.begin_path();
                    ctx.move_to(LABEL_AREA_WIDTH, yy);
                    ctx.line_to(cw, yy);
                    ctx.stroke();
                }
            }
        }

        // Horizontal grid lines (in dot area)
        let min_freq_khz = min_freq / 1000.0;
        let max_freq_khz = max_freq / 1000.0;
        let range_khz = max_freq_khz - min_freq_khz;
        let interval = grid_interval_khz(range_khz);
        let first_grid = ((min_freq_khz / interval).ceil() * interval) as f64;
        ctx.set_stroke_style_str("#222");
        ctx.set_line_width(1.0);
        ctx.set_fill_style_str("#555");
        ctx.set_font("10px monospace");
        let mut freq_khz = first_grid;
        while freq_khz < max_freq_khz {
            let y = freq_to_y(freq_khz * 1000.0, min_freq, max_freq, ch);
            ctx.begin_path();
            ctx.move_to(LABEL_AREA_WIDTH, y);
            ctx.line_to(cw, y);
            ctx.stroke();
            freq_khz += interval;
        }

        // Dot size scaling based on zoom
        let dot_spacing_px = ZC_BIN_DURATION * px_per_sec;
        let radius_armed = (dot_spacing_px * 0.4).clamp(0.5, 3.0);
        let radius_unarmed = (dot_spacing_px * 0.3).clamp(0.4, 2.5);

        // Only iterate visible bins
        let end_time = start_time + visible_time;
        let first_bin = ((start_time / ZC_BIN_DURATION) as usize).saturating_sub(1);
        let last_bin = ((end_time / ZC_BIN_DURATION) as usize + 2).min(bins.len());

        // Batch armed dots
        ctx.set_fill_style_str("rgba(100, 200, 100, 0.9)");
        ctx.begin_path();
        for bin_idx in first_bin..last_bin {
            let (rate_hz, armed) = bins[bin_idx];
            if rate_hz <= 0.0 || !armed { continue; }
            if rate_hz < min_freq || rate_hz > max_freq { continue; }
            let bin_time = bin_idx as f64 * ZC_BIN_DURATION;
            let x = LABEL_AREA_WIDTH + (bin_time - start_time) * px_per_sec;
            let y = freq_to_y(rate_hz, min_freq, max_freq, ch);
            let _ = ctx.move_to(x + radius_armed, y);
            let _ = ctx.arc(x, y, radius_armed, 0.0, TAU);
        }
        ctx.fill();

        // Batch unarmed dots (dim green, visible but secondary)
        ctx.set_fill_style_str("rgba(60, 130, 60, 0.35)");
        ctx.begin_path();
        for bin_idx in first_bin..last_bin {
            let (rate_hz, armed) = bins[bin_idx];
            if rate_hz <= 0.0 || armed { continue; }
            if rate_hz < min_freq || rate_hz > max_freq { continue; }
            let bin_time = bin_idx as f64 * ZC_BIN_DURATION;
            let x = LABEL_AREA_WIDTH + (bin_time - start_time) * px_per_sec;
            let y = freq_to_y(rate_hz, min_freq, max_freq, ch);
            let _ = ctx.move_to(x + radius_unarmed, y);
            let _ = ctx.arc(x, y, radius_unarmed, 0.0, TAU);
        }
        ctx.fill();

        // Draw "play here" marker when not playing
        if !is_playing && canvas_tool == CanvasTool::Hand {
            let here_x = LABEL_AREA_WIDTH + dot_area_w * 0.10;
            let here_time = scroll + visible_time * 0.10;
            state.play_from_here_time.set(here_time);
            ctx.set_stroke_style_str("rgba(100, 160, 255, 0.35)");
            ctx.set_line_width(1.5);
            let _ = ctx.set_line_dash(&js_sys::Array::of2(
                &wasm_bindgen::JsValue::from_f64(4.0),
                &wasm_bindgen::JsValue::from_f64(3.0),
            ));
            ctx.begin_path();
            ctx.move_to(here_x, 0.0);
            ctx.line_to(here_x, ch);
            ctx.stroke();
            let _ = ctx.set_line_dash(&js_sys::Array::new());
        }

        ctx.restore(); // un-clip dot area

        // ── Left label area ────────────────────────────────────────────
        // Background
        ctx.set_fill_style_str("#0e0e0e");
        ctx.fill_rect(0.0, 0.0, LABEL_AREA_WIDTH, ch);

        // Separator line
        ctx.set_stroke_style_str("#333");
        ctx.set_line_width(1.0);
        ctx.begin_path();
        ctx.move_to(LABEL_AREA_WIDTH, 0.0);
        ctx.line_to(LABEL_AREA_WIDTH, ch);
        ctx.stroke();

        // Frequency labels
        ctx.set_fill_style_str("#888");
        ctx.set_font("10px monospace");
        ctx.set_text_align("right");
        let mut freq_khz2 = first_grid;
        while freq_khz2 < max_freq_khz {
            let y = freq_to_y(freq_khz2 * 1000.0, min_freq, max_freq, ch);
            if y > 6.0 && y < ch - 2.0 {
                let label = if freq_khz2.fract() == 0.0 {
                    format!("{:.0}k", freq_khz2)
                } else {
                    format!("{:.1}k", freq_khz2)
                };
                let _ = ctx.fill_text(&label, LABEL_AREA_WIDTH - 4.0, y + 3.5);
            }
            freq_khz2 += interval;
        }
        ctx.set_text_align("start"); // reset

        // FF range labels on axis
        if ff_hi > ff_lo {
            ctx.set_fill_style_str("rgba(80, 160, 255, 0.6)");
            ctx.set_font("9px monospace");
            let y_top = freq_to_y(ff_hi.min(max_freq), min_freq, max_freq, ch);
            let y_bot = freq_to_y(ff_lo.max(min_freq), min_freq, max_freq, ch);
            if y_top > 6.0 && y_top < ch - 2.0 {
                ctx.set_text_align("right");
                let _ = ctx.fill_text(&format!("{:.0}k", ff_hi / 1000.0), LABEL_AREA_WIDTH - 4.0, y_top - 2.0);
                ctx.set_text_align("start");
            }
            if y_bot > 6.0 && y_bot < ch - 2.0 {
                ctx.set_text_align("right");
                let _ = ctx.fill_text(&format!("{:.0}k", ff_lo / 1000.0), LABEL_AREA_WIDTH - 4.0, y_bot + 10.0);
                ctx.set_text_align("start");
            }
        }
    });

    // Auto-scroll to follow playhead during playback (with suspension support)
    Effect::new(move || {
        let playhead = state.playhead_time.get();
        let is_playing = state.is_playing.get();
        let follow = state.follow_cursor.get();
        let suspended = state.follow_suspended.get();

        if !follow { return; }
        if !is_playing {
            if suspended {
                state.follow_suspended.set(false);
                state.follow_visible_since.set(None);
            }
            return;
        }

        let Some(canvas_el) = canvas_ref.get() else { return };
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();
        let display_w = canvas.width() as f64;
        if display_w == 0.0 { return; }

        let files = state.files.get_untracked();
        let idx = state.current_file_index.get_untracked();
        let (time_res, duration) = idx
            .and_then(|i| files.get(i))
            .map(|f| (f.spectrogram.time_resolution, f.audio.duration_secs))
            .unwrap_or((1.0, 0.0));
        let zoom = state.zoom_level.get_untracked();
        let scroll = state.scroll_offset.get_untracked();

        let visible_time = (display_w / zoom) * time_res;
        let playhead_rel = playhead - scroll;

        if suspended {
            let playhead_visible = playhead_rel >= 0.0 && playhead_rel <= visible_time;
            if playhead_visible {
                let now = js_sys::Date::now();
                match state.follow_visible_since.get_untracked() {
                    None => { state.follow_visible_since.set(Some(now)); }
                    Some(since) if now - since >= 500.0 => {
                        state.follow_suspended.set(false);
                        state.follow_visible_since.set(None);
                    }
                    _ => {}
                }
            } else {
                state.follow_visible_since.set(None);
            }
            return;
        }

        if playhead_rel > visible_time * 0.8 || playhead_rel < 0.0 {
            let max_scroll = (duration - visible_time).max(0.0);
            state.scroll_offset.set((playhead - visible_time * 0.2).max(0.0).min(max_scroll));
        }
    });

    // Helper: get display freq range from canvas + state (untracked)
    let get_freq_range = move || -> (f64, f64) {
        let files = state.files.get_untracked();
        let idx = state.current_file_index.get_untracked();
        let file_max = idx.and_then(|i| files.get(i))
            .map(|f| f.spectrogram.max_freq)
            .unwrap_or(96_000.0);
        let min_freq = state.min_display_freq.get_untracked().unwrap_or(0.0);
        let max_freq = state.max_display_freq.get_untracked().unwrap_or(file_max);
        (min_freq, max_freq)
    };

    // Helper: convert mouse event to (px_x, px_y, freq)
    let mouse_to_xf = move |ev: &MouseEvent| -> Option<(f64, f64, f64)> {
        let canvas_el = canvas_ref.get()?;
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();
        let rect = canvas.get_bounding_client_rect();
        let px_x = ev.client_x() as f64 - rect.left();
        let px_y = ev.client_y() as f64 - rect.top();
        let ch = canvas.height() as f64;
        if ch <= 0.0 { return None; }
        let (min_freq, max_freq) = get_freq_range();
        let freq = y_to_freq(px_y, min_freq, max_freq, ch);
        Some((px_x, px_y, freq))
    };

    let on_wheel = move |ev: web_sys::WheelEvent| {
        ev.prevent_default();
        if ev.ctrl_key() {
            let delta = if ev.delta_y() > 0.0 { 0.9 } else { 1.1 };
            state.zoom_level.update(|z| *z = (*z * delta).max(0.1).min(100.0));
        } else {
            let delta = ev.delta_y() * 0.001;
            let max_scroll = {
                let files = state.files.get_untracked();
                let idx = state.current_file_index.get_untracked().unwrap_or(0);
                if let Some(file) = files.get(idx) {
                    let zoom = state.zoom_level.get_untracked();
                    let canvas_w = state.spectrogram_canvas_width.get_untracked();
                    let visible_time = (canvas_w / zoom) * file.spectrogram.time_resolution;
                    (file.audio.duration_secs - visible_time).max(0.0)
                } else {
                    f64::MAX
                }
            };
            state.suspend_follow();
            state.scroll_offset.update(|s| *s = (*s + delta).clamp(0.0, max_scroll));
        }
    };

    let on_mousedown = move |ev: MouseEvent| {
        if ev.button() != 0 { return; }

        // Check for axis drag (left label area)
        if let Some((px_x, _px_y, freq)) = mouse_to_xf(&ev) {
            if px_x < LABEL_AREA_WIDTH {
                let snap = if ev.shift_key() { 10_000.0 } else { 5_000.0 };
                let snapped = (freq / snap).round() * snap;
                axis_drag_raw_start.set(freq);
                state.axis_drag_start_freq.set(Some(snapped));
                state.axis_drag_current_freq.set(Some(snapped));
                state.is_dragging.set(true);
                ev.prevent_default();
                return;
            }
        }

        if state.canvas_tool.get_untracked() != CanvasTool::Hand { return; }
        if state.is_playing.get_untracked() {
            let t = state.playhead_time.get_untracked();
            state.bookmarks.update(|bm| bm.push(crate::state::Bookmark { time: t }));
            return;
        }
        state.is_dragging.set(true);
        hand_drag_start.set((ev.client_x() as f64, state.scroll_offset.get_untracked()));
    };

    let on_mousemove = move |ev: MouseEvent| {
        if !state.is_dragging.get_untracked() { return; }

        // Axis drag takes priority
        if state.axis_drag_start_freq.get_untracked().is_some() {
            if let Some((_px_x, _px_y, freq)) = mouse_to_xf(&ev) {
                let raw_start = axis_drag_raw_start.get_untracked();
                let snap = if ev.shift_key() { 10_000.0 } else { 5_000.0 };
                let (snapped_start, snapped_end) = if freq > raw_start {
                    ((raw_start / snap).floor() * snap, (freq / snap).ceil() * snap)
                } else if freq < raw_start {
                    ((raw_start / snap).ceil() * snap, (freq / snap).floor() * snap)
                } else {
                    let s = (raw_start / snap).round() * snap;
                    (s, s)
                };
                state.axis_drag_start_freq.set(Some(snapped_start));
                state.axis_drag_current_freq.set(Some(snapped_end));
                let lo = snapped_start.min(snapped_end);
                let hi = snapped_start.max(snapped_end);
                if hi - lo > 500.0 {
                    state.ff_freq_lo.set(lo);
                    state.ff_freq_hi.set(hi);
                }
            }
            return;
        }

        if state.canvas_tool.get_untracked() != CanvasTool::Hand { return; }
        let (start_client_x, start_scroll) = hand_drag_start.get_untracked();
        let dx = ev.client_x() as f64 - start_client_x;
        let cw = state.spectrogram_canvas_width.get_untracked();
        if cw == 0.0 { return; }
        let files = state.files.get_untracked();
        let idx = state.current_file_index.get_untracked();
        let file = idx.and_then(|i| files.get(i));
        let time_res = file.as_ref().map(|f| f.spectrogram.time_resolution).unwrap_or(1.0);
        let zoom = state.zoom_level.get_untracked();
        let visible_time = (cw / zoom) * time_res;
        let duration = file.as_ref().map(|f| f.audio.duration_secs).unwrap_or(f64::MAX);
        let max_scroll = (duration - visible_time).max(0.0);
        let dt = -(dx / cw) * visible_time;
        state.suspend_follow();
        state.scroll_offset.set((start_scroll + dt).clamp(0.0, max_scroll));
    };

    let on_mouseup = move |_ev: MouseEvent| {
        // End axis drag
        if state.axis_drag_start_freq.get_untracked().is_some() {
            let lo = state.ff_freq_lo.get_untracked();
            let hi = state.ff_freq_hi.get_untracked();
            if hi - lo > 500.0 && !state.hfr_enabled.get_untracked() {
                state.hfr_saved_ff_lo.set(Some(lo));
                state.hfr_saved_ff_hi.set(Some(hi));
                state.hfr_enabled.set(true);
            }
            state.axis_drag_start_freq.set(None);
            state.axis_drag_current_freq.set(None);
            state.is_dragging.set(false);
            return;
        }
        state.is_dragging.set(false);
    };

    let on_mouseleave = move |_ev: MouseEvent| {
        if state.axis_drag_start_freq.get_untracked().is_some() {
            state.axis_drag_start_freq.set(None);
            state.axis_drag_current_freq.set(None);
        }
        state.is_dragging.set(false);
    };

    // Touch event handlers (mobile)
    let on_touchstart = move |ev: web_sys::TouchEvent| {
        let touches = ev.touches();
        let n = touches.length();

        if n == 2 {
            ev.prevent_default();
            use crate::components::pinch::{two_finger_geometry, PinchState};
            if let Some((mid_x, dist)) = two_finger_geometry(&touches) {
                let files = state.files.get_untracked();
                let idx = state.current_file_index.get_untracked();
                let file = idx.and_then(|i| files.get(i));
                let time_res = file.as_ref().map(|f| f.spectrogram.time_resolution).unwrap_or(1.0);
                let duration = file.as_ref().map(|f| f.audio.duration_secs).unwrap_or(f64::MAX);
                pinch_state.set(Some(PinchState {
                    initial_dist: dist,
                    initial_zoom: state.zoom_level.get_untracked(),
                    initial_scroll: state.scroll_offset.get_untracked(),
                    initial_mid_client_x: mid_x,
                    time_res,
                    duration,
                }));
            }
            state.is_dragging.set(false);
            return;
        }

        if n != 1 { return; }
        pinch_state.set(None);

        let touch = touches.get(0).unwrap();
        if state.canvas_tool.get_untracked() != CanvasTool::Hand { return; }
        if state.is_playing.get_untracked() {
            let t = state.playhead_time.get_untracked();
            state.bookmarks.update(|bm| bm.push(crate::state::Bookmark { time: t }));
            return;
        }
        ev.prevent_default();
        state.is_dragging.set(true);
        hand_drag_start.set((touch.client_x() as f64, state.scroll_offset.get_untracked()));
    };

    let on_touchmove = move |ev: web_sys::TouchEvent| {
        let touches = ev.touches();
        let n = touches.length();

        if n == 2 {
            if let Some(ps) = pinch_state.get_untracked() {
                ev.prevent_default();
                use crate::components::pinch::{two_finger_geometry, apply_pinch};
                if let Some((mid_x, dist)) = two_finger_geometry(&touches) {
                    let Some(canvas_el) = canvas_ref.get() else { return };
                    let canvas: &HtmlCanvasElement = canvas_el.as_ref();
                    let rect = canvas.get_bounding_client_rect();
                    let cw = canvas.width() as f64;
                    let (new_zoom, new_scroll) = apply_pinch(&ps, dist, mid_x, rect.left(), cw);
                    state.suspend_follow();
                    state.zoom_level.set(new_zoom);
                    state.scroll_offset.set(new_scroll);
                }
            }
            return;
        }

        if n != 1 { return; }
        let touch = touches.get(0).unwrap();
        if !state.is_dragging.get_untracked() { return; }
        if state.canvas_tool.get_untracked() != CanvasTool::Hand { return; }
        ev.prevent_default();
        let (start_client_x, start_scroll) = hand_drag_start.get_untracked();
        let dx = touch.client_x() as f64 - start_client_x;
        let cw = state.spectrogram_canvas_width.get_untracked();
        if cw == 0.0 { return; }
        let files = state.files.get_untracked();
        let idx = state.current_file_index.get_untracked();
        let file = idx.and_then(|i| files.get(i));
        let time_res = file.as_ref().map(|f| f.spectrogram.time_resolution).unwrap_or(1.0);
        let zoom = state.zoom_level.get_untracked();
        let visible_time = (cw / zoom) * time_res;
        let duration = file.as_ref().map(|f| f.audio.duration_secs).unwrap_or(f64::MAX);
        let max_scroll = (duration - visible_time).max(0.0);
        let dt = -(dx / cw) * visible_time;
        state.suspend_follow();
        state.scroll_offset.set((start_scroll + dt).clamp(0.0, max_scroll));
    };

    let on_touchend = move |_ev: web_sys::TouchEvent| {
        let remaining = _ev.touches().length();
        if remaining < 2 {
            pinch_state.set(None);
        }
        if remaining == 1 {
            if let Some(touch) = _ev.touches().get(0) {
                hand_drag_start.set((touch.client_x() as f64, state.scroll_offset.get_untracked()));
                if state.canvas_tool.get_untracked() == CanvasTool::Hand {
                    state.is_dragging.set(true);
                }
            }
            return;
        }
        if remaining == 0 {
            state.is_dragging.set(false);
        }
    };

    view! {
        <div class="waveform-container"
            style=move || match state.canvas_tool.get() {
                CanvasTool::Hand => if state.is_dragging.get() {
                    "cursor: grabbing; touch-action: none;"
                } else {
                    "cursor: grab; touch-action: none;"
                },
                CanvasTool::Selection => "cursor: crosshair; touch-action: none;",
            }
        >
            <canvas
                node_ref=canvas_ref
                on:wheel=on_wheel
                on:mousedown=on_mousedown
                on:mousemove=on_mousemove
                on:mouseup=on_mouseup
                on:mouseleave=on_mouseleave
                on:touchstart=on_touchstart
                on:touchmove=on_touchmove
                on:touchend=on_touchend
            />
            // DOM playhead overlay
            <div
                class="playhead-line"
                style:transform=move || {
                    let playhead = state.playhead_time.get();
                    let scroll = state.scroll_offset.get();
                    let zoom = state.zoom_level.get();
                    let cw = state.spectrogram_canvas_width.get();
                    let files = state.files.get_untracked();
                    let idx = state.current_file_index.get_untracked();
                    let time_res = idx.and_then(|i| files.get(i))
                        .map(|f| f.spectrogram.time_resolution)
                        .unwrap_or(1.0);
                    let dot_area_w = (cw - LABEL_AREA_WIDTH).max(0.0);
                    let visible_time = (dot_area_w / zoom) * time_res;
                    let px_per_sec = if visible_time > 0.0 { dot_area_w / visible_time } else { 0.0 };
                    let x = LABEL_AREA_WIDTH + (playhead - scroll) * px_per_sec;
                    format!("translateX({:.1}px)", x)
                }
                style:display=move || if state.is_playing.get() { "block" } else { "none" }
            />
        </div>
    }
}
