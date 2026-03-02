use leptos::prelude::*;
use crate::state::{AppState, AutoFactorMode, BandpassMode, BandpassRange, FilterQuality, PlaybackMode};

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
            // Restore saved HFR settings, or use defaults
            let saved_lo = state.hfr_saved_ff_lo.get_untracked();
            let saved_hi = state.hfr_saved_ff_hi.get_untracked();
            let saved_mode = state.hfr_saved_playback_mode.get_untracked();
            let saved_bp = state.hfr_saved_bandpass_mode.get_untracked();

            state.ff_freq_lo.set(saved_lo.unwrap_or(18_000.0));
            state.ff_freq_hi.set(saved_hi.unwrap_or(nyquist));

            match saved_mode {
                Some(mode) => state.playback_mode.set(mode),
                None => {
                    if state.playback_mode.get_untracked() == PlaybackMode::Normal {
                        state.playback_mode.set(PlaybackMode::PitchShift);
                    }
                }
            }

            // Restore bandpass mode (defaults to Auto)
            state.bandpass_mode.set(saved_bp.unwrap_or(BandpassMode::Auto));

            state.min_display_freq.set(None);
            state.max_display_freq.set(None);
        } else {
            // Save current HFR settings before clearing
            let current_lo = state.ff_freq_lo.get_untracked();
            let current_hi = state.ff_freq_hi.get_untracked();
            let current_mode = state.playback_mode.get_untracked();
            let current_bp = state.bandpass_mode.get_untracked();

            if current_hi > current_lo {
                state.hfr_saved_ff_lo.set(Some(current_lo));
                state.hfr_saved_ff_hi.set(Some(current_hi));
                state.hfr_saved_playback_mode.set(Some(current_mode));
                state.hfr_saved_bandpass_mode.set(Some(current_bp));
                // Only force bandpass off when HFR was actually active
                state.bandpass_mode.set(BandpassMode::Off);
            }

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

        let ratio = match mode {
            AutoFactorMode::Target3k => ff_center / 3000.0,
            AutoFactorMode::MinAudible => ff_hi / 20_000.0,
            AutoFactorMode::Fixed10x => {
                if ff_center < 3000.0 { 0.1 } else { 10.0 }
            }
        };

        // ratio >= 1.0 → shift down (positive factor)
        // ratio < 1.0  → shift up (negative factor = multiply)
        if state.te_factor_auto.get_untracked() {
            let te = if ratio >= 1.0 {
                ratio.round().clamp(2.0, 40.0)
            } else {
                -(1.0 / ratio).round().clamp(2.0, 40.0)
            };
            state.te_factor.set(te);
        }
        if state.ps_factor_auto.get_untracked() {
            let ps = if ratio >= 1.0 {
                ratio.round().clamp(2.0, 20.0)
            } else {
                -(1.0 / ratio).round().clamp(2.0, 20.0)
            };
            state.ps_factor.set(ps);
        }
    });

    // Effect: ZC mode display settings save/restore.
    // ZC remembers its own display_eq / display_noise_filter / display_auto_gain.
    {
        let prev_mode = RwSignal::new(state.playback_mode.get_untracked());
        Effect::new(move || {
            let mode = state.playback_mode.get();
            let old = prev_mode.get_untracked();
            if mode == old { return; }
            prev_mode.set(mode);

            let was_zc = old == PlaybackMode::ZeroCrossing;
            let is_zc = mode == PlaybackMode::ZeroCrossing;

            if was_zc && !is_zc {
                // Leaving ZC: save ZC settings, restore normal settings
                state.zc_saved_display_auto_gain.set(state.display_auto_gain.get_untracked());
                state.zc_saved_display_eq.set(state.display_eq.get_untracked());
                state.zc_saved_display_noise_filter.set(state.display_noise_filter.get_untracked());

                state.display_auto_gain.set(state.normal_saved_display_auto_gain.get_untracked());
                state.display_eq.set(state.normal_saved_display_eq.get_untracked());
                state.display_noise_filter.set(state.normal_saved_display_noise_filter.get_untracked());
            } else if !was_zc && is_zc {
                // Entering ZC: save normal settings, restore ZC settings
                state.normal_saved_display_auto_gain.set(state.display_auto_gain.get_untracked());
                state.normal_saved_display_eq.set(state.display_eq.get_untracked());
                state.normal_saved_display_noise_filter.set(state.display_noise_filter.get_untracked());

                state.display_auto_gain.set(state.zc_saved_display_auto_gain.get_untracked());
                state.display_eq.set(state.zc_saved_display_eq.get_untracked());
                state.display_noise_filter.set(state.zc_saved_display_noise_filter.get_untracked());
            }
        });
    }

    // Effect D (carried over): bandpass_mode + bandpass_range + playback_mode → filter_enabled + filter_freq + filter gains
    Effect::new(move || {
        let bp_mode = state.bandpass_mode.get();
        let bp_range = state.bandpass_range.get();
        let ff_lo = state.ff_freq_lo.get();
        let ff_hi = state.ff_freq_hi.get();
        let playback_mode = state.playback_mode.get();

        match bp_mode {
            BandpassMode::Off => {
                state.filter_enabled.set(false);
            }
            BandpassMode::Auto => {
                let has_ff = ff_hi > ff_lo;
                match playback_mode {
                    PlaybackMode::Heterodyne => {
                        // HET AUTO: bandpass off
                        state.filter_enabled.set(false);
                    }
                    PlaybackMode::ZeroCrossing => {
                        // ZC AUTO: HQ with steep -60 dB rolloff
                        state.filter_enabled.set(has_ff);
                        if has_ff {
                            state.filter_freq_low.set(ff_lo);
                            state.filter_freq_high.set(ff_hi);
                            state.filter_quality.set(FilterQuality::HQ);
                            state.filter_db_below.set(-60.0);
                            state.filter_db_selected.set(0.0);
                            state.filter_db_above.set(-60.0);
                        }
                    }
                    _ => {
                        // TE/PS/Normal AUTO: HQ with -40 dB rolloff
                        state.filter_enabled.set(has_ff);
                        if has_ff {
                            state.filter_freq_low.set(ff_lo);
                            state.filter_freq_high.set(ff_hi);
                            state.filter_quality.set(FilterQuality::HQ);
                            state.filter_db_below.set(-40.0);
                            state.filter_db_selected.set(0.0);
                            state.filter_db_above.set(-40.0);
                        }
                    }
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
