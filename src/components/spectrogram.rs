use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};
use crate::canvas::spectrogram_renderer::{self, PreRendered};
use crate::state::AppState;

#[component]
pub fn Spectrogram() -> impl IntoView {
    let state = expect_context::<AppState>();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();

    // Store pre-rendered data in a signal so we only compute it when file changes
    let pre_rendered: RwSignal<Option<PreRendered>> = RwSignal::new(None);

    // Re-compute pre-render when current file changes
    Effect::new(move || {
        let files = state.files.get();
        let idx = state.current_file_index.get();
        if let Some(i) = idx {
            if let Some(file) = files.get(i) {
                let rendered = spectrogram_renderer::pre_render(&file.spectrogram);
                pre_rendered.set(Some(rendered));
            }
        } else {
            pre_rendered.set(None);
        }
    });

    // Redraw when pre-rendered data, scroll, or zoom changes
    Effect::new(move || {
        let scroll = state.scroll_offset.get();
        let zoom = state.zoom_level.get();
        let _pre = pre_rendered.track();

        let Some(canvas_el) = canvas_ref.get() else { return };
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();

        // Sync canvas internal resolution with display size
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
                // Convert scroll_offset (seconds) to column index
                let files = state.files.get_untracked();
                let idx = state.current_file_index.get_untracked();
                let time_res = idx
                    .and_then(|i| files.get(i))
                    .map(|f| f.spectrogram.time_resolution)
                    .unwrap_or(1.0);
                let scroll_col = scroll / time_res;

                spectrogram_renderer::blit_viewport(&ctx, rendered, canvas, scroll_col, zoom);

                // Draw frequency markers
                let max_freq = idx
                    .and_then(|i| files.get(i))
                    .map(|f| f.spectrogram.max_freq)
                    .unwrap_or(96_000.0);
                spectrogram_renderer::draw_freq_markers(
                    &ctx,
                    max_freq,
                    display_h as f64,
                    display_w as f64,
                );
            } else {
                ctx.set_fill_style_str("#000");
                ctx.fill_rect(0.0, 0.0, display_w as f64, display_h as f64);
            }
        });
    });

    // Mouse wheel for scroll/zoom
    let on_wheel = move |ev: web_sys::WheelEvent| {
        ev.prevent_default();
        if ev.ctrl_key() {
            // Zoom
            let delta = if ev.delta_y() > 0.0 { 0.9 } else { 1.1 };
            state.zoom_level.update(|z| {
                *z = (*z * delta).max(0.1).min(100.0);
            });
        } else {
            // Scroll
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
            />
        </div>
    }
}
