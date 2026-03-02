use leptos::prelude::*;
use leptos::ev::MouseEvent;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};
use crate::canvas::waveform_renderer;
use crate::dsp::filters::{apply_eq_filter, apply_eq_filter_fast, cascaded_lowpass};
use crate::dsp::zc_divide::zc_rate_per_bin;
use crate::state::{AppState, CanvasTool, FilterQuality, PlaybackMode};

const ZC_BIN_DURATION: f64 = 0.001; // 1ms bins

#[component]
pub fn Waveform() -> impl IntoView {
    let state = expect_context::<AppState>();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();
    let hand_drag_start = RwSignal::new((0.0f64, 0.0f64));
    let pinch_state: RwSignal<Option<crate::components::pinch::PinchState>> = RwSignal::new(None);

    // Cache ZC bins — recompute when the file or EQ settings change.
    let zc_bins = Memo::new(move |_| {
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let filter_enabled = state.filter_enabled.get();
        // Subscribe to EQ params so memo recomputes when they change
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

    // HFR highpass-filtered samples for waveform overlay
    let hfr_filtered = Memo::new(move |_| {
        let hfr = state.hfr_enabled.get();
        if !hfr { return None; }
        let ff_lo = state.ff_freq_lo.get();
        if ff_lo <= 0.0 { return None; }
        let files = state.files.get();
        let idx = state.current_file_index.get();

        idx.and_then(|i| files.get(i).cloned()).map(|file| {
            let sr = file.audio.sample_rate;
            let lp = cascaded_lowpass(&file.audio.samples, ff_lo, sr, 4);
            file.audio.samples.iter().zip(lp.iter())
                .map(|(s, l)| s - l)
                .collect::<Vec<f32>>()
        })
    });

    Effect::new(move || {
        let scroll = state.scroll_offset.get();
        let zoom = state.zoom_level.get();
        let selection = state.selection.get();
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let mode = state.playback_mode.get();
        let hfr = state.hfr_enabled.get();
        let is_playing = state.is_playing.get();
        let canvas_tool = state.canvas_tool.get();
        let auto_gain = state.auto_gain.get();
        let gain_db = if auto_gain {
            state.compute_auto_gain()
        } else {
            state.gain_db.get()
        };

        let Some(canvas_el) = canvas_ref.get() else { return };
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();

        let rect = canvas.get_bounding_client_rect();
        let display_w = rect.width() as u32;
        let display_h = rect.height() as u32;
        if display_w == 0 || display_h == 0 {
            return;
        }
        if canvas.width() != display_w || canvas.height() != display_h {
            canvas.set_width(display_w);
            canvas.set_height(display_h);
        }

        let ctx = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into::<CanvasRenderingContext2d>()
            .unwrap();

        if let Some(file) = idx.and_then(|i| files.get(i)) {
            let sel_time = selection.map(|s| (s.time_start, s.time_end));
            let max_freq_khz = file.spectrogram.max_freq / 1000.0;

            if mode == PlaybackMode::ZeroCrossing {
                if let Some(bins) = zc_bins.get().as_ref() {
                    waveform_renderer::draw_zc_rate(
                        &ctx,
                        bins,
                        ZC_BIN_DURATION,
                        file.audio.duration_secs,
                        scroll,
                        zoom,
                        file.spectrogram.time_resolution,
                        display_w as f64,
                        display_h as f64,
                        sel_time,
                        max_freq_khz,
                    );
                }
            } else if hfr {
                if let Some(filtered) = hfr_filtered.get().as_ref() {
                    waveform_renderer::draw_waveform_hfr(
                        &ctx,
                        &file.audio.samples,
                        filtered,
                        file.audio.sample_rate,
                        scroll,
                        zoom,
                        file.spectrogram.time_resolution,
                        display_w as f64,
                        display_h as f64,
                        sel_time,
                        gain_db,
                    );
                } else {
                    waveform_renderer::draw_waveform(
                        &ctx,
                        &file.audio.samples,
                        file.audio.sample_rate,
                        scroll,
                        zoom,
                        file.spectrogram.time_resolution,
                        display_w as f64,
                        display_h as f64,
                        sel_time,
                        gain_db,
                    );
                }
            } else {
                waveform_renderer::draw_waveform(
                    &ctx,
                    &file.audio.samples,
                    file.audio.sample_rate,
                    scroll,
                    zoom,
                    file.spectrogram.time_resolution,
                    display_w as f64,
                    display_h as f64,
                    sel_time,
                    gain_db,
                );
            }

            // Time markers along the bottom edge
            {
                let visible_time = (display_w as f64 / zoom) * file.spectrogram.time_resolution;
                crate::canvas::time_markers::draw_time_markers(
                    &ctx,
                    scroll,
                    visible_time,
                    display_w as f64,
                    display_h as f64,
                    file.audio.duration_secs,
                );
            }

            // Draw "play here" marker when not playing
            if !is_playing && canvas_tool == CanvasTool::Hand {
                let visible_time = (display_w as f64 / zoom) * file.spectrogram.time_resolution;
                let here_x = display_w as f64 * 0.10;
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
                ctx.line_to(here_x, display_h as f64);
                ctx.stroke();
                let _ = ctx.set_line_dash(&js_sys::Array::new());
            }

        } else {
            ctx.set_fill_style_str("#0a0a0a");
            ctx.fill_rect(0.0, 0.0, display_w as f64, display_h as f64);
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

    let on_wheel = move |ev: web_sys::WheelEvent| {
        ev.prevent_default();
        if ev.ctrl_key() {
            let delta = if ev.delta_y() > 0.0 { 0.9 } else { 1.1 };
            state.zoom_level.update(|z| {
                *z = (*z * delta).max(0.1).min(100.0);
            });
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
            state.scroll_offset.update(|s| {
                *s = (*s + delta).clamp(0.0, max_scroll);
            });
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
        state.suspend_follow();
        state.scroll_offset.set((start_scroll + dt).clamp(0.0, max_scroll));
    };

    let on_mouseup = move |_ev: MouseEvent| {
        state.is_dragging.set(false);
    };

    let on_mouseleave = move |_ev: MouseEvent| {
        state.is_dragging.set(false);
    };

    // ── Touch event handlers (mobile) ──────────────────────────────────────────
    let on_touchstart = move |ev: web_sys::TouchEvent| {
        let touches = ev.touches();
        let n = touches.length();

        // Two-finger: initialize pinch-to-zoom
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

        // Two-finger pinch/pan
        if n == 2 {
            if let Some(ps) = pinch_state.get_untracked() {
                ev.prevent_default();
                use crate::components::pinch::{two_finger_geometry, apply_pinch};
                if let Some((mid_x, dist)) = two_finger_geometry(&touches) {
                    let Some(canvas_el) = canvas_ref.get() else { return };
                    let canvas: &web_sys::HtmlCanvasElement = canvas_el.as_ref();
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
            // DOM playhead overlay — decoupled from heavy canvas redraws
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
