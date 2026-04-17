// Gutter components — dedicated drag surfaces for range selection that
// live alongside (not on top of) the spectrogram / waveform axes.
//
// `BandGutter` is a narrow vertical canvas showing the frequency-band
// selection. The time gutter is drawn as an overlay strip on the
// waveform canvas itself (see waveform.rs) rather than as its own
// component — the user asked for it to be "part of the main canvas".

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};
use crate::canvas::gutter_renderer;
use crate::state::AppState;

/// Vertical band-selection gutter. Each drag creates a fresh range —
/// there is no edge-resize or middle-pan affordance (intentional, per
/// design: dragging again over an existing band simply replaces it,
/// matching the time gutter's behaviour). Modifier-key drag handles can
/// be added later.
#[component]
pub fn BandGutter() -> impl IntoView {
    let state = expect_context::<AppState>();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();
    // Start-of-drag anchor in Hz; None when not dragging.
    let drag_anchor: StoredValue<Option<f64>> = StoredValue::new(None);
    // Tooltip position (canvas-local y, in px) — drives the drag tooltip.
    // None while not dragging.
    let tooltip_y = RwSignal::new_local(Option::<f64>::None);

    // Redraw when any relevant signal changes.
    Effect::new(move |_| {
        let band_lo = state.band_ff_freq_lo.get();
        let band_hi = state.band_ff_freq_hi.get();
        let hfr_on = state.hfr_enabled.get();
        let shield_style = state.shield_style.get();
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let _sidebar = state.sidebar_collapsed.get();
        let _sidebar_width = state.sidebar_width.get();
        let _rsidebar = state.right_sidebar_collapsed.get();
        let _rsidebar_width = state.right_sidebar_width.get();
        let _tile_ready = state.tile_ready_signal.get();

        let max_freq = idx
            .and_then(|i| files.get(i))
            .map(|f| f.audio.sample_rate as f64 / 2.0)
            .unwrap_or(0.0);

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

        let Ok(Some(obj)) = canvas.get_context("2d") else { return };
        let Ok(ctx) = obj.dyn_into::<CanvasRenderingContext2d>() else { return };

        gutter_renderer::draw_band_gutter(
            &ctx,
            display_w as f64,
            display_h as f64,
            max_freq,
            band_lo,
            band_hi,
            hfr_on,
            shield_style,
        );
    });

    // Resolve (local_y, canvas_height, file_nyquist) for a pointer event.
    let pointer_context = move |ev: &web_sys::PointerEvent| -> Option<(f64, f64, f64)> {
        let canvas_el = canvas_ref.get()?;
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();
        let rect = canvas.get_bounding_client_rect();
        let h = rect.height();
        if h <= 0.0 { return None; }
        let y = ev.client_y() as f64 - rect.top();
        let files = state.files.get_untracked();
        let idx = state.current_file_index.get_untracked();
        let max_freq = idx
            .and_then(|i| files.get(i))
            .map(|f| f.audio.sample_rate as f64 / 2.0)?;
        if max_freq <= 0.0 { return None; }
        Some((y, h, max_freq))
    };

    // Write the current drag's [anchor..current] range through the canonical
    // state setter so focus_stack / downstream band-split memos pick it up.
    let apply_drag = move |current_freq: f64| {
        let Some(anchor) = drag_anchor.get_value() else { return };
        let lo = anchor.min(current_freq).max(0.0);
        let hi = anchor.max(current_freq);
        if hi > lo {
            state.set_band_ff_range(lo, hi);
        }
    };

    let on_pointerdown = move |ev: web_sys::PointerEvent| {
        if ev.button() != 0 { return; }
        let Some((y, h, max_freq)) = pointer_context(&ev) else { return };
        ev.prevent_default();

        let freq = gutter_renderer::y_to_freq(y, max_freq, h);
        drag_anchor.set_value(Some(freq));
        tooltip_y.set(Some(y));
        // Flag the drag so heavy consumers (waveform band-split) can cache.
        state.band_ff_dragging.set(true);
        // Seed with a zero-width range — it'll expand as the pointer moves.
        state.set_band_ff_range(freq, freq);

        // Ensure HFR engages so the new band actually drives playback.
        if !state.hfr_enabled.get_untracked() {
            state.hfr_enabled.set(true);
        }

        if let Some(target) = ev.target() {
            if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                let _ = el.set_pointer_capture(ev.pointer_id());
            }
        }
    };

    let on_pointermove = move |ev: web_sys::PointerEvent| {
        if drag_anchor.get_value().is_none() { return; }
        let Some((y, h, max_freq)) = pointer_context(&ev) else { return };
        tooltip_y.set(Some(y.clamp(0.0, h)));
        let freq = gutter_renderer::y_to_freq(y, max_freq, h);
        apply_drag(freq);
    };

    let on_pointerup = move |_ev: web_sys::PointerEvent| {
        // If the drag produced a zero-width "tap" range, clear it so we
        // don't strand an invisible selection.
        if drag_anchor.get_value().is_some() {
            drag_anchor.set_value(None);
            tooltip_y.set(None);
            let lo = state.band_ff_freq_lo.get_untracked();
            let hi = state.band_ff_freq_hi.get_untracked();
            if (hi - lo).abs() < 1.0 {
                state.set_band_ff_range(0.0, 0.0);
            }
            // Drag finished — heavy consumers recompute once with final range.
            state.band_ff_dragging.set(false);
        }
    };

    let on_dblclick = move |_ev: web_sys::MouseEvent| {
        state.set_band_ff_range(0.0, 0.0);
    };

    // Format "40.0 – 72.5 kHz" for the drag tooltip.
    let format_range = move || {
        let lo = state.band_ff_freq_lo.get();
        let hi = state.band_ff_freq_hi.get();
        if hi <= lo { return String::new(); }
        format!("{:.1} – {:.1} kHz", lo / 1000.0, hi / 1000.0)
    };

    view! {
        <div class="band-gutter">
            <canvas
                node_ref=canvas_ref
                on:pointerdown=on_pointerdown
                on:pointermove=on_pointermove
                on:pointerup=on_pointerup
                on:dblclick=on_dblclick
            />
            // "kHz" header — tiny label at top so the scale orientation is obvious.
            <div class="band-gutter-header">"kHz"</div>
            // Drag tooltip: floats next to the pointer while dragging, shows the
            // current lo–hi range. Hidden when not dragging.
            <div
                class="band-gutter-tooltip"
                style:top=move || tooltip_y.get().map(|y| format!("{:.0}px", y)).unwrap_or_default()
                style:display=move || if tooltip_y.get().is_some() && !format_range().is_empty() { "block" } else { "none" }
            >
                {format_range}
            </div>
        </div>
    }
}
