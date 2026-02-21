use leptos::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData, MouseEvent};
use crate::canvas::waveform_renderer;
use crate::state::{AppState, LayerPanel, NavEntry, OverviewFreqMode, OverviewView};
use crate::types::PreviewImage;

// ── Navigation helpers ────────────────────────────────────────────────────────

fn push_nav(state: &AppState) {
    let entry = NavEntry {
        scroll_offset: state.scroll_offset.get_untracked(),
        zoom_level: state.zoom_level.get_untracked(),
    };
    let idx = state.nav_index.get_untracked();
    state.nav_history.update(|hist| {
        hist.truncate(idx + 1);
        if hist.last().map(|e: &NavEntry| (e.scroll_offset - entry.scroll_offset).abs() < 0.05).unwrap_or(false) {
            return; // Don't push nearly identical entries
        }
        hist.push(entry);
        if hist.len() > 100 {
            hist.remove(0);
        }
    });
    let new_len = state.nav_history.get_untracked().len();
    state.nav_index.set(new_len.saturating_sub(1));
}

fn nav_back(state: &AppState) {
    let idx = state.nav_index.get_untracked();
    if idx == 0 { return; }
    let new_idx = idx - 1;
    state.nav_index.set(new_idx);
    let hist = state.nav_history.get_untracked();
    if let Some(entry) = hist.get(new_idx) {
        state.scroll_offset.set(entry.scroll_offset);
        state.zoom_level.set(entry.zoom_level);
    }
}

fn nav_forward(state: &AppState) {
    let idx = state.nav_index.get_untracked();
    let hist = state.nav_history.get_untracked();
    if idx + 1 >= hist.len() { return; }
    let new_idx = idx + 1;
    state.nav_index.set(new_idx);
    if let Some(entry) = hist.get(new_idx) {
        state.scroll_offset.set(entry.scroll_offset);
        state.zoom_level.set(entry.zoom_level);
    }
}

// ── Rendering helpers ─────────────────────────────────────────────────────────

fn get_canvas_ctx(canvas: &HtmlCanvasElement) -> Option<CanvasRenderingContext2d> {
    canvas
        .get_context("2d")
        .ok()?
        .and_then(|c| c.dyn_into::<CanvasRenderingContext2d>().ok())
}

/// Blit a PreviewImage (RGBA) to the entire canvas at full width.
/// Also draws a viewport highlight rect and bookmark dots.
fn draw_overview_spectrogram(
    ctx: &CanvasRenderingContext2d,
    canvas: &HtmlCanvasElement,
    preview: &PreviewImage,
    scroll_offset: f64,  // in seconds
    zoom: f64,
    time_resolution: f64, // seconds per spectrogram column
    _max_display_freq: Option<f64>, // Hz, None = show all (currently used only for freq_crop)
    _max_freq: f64,       // file Nyquist Hz
    bookmarks: &[(f64,)], // list of bookmark times
    playhead_time: f64,
    is_playing: bool,
    freq_crop: f64,       // 0..1, what fraction of vertical to show (1.0 = all)
) {
    let cw = canvas.width() as f64;
    let ch = canvas.height() as f64;

    ctx.set_fill_style_str("#000");
    ctx.fill_rect(0.0, 0.0, cw, ch);

    if preview.width == 0 || preview.height == 0 {
        return;
    }

    let fc = freq_crop.clamp(0.01, 2.0);
    let full_h = preview.height as f64;
    let (src_y, src_h, dst_y, dst_h) = if fc <= 1.0 {
        let sy = full_h * (1.0 - fc);
        (sy, full_h * fc, 0.0, ch)
    } else {
        let frac = 1.0 / fc;
        (0.0, full_h, ch * (1.0 - frac), ch * frac)
    };

    // Blit entire image into canvas
    let clamped = Clamped(&preview.pixels[..]);
    let image_data = ImageData::new_with_u8_clamped_array_and_sh(
        clamped, preview.width, preview.height,
    );
    if let Ok(img) = image_data {
        let doc = web_sys::window().unwrap().document().unwrap();
        let tmp = doc.create_element("canvas").unwrap()
            .dyn_into::<HtmlCanvasElement>().unwrap();
        tmp.set_width(preview.width);
        tmp.set_height(preview.height);
        if let Some(tc) = tmp.get_context("2d").ok()
            .flatten()
            .and_then(|c| c.dyn_into::<CanvasRenderingContext2d>().ok())
        {
            let _ = tc.put_image_data(&img, 0.0, 0.0);
            let _ = ctx.draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                &tmp,
                0.0, src_y,
                preview.width as f64, src_h,
                0.0, dst_y,
                cw, dst_h,
            );
        }
    }

    // Total file duration in spectrogram columns
    let total_cols = preview.width as f64;
    if total_cols == 0.0 { return; }
    let total_duration = total_cols * time_resolution;

    // Pixels per second in overview
    let px_per_sec = cw / total_duration;

    // Viewport highlight: show where the main view currently is
    // Visible time in main view at this zoom
    // Approximate: use a reasonable canvas width (use a stored signal or estimate)
    // We'll use 1000px as a reasonable estimate; actual width tracked via js
    let approx_main_w = 1000.0_f64;
    let visible_time = (approx_main_w / zoom) * time_resolution;
    let vp_x = scroll_offset * px_per_sec;
    let vp_w = (visible_time * px_per_sec).max(2.0);
    ctx.set_fill_style_str("rgba(80, 180, 130, 0.15)");
    ctx.fill_rect(vp_x, 0.0, vp_w, ch);
    ctx.set_stroke_style_str("rgba(80, 180, 130, 0.5)");
    ctx.set_line_width(1.0);
    ctx.stroke_rect(vp_x, 0.0, vp_w, ch);

    // Bookmark dots (yellow, top edge)
    ctx.set_fill_style_str("rgba(255, 200, 50, 0.9)");
    for &(t,) in bookmarks {
        let x = t * px_per_sec;
        if x >= 0.0 && x <= cw {
            ctx.begin_path();
            let _ = ctx.arc(x, 5.0, 3.0, 0.0, std::f64::consts::TAU);
            let _ = ctx.fill();
        }
    }

    // Playhead dot (when playing)
    if is_playing {
        let ph_x = playhead_time * px_per_sec;
        if ph_x >= 0.0 && ph_x <= cw {
            ctx.set_fill_style_str("rgba(255, 80, 80, 0.9)");
            ctx.begin_path();
            let _ = ctx.arc(ph_x, ch - 5.0, 3.0, 0.0, std::f64::consts::TAU);
            let _ = ctx.fill();
        }
    }
}

fn draw_overview_waveform(
    ctx: &CanvasRenderingContext2d,
    canvas: &HtmlCanvasElement,
    samples: &[f32],
    sample_rate: u32,
    time_resolution: f64,
    scroll_offset: f64,
    zoom: f64,
    bookmarks: &[(f64,)],
) {
    let cw = canvas.width() as f64;
    let ch = canvas.height() as f64;

    // Draw full file at zoom = 1 column per pixel
    let total_cols = (samples.len() as f64 / sample_rate as f64) / time_resolution;
    let wv_zoom = cw / total_cols;
    waveform_renderer::draw_waveform(
        ctx, samples, sample_rate,
        0.0, // start from beginning
        wv_zoom,
        time_resolution,
        cw, ch,
        None,
    );

    // Viewport highlight
    let total_duration = samples.len() as f64 / sample_rate as f64;
    let px_per_sec = cw / total_duration;
    let approx_main_w = 1000.0_f64;
    let visible_time = (approx_main_w / zoom) * time_resolution;
    let vp_x = scroll_offset * px_per_sec;
    let vp_w = (visible_time * px_per_sec).max(2.0);
    ctx.set_fill_style_str("rgba(80, 180, 130, 0.15)");
    ctx.fill_rect(vp_x, 0.0, vp_w, ch);
    ctx.set_stroke_style_str("rgba(80, 180, 130, 0.5)");
    ctx.set_line_width(1.0);
    ctx.stroke_rect(vp_x, 0.0, vp_w, ch);

    // Bookmark dots
    ctx.set_fill_style_str("rgba(255, 200, 50, 0.9)");
    for &(t,) in bookmarks {
        let x = t * px_per_sec;
        if x >= 0.0 && x <= cw {
            ctx.begin_path();
            let _ = ctx.arc(x, 5.0, 3.0, 0.0, std::f64::consts::TAU);
            let _ = ctx.fill();
        }
    }
}

// ── Layers button ─────────────────────────────────────────────────────────────

fn toggle_panel(state: &AppState, panel: LayerPanel) {
    state.layer_panel_open.update(|p| {
        *p = if *p == Some(panel) { None } else { Some(panel) };
    });
}

fn layer_opt_class(active: bool) -> &'static str {
    if active { "layer-panel-opt sel" } else { "layer-panel-opt" }
}

#[component]
fn OverviewLayersButton() -> impl IntoView {
    let state = expect_context::<AppState>();
    let is_open = move || state.layer_panel_open.get() == Some(LayerPanel::OverviewLayers);

    view! {
        <div
            style="position: absolute; bottom: 4px; left: 32px; pointer-events: none;"
            on:click=|ev: MouseEvent| ev.stop_propagation()
        >
            <div style="position: relative; pointer-events: auto;">
                <button
                    class=move || if is_open() { "layer-btn open" } else { "layer-btn" }
                    style="font-size: 10px; padding: 3px 7px;"
                    on:click=move |_| toggle_panel(&state, LayerPanel::OverviewLayers)
                    title="Overview options"
                >"Layers"</button>
                {move || is_open().then(|| view! {
                    <div class="layer-panel" style="bottom: 28px; left: 0;">
                        <div class="layer-panel-title">"View"</div>
                        <button class=move || layer_opt_class(state.overview_view.get() == OverviewView::Spectrogram)
                            on:click=move |_| state.overview_view.set(OverviewView::Spectrogram)
                        >"Spectrogram"</button>
                        <button class=move || layer_opt_class(state.overview_view.get() == OverviewView::Waveform)
                            on:click=move |_| state.overview_view.set(OverviewView::Waveform)
                        >"Waveform"</button>
                        <hr />
                        <div class="layer-panel-title">"Frequency"</div>
                        <button class=move || layer_opt_class(state.overview_freq_mode.get() == OverviewFreqMode::All)
                            on:click=move |_| state.overview_freq_mode.set(OverviewFreqMode::All)
                        >"All"</button>
                        <button class=move || layer_opt_class(state.overview_freq_mode.get() == OverviewFreqMode::Human)
                            on:click=move |_| state.overview_freq_mode.set(OverviewFreqMode::Human)
                        >"Human (20–20k)"</button>
                        <button class=move || layer_opt_class(state.overview_freq_mode.get() == OverviewFreqMode::MatchMain)
                            on:click=move |_| state.overview_freq_mode.set(OverviewFreqMode::MatchMain)
                        >"Match main view"</button>
                    </div>
                })}
            </div>
        </div>
    }
}

// ── Main OverviewPanel component ──────────────────────────────────────────────

#[component]
pub fn OverviewPanel() -> impl IntoView {
    let state = expect_context::<AppState>();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();

    // Dragging state: (is_dragging, initial_client_x, initial_scroll)
    let drag_active = RwSignal::new(false);
    let drag_start_x = RwSignal::new(0.0f64);
    let drag_start_scroll = RwSignal::new(0.0f64);

    // Redraw effect — runs when anything that affects the overview display changes
    Effect::new(move || {
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let scroll = state.scroll_offset.get();
        let zoom = state.zoom_level.get();
        let overview_view = state.overview_view.get();
        let freq_mode = state.overview_freq_mode.get();
        let max_display_freq = state.max_display_freq.get();
        let bookmarks = state.bookmarks.get();
        let playhead = state.playhead_time.get();
        let is_playing = state.is_playing.get();

        let Some(canvas_el) = canvas_ref.get() else { return };
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();

        // Size canvas to match display
        let w = canvas.client_width() as u32;
        let h = canvas.client_height() as u32;
        if w == 0 || h == 0 { return; }
        if canvas.width() != w { canvas.set_width(w); }
        if canvas.height() != h { canvas.set_height(h); }

        let Some(ctx) = get_canvas_ctx(canvas) else { return };
        ctx.set_fill_style_str("#000");
        ctx.fill_rect(0.0, 0.0, w as f64, h as f64);

        let Some(i) = idx else { return };
        let Some(file) = files.get(i) else { return };

        let bm_tuples: Vec<(f64,)> = bookmarks.iter().map(|b| (b.time,)).collect();

        match overview_view {
            OverviewView::Spectrogram => {
                // Use file.preview if available, else nothing for now
                if let Some(ref preview) = file.preview {
                    // Compute freq_crop
                    let max_freq = file.spectrogram.max_freq;
                    let display_max = match freq_mode {
                        OverviewFreqMode::All => max_freq,
                        OverviewFreqMode::Human => 20_000.0f64.min(max_freq),
                        OverviewFreqMode::MatchMain => max_display_freq.unwrap_or(max_freq),
                    };
                    let freq_crop = (display_max / max_freq).clamp(0.01, 1.0);

                    draw_overview_spectrogram(
                        &ctx, canvas, preview,
                        scroll, zoom,
                        file.spectrogram.time_resolution,
                        max_display_freq,
                        max_freq,
                        &bm_tuples,
                        playhead,
                        is_playing,
                        freq_crop,
                    );
                } else {
                    // No preview yet — show loading message
                    ctx.set_fill_style_str("#333");
                    ctx.fill_rect(0.0, 0.0, w as f64, h as f64);
                    ctx.set_fill_style_str("#666");
                    ctx.set_font("11px system-ui");
                    ctx.set_text_align("center");
                    ctx.set_text_baseline("middle");
                    let _ = ctx.fill_text("Loading overview…", w as f64 / 2.0, h as f64 / 2.0);
                }
            }
            OverviewView::Waveform => {
                draw_overview_waveform(
                    &ctx, canvas,
                    &file.audio.samples,
                    file.audio.sample_rate,
                    file.spectrogram.time_resolution,
                    scroll, zoom,
                    &bm_tuples,
                );
            }
        }
    });

    // ── Mouse handlers ────────────────────────────────────────────────────────

    // Convert a click x-coordinate to a time offset (seconds)
    let x_to_time = move |canvas_x: f64, canvas_w: f64| -> Option<f64> {
        let files = state.files.get_untracked();
        let idx = state.current_file_index.get_untracked();
        let file = idx.and_then(|i| files.get(i))?;
        let total_cols = file.preview.as_ref().map(|p| p.width as f64)
            .unwrap_or_else(|| file.spectrogram.columns.len() as f64);
        if total_cols == 0.0 || canvas_w == 0.0 { return None; }
        let total_duration = total_cols * file.spectrogram.time_resolution;
        Some((canvas_x / canvas_w) * total_duration)
    };

    let on_mousedown = move |ev: MouseEvent| {
        ev.prevent_default();
        let Some(canvas_el) = canvas_ref.get_untracked() else { return };
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();
        let rect = canvas.get_bounding_client_rect();
        let canvas_x = ev.client_x() as f64 - rect.left();
        let cw = rect.width();
        if let Some(t) = x_to_time(canvas_x, cw) {
            push_nav(&state);
            state.scroll_offset.set(t.max(0.0));
        }
        drag_active.set(true);
        drag_start_x.set(ev.client_x() as f64);
        drag_start_scroll.set(state.scroll_offset.get_untracked());
    };

    let on_mousemove = move |ev: MouseEvent| {
        if !drag_active.get_untracked() { return; }
        let Some(canvas_el) = canvas_ref.get_untracked() else { return };
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();
        let rect = canvas.get_bounding_client_rect();
        let cw = rect.width();
        let files = state.files.get_untracked();
        let idx = state.current_file_index.get_untracked();
        let Some(file) = idx.and_then(|i| files.get(i)) else { return };
        let total_cols = file.preview.as_ref().map(|p| p.width as f64)
            .unwrap_or_else(|| file.spectrogram.columns.len() as f64);
        if total_cols == 0.0 || cw == 0.0 { return; }
        let total_duration = total_cols * file.spectrogram.time_resolution;
        let dx = ev.client_x() as f64 - drag_start_x.get_untracked();
        let dt = -(dx / cw) * total_duration;
        let new_scroll = (drag_start_scroll.get_untracked() + dt).max(0.0);
        state.scroll_offset.set(new_scroll);
    };

    let on_mouseup = move |_: MouseEvent| {
        drag_active.set(false);
    };

    let on_wheel = move |ev: web_sys::WheelEvent| {
        ev.prevent_default();
        let delta = ev.delta_y() * 0.001;
        state.scroll_offset.update(|s| *s = (*s + delta).max(0.0));
    };

    // Back/forward can_back and can_forward
    let can_back = move || state.nav_index.get() > 0;
    let can_forward = move || {
        let idx = state.nav_index.get();
        let len = state.nav_history.get().len();
        idx + 1 < len
    };

    view! {
        <div class="overview-strip">
            // Back/Forward navigation buttons (top-left)
            <div class="overview-nav"
                on:click=|ev: MouseEvent| ev.stop_propagation()
            >
                <button
                    class="overview-nav-btn"
                    disabled=move || !can_back()
                    on:click=move |_| nav_back(&state)
                    title="Back"
                >"←"</button>
                <button
                    class="overview-nav-btn"
                    disabled=move || !can_forward()
                    on:click=move |_| nav_forward(&state)
                    title="Forward"
                >"→"</button>
            </div>

            <canvas
                node_ref=canvas_ref
                on:mousedown=on_mousedown
                on:mousemove=on_mousemove
                on:mouseup=on_mouseup
                on:mouseleave=on_mouseup
                on:wheel=on_wheel
                style="cursor: crosshair;"
            />

            // Layers button (bottom-left, after nav buttons)
            <OverviewLayersButton />
        </div>
    }
}
