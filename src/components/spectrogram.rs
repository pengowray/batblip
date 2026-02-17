use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, MouseEvent};
use crate::canvas::spectrogram_renderer::{self, FreqShiftMode, MovementAlgo, MovementData, PreRendered};
use crate::state::{AppState, PlaybackMode, Selection, SpectrogramDisplay};

#[component]
pub fn Spectrogram() -> impl IntoView {
    let state = expect_context::<AppState>();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();

    let pre_rendered: RwSignal<Option<PreRendered>> = RwSignal::new(None);
    let movement_cache: RwSignal<Option<MovementData>> = RwSignal::new(None);

    // Drag state for selection
    let drag_start = RwSignal::new((0.0f64, 0.0f64));

    // Effect 1 (expensive): recompute when file or algorithm changes
    Effect::new(move || {
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let display = state.spectrogram_display.get();
        let enabled = state.mv_enabled.get();
        if let Some(i) = idx {
            if let Some(file) = files.get(i) {
                if file.spectrogram.columns.is_empty() {
                    // Full spectrogram not yet computed; show preview as placeholder
                    movement_cache.set(None);
                    if let Some(ref pv) = file.preview {
                        pre_rendered.set(Some(PreRendered {
                            width: pv.width,
                            height: pv.height,
                            pixels: pv.pixels.clone(),
                        }));
                    } else {
                        pre_rendered.set(None);
                    }
                } else if !enabled {
                    movement_cache.set(None);
                    pre_rendered.set(Some(spectrogram_renderer::pre_render(&file.spectrogram)));
                } else {
                    let algo = match display {
                        SpectrogramDisplay::MovementCentroid => MovementAlgo::Centroid,
                        SpectrogramDisplay::MovementGradient => MovementAlgo::Gradient,
                        SpectrogramDisplay::MovementFlow => MovementAlgo::Flow,
                    };
                    let md = spectrogram_renderer::compute_movement_data(&file.spectrogram, algo);
                    let ig = state.mv_intensity_gate.get_untracked();
                    let mg = state.mv_movement_gate.get_untracked();
                    let op = state.mv_opacity.get_untracked();
                    pre_rendered.set(Some(spectrogram_renderer::composite_movement(&md, ig, mg, op)));
                    movement_cache.set(Some(md));
                }
            }
        } else {
            movement_cache.set(None);
            pre_rendered.set(None);
        }
    });

    // Effect 2 (cheap): re-composite when gate/opacity sliders change
    Effect::new(move || {
        let ig = state.mv_intensity_gate.get();
        let mg = state.mv_movement_gate.get();
        let op = state.mv_opacity.get();
        // Only run if we have cached movement data (i.e., movement mode is active)
        movement_cache.with_untracked(|mc| {
            if let Some(md) = mc {
                pre_rendered.set(Some(spectrogram_renderer::composite_movement(md, ig, mg, op)));
            }
        });
    });

    // Effect 3: redraw when pre-rendered data, scroll, zoom, selection, playhead, or overlays change
    Effect::new(move || {
        let scroll = state.scroll_offset.get();
        let zoom = state.zoom_level.get();
        let selection = state.selection.get();
        let playhead = state.playhead_time.get();
        let is_playing = state.is_playing.get();
        let het_interacting = state.het_interacting.get();
        let dragging = state.is_dragging.get();
        let het_freq = state.het_frequency.get();
        let te_factor = state.te_factor.get();
        let ps_factor = state.ps_factor.get();
        let playback_mode = state.playback_mode.get();
        let max_display_freq = state.max_display_freq.get();
        let _pre = pre_rendered.track();

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

        pre_rendered.with_untracked(|pr| {
            if let Some(rendered) = pr {
                let files = state.files.get_untracked();
                let idx = state.current_file_index.get_untracked();
                let time_res = idx
                    .and_then(|i| files.get(i))
                    .map(|f| f.spectrogram.time_resolution)
                    .unwrap_or(1.0);
                let scroll_col = scroll / time_res;

                let file_max_freq = idx
                    .and_then(|i| files.get(i))
                    .map(|f| f.spectrogram.max_freq)
                    .unwrap_or(96_000.0);
                let max_freq = max_display_freq.unwrap_or(file_max_freq);
                let freq_crop = max_freq / file_max_freq;

                spectrogram_renderer::blit_viewport(&ctx, rendered, canvas, scroll_col, zoom, freq_crop);

                // Determine frequency shift mode for marker labels
                let show_het = het_interacting
                    || (playback_mode == PlaybackMode::Heterodyne && is_playing);
                let shift_mode = if show_het {
                    FreqShiftMode::Heterodyne(het_freq)
                } else {
                    match playback_mode {
                        PlaybackMode::TimeExpansion => FreqShiftMode::Divide(te_factor),
                        PlaybackMode::PitchShift => FreqShiftMode::Divide(ps_factor),
                        _ => FreqShiftMode::None,
                    }
                };

                spectrogram_renderer::draw_freq_markers(
                    &ctx,
                    max_freq,
                    display_h as f64,
                    display_w as f64,
                    shift_mode,
                );

                if show_het {
                    spectrogram_renderer::draw_het_overlay(
                        &ctx,
                        het_freq,
                        max_freq,
                        display_h as f64,
                        display_w as f64,
                    );
                }

                // Draw selection overlay
                if let Some(sel) = selection {
                    spectrogram_renderer::draw_selection(
                        &ctx,
                        &sel,
                        max_freq,
                        scroll,
                        time_res,
                        zoom,
                        display_w as f64,
                        display_h as f64,
                    );
                    if dragging {
                        spectrogram_renderer::draw_harmonic_shadows(
                            &ctx,
                            &sel,
                            max_freq,
                            scroll,
                            time_res,
                            zoom,
                            display_w as f64,
                            display_h as f64,
                        );
                    }
                }

                // Draw playhead
                if is_playing {
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
                ctx.set_fill_style_str("#000");
                ctx.fill_rect(0.0, 0.0, display_w as f64, display_h as f64);
            }
        });
    });

    // Helper to get time/freq from mouse event
    let mouse_to_tf = move |ev: &MouseEvent| -> Option<(f64, f64)> {
        let canvas_el = canvas_ref.get()?;
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();
        let rect = canvas.get_bounding_client_rect();
        let px_x = ev.client_x() as f64 - rect.left();
        let px_y = ev.client_y() as f64 - rect.top();
        let cw = canvas.width() as f64;
        let ch = canvas.height() as f64;

        let files = state.files.get_untracked();
        let idx = state.current_file_index.get_untracked()?;
        let file = files.get(idx)?;
        let time_res = file.spectrogram.time_resolution;
        let file_max_freq = file.spectrogram.max_freq;
        let max_freq = state.max_display_freq.get_untracked()
            .unwrap_or(file_max_freq);
        let scroll = state.scroll_offset.get_untracked();
        let zoom = state.zoom_level.get_untracked();

        Some(spectrogram_renderer::pixel_to_time_freq(
            px_x, px_y, max_freq, scroll, time_res, zoom, cw, ch,
        ))
    };

    let on_mousedown = move |ev: MouseEvent| {
        if ev.button() != 0 { return; }
        if let Some((t, f)) = mouse_to_tf(&ev) {
            state.is_dragging.set(true);
            drag_start.set((t, f));
            state.selection.set(None);
        }
    };

    let on_mousemove = move |ev: MouseEvent| {
        if !state.is_dragging.get_untracked() { return; }
        if let Some((t, f)) = mouse_to_tf(&ev) {
            let (t0, f0) = drag_start.get_untracked();
            state.selection.set(Some(Selection {
                time_start: t0.min(t),
                time_end: t0.max(t),
                freq_low: f0.min(f),
                freq_high: f0.max(f),
            }));
        }
    };

    let on_mouseup = move |ev: MouseEvent| {
        if !state.is_dragging.get_untracked() { return; }
        state.is_dragging.set(false);
        if let Some((t, f)) = mouse_to_tf(&ev) {
            let (t0, f0) = drag_start.get_untracked();
            let sel = Selection {
                time_start: t0.min(t),
                time_end: t0.max(t),
                freq_low: f0.min(f),
                freq_high: f0.max(f),
            };
            if sel.time_end - sel.time_start > 0.0001 {
                state.selection.set(Some(sel));
            } else {
                state.selection.set(None);
            }
        }
    };

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
        <div class="spectrogram-container">
            <canvas
                node_ref=canvas_ref
                on:wheel=on_wheel
                on:mousedown=on_mousedown
                on:mousemove=on_mousemove
                on:mouseup=on_mouseup
            />
        </div>
    }
}
