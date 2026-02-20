use leptos::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use wasm_bindgen::closure::Closure;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData, MouseEvent};
use crate::canvas::spectrogram_renderer::{self, FreqMarkerState, FreqShiftMode, MovementAlgo, MovementData, PreRendered};
use crate::dsp::harmonics;
use crate::state::{AppState, PlaybackMode, Selection, SidebarTab, SpectrogramDisplay};

const LABEL_AREA_WIDTH: f64 = 60.0;

#[component]
pub fn Spectrogram() -> impl IntoView {
    let state = expect_context::<AppState>();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();

    let pre_rendered: RwSignal<Option<PreRendered>> = RwSignal::new(None);
    let movement_cache: RwSignal<Option<MovementData>> = RwSignal::new(None);

    // Phase coherence heatmap data — computed only when Harmonics tab is active.
    let coherence_frames: RwSignal<Option<Vec<Vec<f32>>>> = RwSignal::new(None);

    // Drag state for selection
    let drag_start = RwSignal::new((0.0f64, 0.0f64));

    // Label hover animation: lerp label_hover_opacity toward target
    let label_hover_target = RwSignal::new(0.0f64);
    Effect::new(move || {
        let target = label_hover_target.get();
        let current = state.label_hover_opacity.get_untracked();
        if (current - target).abs() < 0.01 {
            // Snap to target
            if current != target {
                state.label_hover_opacity.set(target);
            }
            return;
        }
        // Schedule animation frame
        let cb = Closure::once(move || {
            let cur = state.label_hover_opacity.get_untracked();
            let tgt = label_hover_target.get_untracked();
            let lerp_speed = 0.15;
            let next = cur + (tgt - cur) * lerp_speed;
            let next = if (next - tgt).abs() < 0.01 { tgt } else { next };
            state.label_hover_opacity.set(next);
            // Re-trigger if not at target
            if next != tgt {
                label_hover_target.set(tgt);
            }
        });
        let _ = web_sys::window().unwrap().request_animation_frame(
            cb.as_ref().unchecked_ref(),
        );
        cb.forget();
    });

    // Effect 1 (expensive): recompute when file or algorithm changes
    Effect::new(move || {
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let display = state.spectrogram_display.get();
        let enabled = state.mv_enabled.get();
        if let Some(i) = idx {
            if let Some(file) = files.get(i) {
                if file.spectrogram.columns.is_empty() {
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
        movement_cache.with_untracked(|mc| {
            if let Some(md) = mc {
                pre_rendered.set(Some(spectrogram_renderer::composite_movement(md, ig, mg, op)));
            }
        });
    });

    // Effect 2b: compute phase coherence frames when the Harmonics tab becomes active or the file changes.
    // Only reads files/idx when the tab is Harmonics, so it doesn't run for every file change otherwise.
    Effect::new(move || {
        let tab = state.sidebar_tab.get();
        if tab != SidebarTab::Harmonics {
            return;
        }
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let frames = idx.and_then(|i| files.get(i).cloned()).map(|file| {
            harmonics::compute_coherence_frames(&file.audio, &file.spectrogram)
        });
        coherence_frames.set(frames);
    });

    // Effect 3: redraw when pre-rendered data, scroll, zoom, selection, playhead, overlays, or hover change
    Effect::new(move || {
        let scroll = state.scroll_offset.get();
        let zoom = state.zoom_level.get();
        let selection = state.selection.get();
        let playhead = state.playhead_time.get();
        let is_playing = state.is_playing.get();
        let het_interacting = state.het_interacting.get();
        let dragging = state.is_dragging.get();
        let het_freq = state.het_frequency.get();
        let het_cutoff = state.het_cutoff.get();
        let te_factor = state.te_factor.get();
        let ps_factor = state.ps_factor.get();
        let playback_mode = state.playback_mode.get();
        let max_display_freq = state.max_display_freq.get();
        let mouse_freq = state.mouse_freq.get();
        let mouse_cx = state.mouse_canvas_x.get();
        let label_opacity = state.label_hover_opacity.get();
        let filter_hovering = state.filter_hovering_band.get();
        let filter_enabled = state.filter_enabled.get();
        let sidebar_tab = state.sidebar_tab.get();
        let _pre = pre_rendered.track();
        let _coh = coherence_frames.track();

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

        if sidebar_tab == SidebarTab::Harmonics {
            // --- Phase coherence heatmap mode ---
            coherence_frames.with_untracked(|cf| {
                match cf {
                    Some(frames) if !frames.is_empty() => {
                        draw_coherence_heatmap(
                            &ctx,
                            frames,
                            display_w,
                            display_h,
                            scroll_col,
                            zoom,
                            freq_crop,
                        );
                    }
                    _ => {
                        // Coherence not yet computed — show dim background
                        ctx.set_fill_style_str("#0a0a0a");
                        ctx.fill_rect(0.0, 0.0, display_w as f64, display_h as f64);
                        ctx.set_fill_style_str("#444");
                        ctx.set_font("13px sans-serif");
                        let _ = ctx.fill_text(
                            "Computing phase coherence…",
                            display_w as f64 / 2.0 - 100.0,
                            display_h as f64 / 2.0,
                        );
                    }
                }
            });

            // Always draw freq markers and playhead on top of the heatmap.
            let marker_state = FreqMarkerState {
                mouse_freq,
                mouse_in_label_area: mouse_freq.is_some() && mouse_cx < LABEL_AREA_WIDTH,
                label_hover_opacity: label_opacity,
                has_selection: selection.is_some() || dragging,
                file_max_freq,
            };
            spectrogram_renderer::draw_freq_markers(
                &ctx,
                max_freq,
                display_h as f64,
                display_w as f64,
                FreqShiftMode::None,
                &marker_state,
                het_cutoff,
            );
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
            return;
        }

        // --- Normal spectrogram mode ---
        pre_rendered.with_untracked(|pr| {
            if let Some(rendered) = pr {
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
                        PlaybackMode::ZeroCrossing => FreqShiftMode::Divide(state.zc_factor.get()),
                        _ => FreqShiftMode::None,
                    }
                };

                let marker_state = FreqMarkerState {
                    mouse_freq,
                    mouse_in_label_area: mouse_freq.is_some() && mouse_cx < LABEL_AREA_WIDTH,
                    label_hover_opacity: label_opacity,
                    has_selection: selection.is_some() || dragging,
                    file_max_freq,
                };

                spectrogram_renderer::draw_freq_markers(
                    &ctx,
                    max_freq,
                    display_h as f64,
                    display_w as f64,
                    shift_mode,
                    &marker_state,
                    het_cutoff,
                );

                if show_het {
                    spectrogram_renderer::draw_het_overlay(
                        &ctx,
                        het_freq,
                        het_cutoff,
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

                // Draw filter band overlay when hovering a slider
                if filter_enabled {
                    if let Some(band) = filter_hovering {
                        spectrogram_renderer::draw_filter_overlay(
                            &ctx,
                            band,
                            state.filter_freq_low.get_untracked(),
                            state.filter_freq_high.get_untracked(),
                            state.filter_band_mode.get_untracked(),
                            max_freq,
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

    // Effect 4: auto-scroll to follow playhead during playback
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
        let time_res = idx
            .and_then(|i| files.get(i))
            .map(|f| f.spectrogram.time_resolution)
            .unwrap_or(1.0);
        let zoom = state.zoom_level.get_untracked();
        let scroll = state.scroll_offset.get_untracked();

        let visible_time = (display_w / zoom) * time_res;
        let playhead_rel = playhead - scroll;

        if playhead_rel > visible_time * 0.8 || playhead_rel < 0.0 {
            state.scroll_offset.set((playhead - visible_time * 0.2).max(0.0));
        }
    });

    // Helper to get (px_x, time, freq) from mouse event
    let mouse_to_xtf = move |ev: &MouseEvent| -> Option<(f64, f64, f64)> {
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

        let (t, f) = spectrogram_renderer::pixel_to_time_freq(
            px_x, px_y, max_freq, scroll, time_res, zoom, cw, ch,
        );
        Some((px_x, t, f))
    };

    let on_mousedown = move |ev: MouseEvent| {
        if ev.button() != 0 { return; }
        if let Some((_, t, f)) = mouse_to_xtf(&ev) {
            state.is_dragging.set(true);
            drag_start.set((t, f));
            state.selection.set(None);
        }
    };

    let on_mousemove = move |ev: MouseEvent| {
        if let Some((px_x, t, f)) = mouse_to_xtf(&ev) {
            // Always track hover position
            state.mouse_freq.set(Some(f));
            state.mouse_canvas_x.set(px_x);

            // Update label hover target
            let in_label_area = px_x < LABEL_AREA_WIDTH;
            let current_target = label_hover_target.get_untracked();
            let new_target = if in_label_area { 1.0 } else { 0.0 };
            if current_target != new_target {
                label_hover_target.set(new_target);
            }

            // Update selection if dragging
            if state.is_dragging.get_untracked() {
                let (t0, f0) = drag_start.get_untracked();
                state.selection.set(Some(Selection {
                    time_start: t0.min(t),
                    time_end: t0.max(t),
                    freq_low: f0.min(f),
                    freq_high: f0.max(f),
                }));
            }
        }
    };

    let on_mouseleave = move |_ev: MouseEvent| {
        state.mouse_freq.set(None);
        label_hover_target.set(0.0);
    };

    let on_mouseup = move |ev: MouseEvent| {
        if !state.is_dragging.get_untracked() { return; }
        state.is_dragging.set(false);
        if let Some((_, t, f)) = mouse_to_xtf(&ev) {
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
                on:mouseleave=on_mouseleave
            />
        </div>
    }
}

// ---------------------------------------------------------------------------
// Phase coherence heatmap rendering helpers
// ---------------------------------------------------------------------------

/// Render a phase coherence heatmap onto the canvas.
/// `frames[t][k]` = coherence at frame-transition t, frequency bin k, in [0,1].
/// The viewport mapping mirrors blit_viewport: scroll_col + x/zoom gives the source frame.
fn draw_coherence_heatmap(
    ctx: &CanvasRenderingContext2d,
    frames: &[Vec<f32>],
    display_w: u32,
    display_h: u32,
    scroll_col: f64,
    zoom: f64,
    freq_crop: f64,
) {
    let w = display_w as usize;
    let h = display_h as usize;
    let n_frames = frames.len();
    let n_bins = frames.first().map(|f| f.len()).unwrap_or(0);

    if n_frames == 0 || n_bins == 0 || w == 0 || h == 0 {
        ctx.set_fill_style_str("#0a0a0a");
        ctx.fill_rect(0.0, 0.0, display_w as f64, display_h as f64);
        return;
    }

    // Initialise all pixels to opaque black; columns past the end of the file stay black.
    let mut pixels = vec![0u8; w * h * 4];
    for i in 0..w * h {
        pixels[i * 4 + 3] = 255;
    }

    for px_x in 0..w {
        // Map pixel column → source frame index.
        let frame_f = scroll_col + px_x as f64 / zoom;
        // Past the end of the recording → leave as black.
        if frame_f >= n_frames as f64 {
            break;
        }
        let frame_i = frame_f as usize;
        let frame_row = &frames[frame_i];

        for px_y in 0..h {
            // Map pixel row → frequency bin.
            // Row 0 (top) = max displayed freq, row h (bottom) = 0 Hz.
            // freq_crop = max_display_freq / file_max_freq, so max bin shown = n_bins * freq_crop.
            let bin_f = n_bins as f64 * freq_crop * (1.0 - px_y as f64 / h as f64);
            let bin_i = (bin_f as usize).min(n_bins - 1);

            let coherence = frame_row[bin_i];
            let [r, g, b] = coherence_to_rgb(coherence);
            let idx = (px_y * w + px_x) * 4;
            pixels[idx] = r;
            pixels[idx + 1] = g;
            pixels[idx + 2] = b;
            // alpha already 255
        }
    }

    let clamped = Clamped(pixels.as_slice());
    if let Ok(img) = ImageData::new_with_u8_clamped_array_and_sh(clamped, display_w, display_h) {
        let _ = ctx.put_image_data(&img, 0.0, 0.0);
    }
}

/// Map a coherence value [0,1] to an RGB colour.
/// Sequential blue scale: black → navy → steel blue → pale blue-white.
/// 0.00 → #000000 (black)
/// 0.40 → #0d3a6e (dark navy)
/// 0.70 → #2d7fc0 (steel blue)
/// 1.00 → #c8e8ff (pale ice blue)
fn coherence_to_rgb(c: f32) -> [u8; 3] {
    const STOPS: [(u8, u8, u8); 4] = [
        (0x00, 0x00, 0x00), // 0.00 — black
        (0x0d, 0x3a, 0x6e), // 0.40 — dark navy
        (0x2d, 0x7f, 0xc0), // 0.70 — steel blue
        (0xc8, 0xe8, 0xff), // 1.00 — pale ice blue
    ];
    let c = c.clamp(0.0, 1.0);
    let scaled = c * (STOPS.len() - 1) as f32;
    let lo = (scaled as usize).min(STOPS.len() - 2);
    let t = scaled - lo as f32;
    let (r0, g0, b0) = STOPS[lo];
    let (r1, g1, b1) = STOPS[lo + 1];
    [
        lerp_u8(r0, r1, t),
        lerp_u8(g0, g1, t),
        lerp_u8(b0, b1, t),
    ]
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}
