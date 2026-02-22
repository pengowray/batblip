use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use crate::state::{AppState, LayerPanel, OverviewView};
use crate::audio::playback;
use crate::components::file_sidebar::FileSidebar;
use crate::components::spectrogram::Spectrogram;
use crate::components::waveform::Waveform;
use crate::components::toolbar::Toolbar;
use crate::components::analysis_panel::AnalysisPanel;
use crate::components::overview::OverviewPanel;
use crate::components::play_controls::PlayControls;
use crate::components::frequency_focus_button::FrequencyFocusButton;
use crate::components::listen_mode_button::ListenModeButton;
use crate::components::tool_button::ToolButton;
use crate::components::freq_range_button::FreqRangeButton;

#[component]
pub fn App() -> impl IntoView {
    let state = AppState::new();
    provide_context(state);

    // Global keyboard shortcut: Space = play/stop
    let state_kb = state.clone();
    let handler = Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(move |ev: web_sys::KeyboardEvent| {
        // Ignore if focus is on an input/select/textarea
        if let Some(target) = ev.target() {
            if let Ok(el) = target.dyn_into::<web_sys::HtmlElement>() {
                let tag = el.tag_name();
                if tag == "INPUT" || tag == "SELECT" || tag == "TEXTAREA" {
                    return;
                }
            }
        }
        if ev.key() == " " {
            ev.prevent_default();
            if state_kb.current_file_index.get_untracked().is_some() {
                if state_kb.is_playing.get_untracked() {
                    playback::stop(&state_kb);
                } else {
                    playback::play(&state_kb);
                }
            }
        }
    });
    let window = web_sys::window().unwrap();
    let _ = window.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref());
    handler.forget();

    let grid_style = move || {
        if state.sidebar_collapsed.get() {
            "grid-template-columns: 0px 1fr".to_string()
        } else {
            format!("grid-template-columns: {}px 1fr", state.sidebar_width.get() as i32)
        }
    };

    view! {
        <div class="app" style=grid_style>
            <FileSidebar />
            <MainArea />
        </div>
    }
}

#[component]
fn MainArea() -> impl IntoView {
    let state = expect_context::<AppState>();
    let has_file = move || state.current_file_index.get().is_some();

    // Click anywhere in the main area closes open layer panels
    let on_main_click = move |_: web_sys::MouseEvent| {
        state.layer_panel_open.set(None);
    };

    view! {
        <div class="main" on:click=on_main_click>
            <Toolbar />
            {move || {
                if has_file() {
                    view! {
                        // Overview strip (top)
                        <OverviewPanel />

                        // Main view (takes remaining space)
                        <div class="main-view">
                            // Show spectrogram or waveform based on main_view signal
                            {move || match state.main_view.get() {
                                OverviewView::Spectrogram => view! { <Spectrogram /> }.into_any(),
                                OverviewView::Waveform => view! {
                                    <div class="main-waveform-full">
                                        <Waveform />
                                    </div>
                                }.into_any(),
                            }}

                            // Floating overlay layer
                            <div class="main-overlays">
                                <PlayControls />
                                <MainViewButton />
                                <FreqRangeButton />
                                <FrequencyFocusButton />
                                <ListenModeButton />
                                <ToolButton />
                            </div>
                        </div>

                        <AnalysisPanel />
                    }.into_any()
                } else {
                    view! {
                        <div class="empty-state">
                            "Drop WAV, FLAC or MP3 files into the sidebar"
                        </div>
                    }.into_any()
                }
            }}
        </div>
    }
}

/// Floating button (top-left of main overlays) to toggle the main panel between
/// Spectrogram and Waveform view.
#[component]
fn MainViewButton() -> impl IntoView {
    let state = expect_context::<AppState>();
    let is_open = move || state.layer_panel_open.get() == Some(LayerPanel::Tool);

    // Use a dedicated LayerPanel variant for view — reuse OverviewLayers isn't right.
    // We'll create an inline toggle without a panel (simpler):
    // Clicking cycles Spectrogram → Waveform → Spectrogram.
    let _ = is_open; // suppress unused warning

    view! {
        <div
            style=move || format!("position: absolute; top: 10px; left: 56px; pointer-events: none; opacity: {}; transition: opacity 0.1s;",
                if state.mouse_in_label_area.get() { "0" } else { "1" })
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
        >
            <button
                class="layer-btn"
                style=move || format!("pointer-events: {};", if state.mouse_in_label_area.get() { "none" } else { "auto" })
                title="Toggle view (Spectrogram / Waveform)"
                on:click=move |_| {
                    state.main_view.update(|v| {
                        *v = match *v {
                            OverviewView::Spectrogram => OverviewView::Waveform,
                            OverviewView::Waveform    => OverviewView::Spectrogram,
                        };
                    });
                }
            >
                <span class="layer-btn-category">"View"</span>
                <span class="layer-btn-value">{move || match state.main_view.get() {
                    OverviewView::Spectrogram => "Spec",
                    OverviewView::Waveform    => "Wave",
                }}</span>
            </button>
        </div>
    }
}
