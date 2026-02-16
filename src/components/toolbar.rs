use leptos::prelude::*;
use wasm_bindgen::JsCast;
use crate::state::{AppState, PlaybackMode};
use crate::audio::playback;

#[component]
pub fn Toolbar() -> impl IntoView {
    let state = expect_context::<AppState>();

    let state_play = state.clone();
    let on_play_stop = move |_| {
        if state_play.is_playing.get_untracked() {
            playback::stop(&state_play);
        } else {
            playback::play(&state_play);
        }
    };

    let has_file = move || state.current_file_index.get().is_some();
    let is_playing = move || state.is_playing.get();
    let current_mode = move || state.playback_mode.get();

    let set_mode = move |mode: PlaybackMode| {
        state.playback_mode.set(mode);
    };

    // Track shift key for HET 5 kHz stepping
    let shift_held = RwSignal::new(false);

    let on_het_freq_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            let freq_khz = if shift_held.get_untracked() {
                (val / 5.0).round() * 5.0
            } else {
                val
            };
            state.het_frequency.set(freq_khz * 1000.0);
        }
    };

    let on_te_factor_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            state.te_factor.set(val);
        }
    };

    let on_ps_factor_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            state.ps_factor.set(val);
        }
    };

    view! {
        <div class="toolbar"
            on:keydown=move |ev: web_sys::KeyboardEvent| {
                if ev.key() == "Shift" { shift_held.set(true); }
            }
            on:keyup=move |ev: web_sys::KeyboardEvent| {
                if ev.key() == "Shift" { shift_held.set(false); }
            }
        >
            <span class="toolbar-brand">"Batgram"</span>
            <div class="toolbar-sep"></div>

            // Play/Stop
            <button
                class=move || if is_playing() { "play-btn playing" } else { "play-btn" }
                on:click=on_play_stop
                disabled=move || !has_file()
            >
                {move || if is_playing() { "Stop" } else { "Play" }}
            </button>

            <div class="toolbar-sep"></div>

            // Mode selector — radio-style buttons
            <div class="mode-group">
                <button
                    class=move || if current_mode() == PlaybackMode::TimeExpansion { "mode-btn active" } else { "mode-btn" }
                    on:click=move |_| set_mode(PlaybackMode::TimeExpansion)
                >
                    "TE"
                </button>
                <button
                    class=move || if current_mode() == PlaybackMode::Heterodyne { "mode-btn active" } else { "mode-btn" }
                    on:click=move |_| set_mode(PlaybackMode::Heterodyne)
                >
                    "HET"
                </button>
                <button
                    class=move || if current_mode() == PlaybackMode::PitchShift { "mode-btn active" } else { "mode-btn" }
                    on:click=move |_| set_mode(PlaybackMode::PitchShift)
                >
                    "PS"
                </button>
                <button
                    class=move || if current_mode() == PlaybackMode::Normal { "mode-btn active" } else { "mode-btn" }
                    on:click=move |_| set_mode(PlaybackMode::Normal)
                >
                    "1:1"
                </button>
            </div>

            // Mode-specific controls
            {move || {
                match current_mode() {
                    PlaybackMode::TimeExpansion => {
                        view! {
                            <label class="mode-param">
                                <span class="mode-param-value">{move || format!("{}x", state.te_factor.get() as u32)}</span>
                                <input
                                    type="range"
                                    min="2"
                                    max="40"
                                    step="1"
                                    prop:value=move || (state.te_factor.get()).to_string()
                                    on:input=on_te_factor_change
                                />
                            </label>
                        }.into_any()
                    }
                    PlaybackMode::Heterodyne => {
                        let on_het_enter = move |_: web_sys::MouseEvent| {
                            state.het_interacting.set(true);
                        };
                        let on_het_leave = move |_: web_sys::MouseEvent| {
                            state.het_interacting.set(false);
                        };
                        view! {
                            <label class="mode-param"
                                on:mouseenter=on_het_enter
                                on:mouseleave=on_het_leave
                            >
                                <span class="mode-param-value">{move || format!("{:.0} kHz", state.het_frequency.get() / 1000.0)}</span>
                                <input
                                    type="range"
                                    min="10"
                                    max="150"
                                    step="1"
                                    prop:value=move || (state.het_frequency.get() / 1000.0).to_string()
                                    on:input=on_het_freq_change
                                />
                            </label>
                        }.into_any()
                    }
                    PlaybackMode::PitchShift => {
                        view! {
                            <label class="mode-param">
                                <span class="mode-param-value">{move || format!("÷{}", state.ps_factor.get() as u32)}</span>
                                <input
                                    type="range"
                                    min="2"
                                    max="20"
                                    step="1"
                                    prop:value=move || (state.ps_factor.get()).to_string()
                                    on:input=on_ps_factor_change
                                />
                            </label>
                        }.into_any()
                    }
                    PlaybackMode::Normal => {
                        view! {
                            <span class="mode-hint">"Native rate"</span>
                        }.into_any()
                    }
                }
            }}
        </div>
    }
}
