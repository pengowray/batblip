use leptos::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use wasm_bindgen::closure::Closure;
use std::cell::Cell;
use std::rc::Rc;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData, MouseEvent};
use crate::canvas::spectrogram_renderer::{self, Colormap, ColormapMode, FreqMarkerState, FreqShiftMode, MovementAlgo, MovementData, PreRendered};
use crate::dsp::harmonics;
use crate::state::{AppState, CanvasTool, ColormapPreference, SpectrogramHandle, PlaybackMode, Selection, RightSidebarTab, SpectrogramDisplay};

const LABEL_AREA_WIDTH: f64 = 60.0;

/// Hit-test all spectrogram overlay handles (FF + HET).
/// Returns the closest handle within `threshold` pixels, or None.
/// HET handles take priority over FF when they overlap and HET is manual.
fn hit_test_spec_handles(
    state: &AppState,
    mouse_y: f64,
    min_freq: f64,
    max_freq: f64,
    canvas_height: f64,
    threshold: f64,
) -> Option<SpectrogramHandle> {
    let mut candidates: Vec<(SpectrogramHandle, f64)> = Vec::new();

    // FF handles (always active when FF range is set)
    let ff_lo = state.ff_freq_lo.get_untracked();
    let ff_hi = state.ff_freq_hi.get_untracked();
    if ff_hi > ff_lo {
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

#[component]
pub fn Spectrogram() -> impl IntoView {
    let state = expect_context::<AppState>();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();

    let pre_rendered: RwSignal<Option<PreRendered>> = RwSignal::new(None);
    let movement_cache: RwSignal<Option<MovementData>> = RwSignal::new(None);

    // Phase coherence heatmap data — computed only when Harmonics tab is active.
    let coherence_frames: RwSignal<Option<Vec<Vec<f32>>>> = RwSignal::new(None);

    // Drag state for selection (time, freq)
    let drag_start = RwSignal::new((0.0f64, 0.0f64));
    // Hand-tool drag state: (initial_client_x, initial_scroll_offset)
    let hand_drag_start = RwSignal::new((0.0f64, 0.0f64));
    let axis_drag_raw_start = RwSignal::new(0.0f64);

    // Label hover animation: lerp label_hover_opacity toward target.
    // The Effect subscribes to BOTH label_hover_target and label_hover_opacity.
    // When the rAF callback sets opacity, the Effect re-runs automatically —
    // no need to re-trigger via setting the target signal.
    // A generation counter ensures stale rAF callbacks are discarded when a new
    // animation cycle starts (e.g. target changes mid-flight).
    let label_hover_target = RwSignal::new(0.0f64);
    let anim_gen: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    Effect::new(move || {
        let target = label_hover_target.get();
        let current = state.label_hover_opacity.get();
        if (current - target).abs() < 0.01 {
            if current != target {
                state.label_hover_opacity.set(target);
            }
            return;
        }
        let gen = anim_gen.get().wrapping_add(1);
        anim_gen.set(gen);
        let ag = anim_gen.clone();
        let cb = Closure::once(move || {
            if ag.get() != gen { return; }
            let cur = state.label_hover_opacity.get_untracked();
            let tgt = label_hover_target.get_untracked();
            let speed = if tgt > cur { 0.35 } else { 0.20 };
            let next = cur + (tgt - cur) * speed;
            let next = if (next - tgt).abs() < 0.01 { tgt } else { next };
            state.label_hover_opacity.set(next);
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
                    // Columns not loaded in memory.  Either:
                    //  - Still loading (total_columns > 0 but tiles rendering progressively), or
                    //  - Large file mode (columns kept in spectral store, not in SpectrogramData).
                    // Either way, don't store a monolithic PreRendered.  Effect 3 will use
                    // tile-based rendering or blit_preview_as_background() as fallback.
                    if enabled && file.spectrogram.total_columns > 0 {
                        // Large file — movement mode not available
                        state.show_info_toast("Movement mode not available for large files");
                        state.mv_enabled.set(false);
                    }
                    movement_cache.set(None);
                    pre_rendered.set(None);
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
        let tab = state.right_sidebar_tab.get();
        if tab != RightSidebarTab::Harmonics {
            return;
        }
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let frames = idx.and_then(|i| files.get(i).cloned()).map(|file| {
            harmonics::compute_coherence_frames(&file.audio, &file.spectrogram)
        });
        coherence_frames.set(frames);
    });

    // Effect 3: redraw when pre-rendered data, scroll, zoom, selection, playhead, overlays, hover, or new tile change
    Effect::new(move || {
        let _tile_ready = state.tile_ready_signal.get(); // trigger redraw when tiles arrive
        let scroll = state.scroll_offset.get();
        let zoom = state.zoom_level.get();
        let bookmarks = state.bookmarks.get();
        let canvas_tool = state.canvas_tool.get();
        let selection = state.selection.get();
        let is_playing = state.is_playing.get();
        let het_interacting = state.het_interacting.get();
        let dragging = state.is_dragging.get();
        let het_freq = state.het_frequency.get();
        let het_cutoff = state.het_cutoff.get();
        let te_factor = state.te_factor.get();
        let ps_factor = state.ps_factor.get();
        let playback_mode = state.playback_mode.get();
        let min_display_freq = state.min_display_freq.get();
        let max_display_freq = state.max_display_freq.get();
        let mouse_freq = state.mouse_freq.get();
        let mouse_cx = state.mouse_canvas_x.get();
        let label_opacity = state.label_hover_opacity.get();
        let filter_hovering = state.filter_hovering_band.get();
        let filter_enabled = state.filter_enabled.get();
        let sidebar_tab = state.right_sidebar_tab.get();
        let spec_hover = state.spec_hover_handle.get();
        let spec_drag = state.spec_drag_handle.get();
        let ff_lo = state.ff_freq_lo.get();
        let ff_hi = state.ff_freq_hi.get();
        let het_freq_auto = state.het_freq_auto.get();
        let het_cutoff_auto = state.het_cutoff_auto.get();
        let hfr_enabled = state.hfr_enabled.get();
        let mv_on = state.mv_enabled.get_untracked();
        let colormap_pref = state.colormap_preference.get();
        let hfr_colormap_pref = state.hfr_colormap_preference.get();
        let axis_drag_start = state.axis_drag_start_freq.get();
        let axis_drag_current = state.axis_drag_current_freq.get();
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
        // Keep overview in sync with actual canvas width
        state.spectrogram_canvas_width.set(display_w as f64);

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
        let min_freq = min_display_freq.unwrap_or(0.0);
        let freq_crop_lo = min_freq / file_max_freq;
        let freq_crop_hi = max_freq / file_max_freq;

        if sidebar_tab == RightSidebarTab::Harmonics {
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
                            freq_crop_lo,
                            freq_crop_hi,
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
            let (adl, adh) = match (axis_drag_start, axis_drag_current) {
                (Some(a), Some(b)) => (Some(a.min(b)), Some(a.max(b))),
                _ => (None, None),
            };
            let ff_drag_active = matches!(spec_drag, Some(SpectrogramHandle::FfUpper) | Some(SpectrogramHandle::FfLower) | Some(SpectrogramHandle::FfMiddle));
            let marker_state = FreqMarkerState {
                mouse_freq,
                mouse_in_label_area: mouse_freq.is_some() && mouse_cx < LABEL_AREA_WIDTH,
                label_hover_opacity: label_opacity,
                has_selection: selection.is_some() || (dragging && axis_drag_start.is_none()),
                file_max_freq,
                axis_drag_lo: adl,
                axis_drag_hi: adh,
                ff_drag_active,
                ff_lo,
                ff_hi,
                ff_handles_active: spec_hover.is_some() || spec_drag.is_some(),
            };
            spectrogram_renderer::draw_freq_markers(
                &ctx,
                min_freq,
                max_freq,
                display_h as f64,
                display_w as f64,
                FreqShiftMode::None,
                &marker_state,
                het_cutoff,
            );
            return;
        }

        // --- Normal spectrogram mode ---

        // Build colormap
        let pref_to_colormap = |p: ColormapPreference| -> Colormap {
            match p {
                ColormapPreference::Viridis => Colormap::Viridis,
                ColormapPreference::Inferno => Colormap::Inferno,
                ColormapPreference::Magma => Colormap::Magma,
                ColormapPreference::Plasma => Colormap::Plasma,
                ColormapPreference::Cividis => Colormap::Cividis,
                ColormapPreference::Turbo => Colormap::Turbo,
                ColormapPreference::Greyscale => Colormap::Greyscale,
            }
        };
        let colormap = if mv_on {
            ColormapMode::Uniform(Colormap::Greyscale)
        } else if hfr_enabled && ff_hi > ff_lo {
            ColormapMode::HfrFocus {
                colormap: pref_to_colormap(hfr_colormap_pref),
                ff_lo_frac: ff_lo / file_max_freq,
                ff_hi_frac: ff_hi / file_max_freq,
            }
        } else if hfr_enabled {
            ColormapMode::Uniform(pref_to_colormap(hfr_colormap_pref))
        } else {
            ColormapMode::Uniform(pref_to_colormap(colormap_pref))
        };

        let file = idx.and_then(|i| files.get(i));
        let total_cols = file.map(|f| {
            let tc = f.spectrogram.total_columns;
            if tc > 0 { tc } else { f.spectrogram.columns.len() }
        }).unwrap_or(0);
        let file_idx_val = idx.unwrap_or(0);
        let visible_time = (display_w as f64 / zoom) * time_res;
        let duration = file.map(|f| f.audio.duration_secs).unwrap_or(0.0);

        // Step 1: Render base spectrogram.
        // Priority: tiles (normal mode) > pre_rendered (movement mode) > preview > black
        let base_drawn = if !mv_on && total_cols > 0 {
            // Try tile-based rendering
            let drawn = spectrogram_renderer::blit_tiles_viewport(
                &ctx, canvas, file_idx_val, total_cols,
                scroll_col, zoom, freq_crop_lo, freq_crop_hi, colormap,
                file.and_then(|f| f.preview.as_ref()),
                scroll, visible_time, duration,
            );

            // Schedule any missing visible tiles for on-demand generation.
            // This handles large-file mode where columns may be evicted from
            // the spectral store and need STFT recomputation from audio.
            {
                use crate::canvas::tile_cache::{self, TILE_COLS};
                use crate::canvas::spectral_store;

                let visible_cols_f = display_w as f64 / zoom;
                let src_start = scroll_col.max(0.0);
                let src_end = (src_start + visible_cols_f).min(total_cols as f64);
                let first_tile = (src_start / TILE_COLS as f64).floor() as usize;
                let last_tile = ((src_end - 1.0).max(0.0) / TILE_COLS as f64).floor() as usize;
                let n_tiles = (total_cols + TILE_COLS - 1) / TILE_COLS;

                // Don't schedule on-demand tiles while loading is in progress —
                // the loading loop already schedules tiles as chunks complete.
                // Only schedule LOD 0 (fast blurry preview) for instant feedback.
                let is_loading = state.loading_count.get_untracked() > 0;

                for t in first_tile..=last_tile.min(n_tiles.saturating_sub(1)) {
                    if tile_cache::get_tile(file_idx_val, t).is_none() {
                        // Schedule LOD 0 (quick blurry preview) immediately
                        tile_cache::schedule_lod0_tile(state.clone(), file_idx_val, t);

                        if !is_loading {
                            // Schedule full-quality LOD 1 tile
                            let tile_start = t * TILE_COLS;
                            let tile_end = (tile_start + TILE_COLS).min(total_cols);
                            if spectral_store::has_store(file_idx_val)
                                && spectral_store::tile_complete(file_idx_val, tile_start, tile_end)
                            {
                                tile_cache::schedule_tile_from_store(state.clone(), file_idx_val, t);
                            } else {
                                tile_cache::schedule_tile_on_demand(state.clone(), file_idx_val, t);
                            }
                        }
                    }
                }
            }

            drawn
        } else if pre_rendered.with_untracked(|pr| pr.is_some()) {
            // Movement mode or no tile data — use monolithic pre_rendered
            pre_rendered.with_untracked(|pr| {
                if let Some(rendered) = pr {
                    spectrogram_renderer::blit_viewport(
                        &ctx, rendered, canvas, scroll_col, zoom,
                        freq_crop_lo, freq_crop_hi, colormap,
                    );
                }
            });
            true
        } else if let Some(pv) = file.and_then(|f| f.preview.as_ref()) {
            // Preview fallback during loading
            spectrogram_renderer::blit_preview_as_background(
                &ctx, pv, canvas,
                scroll, visible_time, duration,
                freq_crop_lo, freq_crop_hi,
                colormap,
            );
            true
        } else {
            ctx.set_fill_style_str("#000");
            ctx.fill_rect(0.0, 0.0, display_w as f64, display_h as f64);
            false
        };

        // Step 2: Draw overlays on top of the base spectrogram
        if base_drawn {
            let show_het = het_interacting
                || playback_mode == PlaybackMode::Heterodyne;
            let shift_mode = if show_het {
                FreqShiftMode::Heterodyne(het_freq)
            } else {
                match playback_mode {
                    PlaybackMode::TimeExpansion if te_factor > 1.0 => FreqShiftMode::Divide(te_factor),
                    PlaybackMode::TimeExpansion if te_factor < -1.0 => FreqShiftMode::Multiply(te_factor.abs()),
                    PlaybackMode::PitchShift if ps_factor > 1.0 => FreqShiftMode::Divide(ps_factor),
                    PlaybackMode::PitchShift if ps_factor < -1.0 => FreqShiftMode::Multiply(ps_factor.abs()),
                    PlaybackMode::ZeroCrossing => FreqShiftMode::Divide(state.zc_factor.get()),
                    _ => FreqShiftMode::None,
                }
            };

            let (adl2, adh2) = match (axis_drag_start, axis_drag_current) {
                (Some(a), Some(b)) => (Some(a.min(b)), Some(a.max(b))),
                _ => (None, None),
            };
            let ff_drag_active2 = matches!(spec_drag, Some(SpectrogramHandle::FfUpper) | Some(SpectrogramHandle::FfLower) | Some(SpectrogramHandle::FfMiddle));
            let marker_state = FreqMarkerState {
                mouse_freq,
                mouse_in_label_area: mouse_freq.is_some() && mouse_cx < LABEL_AREA_WIDTH,
                label_hover_opacity: label_opacity,
                has_selection: selection.is_some() || (dragging && axis_drag_start.is_none()),
                file_max_freq,
                axis_drag_lo: adl2,
                axis_drag_hi: adh2,
                ff_drag_active: ff_drag_active2,
                ff_lo,
                ff_hi,
                ff_handles_active: spec_hover.is_some() || spec_drag.is_some(),
            };

            spectrogram_renderer::draw_freq_markers(
                &ctx,
                min_freq,
                max_freq,
                display_h as f64,
                display_w as f64,
                shift_mode,
                &marker_state,
                het_cutoff,
            );

            // FF overlay (dim outside focus range)
            if ff_hi > ff_lo {
                spectrogram_renderer::draw_ff_overlay(
                    &ctx,
                    ff_lo, ff_hi,
                    min_freq, max_freq,
                    display_h as f64, display_w as f64,
                    spec_hover, spec_drag,
                );
            }

            // HET overlay (cyan lines on top, no dimming)
            if show_het {
                let het_interactive = !het_freq_auto || !het_cutoff_auto;
                spectrogram_renderer::draw_het_overlay(
                    &ctx,
                    het_freq,
                    het_cutoff,
                    min_freq,
                    max_freq,
                    display_h as f64,
                    display_w as f64,
                    spec_hover,
                    spec_drag,
                    het_interactive,
                );
            }

            // Draw selection overlay
            if let Some(sel) = selection {
                spectrogram_renderer::draw_selection(
                    &ctx,
                    &sel,
                    min_freq,
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
                        min_freq,
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
                        min_freq,
                        max_freq,
                        display_w as f64,
                        display_h as f64,
                    );
                }
            }

            let px_per_sec = display_w as f64 / visible_time;

            // Draw static position marker when not playing
            if !is_playing && canvas_tool == CanvasTool::Hand {
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

            // Draw bookmark dots (yellow circles at top edge)
            ctx.set_fill_style_str("rgba(255, 200, 50, 0.9)");
            for bm in &bookmarks {
                let x = (bm.time - scroll) * px_per_sec;
                if x >= 0.0 && x <= display_w as f64 {
                    ctx.begin_path();
                    let _ = ctx.arc(x, 6.0, 4.0, 0.0, std::f64::consts::TAU);
                    let _ = ctx.fill();
                }
            }
        }
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
        let (time_res, duration) = idx
            .and_then(|i| files.get(i))
            .map(|f| (f.spectrogram.time_resolution, f.audio.duration_secs))
            .unwrap_or((1.0, 0.0));
        let zoom = state.zoom_level.get_untracked();
        let scroll = state.scroll_offset.get_untracked();

        let visible_time = (display_w / zoom) * time_res;
        let playhead_rel = playhead - scroll;

        if playhead_rel > visible_time * 0.8 || playhead_rel < 0.0 {
            // Clamp so the viewport doesn't extend past the file end
            let max_scroll = (duration - visible_time).max(0.0);
            state.scroll_offset.set((playhead - visible_time * 0.2).max(0.0).min(max_scroll));
        }
    });

    // Helper to get (px_x, px_y, time, freq) from mouse event
    let mouse_to_xtf = move |ev: &MouseEvent| -> Option<(f64, f64, f64, f64)> {
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
        let min_freq = state.min_display_freq.get_untracked()
            .unwrap_or(0.0);
        let scroll = state.scroll_offset.get_untracked();
        let zoom = state.zoom_level.get_untracked();

        let (t, f) = spectrogram_renderer::pixel_to_time_freq(
            px_x, px_y, min_freq, max_freq, scroll, time_res, zoom, cw, ch,
        );
        Some((px_x, px_y, t, f))
    };

    let on_mousedown = move |ev: MouseEvent| {
        if ev.button() != 0 { return; }

        // Check for spec handle drag first (FF or HET — takes priority over tool)
        if let Some(handle) = state.spec_hover_handle.get_untracked() {
            state.spec_drag_handle.set(Some(handle));
            state.is_dragging.set(true);
            ev.prevent_default();
            return;
        }

        // Check for axis drag (left axis frequency range selection)
        if let Some((px_x, _, _, freq)) = mouse_to_xtf(&ev) {
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

        match state.canvas_tool.get_untracked() {
            CanvasTool::Hand => {
                // Bookmark tap while playing
                if state.is_playing.get_untracked() {
                    let t = state.playhead_time.get_untracked();
                    state.bookmarks.update(|bm| bm.push(crate::state::Bookmark { time: t }));
                    return;
                }
                // Start hand panning
                state.is_dragging.set(true);
                hand_drag_start.set((ev.client_x() as f64, state.scroll_offset.get_untracked()));
            }
            CanvasTool::Selection => {
                if let Some((_, _, t, f)) = mouse_to_xtf(&ev) {
                    state.is_dragging.set(true);
                    drag_start.set((t, f));
                    state.selection.set(None);
                }
            }
        }
    };

    let on_mousemove = move |ev: MouseEvent| {
        if let Some((px_x, px_y, t, f)) = mouse_to_xtf(&ev) {
            // Always track hover position
            state.mouse_freq.set(Some(f));
            state.mouse_canvas_x.set(px_x);
            state.cursor_time.set(Some(t));

            // Update label hover target and in-label-area state
            let in_label_area = px_x < LABEL_AREA_WIDTH;
            state.mouse_in_label_area.set(in_label_area);
            let current_target = label_hover_target.get_untracked();
            let new_target = if in_label_area { 1.0 } else { 0.0 };
            if current_target != new_target {
                label_hover_target.set(new_target);
            }

            if state.is_dragging.get_untracked() {
                // Spec handle drag takes priority
                if let Some(handle) = state.spec_drag_handle.get_untracked() {
                    let Some(canvas_el) = canvas_ref.get() else { return };
                    let canvas: &HtmlCanvasElement = canvas_el.as_ref();
                    let ch = canvas.height() as f64;
                    let files = state.files.get_untracked();
                    let idx = state.current_file_index.get_untracked();
                    let file = idx.and_then(|i| files.get(i));
                    let file_max_freq = file.map(|f| f.spectrogram.max_freq).unwrap_or(96_000.0);
                    let min_freq_val = state.min_display_freq.get_untracked().unwrap_or(0.0);
                    let max_freq_val = state.max_display_freq.get_untracked().unwrap_or(file_max_freq);
                    let freq_at_mouse = spectrogram_renderer::y_to_freq(px_y, min_freq_val, max_freq_val, ch);

                    match handle {
                        SpectrogramHandle::FfUpper => {
                            let lo = state.ff_freq_lo.get_untracked();
                            let clamped = freq_at_mouse.clamp(lo + 500.0, file_max_freq);
                            state.ff_freq_hi.set(clamped);
                        }
                        SpectrogramHandle::FfLower => {
                            let hi = state.ff_freq_hi.get_untracked();
                            let clamped = freq_at_mouse.clamp(0.0, hi - 500.0);
                            state.ff_freq_lo.set(clamped);
                        }
                        SpectrogramHandle::FfMiddle => {
                            let lo = state.ff_freq_lo.get_untracked();
                            let hi = state.ff_freq_hi.get_untracked();
                            let bw = hi - lo;
                            let mid = (lo + hi) / 2.0;
                            let delta = freq_at_mouse - mid;
                            let new_lo = (lo + delta).clamp(0.0, file_max_freq - bw);
                            let new_hi = new_lo + bw;
                            state.ff_freq_lo.set(new_lo);
                            state.ff_freq_hi.set(new_hi);
                        }
                        SpectrogramHandle::HetCenter => {
                            state.het_freq_auto.set(false);
                            let clamped = freq_at_mouse.clamp(1000.0, file_max_freq);
                            state.het_frequency.set(clamped);
                        }
                        SpectrogramHandle::HetBandUpper => {
                            state.het_cutoff_auto.set(false);
                            let het_freq = state.het_frequency.get_untracked();
                            let new_cutoff = (freq_at_mouse - het_freq).clamp(1000.0, 30000.0);
                            state.het_cutoff.set(new_cutoff);
                        }
                        SpectrogramHandle::HetBandLower => {
                            state.het_cutoff_auto.set(false);
                            let het_freq = state.het_frequency.get_untracked();
                            let new_cutoff = (het_freq - freq_at_mouse).clamp(1000.0, 30000.0);
                            state.het_cutoff.set(new_cutoff);
                        }
                    }
                    return;
                }

                // Axis drag takes second priority (after spec handle drag)
                if state.axis_drag_start_freq.get_untracked().is_some() {
                    let raw_start = axis_drag_raw_start.get_untracked();
                    let snap = if ev.shift_key() { 10_000.0 } else { 5_000.0 };
                    // Snap both start and end away from each other to include
                    // the full segment under each endpoint
                    let (snapped_start, snapped_end) = if f > raw_start {
                        // Dragging up: start floors down, end ceils up
                        ((raw_start / snap).floor() * snap, (f / snap).ceil() * snap)
                    } else if f < raw_start {
                        // Dragging down: start ceils up, end floors down
                        ((raw_start / snap).ceil() * snap, (f / snap).floor() * snap)
                    } else {
                        let s = (raw_start / snap).round() * snap;
                        (s, s)
                    };
                    state.axis_drag_start_freq.set(Some(snapped_start));
                    state.axis_drag_current_freq.set(Some(snapped_end));
                    // Live update FF range
                    let lo = snapped_start.min(snapped_end);
                    let hi = snapped_start.max(snapped_end);
                    if hi - lo > 500.0 {
                        state.ff_freq_lo.set(lo);
                        state.ff_freq_hi.set(hi);
                    }
                    return;
                }

                match state.canvas_tool.get_untracked() {
                    CanvasTool::Hand => {
                        // Pan view
                        let (start_client_x, start_scroll) = hand_drag_start.get_untracked();
                        let dx = ev.client_x() as f64 - start_client_x;
                        let Some(canvas_el) = canvas_ref.get() else { return };
                        let canvas: &HtmlCanvasElement = canvas_el.as_ref();
                        let cw = canvas.width() as f64;
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
                    }
                    CanvasTool::Selection => {
                        let (t0, f0) = drag_start.get_untracked();
                        state.selection.set(Some(Selection {
                            time_start: t0.min(t),
                            time_end: t0.max(t),
                            freq_low: f0.min(f),
                            freq_high: f0.max(f),
                        }));
                    }
                }
            } else {
                // Not dragging — do spec handle hover detection (FF + HET)
                // Skip handle hover when in label area (to allow axis drag)
                if !in_label_area {
                    let Some(canvas_el) = canvas_ref.get() else { return };
                    let canvas: &HtmlCanvasElement = canvas_el.as_ref();
                    let ch = canvas.height() as f64;
                    let files = state.files.get_untracked();
                    let idx = state.current_file_index.get_untracked();
                    let file = idx.and_then(|i| files.get(i));
                    let file_max_freq = file.map(|f| f.spectrogram.max_freq).unwrap_or(96_000.0);
                    let min_freq_val = state.min_display_freq.get_untracked().unwrap_or(0.0);
                    let max_freq_val = state.max_display_freq.get_untracked().unwrap_or(file_max_freq);

                    let handle = hit_test_spec_handles(
                        &state, px_y, min_freq_val, max_freq_val, ch, 8.0,
                    );
                    state.spec_hover_handle.set(handle);
                } else {
                    state.spec_hover_handle.set(None);
                }
            }
        }
    };

    let on_mouseleave = move |_ev: MouseEvent| {
        state.mouse_freq.set(None);
        state.mouse_in_label_area.set(false);
        state.cursor_time.set(None);
        label_hover_target.set(0.0);
        state.is_dragging.set(false);
        state.spec_drag_handle.set(None);
        state.spec_hover_handle.set(None);
        state.axis_drag_start_freq.set(None);
        state.axis_drag_current_freq.set(None);
    };

    let on_mouseup = move |ev: MouseEvent| {
        if !state.is_dragging.get_untracked() { return; }

        // End HET/FF handle drag
        if state.spec_drag_handle.get_untracked().is_some() {
            state.spec_drag_handle.set(None);
            state.is_dragging.set(false);
            return;
        }

        // End axis drag (FF range already updated live during drag)
        if state.axis_drag_start_freq.get_untracked().is_some() {
            let lo = state.ff_freq_lo.get_untracked();
            let hi = state.ff_freq_hi.get_untracked();
            if hi - lo > 500.0 && !state.hfr_enabled.get_untracked() {
                // Save dragged bounds before enabling HFR so the effect
                // restores them instead of resetting to defaults.
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
        if state.canvas_tool.get_untracked() != CanvasTool::Selection { return; }
        if let Some((_, _, t, f)) = mouse_to_xtf(&ev) {
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

    // Helper to get (px_x, px_y, time, freq) from touch event
    let touch_to_xtf = move |touch: &web_sys::Touch| -> Option<(f64, f64, f64, f64)> {
        let canvas_el = canvas_ref.get()?;
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();
        let rect = canvas.get_bounding_client_rect();
        let px_x = touch.client_x() as f64 - rect.left();
        let px_y = touch.client_y() as f64 - rect.top();
        let cw = canvas.width() as f64;
        let ch = canvas.height() as f64;

        let files = state.files.get_untracked();
        let idx = state.current_file_index.get_untracked()?;
        let file = files.get(idx)?;
        let time_res = file.spectrogram.time_resolution;
        let file_max_freq = file.spectrogram.max_freq;
        let max_freq = state.max_display_freq.get_untracked().unwrap_or(file_max_freq);
        let min_freq = state.min_display_freq.get_untracked().unwrap_or(0.0);
        let scroll = state.scroll_offset.get_untracked();
        let zoom = state.zoom_level.get_untracked();

        let (t, f) = spectrogram_renderer::pixel_to_time_freq(
            px_x, px_y, min_freq, max_freq, scroll, time_res, zoom, cw, ch,
        );
        Some((px_x, px_y, t, f))
    };

    // ── Touch event handlers (mobile) ──────────────────────────────────────────
    let on_touchstart = move |ev: web_sys::TouchEvent| {
        let touches = ev.touches();
        if touches.length() != 1 { return; }
        let touch = touches.get(0).unwrap();

        // Check for spec handle drag first — hit-test at touch position
        if let Some((_, px_y, _, _)) = touch_to_xtf(&touch) {
            let canvas_el = canvas_ref.get();
            if let Some(canvas_el) = canvas_el {
                let canvas: &HtmlCanvasElement = canvas_el.as_ref();
                let ch = canvas.height() as f64;
                let files = state.files.get_untracked();
                let idx = state.current_file_index.get_untracked();
                let file = idx.and_then(|i| files.get(i));
                let file_max_freq = file.map(|f| f.spectrogram.max_freq).unwrap_or(96_000.0);
                let min_freq_val = state.min_display_freq.get_untracked().unwrap_or(0.0);
                let max_freq_val = state.max_display_freq.get_untracked().unwrap_or(file_max_freq);
                let handle = hit_test_spec_handles(
                    &state, px_y, min_freq_val, max_freq_val, ch, 16.0, // wider touch target
                );
                if let Some(handle) = handle {
                    state.spec_drag_handle.set(Some(handle));
                    state.is_dragging.set(true);
                    ev.prevent_default();
                    return;
                }
            }
        }

        // Check for axis drag (left axis frequency range selection)
        if let Some((px_x, _, _, freq)) = touch_to_xtf(&touch) {
            if px_x < LABEL_AREA_WIDTH {
                let snap = 5_000.0;
                let snapped = (freq / snap).round() * snap;
                axis_drag_raw_start.set(freq);
                state.axis_drag_start_freq.set(Some(snapped));
                state.axis_drag_current_freq.set(Some(snapped));
                state.is_dragging.set(true);
                ev.prevent_default();
                return;
            }
        }

        match state.canvas_tool.get_untracked() {
            CanvasTool::Hand => {
                if state.is_playing.get_untracked() {
                    let t = state.playhead_time.get_untracked();
                    state.bookmarks.update(|bm| bm.push(crate::state::Bookmark { time: t }));
                    return;
                }
                ev.prevent_default();
                state.is_dragging.set(true);
                hand_drag_start.set((touch.client_x() as f64, state.scroll_offset.get_untracked()));
            }
            CanvasTool::Selection => {
                ev.prevent_default();
            }
        }
    };

    let on_touchmove = move |ev: web_sys::TouchEvent| {
        let touches = ev.touches();
        if touches.length() != 1 { return; }
        let touch = touches.get(0).unwrap();

        if !state.is_dragging.get_untracked() { return; }
        ev.prevent_default();

        // Spec handle drag takes priority
        if let Some(handle) = state.spec_drag_handle.get_untracked() {
            if let Some((_, px_y, _, _)) = touch_to_xtf(&touch) {
                let Some(canvas_el) = canvas_ref.get() else { return };
                let canvas: &HtmlCanvasElement = canvas_el.as_ref();
                let ch = canvas.height() as f64;
                let files = state.files.get_untracked();
                let idx = state.current_file_index.get_untracked();
                let file = idx.and_then(|i| files.get(i));
                let file_max_freq = file.map(|f| f.spectrogram.max_freq).unwrap_or(96_000.0);
                let min_freq_val = state.min_display_freq.get_untracked().unwrap_or(0.0);
                let max_freq_val = state.max_display_freq.get_untracked().unwrap_or(file_max_freq);
                let freq_at_touch = spectrogram_renderer::y_to_freq(px_y, min_freq_val, max_freq_val, ch);

                match handle {
                    SpectrogramHandle::FfUpper => {
                        let lo = state.ff_freq_lo.get_untracked();
                        state.ff_freq_hi.set(freq_at_touch.clamp(lo + 500.0, file_max_freq));
                    }
                    SpectrogramHandle::FfLower => {
                        let hi = state.ff_freq_hi.get_untracked();
                        state.ff_freq_lo.set(freq_at_touch.clamp(0.0, hi - 500.0));
                    }
                    SpectrogramHandle::FfMiddle => {
                        let lo = state.ff_freq_lo.get_untracked();
                        let hi = state.ff_freq_hi.get_untracked();
                        let bw = hi - lo;
                        let mid = (lo + hi) / 2.0;
                        let delta = freq_at_touch - mid;
                        let new_lo = (lo + delta).clamp(0.0, file_max_freq - bw);
                        let new_hi = new_lo + bw;
                        state.ff_freq_lo.set(new_lo);
                        state.ff_freq_hi.set(new_hi);
                    }
                    SpectrogramHandle::HetCenter => {
                        state.het_freq_auto.set(false);
                        state.het_frequency.set(freq_at_touch.clamp(1000.0, file_max_freq));
                    }
                    SpectrogramHandle::HetBandUpper => {
                        state.het_cutoff_auto.set(false);
                        let het_freq = state.het_frequency.get_untracked();
                        state.het_cutoff.set((freq_at_touch - het_freq).clamp(1000.0, 30000.0));
                    }
                    SpectrogramHandle::HetBandLower => {
                        state.het_cutoff_auto.set(false);
                        let het_freq = state.het_frequency.get_untracked();
                        state.het_cutoff.set((het_freq - freq_at_touch).clamp(1000.0, 30000.0));
                    }
                }
            }
            return;
        }

        // Axis drag takes second priority
        if state.axis_drag_start_freq.get_untracked().is_some() {
            if let Some((_, _, _, f)) = touch_to_xtf(&touch) {
                let raw_start = axis_drag_raw_start.get_untracked();
                let snap = 5_000.0;
                let (snapped_start, snapped_end) = if f > raw_start {
                    ((raw_start / snap).floor() * snap, (f / snap).ceil() * snap)
                } else if f < raw_start {
                    ((raw_start / snap).ceil() * snap, (f / snap).floor() * snap)
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

        match state.canvas_tool.get_untracked() {
            CanvasTool::Hand => {
                let (start_client_x, start_scroll) = hand_drag_start.get_untracked();
                let dx = touch.client_x() as f64 - start_client_x;
                let Some(canvas_el) = canvas_ref.get() else { return };
                let canvas: &HtmlCanvasElement = canvas_el.as_ref();
                let cw = canvas.width() as f64;
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
            }
            CanvasTool::Selection => {}
        }
    };

    let on_touchend = move |_ev: web_sys::TouchEvent| {
        if state.spec_drag_handle.get_untracked().is_some() {
            state.spec_drag_handle.set(None);
            state.is_dragging.set(false);
            return;
        }
        // Finalize axis drag — auto-enable HFR if a meaningful range was selected
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

    let on_wheel = move |ev: web_sys::WheelEvent| {
        ev.prevent_default();
        if ev.shift_key() {
            // Shift+scroll: vertical freq zoom around mouse position
            let files = state.files.get_untracked();
            let idx = state.current_file_index.get_untracked();
            let file_max_freq = idx
                .and_then(|i| files.get(i))
                .map(|f| f.spectrogram.max_freq)
                .unwrap_or(96_000.0);
            let cur_max = state.max_display_freq.get_untracked().unwrap_or(file_max_freq);
            let cur_min = state.min_display_freq.get_untracked().unwrap_or(0.0);
            let range = cur_max - cur_min;
            if range < 1.0 { return; }

            // Determine anchor freq from mouse Y
            let anchor_frac = if let Some(mf) = state.mouse_freq.get_untracked() {
                ((mf - cur_min) / range).clamp(0.0, 1.0)
            } else {
                0.5
            };

            let factor = if ev.delta_y() > 0.0 { 1.15 } else { 1.0 / 1.15 };
            let new_range = (range * factor).clamp(500.0, file_max_freq);
            let anchor_freq = cur_min + anchor_frac * range;
            let new_min = (anchor_freq - anchor_frac * new_range).max(0.0);
            let new_max = (new_min + new_range).min(file_max_freq);
            let new_min = (new_max - new_range).max(0.0);

            state.min_display_freq.set(Some(new_min));
            state.max_display_freq.set(Some(new_max));
        } else if ev.ctrl_key() {
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
            state.scroll_offset.update(|s| {
                *s = (*s + delta).clamp(0.0, max_scroll);
            });
        }
    };

    view! {
        <div class="spectrogram-container"
            style=move || {
                if state.axis_drag_start_freq.get().is_some() || state.mouse_in_label_area.get() {
                    return "cursor: crosshair; touch-action: none;".to_string();
                }
                if state.spec_drag_handle.get().is_some() || state.spec_hover_handle.get().is_some() {
                    return "cursor: ns-resize; touch-action: none;".to_string();
                }
                match state.canvas_tool.get() {
                    CanvasTool::Hand => "cursor: grab; touch-action: none;".to_string(),
                    CanvasTool::Selection => "cursor: crosshair; touch-action: none;".to_string(),
                }
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
    freq_crop_lo: f64,
    freq_crop_hi: f64,
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
            continue;
        }
        let frame_i = frame_f as usize;
        let frame_row = &frames[frame_i];

        for px_y in 0..h {
            // Map pixel row → frequency bin.
            // Row 0 (top) = max displayed freq, row h (bottom) = min displayed freq.
            let frac = freq_crop_lo + (freq_crop_hi - freq_crop_lo) * (1.0 - px_y as f64 / h as f64);
            let bin_f = (n_bins as f64 * frac).min((n_bins - 1) as f64);
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
