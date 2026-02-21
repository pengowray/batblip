use leptos::prelude::*;
use crate::state::{AppState, BandpassStrength, FrequencyFocus, LayerPanel, ListenAdjustment};

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

    // When FrequencyFocus changes, update display freq and auto-listen mode
    Effect::new(move || {
        let ff = state.frequency_focus.get();
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let file_nyquist = idx
            .and_then(|i| files.get(i))
            .map(|f| f.spectrogram.max_freq)
            .unwrap_or(96_000.0);

        match ff.view_range_hz(file_nyquist) {
            Some((view_lo, view_hi)) => {
                state.min_display_freq.set(Some(view_lo));
                state.max_display_freq.set(Some(view_hi));
            }
            None => {
                state.min_display_freq.set(None);
                state.max_display_freq.set(None);
            }
        }

        if state.listen_adjustment.get_untracked() == ListenAdjustment::Auto {
            state.playback_mode.set(ff.auto_listen_mode());
            // Auto-set HET frequency for bat ranges
            match ff {
                FrequencyFocus::Bat1 => state.het_frequency.set(27_500.0),
                FrequencyFocus::Bat2 => state.het_frequency.set(42_500.0),
                _ => {}
            }
        }
    });

    view! {
        // Stacked above Mode button at bottom-left of main-overlays
        // z-index: 20 ensures the panel (z-index:30) renders above sibling layer buttons.
        <div
            style=move || format!("position: absolute; left: 28px; bottom: 46px; pointer-events: none; z-index: 20; opacity: {}; transition: opacity 0.1s;",
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
                            <hr />
                            <div class="layer-panel-title">"Filter other freqs"</div>
                            <button class=move || layer_opt_class(state.ff_filter_strength.get() == BandpassStrength::Off)
                                on:click=move |_| state.ff_filter_strength.set(BandpassStrength::Off)
                            >"Off"</button>
                            <button class=move || layer_opt_class(state.ff_filter_strength.get() == BandpassStrength::Some)
                                on:click=move |_| state.ff_filter_strength.set(BandpassStrength::Some)
                            >"Some"</button>
                            <button class=move || layer_opt_class(state.ff_filter_strength.get() == BandpassStrength::Strong)
                                on:click=move |_| state.ff_filter_strength.set(BandpassStrength::Strong)
                            >"Strong"</button>
                        </div>
                    }
                })}
            </div>
        </div>
    }
}
