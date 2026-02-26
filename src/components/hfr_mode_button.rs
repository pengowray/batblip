use leptos::prelude::*;
use crate::state::{AppState, AutoFactorMode, BandpassMode, BandpassRange, FilterQuality, SpectrogramHandle, LayerPanel, PlaybackMode};

fn layer_opt_class(active: bool) -> &'static str {
    if active { "layer-panel-opt sel" } else { "layer-panel-opt" }
}

fn toggle_panel(state: &AppState, panel: LayerPanel) {
    state.layer_panel_open.update(|p| {
        *p = if *p == Some(panel) { None } else { Some(panel) };
    });
}

#[component]
pub fn HfrModeButton() -> impl IntoView {
    let state = expect_context::<AppState>();
    let is_open = move || state.layer_panel_open.get() == Some(LayerPanel::HfrMode);

    let mode_abbr = move || match state.playback_mode.get() {
        PlaybackMode::Heterodyne   => "HET",
        PlaybackMode::TimeExpansion => "TE",
        PlaybackMode::PitchShift   => "PS",
        PlaybackMode::ZeroCrossing => "ZC",
        PlaybackMode::Normal       => "1:1",
    };

    let set_mode = |state: AppState, mode: PlaybackMode| {
        move |_: web_sys::MouseEvent| {
            state.playback_mode.set(mode);
        }
    };

    let on_te_change = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let input: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            state.te_factor_auto.set(false);
            state.playback_mode.set(PlaybackMode::TimeExpansion);
            state.te_factor.set(val);
        }
    };

    let on_ps_change = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let input: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            state.ps_factor_auto.set(false);
            state.playback_mode.set(PlaybackMode::PitchShift);
            state.ps_factor.set(val);
        }
    };

    let on_zc_change = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let input: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            state.playback_mode.set(PlaybackMode::ZeroCrossing);
            state.zc_factor.set(val);
        }
    };

    view! {
        // Only visible when HFR is enabled
        <Show when=move || state.hfr_enabled.get()>
            <div
                style=move || format!("position: absolute; bottom: 46px; left: 56px; pointer-events: none; opacity: {}; transition: opacity 0.1s;",
                    if state.mouse_in_label_area.get() { "0" } else { "1" })
                on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
            >
                <div style=move || format!("position: relative; pointer-events: {};",
                    if state.mouse_in_label_area.get() { "none" } else { "auto" })>
                    <button
                        class=move || if is_open() { "layer-btn open" } else { "layer-btn" }
                        on:click=move |_| toggle_panel(&state, LayerPanel::HfrMode)
                        title="HFR playback mode"
                    >
                        <span class="layer-btn-category">"HFR Mode"</span>
                        <span class="layer-btn-value">{mode_abbr}</span>
                    </button>
                    {move || is_open().then(|| {
                        let mode = state.playback_mode.get();

                        view! {
                            <div class="layer-panel" style="bottom: 34px; left: 0; min-width: 210px;">
                                // ── HFR Mode ─────────────────────────────────
                                <div class="layer-panel-title">"HFR Mode"</div>
                                <button class=move || layer_opt_class(state.playback_mode.get() == PlaybackMode::Normal)
                                    on:click=set_mode(state, PlaybackMode::Normal)
                                >"1:1 — Normal"</button>
                                <button class=move || layer_opt_class(state.playback_mode.get() == PlaybackMode::Heterodyne)
                                    on:click=set_mode(state, PlaybackMode::Heterodyne)
                                >"HET — Heterodyne"</button>
                                <button class=move || layer_opt_class(state.playback_mode.get() == PlaybackMode::TimeExpansion)
                                    on:click=set_mode(state, PlaybackMode::TimeExpansion)
                                >"TE — Time Expansion"</button>
                                <button class=move || layer_opt_class(state.playback_mode.get() == PlaybackMode::PitchShift)
                                    on:click=set_mode(state, PlaybackMode::PitchShift)
                                >"PS — Pitch Shift"</button>
                                <button class=move || layer_opt_class(state.playback_mode.get() == PlaybackMode::ZeroCrossing)
                                    on:click=set_mode(state, PlaybackMode::ZeroCrossing)
                                >"ZC — Zero Crossing"</button>

                                // ── Inaudible notice for 1:1 with ultrasonic focus ──
                                {(mode == PlaybackMode::Normal && state.ff_freq_lo.get() >= 20_000.0).then(|| {
                                    view! {
                                        <div style="padding: 4px 8px; font-size: 10px; color: #e0a030; line-height: 1.3;">
                                            "Focus is above human hearing. 1:1 mode won\u{2019}t make it audible"
                                        </div>
                                    }
                                })}

                                // ── Adjustment ─────────────────────────────────
                                {(mode != PlaybackMode::Normal).then(|| {
                                    view! {
                                        <hr />
                                        <div class="layer-panel-title">"Adjustment"</div>
                                        {match mode {
                                            PlaybackMode::Heterodyne => view! {
                                                <div class="layer-panel-slider-row het-text-row"
                                                    on:mouseenter=move |_| {
                                                        state.het_interacting.set(true);
                                                        state.spec_hover_handle.set(Some(SpectrogramHandle::HetCenter));
                                                    }
                                                    on:mouseleave=move |_| {
                                                        state.het_interacting.set(false);
                                                        state.spec_hover_handle.set(None);
                                                    }
                                                >
                                                    <label>"Freq"</label>
                                                    <span class="het-value">{move || format!("{:.1} kHz", state.het_frequency.get() / 1000.0)}</span>
                                                    <button class=move || if state.het_freq_auto.get() { "auto-toggle on" } else { "auto-toggle" }
                                                        on:click=move |_| state.het_freq_auto.update(|v| *v = !*v)
                                                        title="Toggle auto HET frequency"
                                                    >"A"</button>
                                                </div>
                                                <div class="layer-panel-slider-row het-text-row"
                                                    on:mouseenter=move |_| {
                                                        state.het_interacting.set(true);
                                                        state.spec_hover_handle.set(Some(SpectrogramHandle::HetBandUpper));
                                                    }
                                                    on:mouseleave=move |_| {
                                                        state.het_interacting.set(false);
                                                        state.spec_hover_handle.set(None);
                                                    }
                                                >
                                                    <label>"LP cutoff"</label>
                                                    <span class="het-value">{move || format!("{:.1} kHz", state.het_cutoff.get() / 1000.0)}</span>
                                                    <button class=move || if state.het_cutoff_auto.get() { "auto-toggle on" } else { "auto-toggle" }
                                                        on:click=move |_| state.het_cutoff_auto.update(|v| *v = !*v)
                                                        title="Toggle auto LP cutoff"
                                                    >"A"</button>
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
                                                    <button class=move || if state.te_factor_auto.get() { "auto-toggle on" } else { "auto-toggle" }
                                                        on:click=move |_| state.te_factor_auto.update(|v| *v = !*v)
                                                        title="Toggle auto TE factor"
                                                    >"A"</button>
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
                                                    <button class=move || if state.ps_factor_auto.get() { "auto-toggle on" } else { "auto-toggle" }
                                                        on:click=move |_| state.ps_factor_auto.update(|v| *v = !*v)
                                                        title="Toggle auto PS factor"
                                                    >"A"</button>
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

                                        // Auto factor mode switch
                                        {move || {
                                            let any_auto = state.te_factor_auto.get()
                                                || state.ps_factor_auto.get()
                                                || state.het_freq_auto.get()
                                                || state.het_cutoff_auto.get();
                                            any_auto.then(|| view! {
                                                <div class="layer-panel-title" style="margin-top: 4px;">"Auto mode"</div>
                                                <div style="display: flex; gap: 2px; padding: 0 6px 4px;">
                                                    <button class=move || layer_opt_class(state.auto_factor_mode.get() == AutoFactorMode::Target3k)
                                                        on:click=move |_| state.auto_factor_mode.set(AutoFactorMode::Target3k)
                                                        title="Factor = FF center / 3 kHz"
                                                    >"3k"</button>
                                                    <button class=move || layer_opt_class(state.auto_factor_mode.get() == AutoFactorMode::MinAudible)
                                                        on:click=move |_| state.auto_factor_mode.set(AutoFactorMode::MinAudible)
                                                        title="Factor = FF high / 20 kHz"
                                                    >"Min aud"</button>
                                                    <button class=move || layer_opt_class(state.auto_factor_mode.get() == AutoFactorMode::Fixed10x)
                                                        on:click=move |_| state.auto_factor_mode.set(AutoFactorMode::Fixed10x)
                                                        title="Factor = 10x"
                                                    >"10x"</button>
                                                </div>
                                            })
                                        }}
                                    }
                                })}

                                // ── Bandpass ─────────────────────────────────────
                                <hr />
                                <div class="layer-panel-title">"Bandpass"</div>
                                <div style="display: flex; gap: 2px; padding: 0 6px 4px;">
                                    <button class=move || layer_opt_class(state.bandpass_mode.get() == BandpassMode::Auto)
                                        on:click=move |_| state.bandpass_mode.set(BandpassMode::Auto)
                                    >"AUTO"</button>
                                    <button class=move || layer_opt_class(state.bandpass_mode.get() == BandpassMode::Off)
                                        on:click=move |_| state.bandpass_mode.set(BandpassMode::Off)
                                    >"OFF"</button>
                                    <button class=move || layer_opt_class(state.bandpass_mode.get() == BandpassMode::On)
                                        on:click=move |_| state.bandpass_mode.set(BandpassMode::On)
                                    >"ON"</button>
                                </div>
                                {move || {
                                    let bp = state.bandpass_mode.get();
                                    let show = bp == BandpassMode::On
                                        || (bp == BandpassMode::Auto && state.ff_freq_hi.get() > state.ff_freq_lo.get());
                                    show.then(|| {
                                        let make_db_handler = |signal: RwSignal<f64>| {
                                            move |ev: web_sys::Event| {
                                                use wasm_bindgen::JsCast;
                                                let input: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
                                                if let Ok(val) = input.value().parse::<f64>() {
                                                    if state.bandpass_mode.get_untracked() == BandpassMode::Auto {
                                                        state.bandpass_mode.set(BandpassMode::On);
                                                    }
                                                    signal.set(val);
                                                }
                                            }
                                        };
                                        let on_above_change = make_db_handler(state.filter_db_above);
                                        let on_selected_change = make_db_handler(state.filter_db_selected);
                                        let on_harmonics_change = make_db_handler(state.filter_db_harmonics);
                                        let on_below_change = make_db_handler(state.filter_db_below);

                                        let on_quality_click = move |q: FilterQuality| {
                                            move |_: web_sys::MouseEvent| {
                                                if state.bandpass_mode.get_untracked() == BandpassMode::Auto {
                                                    state.bandpass_mode.set(BandpassMode::On);
                                                }
                                                state.filter_quality.set(q);
                                            }
                                        };
                                        let on_band_click = move |b: u8| {
                                            move |_: web_sys::MouseEvent| {
                                                if state.bandpass_mode.get_untracked() == BandpassMode::Auto {
                                                    state.bandpass_mode.set(BandpassMode::On);
                                                }
                                                state.filter_band_mode.set(b);
                                            }
                                        };

                                        view! {
                                            <div style="display: flex; gap: 2px; padding: 0 6px 2px;">
                                                <button class=move || layer_opt_class(state.bandpass_range.get() == BandpassRange::FollowFocus)
                                                    on:click=move |_| state.bandpass_range.set(BandpassRange::FollowFocus)
                                                >"Focus"</button>
                                                <button class=move || layer_opt_class(state.bandpass_range.get() == BandpassRange::Custom)
                                                    on:click=move |_| state.bandpass_range.set(BandpassRange::Custom)
                                                >"Custom"</button>
                                            </div>
                                            <div style="padding: 0 8px 2px; font-size: 10px; opacity: 0.7;">
                                                {move || format!("{:.1}–{:.1} kHz",
                                                    state.filter_freq_low.get() / 1000.0,
                                                    state.filter_freq_high.get() / 1000.0
                                                )}
                                            </div>
                                            <div style="display: flex; gap: 2px; padding: 0 6px 2px;">
                                                <button class=move || layer_opt_class(state.filter_quality.get() == FilterQuality::Fast)
                                                    on:click=on_quality_click(FilterQuality::Fast)
                                                    title="IIR band-split — low latency, softer edges"
                                                >"Fast"</button>
                                                <button class=move || layer_opt_class(state.filter_quality.get() == FilterQuality::HQ)
                                                    on:click=on_quality_click(FilterQuality::HQ)
                                                    title="FFT spectral EQ — sharp edges, higher latency"
                                                >"HQ"</button>
                                                <span style="width: 8px;"></span>
                                                <button class=move || layer_opt_class(state.filter_band_mode.get() == 3)
                                                    on:click=on_band_click(3)
                                                >"3"</button>
                                                <button class=move || layer_opt_class(state.filter_band_mode.get() == 4)
                                                    on:click=on_band_click(4)
                                                >"4"</button>
                                            </div>
                                            <div class="layer-panel-slider-row"
                                                on:mouseenter=move |_| state.filter_hovering_band.set(Some(3))
                                                on:mouseleave=move |_| state.filter_hovering_band.set(None)
                                            >
                                                <label>"Above"</label>
                                                <input type="range" min="-60" max="6" step="1"
                                                    prop:value=move || state.filter_db_above.get().to_string()
                                                    on:input=on_above_change
                                                />
                                                <span>{move || format!("{:.0}", state.filter_db_above.get())}</span>
                                            </div>
                                            {move || (state.filter_band_mode.get() >= 4).then(|| view! {
                                                <div class="layer-panel-slider-row"
                                                    on:mouseenter=move |_| state.filter_hovering_band.set(Some(2))
                                                    on:mouseleave=move |_| state.filter_hovering_band.set(None)
                                                >
                                                    <label>"Harm"</label>
                                                    <input type="range" min="-60" max="6" step="1"
                                                        prop:value=move || state.filter_db_harmonics.get().to_string()
                                                        on:input=on_harmonics_change
                                                    />
                                                    <span>{move || format!("{:.0}", state.filter_db_harmonics.get())}</span>
                                                </div>
                                            })}
                                            <div class="layer-panel-slider-row"
                                                on:mouseenter=move |_| state.filter_hovering_band.set(Some(1))
                                                on:mouseleave=move |_| state.filter_hovering_band.set(None)
                                            >
                                                <label>"Focus"</label>
                                                <input type="range" min="-60" max="6" step="1"
                                                    prop:value=move || state.filter_db_selected.get().to_string()
                                                    on:input=on_selected_change
                                                />
                                                <span>{move || format!("{:.0}", state.filter_db_selected.get())}</span>
                                            </div>
                                            <div class="layer-panel-slider-row"
                                                on:mouseenter=move |_| state.filter_hovering_band.set(Some(0))
                                                on:mouseleave=move |_| state.filter_hovering_band.set(None)
                                            >
                                                <label>"Below"</label>
                                                <input type="range" min="-60" max="6" step="1"
                                                    prop:value=move || state.filter_db_below.get().to_string()
                                                    on:input=on_below_change
                                                />
                                                <span>{move || format!("{:.0}", state.filter_db_below.get())}</span>
                                            </div>
                                        }
                                    })
                                }}
                            </div>
                        }
                    })}
                </div>
            </div>
        </Show>
    }
}
