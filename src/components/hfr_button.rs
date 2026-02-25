use leptos::prelude::*;
use crate::state::{AppState, AutoFactorMode, BandpassMode, BandpassRange, PlaybackMode};

#[component]
pub fn HfrButton() -> impl IntoView {
    let state = expect_context::<AppState>();

    // Effect: HFR toggle → set ff range, playback mode, display freq
    Effect::new(move || {
        let enabled = state.hfr_enabled.get();
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let nyquist = idx
            .and_then(|i| files.get(i))
            .map(|f| f.spectrogram.max_freq)
            .unwrap_or(96_000.0);

        if enabled {
            // Set default HFR range: 18kHz to Nyquist
            state.ff_freq_lo.set(18_000.0);
            state.ff_freq_hi.set(nyquist);

            // Set playback mode to PS if currently Normal
            if state.playback_mode.get_untracked() == PlaybackMode::Normal {
                state.playback_mode.set(PlaybackMode::PitchShift);
            }

            state.min_display_freq.set(None);
            state.max_display_freq.set(None);
        } else {
            // HFR OFF: reset to 1:1
            state.ff_freq_lo.set(0.0);
            state.ff_freq_hi.set(0.0);
            state.playback_mode.set(PlaybackMode::Normal);
            state.min_display_freq.set(None);
            state.max_display_freq.set(None);
        }
    });

    // Effect C (carried over): FF range → auto parameter values
    Effect::new(move || {
        let ff_lo = state.ff_freq_lo.get();
        let ff_hi = state.ff_freq_hi.get();
        let mode = state.auto_factor_mode.get();

        if ff_hi <= ff_lo {
            return;
        }

        let ff_center = (ff_lo + ff_hi) / 2.0;
        let ff_bandwidth = ff_hi - ff_lo;

        if state.het_freq_auto.get_untracked() {
            state.het_frequency.set(ff_center);
        }
        if state.het_cutoff_auto.get_untracked() {
            state.het_cutoff.set((ff_bandwidth / 2.0).min(15_000.0));
        }

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

    // Effect D (carried over): bandpass_mode + bandpass_range → filter_enabled + filter_freq
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
        <div
            style=move || format!("position: absolute; left: 56px; bottom: 82px; pointer-events: none; z-index: 20; opacity: {}; transition: opacity 0.1s;",
                if state.mouse_in_label_area.get() { "0" } else { "1" })
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
        >
            <div style=move || format!("position: relative; pointer-events: {};",
                if state.mouse_in_label_area.get() { "none" } else { "auto" })>
                <button
                    class=move || if state.hfr_enabled.get() { "layer-btn active" } else { "layer-btn" }
                    on:click=move |_| state.hfr_enabled.update(|v| *v = !*v)
                    title="Toggle High Frequency Range mode"
                >
                    <span class="layer-btn-category">"HFR"</span>
                    <span class="layer-btn-value">{move || if state.hfr_enabled.get() { "ON" } else { "OFF" }}</span>
                </button>
            </div>
        </div>
    }
}
