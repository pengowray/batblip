use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use crate::state::{AppState, LayerPanel, OverviewView};
use crate::audio::playback;
use crate::audio::microphone;
use crate::components::file_sidebar::FileSidebar;
use crate::components::right_sidebar::RightSidebar;
use crate::components::spectrogram::Spectrogram;
use crate::components::waveform::Waveform;
use crate::components::toolbar::Toolbar;
use crate::components::analysis_panel::AnalysisPanel;
use crate::components::overview::OverviewPanel;
use crate::components::play_controls::PlayControls;
use crate::components::hfr_button::HfrButton;
use crate::components::hfr_mode_button::HfrModeButton;
use crate::components::tool_button::ToolButton;
use crate::components::freq_range_button::FreqRangeButton;
use crate::components::xc_browser::XcBrowser;
use crate::components::file_sidebar::{fetch_demo_index, load_single_demo};

#[component]
pub fn App() -> impl IntoView {
    let state = AppState::new();
    provide_context(state);

    // Auto-load demo sound from URL hash (e.g. #XC928094)
    if let Some(window) = web_sys::window() {
        if let Ok(hash) = window.location().hash() {
            let trimmed = hash.trim_start_matches('#');
            if trimmed.len() >= 3 && trimmed[..2].eq_ignore_ascii_case("XC") && trimmed[2..].chars().all(|c| c.is_ascii_digit()) {
                let xc_id = trimmed.to_uppercase();
                state.loading_count.update(|c| *c += 1);
                wasm_bindgen_futures::spawn_local(async move {
                    match fetch_demo_index().await {
                        Ok(entries) => {
                            let found = entries.iter().find(|e| {
                                e.filename.to_uppercase().contains(&xc_id)
                            });
                            if let Some(entry) = found {
                                if let Err(e) = load_single_demo(entry, state).await {
                                    log::error!("Failed to load {}: {}", xc_id, e);
                                    state.show_error_toast(format!("Failed to load {}", xc_id));
                                }
                            } else {
                                state.show_info_toast(format!(
                                    "{} is not available in the demo audio. Only a small selection of recordings are included.",
                                    xc_id
                                ));
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to fetch demo index: {e}");
                            state.show_error_toast("Could not load demo sounds index");
                        }
                    }
                    state.loading_count.update(|c| *c = c.saturating_sub(1));
                });
            }
        }
    }

    // Live playback parameter switching: when any playback-relevant signal
    // changes while audio is playing, restart the stream from the current
    // playhead position with fresh parameters.
    {
        let first_run = std::cell::Cell::new(true);
        Effect::new(move |_| {
            // Track all playback-relevant signals (subscribes to changes)
            let _ = state.playback_mode.get();
            let _ = state.te_factor.get();
            let _ = state.ps_factor.get();
            let _ = state.zc_factor.get();
            let _ = state.het_frequency.get();
            let _ = state.het_cutoff.get();
            let _ = state.gain_db.get();
            let _ = state.auto_gain.get();
            let _ = state.filter_enabled.get();
            let _ = state.filter_freq_low.get();
            let _ = state.filter_freq_high.get();
            let _ = state.filter_db_below.get();
            let _ = state.filter_db_selected.get();
            let _ = state.filter_db_harmonics.get();
            let _ = state.filter_db_above.get();
            let _ = state.filter_band_mode.get();
            let _ = state.filter_quality.get();
            let _ = state.bandpass_mode.get();

            if first_run.get() {
                first_run.set(false);
                return;
            }

            if state.is_playing.get_untracked() {
                playback::schedule_replay_live(&state);
            }
        });
    }

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
        if (ev.key() == "l" || ev.key() == "L") && !ev.ctrl_key() && !ev.meta_key() && !ev.alt_key() {
            ev.prevent_default();
            let st = state_kb;
            wasm_bindgen_futures::spawn_local(async move {
                microphone::toggle_listen(&st).await;
            });
        }
        if (ev.key() == "r" || ev.key() == "R") && !ev.ctrl_key() && !ev.meta_key() && !ev.alt_key() {
            ev.prevent_default();
            let st = state_kb;
            wasm_bindgen_futures::spawn_local(async move {
                microphone::toggle_record(&st).await;
            });
        }
        if (ev.key() == "h" || ev.key() == "H") && !ev.ctrl_key() && !ev.meta_key() && !ev.alt_key() {
            ev.prevent_default();
            state_kb.hfr_enabled.update(|v| *v = !*v);
        }
        if ev.key() == "Escape" {
            if state_kb.xc_browser_open.get_untracked() {
                state_kb.xc_browser_open.set(false);
                return;
            }
            if state_kb.mic_listening.get_untracked() || state_kb.mic_recording.get_untracked() {
                microphone::stop_all(&state_kb);
            }
        }
    });
    let window = web_sys::window().unwrap();
    let _ = window.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref());
    handler.forget();

    let is_mobile = state.is_mobile.get_untracked();

    let grid_style = move || {
        if is_mobile {
            // Sidebars are position:fixed overlays, so single column for main content
            "grid-template-columns: 1fr".to_string()
        } else {
            let left = if state.sidebar_collapsed.get() { 0 } else { state.sidebar_width.get() as i32 };
            let right = if state.right_sidebar_collapsed.get() { 0 } else { state.right_sidebar_width.get() as i32 };
            format!("grid-template-columns: {}px 1fr {}px", left, right)
        }
    };

    // Android back button: close sidebar when open
    if is_mobile {
        let state_back = state.clone();
        let on_popstate = wasm_bindgen::closure::Closure::<dyn Fn(web_sys::Event)>::new(move |_: web_sys::Event| {
            if !state_back.right_sidebar_collapsed.get_untracked() {
                state_back.right_sidebar_collapsed.set(true);
                let _ = web_sys::window().unwrap().history().unwrap()
                    .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", None);
            } else if !state_back.sidebar_collapsed.get_untracked() {
                state_back.sidebar_collapsed.set(true);
                let _ = web_sys::window().unwrap().history().unwrap()
                    .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", None);
            }
        });
        let window = web_sys::window().unwrap();
        let _ = window.add_event_listener_with_callback("popstate", on_popstate.as_ref().unchecked_ref());
        on_popstate.forget();
        // Push initial history entry so back button has something to pop
        let _ = window.history().unwrap()
            .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", None);
    }

    view! {
        <div class="app" style=grid_style>
            <FileSidebar />
            {if is_mobile {
                Some(view! {
                    <div
                        class=move || if !state.sidebar_collapsed.get() || !state.right_sidebar_collapsed.get() { "sidebar-backdrop open" } else { "sidebar-backdrop" }
                        on:click=move |_| {
                            state.sidebar_collapsed.set(true);
                            state.right_sidebar_collapsed.set(true);
                        }
                    ></div>
                })
            } else {
                None
            }}
            <MainArea />
            <RightSidebar />
            {move || state.xc_browser_open.get().then(|| view! { <XcBrowser /> })}
        </div>
    }
}

#[component]
fn MainArea() -> impl IntoView {
    let state = expect_context::<AppState>();
    let has_file = move || state.current_file_index.get().is_some();

    let is_mobile = state.is_mobile.get_untracked();

    // Click anywhere in the main area closes open layer panels (and sidebar on mobile)
    let on_main_click = move |_: web_sys::MouseEvent| {
        state.layer_panel_open.set(None);
        if is_mobile {
            state.sidebar_collapsed.set(true);
            state.right_sidebar_collapsed.set(true);
        }
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
                                // Unsaved recording banner (web only)
                                {move || {
                                    if state.is_tauri { return None; }
                                    let files = state.files.get();
                                    let idx = state.current_file_index.get()?;
                                    let file = files.get(idx)?;
                                    if !file.is_recording { return None; }
                                    let name = file.name.clone();
                                    Some(view! {
                                        <div
                                            class="unsaved-banner"
                                            on:click=move |_| {
                                                let files = state.files.get_untracked();
                                                let idx = state.current_file_index.get_untracked();
                                                if let Some(i) = idx {
                                                    if let Some(f) = files.get(i) {
                                                        microphone::download_wav(&f.audio.samples, f.audio.sample_rate, &name);
                                                    }
                                                }
                                            }
                                        >
                                            "Unsaved recording \u{2014} click to download"
                                        </div>
                                    })
                                }}
                                <PlayControls />
                                <MainViewButton />
                                <FreqRangeButton />
                                <HfrButton />
                                <HfrModeButton />
                                <ToolButton />
                            </div>
                        </div>

                        <AnalysisPanel />
                    }.into_any()
                } else {
                    if is_mobile {
                        view! {
                            <div class="empty-state">
                                "Tap \u{2630} to load audio files"
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div class="empty-state">
                                "Drop WAV, FLAC or MP3 files into the sidebar"
                            </div>
                        }.into_any()
                    }
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
