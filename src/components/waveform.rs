use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};
use crate::canvas::waveform_renderer;
use crate::dsp::filters::{apply_eq_filter, apply_eq_filter_fast};
use crate::dsp::zc_divide::zc_rate_per_bin;
use crate::state::{AppState, FilterQuality, PlaybackMode};

const ZC_BIN_DURATION: f64 = 0.001; // 1ms bins

#[component]
pub fn Waveform() -> impl IntoView {
    let state = expect_context::<AppState>();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();

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
                file.audio.samples.clone()
            };
            zc_rate_per_bin(&samples, sr, ZC_BIN_DURATION, filter_enabled)
        })
    });

    Effect::new(move || {
        let scroll = state.scroll_offset.get();
        let zoom = state.zoom_level.get();
        let selection = state.selection.get();
        let playhead = state.playhead_time.get();
        let is_playing = state.is_playing.get();
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let mode = state.playback_mode.get();

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
                );
            }

            // Draw playhead
            if is_playing {
                let time_res = file.spectrogram.time_resolution;
                let visible_time = (display_w as f64 / zoom) * time_res;
                let px_per_sec = display_w as f64 / visible_time;
                let x = (playhead - scroll) * px_per_sec;
                if x >= 0.0 && x <= display_w as f64 {
                    ctx.set_stroke_style_str("rgba(255, 80, 80, 0.9)");
                    ctx.set_line_width(2.0);
                    ctx.begin_path();
                    ctx.move_to(x, 0.0);
                    ctx.line_to(x, display_h as f64);
                    ctx.stroke();
                }
            }
        } else {
            ctx.set_fill_style_str("#0a0a0a");
            ctx.fill_rect(0.0, 0.0, display_w as f64, display_h as f64);
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
            state.scroll_offset.update(|s| {
                *s = (*s + delta).max(0.0);
            });
        }
    };

    view! {
        <div class="waveform-container">
            <canvas
                node_ref=canvas_ref
                on:wheel=on_wheel
            />
        </div>
    }
}
