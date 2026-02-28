use leptos::prelude::*;
use leptos::ev::MouseEvent;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};
use crate::dsp::filters::{apply_eq_filter, apply_eq_filter_fast};
use crate::dsp::zc_divide::zc_rate_per_bin;
use crate::state::{AppState, CanvasTool, FilterQuality};

const ZC_BIN_DURATION: f64 = 0.001; // 1ms bins
const TAU: f64 = std::f64::consts::TAU;

#[component]
pub fn ZcDotChart() -> impl IntoView {
    let state = expect_context::<AppState>();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();
    let hand_drag_start = RwSignal::new((0.0f64, 0.0f64));

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
        let max_freq_khz = file.spectrogram.max_freq / 1000.0;
        let max_freq_hz = max_freq_khz * 1000.0;

        let visible_time = (cw / zoom) * time_res;
        let start_time = scroll.max(0.0).min((total_duration - visible_time).max(0.0));
        let px_per_sec = cw / visible_time;

        // Selection highlight
        if let Some(sel) = selection {
            let x0 = ((sel.time_start - start_time) * px_per_sec).max(0.0);
            let x1 = ((sel.time_end - start_time) * px_per_sec).min(cw);
            if x1 > x0 {
                ctx.set_fill_style_str("rgba(50, 120, 200, 0.2)");
                ctx.fill_rect(x0, 0.0, x1 - x0, ch);
            }
        }

        // Horizontal grid lines
        ctx.set_stroke_style_str("#222");
        ctx.set_line_width(1.0);
        ctx.set_fill_style_str("#555");
        ctx.set_font("10px monospace");
        let grid_freqs = [20.0, 40.0, 60.0, 80.0, 100.0, 120.0];
        for &freq_khz in &grid_freqs {
            if freq_khz >= max_freq_khz { break; }
            let y = ch * (1.0 - freq_khz / max_freq_khz);
            ctx.begin_path();
            ctx.move_to(0.0, y);
            ctx.line_to(cw, y);
            ctx.stroke();
            let _ = ctx.fill_text(&format!("{:.0}k", freq_khz), 2.0, y - 2.0);
        }

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
            let bin_time = bin_idx as f64 * ZC_BIN_DURATION;
            let x = (bin_time - start_time) * px_per_sec;
            let y = ch * (1.0 - rate_hz / max_freq_hz);
            let _ = ctx.move_to(x + 2.0, y);
            let _ = ctx.arc(x, y, 2.0, 0.0, TAU);
        }
        ctx.fill();

        // Batch unarmed dots
        ctx.set_fill_style_str("rgba(80, 80, 80, 0.4)");
        ctx.begin_path();
        for bin_idx in first_bin..last_bin {
            let (rate_hz, armed) = bins[bin_idx];
            if rate_hz <= 0.0 || armed { continue; }
            let bin_time = bin_idx as f64 * ZC_BIN_DURATION;
            let x = (bin_time - start_time) * px_per_sec;
            let y = ch * (1.0 - rate_hz / max_freq_hz);
            let _ = ctx.move_to(x + 1.5, y);
            let _ = ctx.arc(x, y, 1.5, 0.0, TAU);
        }
        ctx.fill();
    });

    // Auto-scroll to follow playhead during playback
    Effect::new(move || {
        let playhead = state.playhead_time.get();
        let is_playing = state.is_playing.get();
        let follow = state.follow_cursor.get();
        if !is_playing || !follow { return; }

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
        if playhead_rel > visible_time * 0.8 || playhead_rel < 0.0 {
            let max_scroll = (duration - visible_time).max(0.0);
            state.scroll_offset.set((playhead - visible_time * 0.2).max(0.0).min(max_scroll));
        }
    });

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
            state.scroll_offset.update(|s| *s = (*s + delta).clamp(0.0, max_scroll));
        }
    };

    let on_mousedown = move |ev: MouseEvent| {
        if ev.button() != 0 { return; }
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
        state.scroll_offset.set((start_scroll + dt).clamp(0.0, max_scroll));
    };

    let on_mouseup = move |_ev: MouseEvent| {
        state.is_dragging.set(false);
    };

    let on_mouseleave = move |_ev: MouseEvent| {
        state.is_dragging.set(false);
    };

    // Touch event handlers (mobile)
    let on_touchstart = move |ev: web_sys::TouchEvent| {
        let touches = ev.touches();
        if touches.length() != 1 { return; }
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
        if touches.length() != 1 { return; }
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
        state.scroll_offset.set((start_scroll + dt).clamp(0.0, max_scroll));
    };

    let on_touchend = move |_ev: web_sys::TouchEvent| {
        state.is_dragging.set(false);
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
                    let visible_time = (cw / zoom) * time_res;
                    let px_per_sec = if visible_time > 0.0 { cw / visible_time } else { 0.0 };
                    let x = (playhead - scroll) * px_per_sec;
                    format!("translateX({:.1}px)", x)
                }
                style:display=move || if state.is_playing.get() { "block" } else { "none" }
            />
        </div>
    }
}
