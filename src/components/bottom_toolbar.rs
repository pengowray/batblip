use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use crate::state::{AppState, CanvasTool, LayerPanel};
use crate::audio::{microphone, playback};
use crate::components::hfr_button::HfrButton;

fn layer_opt_class(active: bool) -> &'static str {
    if active { "layer-panel-opt sel" } else { "layer-panel-opt" }
}

fn toggle_panel(state: &AppState, panel: LayerPanel) {
    state.layer_panel_open.update(|p| {
        *p = if *p == Some(panel) { None } else { Some(panel) };
    });
}

#[component]
pub fn BottomToolbar() -> impl IntoView {
    let state = expect_context::<AppState>();
    let has_file = move || state.current_file_index.get().is_some();
    let is_playing = move || state.is_playing.get();

    // ── Recording timer (moved from toolbar.rs) ──
    let interval_id: StoredValue<Option<i32>> = StoredValue::new(None);
    Effect::new(move |_| {
        let recording = state.mic_recording.get();
        if recording {
            let cb = Closure::<dyn FnMut()>::new(move || {
                state.mic_timer_tick.update(|n| *n = n.wrapping_add(1));
            });
            if let Some(window) = web_sys::window() {
                if let Ok(id) = window.set_interval_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(), 100,
                ) {
                    interval_id.set_value(Some(id));
                }
            }
            cb.forget();
        } else {
            if let Some(id) = interval_id.get_value() {
                if let Some(window) = web_sys::window() {
                    window.clear_interval_with_handle(id);
                }
                interval_id.set_value(None);
            }
        }
    });

    // ── Play callbacks ──
    let state_play = state.clone();
    let on_play_start = move |_| {
        playback::play_from_start(&state_play);
    };

    let state_here = state.clone();
    let on_play_here = move |_| {
        playback::play_from_here(&state_here);
    };

    let state_stop = state.clone();
    let on_stop = move |_| {
        playback::stop(&state_stop);
    };

    view! {
        <div class="bottom-toolbar"
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
        >
            // ── HFR combo button ──
            <HfrButton />

            <div class="bottom-toolbar-sep"></div>

            // ── Play / Stop / Gain ──
            {move || if !has_file() {
                view! { <span></span> }.into_any()
            } else if is_playing() {
                view! {
                    <button class="layer-btn" on:click=on_stop.clone()>"Stop"</button>
                }.into_any()
            } else {
                view! {
                    <button class="layer-btn" on:click=on_play_start.clone()
                        title="Play from start of file"
                    >"Play start"</button>
                    <button class="layer-btn" on:click=on_play_here.clone()
                        title="Play from current position"
                    >"Play here"</button>
                }.into_any()
            }}

            // Gain toggle
            {move || has_file().then(|| {
                let auto = state.auto_gain.get();
                let db = if auto {
                    state.compute_auto_gain()
                } else {
                    state.gain_db.get()
                };
                let label = if auto {
                    "Auto".to_string()
                } else if db > 0.0 {
                    format!("+{:.0}dB", db)
                } else {
                    format!("{:.0}dB", db)
                };
                view! {
                    <button
                        class=move || if state.auto_gain.get() { "layer-btn active" } else { "layer-btn" }
                        on:click=move |_| state.auto_gain.update(|v| *v = !*v)
                        title="Toggle auto gain"
                    >
                        <span class="layer-btn-category">"Gain"</span>
                        <span class="layer-btn-value">{label}</span>
                    </button>
                }
            })}

            <div class="bottom-toolbar-sep"></div>

            // ── Listen button ──
            <button
                class=move || if state.mic_listening.get() { "layer-btn mic-armed" } else { "layer-btn" }
                on:click=move |_| {
                    let st = state;
                    wasm_bindgen_futures::spawn_local(async move {
                        microphone::toggle_listen(&st).await;
                    });
                }
                title=move || if state.mic_needs_permission.get() && state.is_tauri {
                    "Grant USB mic permission to start listening"
                } else {
                    "Toggle live listening (L)"
                }
            >
                <span class="layer-btn-category">"Mic"</span>
                <span class="layer-btn-value">{move || if state.mic_needs_permission.get() && state.is_tauri && !state.mic_listening.get() {
                    "USB mic"
                } else {
                    "Listen"
                }}</span>
            </button>

            // ── Record button ──
            <button
                class=move || if state.mic_recording.get() { "layer-btn mic-recording" } else { "layer-btn" }
                on:click=move |_| {
                    let st = state;
                    wasm_bindgen_futures::spawn_local(async move {
                        microphone::toggle_record(&st).await;
                    });
                }
                title=move || if state.mic_needs_permission.get() && state.is_tauri {
                    "Grant USB mic permission to start recording"
                } else {
                    "Toggle recording (R)"
                }
            >
                <span class="layer-btn-category">"Mic"</span>
                <span class="layer-btn-value">{move || if state.mic_recording.get() {
                    let _ = state.mic_timer_tick.get();
                    let start = state.mic_recording_start_time.get_untracked().unwrap_or(0.0);
                    let now = js_sys::Date::now();
                    let secs = (now - start) / 1000.0;
                    format!("Rec {:.1}s", secs)
                } else if state.mic_needs_permission.get() && state.is_tauri {
                    "USB mic".to_string()
                } else {
                    "Record".to_string()
                }}</span>
            </button>

            <div class="bottom-toolbar-sep"></div>

            // ── Tool button (Hand / Selection) ──
            <ToolButtonInline />
        </div>
    }
}

/// Tool button adapted for inline use in the bottom toolbar (no absolute positioning).
#[component]
fn ToolButtonInline() -> impl IntoView {
    let state = expect_context::<AppState>();
    let is_open = move || state.layer_panel_open.get() == Some(LayerPanel::Tool);

    view! {
        <div style="position: relative;">
            <button
                class=move || if is_open() { "layer-btn open" } else { "layer-btn" }
                on:click=move |_| toggle_panel(&state, LayerPanel::Tool)
                title="Tool"
            >
                <span class="layer-btn-category">"Tool"</span>
                <span class="layer-btn-value">{move || match state.canvas_tool.get() {
                    CanvasTool::Hand => "Hand",
                    CanvasTool::Selection => "Select",
                }}</span>
            </button>
            {move || is_open().then(|| view! {
                <div class="layer-panel" style="bottom: calc(100% + 4px); right: 0;">
                    <div class="layer-panel-title">"Tool"</div>
                    <button
                        class=move || layer_opt_class(state.canvas_tool.get() == CanvasTool::Hand)
                        on:click=move |_| {
                            state.canvas_tool.set(CanvasTool::Hand);
                            state.layer_panel_open.set(None);
                        }
                    >"Hand (pan)"</button>
                    <button
                        class=move || layer_opt_class(state.canvas_tool.get() == CanvasTool::Selection)
                        on:click=move |_| {
                            state.canvas_tool.set(CanvasTool::Selection);
                            state.layer_panel_open.set(None);
                        }
                    >"Selection"</button>
                </div>
            })}
        </div>
    }
}
