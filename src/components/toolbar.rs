use leptos::prelude::*;
use wasm_bindgen::JsCast;
use crate::state::{AppState, PlaybackMode};
use crate::audio::playback;

fn compute_auto_gain(state: &AppState) -> f64 {
    let files = state.files.get();
    let idx = state.current_file_index.get();
    let Some(file) = idx.and_then(|i| files.get(i)) else { return 0.0 };
    let peak = file.audio.samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    if peak < 1e-10 { return 0.0; }
    let peak_db = 20.0 * (peak as f64).log10();
    -3.0 - peak_db
}

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
        if mode == PlaybackMode::ZeroCrossing {
            state.auto_gain.set(false);
        }
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
            // Re-render HET audio from current position if playing
            if state.is_playing.get_untracked()
                && state.playback_mode.get_untracked() == PlaybackMode::Heterodyne
            {
                playback::replay_het(&state);
            }
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

    let on_zc_factor_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            state.zc_factor.set(val);
        }
    };

    let on_gain_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            state.gain_db.set(val);
        }
    };

    let on_auto_gain_toggle = move |_| {
        state.auto_gain.update(|v| *v = !*v);
    };

    let on_gain_reset = move |_: web_sys::MouseEvent| {
        state.gain_db.set(0.0);
    };

    let show_about = RwSignal::new(false);

    view! {
        <div class="toolbar"
            on:keydown=move |ev: web_sys::KeyboardEvent| {
                if ev.key() == "Shift" { shift_held.set(true); }
            }
            on:keyup=move |ev: web_sys::KeyboardEvent| {
                if ev.key() == "Shift" { shift_held.set(false); }
            }
        >
            <span
                class="toolbar-brand"
                style=move || if state.sidebar_collapsed.get() { "margin-left: 24px; cursor: pointer" } else { "cursor: pointer" }
                on:click=move |_| show_about.set(true)
                title="About Batchi"
            >"Batchi"</span>

            {move || show_about.get().then(|| view! {
                <div class="about-overlay" on:click=move |_| show_about.set(false)>
                    <div class="about-dialog" on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()>
                        <div class="about-header">
                            <span class="about-title">"Batchi"</span>
                            <span class="about-version">"v0.1.1"</span>
                        </div>
                        <p class="about-desc">"Bat call viewer and acoustic analysis tool for WAV and FLAC recordings."</p>
                        <div class="about-modes">
                            <div class="about-mode"><span class="about-mode-tag">"HET"</span>" Heterodyne — mix with a local oscillator to shift ultrasonic calls into audible range"</div>
                            <div class="about-mode"><span class="about-mode-tag">"TE"</span>" Time Expansion — slow playback to lower pitch proportionally"</div>
                            <div class="about-mode"><span class="about-mode-tag">"PS"</span>" Pitch Shift — lower pitch while preserving original duration"</div>
                            <div class="about-mode"><span class="about-mode-tag">"ZC"</span>" Zero Crossing — frequency division via zero-crossing detection"</div>
                        </div>
                        <button class="about-close" on:click=move |_| show_about.set(false)>"Close"</button>
                    </div>
                </div>
            })}
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
                    class=move || if current_mode() == PlaybackMode::Heterodyne { "mode-btn active" } else { "mode-btn" }
                    on:click=move |_| set_mode(PlaybackMode::Heterodyne)
                    title="Heterodyne — mix with a local oscillator to shift ultrasonic frequencies into audible range"
                >
                    "HET"
                </button>
                <button
                    class=move || if current_mode() == PlaybackMode::TimeExpansion { "mode-btn active" } else { "mode-btn" }
                    on:click=move |_| set_mode(PlaybackMode::TimeExpansion)
                    title="Time Expansion — slow down playback to lower pitch proportionally"
                >
                    "TE"
                </button>
                <button
                    class=move || if current_mode() == PlaybackMode::PitchShift { "mode-btn active" } else { "mode-btn" }
                    on:click=move |_| set_mode(PlaybackMode::PitchShift)
                    title="Pitch Shift — lower pitch while preserving original duration"
                >
                    "PS"
                </button>
                <button
                    class=move || if current_mode() == PlaybackMode::ZeroCrossing { "mode-btn active" } else { "mode-btn" }
                    on:click=move |_| set_mode(PlaybackMode::ZeroCrossing)
                    title="Zero Crossing — frequency division via zero-crossing detection"
                >
                    "ZC"
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
                    PlaybackMode::ZeroCrossing => {
                        view! {
                            <label class="mode-param">
                                <span class="mode-param-value">{move || format!("÷{}", state.zc_factor.get() as u32)}</span>
                                <input
                                    type="range"
                                    min="2"
                                    max="32"
                                    step="1"
                                    prop:value=move || (state.zc_factor.get()).to_string()
                                    on:input=on_zc_factor_change
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

            <div class="toolbar-sep"></div>

            // Gain controls
            <label class="mode-param gain-control">
                <span
                    class="mode-param-value gain-label"
                    title="Double-click to reset to 0 dB"
                    on:dblclick=on_gain_reset
                >
                    {move || {
                        let db = if state.auto_gain.get() {
                            compute_auto_gain(&state)
                        } else {
                            state.gain_db.get()
                        };
                        if db > 0.0 {
                            format!("+{:.1} dB", db)
                        } else {
                            format!("{:.1} dB", db)
                        }
                    }}
                </span>
                <input
                    type="range"
                    min="-30"
                    max="30"
                    step="0.5"
                    prop:value=move || {
                        if state.auto_gain.get() {
                            compute_auto_gain(&state).to_string()
                        } else {
                            state.gain_db.get().to_string()
                        }
                    }
                    on:input=on_gain_change
                    disabled=move || state.auto_gain.get()
                />
            </label>
            <button
                class=move || if state.auto_gain.get() { "mode-btn active" } else { "mode-btn" }
                on:click=on_auto_gain_toggle
                title="Auto-gain: normalize playback to -3 dB peak"
                style:display=move || if current_mode() == PlaybackMode::ZeroCrossing { "none" } else { "" }
            >
                "Auto"
            </button>
        </div>
    }
}
