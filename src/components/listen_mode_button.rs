use leptos::prelude::*;
use crate::state::{AppState, BandpassStrength, LayerPanel, ListenAdjustment, PlaybackMode};

fn layer_opt_class(active: bool) -> &'static str {
    if active { "layer-panel-opt sel" } else { "layer-panel-opt" }
}

fn toggle_panel(state: &AppState, panel: LayerPanel) {
    state.layer_panel_open.update(|p| {
        *p = if *p == Some(panel) { None } else { Some(panel) };
    });
}

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
pub fn ListenModeButton() -> impl IntoView {
    let state = expect_context::<AppState>();
    let is_open = move || state.layer_panel_open.get() == Some(LayerPanel::ListenMode);

    let listen_label = move || {
        if state.listen_adjustment.get() == ListenAdjustment::Auto {
            "AUTO"
        } else {
            match state.playback_mode.get() {
                PlaybackMode::Heterodyne   => "HET",
                PlaybackMode::TimeExpansion => "TE",
                PlaybackMode::PitchShift   => "PS",
                PlaybackMode::ZeroCrossing => "ZC",
                PlaybackMode::Normal       => "1:1",
            }
        }
    };

    let set_auto = move |_| {
        state.listen_adjustment.set(ListenAdjustment::Auto);
    };

    let make_set_manual = |state: AppState, mode: PlaybackMode| {
        move |_: web_sys::MouseEvent| {
            state.listen_adjustment.set(ListenAdjustment::Manual);
            state.playback_mode.set(mode);
        }
    };

    let on_gain_change = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let input: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            state.gain_db.set(val);
        }
    };

    let on_gain_reset = move |_: web_sys::MouseEvent| {
        state.gain_db.set(0.0);
    };

    let on_het_freq_change = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let input: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            state.het_frequency.set(val * 1000.0);
        }
    };

    let on_het_cutoff_change = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let input: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            state.het_cutoff.set(val * 1000.0);
        }
    };

    let on_te_change = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let input: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            state.te_factor.set(val);
        }
    };

    let on_ps_change = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let input: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            state.ps_factor.set(val);
        }
    };

    let on_zc_change = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let input: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            state.zc_factor.set(val);
        }
    };

    view! {
        // Anchored bottom-left of main-overlays (above tool button)
        <div
            style="position: absolute; bottom: 10px; left: 10px; pointer-events: none;"
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
        >
            <div style="position: relative; pointer-events: auto;">
                <button
                    class=move || if is_open() { "layer-btn open" } else { "layer-btn" }
                    on:click=move |_| toggle_panel(&state, LayerPanel::ListenMode)
                    title="Listen mode"
                >
                    {listen_label}
                </button>
                {move || is_open().then(|| {
                    let adj = state.listen_adjustment.get();
                    let mode = state.playback_mode.get();
                    let is_zc = mode == PlaybackMode::ZeroCrossing;

                    view! {
                        <div class="layer-panel" style="bottom: 34px; left: 0; min-width: 210px;">
                            // ── Listen Mode ─────────────────────────────────
                            <div class="layer-panel-title">"Listen Mode"</div>
                            <button class=move || layer_opt_class(state.listen_adjustment.get() == ListenAdjustment::Auto)
                                on:click=set_auto
                            >"AUTO"</button>
                            <button class=move || layer_opt_class(
                                    state.listen_adjustment.get() == ListenAdjustment::Manual
                                    && state.playback_mode.get() == PlaybackMode::Heterodyne
                                )
                                on:click=make_set_manual(state, PlaybackMode::Heterodyne)
                            >"HET — Heterodyne"</button>
                            <button class=move || layer_opt_class(
                                    state.listen_adjustment.get() == ListenAdjustment::Manual
                                    && state.playback_mode.get() == PlaybackMode::TimeExpansion
                                )
                                on:click=make_set_manual(state, PlaybackMode::TimeExpansion)
                            >"TE — Time Expansion"</button>
                            <button class=move || layer_opt_class(
                                    state.listen_adjustment.get() == ListenAdjustment::Manual
                                    && state.playback_mode.get() == PlaybackMode::PitchShift
                                )
                                on:click=make_set_manual(state, PlaybackMode::PitchShift)
                            >"PS — Pitch Shift"</button>
                            <button class=move || layer_opt_class(
                                    state.listen_adjustment.get() == ListenAdjustment::Manual
                                    && state.playback_mode.get() == PlaybackMode::ZeroCrossing
                                )
                                on:click=make_set_manual(state, PlaybackMode::ZeroCrossing)
                            >"ZC — Zero Crossing"</button>
                            <button class=move || layer_opt_class(
                                    state.listen_adjustment.get() == ListenAdjustment::Manual
                                    && state.playback_mode.get() == PlaybackMode::Normal
                                )
                                on:click=make_set_manual(state, PlaybackMode::Normal)
                            >"1:1 — Native rate"</button>

                            // ── Adjustment sliders (when Manual) ─────────────
                            {(adj == ListenAdjustment::Manual).then(|| {
                                view! {
                                    <hr />
                                    <div class="layer-panel-title">"Adjustment"</div>
                                    {match mode {
                                        PlaybackMode::Heterodyne => view! {
                                            <div class="layer-panel-slider-row">
                                                <label>"Freq"</label>
                                                <input type="range" min="10" max="150" step="0.5"
                                                    prop:value=move || (state.het_frequency.get() / 1000.0).to_string()
                                                    on:input=on_het_freq_change
                                                />
                                                <span>{move || format!("{:.0} kHz", state.het_frequency.get() / 1000.0)}</span>
                                            </div>
                                            <div class="layer-panel-slider-row">
                                                <label>"LP cutoff"</label>
                                                <input type="range" min="1" max="30" step="0.5"
                                                    prop:value=move || (state.het_cutoff.get() / 1000.0).to_string()
                                                    on:input=on_het_cutoff_change
                                                />
                                                <span>{move || format!("{:.0} kHz", state.het_cutoff.get() / 1000.0)}</span>
                                            </div>
                                        }.into_any(),
                                        PlaybackMode::TimeExpansion => view! {
                                            <div class="layer-panel-slider-row">
                                                <label>"Factor"</label>
                                                <input type="range" min="2" max="40" step="1"
                                                    prop:value=move || (state.te_factor.get() as u32).to_string()
                                                    on:input=on_te_change
                                                />
                                                <span>{move || format!("{}x", state.te_factor.get() as u32)}</span>
                                            </div>
                                        }.into_any(),
                                        PlaybackMode::PitchShift => view! {
                                            <div class="layer-panel-slider-row">
                                                <label>"Factor"</label>
                                                <input type="range" min="2" max="20" step="1"
                                                    prop:value=move || (state.ps_factor.get() as u32).to_string()
                                                    on:input=on_ps_change
                                                />
                                                <span>{move || format!("÷{}", state.ps_factor.get() as u32)}</span>
                                            </div>
                                        }.into_any(),
                                        PlaybackMode::ZeroCrossing => view! {
                                            <div class="layer-panel-slider-row">
                                                <label>"Division"</label>
                                                <input type="range" min="2" max="32" step="1"
                                                    prop:value=move || (state.zc_factor.get() as u32).to_string()
                                                    on:input=on_zc_change
                                                />
                                                <span>{move || format!("÷{}", state.zc_factor.get() as u32)}</span>
                                            </div>
                                        }.into_any(),
                                        PlaybackMode::Normal => view! { <span></span> }.into_any(),
                                    }}
                                }
                            })}

                            // ── Bandpass ─────────────────────────────────────
                            <hr />
                            <div class="layer-panel-title">"Bandpass"</div>
                            <div style="display: flex; flex-wrap: wrap; gap: 2px; padding: 0 6px 4px;">
                                <button class=move || layer_opt_class(state.bandpass_strength.get() == BandpassStrength::Auto)
                                    on:click=move |_| state.bandpass_strength.set(BandpassStrength::Auto)
                                >"Auto"</button>
                                <button class=move || layer_opt_class(state.bandpass_strength.get() == BandpassStrength::Off)
                                    on:click=move |_| state.bandpass_strength.set(BandpassStrength::Off)
                                >"Off"</button>
                                <button class=move || layer_opt_class(state.bandpass_strength.get() == BandpassStrength::Some)
                                    on:click=move |_| state.bandpass_strength.set(BandpassStrength::Some)
                                >"Some"</button>
                                <button class=move || layer_opt_class(state.bandpass_strength.get() == BandpassStrength::Strong)
                                    on:click=move |_| state.bandpass_strength.set(BandpassStrength::Strong)
                                >"Strong"</button>
                            </div>

                            // ── Gain ─────────────────────────────────────────
                            {(!is_zc).then(|| view! {
                                <hr />
                                <div class="layer-panel-title">"Gain"</div>
                                <div class="layer-panel-slider-row">
                                    <label>"Level"</label>
                                    <input type="range" min="-30" max="30" step="0.5"
                                        prop:value=move || {
                                            if state.auto_gain.get() {
                                                compute_auto_gain(&state).to_string()
                                            } else {
                                                state.gain_db.get().to_string()
                                            }
                                        }
                                        on:input=on_gain_change
                                        on:dblclick=on_gain_reset
                                        disabled=move || state.auto_gain.get()
                                    />
                                    <span>{move || {
                                        let db = if state.auto_gain.get() {
                                            compute_auto_gain(&state)
                                        } else {
                                            state.gain_db.get()
                                        };
                                        if db > 0.0 { format!("+{:.1} dB", db) }
                                        else         { format!("{:.1} dB",  db) }
                                    }}</span>
                                </div>
                                <button class=move || layer_opt_class(state.auto_gain.get())
                                    on:click=move |_| state.auto_gain.update(|v| *v = !*v)
                                >"Auto gain"</button>
                            })}
                        </div>
                    }
                })}
            </div>
        </div>
    }
}
