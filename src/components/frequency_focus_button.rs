use leptos::prelude::*;
use crate::state::{AppState, AutoFactorMode, BandpassMode, BandpassRange, FrequencyFocus, LayerPanel, ListenAdjustment};

fn layer_opt_class(active: bool) -> &'static str {
    if active { "layer-panel-opt sel" } else { "layer-panel-opt" }
}

fn toggle_panel(state: &AppState, panel: LayerPanel) {
    state.layer_panel_open.update(|p| {
        *p = if *p == Some(panel) { None } else { Some(panel) };
    });
}

#[component]
pub fn FrequencyFocusButton() -> impl IntoView {
    let state = expect_context::<AppState>();

    let is_open = move || state.layer_panel_open.get() == Some(LayerPanel::FrequencyFocus);

    // Effect A: FF preset → ff_freq_lo / ff_freq_hi (+ display freq for human presets only)
    Effect::new(move || {
        let ff = state.frequency_focus.get();
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let file_nyquist = idx
            .and_then(|i| files.get(i))
            .map(|f| f.spectrogram.max_freq)
            .unwrap_or(96_000.0);

        match ff {
            FrequencyFocus::None => {
                state.ff_freq_lo.set(0.0);
                state.ff_freq_hi.set(0.0);
                state.min_display_freq.set(None);
                state.max_display_freq.set(None);
            }
            FrequencyFocus::Custom => {
                // Custom — ff_freq_lo/hi are set by drag handles, don't touch
            }
            _ => {
                if let Some((lo, hi)) = ff.freq_range_hz() {
                    let hi = hi.min(file_nyquist);
                    state.ff_freq_lo.set(lo);
                    state.ff_freq_hi.set(hi);
                }
                // Only human presets zoom the display
                match ff {
                    FrequencyFocus::HumanHearing | FrequencyFocus::HumanSpeech => {
                        state.min_display_freq.set(Some(0.0));
                        state.max_display_freq.set(Some(22_000.0));
                    }
                    _ => {
                        state.min_display_freq.set(None);
                        state.max_display_freq.set(None);
                    }
                }
            }
        }
    });

    // Effect B: FF → auto listen mode
    Effect::new(move || {
        let ff = state.frequency_focus.get();
        if state.listen_adjustment.get_untracked() == ListenAdjustment::Auto {
            state.playback_mode.set(ff.auto_listen_mode());
        }
    });

    // Effect C: FF range → auto parameter values
    Effect::new(move || {
        let ff_lo = state.ff_freq_lo.get();
        let ff_hi = state.ff_freq_hi.get();
        let mode = state.auto_factor_mode.get();

        if ff_hi <= ff_lo {
            return; // no FF active
        }

        let ff_center = (ff_lo + ff_hi) / 2.0;
        let ff_bandwidth = ff_hi - ff_lo;

        // Auto HET frequency
        if state.het_freq_auto.get_untracked() {
            state.het_frequency.set(ff_center);
        }
        // Auto HET cutoff
        if state.het_cutoff_auto.get_untracked() {
            state.het_cutoff.set((ff_bandwidth / 2.0).min(15_000.0));
        }

        // Compute factor based on mode
        let factor = match mode {
            AutoFactorMode::Target3k => ff_center / 3000.0,
            AutoFactorMode::MinAudible => ff_hi / 20_000.0,
            AutoFactorMode::Fixed10x => 10.0,
        };

        if state.te_factor_auto.get_untracked() {
            state.te_factor.set(factor.round().clamp(2.0, 40.0));
        }
        if state.ps_factor_auto.get_untracked() {
            state.ps_factor.set(factor.round().clamp(2.0, 20.0));
        }
    });

    // Effect D: bandpass_mode + bandpass_range → filter_enabled + filter_freq
    Effect::new(move || {
        let bp_mode = state.bandpass_mode.get();
        let bp_range = state.bandpass_range.get();
        let ff_lo = state.ff_freq_lo.get();
        let ff_hi = state.ff_freq_hi.get();

        match bp_mode {
            BandpassMode::Off => {
                state.filter_enabled.set(false);
            }
            BandpassMode::Auto => {
                let has_ff = ff_hi > ff_lo;
                state.filter_enabled.set(has_ff);
                if has_ff {
                    state.filter_freq_low.set(ff_lo);
                    state.filter_freq_high.set(ff_hi);
                }
            }
            BandpassMode::On => {
                state.filter_enabled.set(true);
                if bp_range == BandpassRange::FollowFocus && ff_hi > ff_lo {
                    state.filter_freq_low.set(ff_lo);
                    state.filter_freq_high.set(ff_hi);
                }
            }
        }
    });

    view! {
        // Stacked above Mode button at bottom-left of main-overlays
        // z-index: 20 ensures the panel (z-index:30) renders above sibling layer buttons.
        <div
            style=move || format!("position: absolute; left: 56px; bottom: 46px; pointer-events: none; z-index: 20; opacity: {}; transition: opacity 0.1s;",
                if state.mouse_in_label_area.get() { "0" } else { "1" })
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
        >
            <div style=move || format!("position: relative; pointer-events: {};",
                if state.mouse_in_label_area.get() { "none" } else { "auto" })>
                <button
                    class=move || if is_open() { "layer-btn open" } else { "layer-btn" }
                    on:click=move |_| toggle_panel(&state, LayerPanel::FrequencyFocus)
                    title="Frequency Focus"
                >
                    <span class="layer-btn-category">"Focus"</span>
                    <span class="layer-btn-value">{move || {
                        match state.frequency_focus.get() {
                            FrequencyFocus::None => "All",
                            FrequencyFocus::HumanHearing => "Human",
                            FrequencyFocus::HumanSpeech => "Speech",
                            FrequencyFocus::Bat1 => "Bat 1",
                            FrequencyFocus::Bat2 => "Bat 2",
                            FrequencyFocus::Infra => "Infra",
                            FrequencyFocus::FullUltrasound => "Ultra",
                            FrequencyFocus::FullSpectrum => "Full",
                            FrequencyFocus::Custom => "Custom",
                        }
                    }}</span>
                </button>
                {move || is_open().then(|| {
                    view! {
                        <div class="layer-panel" style="left: 0; bottom: 34px;">
                            <div class="layer-panel-title">"Frequency Focus"</div>
                            {FrequencyFocus::ALL.iter().map(|&variant| {
                                view! {
                                    <button
                                        class=move || layer_opt_class(state.frequency_focus.get() == variant)
                                        on:click=move |_| state.frequency_focus.set(variant)
                                    >{variant.label()}</button>
                                }
                            }).collect_view()}
                        </div>
                    }
                })}
            </div>
        </div>
    }
}
