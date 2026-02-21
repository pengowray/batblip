use leptos::prelude::*;
use web_sys::{AudioContext, AudioContextOptions, AudioBufferSourceNode};
use crate::types::AudioData;
use crate::state::{AppState, Selection, PlaybackMode, FilterQuality};
use crate::dsp::heterodyne::heterodyne_mix;
use crate::dsp::pitch_shift::pitch_shift_realtime;
use crate::dsp::zc_divide::zc_divide;
use crate::dsp::filters::{apply_eq_filter, apply_eq_filter_fast};
use std::cell::RefCell;

thread_local! {
    static CURRENT_SOURCE: RefCell<Option<AudioBufferSourceNode>> = RefCell::new(None);
    static CURRENT_CTX: RefCell<Option<AudioContext>> = RefCell::new(None);
    static PLAYHEAD_HANDLE: RefCell<Option<i32>> = RefCell::new(None);
}

pub fn stop(state: &AppState) {
    cancel_playhead();
    CURRENT_SOURCE.with(|s| {
        if let Some(source) = s.borrow_mut().take() {
            #[allow(deprecated)]
            let _ = source.stop();
        }
    });
    CURRENT_CTX.with(|c| {
        if let Some(ctx) = c.borrow_mut().take() {
            let _ = ctx.close();
        }
    });
    // Restore scroll to pre-play position when stopping
    state.scroll_offset.set(state.pre_play_scroll.get_untracked());
    state.is_playing.set(false);
}

/// Resume HET playback from the current playhead position with the new frequency.
pub fn replay_het(state: &AppState) {
    let current_time = state.playhead_time.get_untracked();
    // Stop audio without resetting scroll
    cancel_playhead();
    CURRENT_SOURCE.with(|s| {
        if let Some(source) = s.borrow_mut().take() {
            #[allow(deprecated)]
            let _ = source.stop();
        }
    });
    CURRENT_CTX.with(|c| {
        if let Some(ctx) = c.borrow_mut().take() {
            let _ = ctx.close();
        }
    });

    let files = state.files.get_untracked();
    let idx = state.current_file_index.get_untracked();
    let Some(file) = idx.and_then(|i| files.get(i)) else { return };

    let selection = state.selection.get_untracked();
    let het_freq = state.het_frequency.get_untracked();

    // Extract remaining samples from current_time to selection end (or file end)
    let sr = file.audio.sample_rate;
    let sel_end = selection.map(|s| s.time_end).unwrap_or(file.audio.duration_secs);
    let start_sample = (current_time * sr as f64) as usize;
    let end_sample = (sel_end * sr as f64) as usize;
    let start_sample = start_sample.min(file.audio.samples.len());
    let end_sample = end_sample.min(file.audio.samples.len());

    if end_sample <= start_sample {
        state.is_playing.set(false);
        return;
    }

    let samples = file.audio.samples[start_sample..end_sample].to_vec();
    let remaining_duration = (end_sample - start_sample) as f64 / sr as f64;

    let effective_lo = if let Some(sel) = selection {
        if sel.freq_low > 0.0 || sel.freq_high > 0.0 {
            (sel.freq_low + sel.freq_high) / 2.0
        } else {
            het_freq
        }
    } else {
        het_freq
    };
    let het_cutoff = state.het_cutoff.get_untracked();
    let mut processed = heterodyne_mix(&samples, sr, effective_lo, het_cutoff);
    let gain = if state.auto_gain.get_untracked() {
        auto_gain_db(&processed)
    } else {
        state.gain_db.get_untracked()
    };
    apply_gain(&mut processed, gain);
    play_samples(&processed, sr);

    // Continue playhead from current position
    start_playhead(state.clone(), current_time, remaining_duration, 1.0);
}

/// Restart playback from the beginning of the current selection.
/// Used when filter/EQ settings change during ZC playback.
pub fn replay(state: &AppState) {
    if state.is_playing.get_untracked() {
        play(state);
    }
}

/// Play from the very start of the current file (ignores selection).
pub fn play_from_start(state: &AppState) {
    // Save scroll position before calling stop (which would overwrite scroll with old pre_play_scroll)
    let pre = state.scroll_offset.get_untracked();
    stop(state);
    state.pre_play_scroll.set(pre);
    // Clear selection so play() plays the full file from the start
    state.selection.set(None);
    state.scroll_offset.set(0.0);
    play(state);
    // Override pre_play_scroll to the position we saved before stop()
    state.pre_play_scroll.set(pre);
}

/// Play from the current "here" time (play_from_here_time signal).
pub fn play_from_here(state: &AppState) {
    let pre = state.scroll_offset.get_untracked();
    let start_secs = state.play_from_here_time.get_untracked();
    stop(state);
    state.pre_play_scroll.set(pre);
    play_from_time(state, start_secs);
    state.pre_play_scroll.set(pre);
}

/// Play from a specific time offset in the current file.
pub fn play_from_time(state: &AppState, start_secs: f64) {
    let files = state.files.get_untracked();
    let idx = state.current_file_index.get_untracked();
    let Some(file) = idx.and_then(|i| files.get(i)) else { return };

    let mode = state.playback_mode.get_untracked();
    let sr = file.audio.sample_rate;

    // Slice from start_secs to end of file (or selection end if selection exists)
    let selection = state.selection.get_untracked();
    let end_secs = selection.map(|s| s.time_end).unwrap_or(file.audio.duration_secs);
    let start_secs = start_secs.max(0.0).min(end_secs);
    let start_sample = (start_secs * sr as f64) as usize;
    let end_sample = ((end_secs * sr as f64) as usize).min(file.audio.samples.len());
    if end_sample <= start_sample { return; }

    let samples_ref = &file.audio.samples[start_sample..end_sample];
    let filter_enabled = state.filter_enabled.get_untracked();
    let samples: Vec<f32> = if filter_enabled {
        let freq_low = state.filter_freq_low.get_untracked();
        let freq_high = state.filter_freq_high.get_untracked();
        let db_below = state.filter_db_below.get_untracked();
        let db_selected = state.filter_db_selected.get_untracked();
        let db_harmonics = state.filter_db_harmonics.get_untracked();
        let db_above = state.filter_db_above.get_untracked();
        let band_mode = state.filter_band_mode.get_untracked();
        let quality = state.filter_quality.get_untracked();
        match quality {
            FilterQuality::Fast => apply_eq_filter_fast(samples_ref, sr, freq_low, freq_high, db_below, db_selected, db_harmonics, db_above, band_mode),
            FilterQuality::HQ => apply_eq_filter(samples_ref, sr, freq_low, freq_high, db_below, db_selected, db_harmonics, db_above, band_mode),
        }
    } else {
        samples_ref.to_vec()
    };

    let te_factor = state.te_factor.get_untracked();
    let ps_factor = state.ps_factor.get_untracked();
    let zc_factor = state.zc_factor.get_untracked();
    let het_freq = state.het_frequency.get_untracked();

    let (mut final_samples, final_rate) = match mode {
        PlaybackMode::Normal => (samples, sr),
        PlaybackMode::Heterodyne => {
            let het_cutoff = state.het_cutoff.get_untracked();
            let processed = heterodyne_mix(&samples, sr, het_freq, het_cutoff);
            (processed, sr)
        }
        PlaybackMode::TimeExpansion => {
            let te_rate = ((sr as f64 / te_factor) as u32).max(8000);
            (samples, te_rate)
        }
        PlaybackMode::PitchShift => {
            let shifted = pitch_shift_realtime(&samples, ps_factor);
            (shifted, sr)
        }
        PlaybackMode::ZeroCrossing => {
            let processed = zc_divide(&samples, sr, zc_factor as u32, filter_enabled);
            (processed, sr)
        }
    };

    let gain = if state.auto_gain.get_untracked() {
        auto_gain_db(&final_samples)
    } else {
        state.gain_db.get_untracked()
    };
    apply_gain(&mut final_samples, gain);
    play_samples(&final_samples, final_rate);

    let play_duration = (end_sample - start_sample) as f64 / sr as f64;
    let playback_speed = match mode {
        PlaybackMode::TimeExpansion => 1.0 / te_factor,
        _ => 1.0,
    };
    state.is_playing.set(true);
    state.playhead_time.set(start_secs);
    start_playhead(state.clone(), start_secs, play_duration, playback_speed);
}

pub fn play(state: &AppState) {
    stop(state);

    let files = state.files.get_untracked();
    let idx = state.current_file_index.get_untracked();
    let Some(file) = idx.and_then(|i| files.get(i)) else { return };

    let mode = state.playback_mode.get_untracked();
    let selection = state.selection.get_untracked();
    let het_freq = state.het_frequency.get_untracked();
    let te_factor = state.te_factor.get_untracked();
    let ps_factor = state.ps_factor.get_untracked();
    let zc_factor = state.zc_factor.get_untracked();

    // extract_selection returns a slice to avoid cloning unless mutation is needed
    let (samples_ref, sample_rate) = extract_selection(&file.audio, selection);

    // Apply EQ filter if enabled, otherwise fall back to selection-based bandpass.
    // DSP functions take &[f32] and allocate their own output; passthrough cases
    // clone only here, so we avoid a full-audio clone when DSP is applied.
    let filter_enabled = state.filter_enabled.get_untracked();
    let samples: Vec<f32> = if filter_enabled {
        let freq_low = state.filter_freq_low.get_untracked();
        let freq_high = state.filter_freq_high.get_untracked();
        let db_below = state.filter_db_below.get_untracked();
        let db_selected = state.filter_db_selected.get_untracked();
        let db_harmonics = state.filter_db_harmonics.get_untracked();
        let db_above = state.filter_db_above.get_untracked();
        let band_mode = state.filter_band_mode.get_untracked();
        let quality = state.filter_quality.get_untracked();
        match quality {
            FilterQuality::Fast => apply_eq_filter_fast(samples_ref, sample_rate, freq_low, freq_high, db_below, db_selected, db_harmonics, db_above, band_mode),
            FilterQuality::HQ => apply_eq_filter(samples_ref, sample_rate, freq_low, freq_high, db_below, db_selected, db_harmonics, db_above, band_mode),
        }
    } else if let Some(sel) = selection {
        if matches!(mode, PlaybackMode::Normal | PlaybackMode::TimeExpansion | PlaybackMode::PitchShift | PlaybackMode::ZeroCrossing)
            && (sel.freq_low > 0.0 || sel.freq_high < (sample_rate as f64 / 2.0))
        {
            apply_bandpass(samples_ref, sample_rate, sel.freq_low, sel.freq_high)
        } else {
            samples_ref.to_vec()
        }
    } else {
        samples_ref.to_vec()
    };

    // Determine playback start time (in the original audio timeline)
    let play_start_time = selection.map(|s| s.time_start).unwrap_or(0.0);
    let play_duration_orig = samples.len() as f64 / sample_rate as f64;

    let (mut final_samples, final_rate) = match mode {
        PlaybackMode::Normal => {
            (samples, sample_rate)
        }
        PlaybackMode::Heterodyne => {
            let effective_lo = if let Some(sel) = selection {
                if sel.freq_low > 0.0 || sel.freq_high > 0.0 {
                    (sel.freq_low + sel.freq_high) / 2.0
                } else {
                    het_freq
                }
            } else {
                het_freq
            };
            let het_cutoff = state.het_cutoff.get_untracked();
            let processed = heterodyne_mix(&samples, sample_rate, effective_lo, het_cutoff);
            (processed, sample_rate)
        }
        PlaybackMode::TimeExpansion => {
            let te_rate = (sample_rate as f64 / te_factor) as u32;
            let te_rate = te_rate.max(8000); // browser minimum
            (samples, te_rate)
        }
        PlaybackMode::PitchShift => {
            let shifted = pitch_shift_realtime(&samples, ps_factor);
            (shifted, sample_rate)
        }
        PlaybackMode::ZeroCrossing => {
            let processed = zc_divide(&samples, sample_rate, zc_factor as u32, filter_enabled);
            (processed, sample_rate)
        }
    };

    // Apply gain
    let gain = if state.auto_gain.get_untracked() {
        auto_gain_db(&final_samples)
    } else {
        state.gain_db.get_untracked()
    };
    apply_gain(&mut final_samples, gain);
    play_samples(&final_samples, final_rate);

    // Start playhead animation
    let playback_speed = match mode {
        PlaybackMode::Normal => 1.0,
        PlaybackMode::Heterodyne => 1.0,
        PlaybackMode::TimeExpansion => 1.0 / te_factor,
        PlaybackMode::PitchShift => 1.0,
        PlaybackMode::ZeroCrossing => 1.0,
    };

    state.pre_play_scroll.set(state.scroll_offset.get_untracked());
    state.is_playing.set(true);
    state.playhead_time.set(play_start_time);
    start_playhead(state.clone(), play_start_time, play_duration_orig, playback_speed);
}

fn extract_selection<'a>(audio: &'a AudioData, selection: Option<Selection>) -> (&'a [f32], u32) {
    let sr = audio.sample_rate;
    if let Some(sel) = selection {
        let start = (sel.time_start * sr as f64) as usize;
        let end = (sel.time_end * sr as f64) as usize;
        let start = start.min(audio.samples.len());
        let end = end.min(audio.samples.len());
        if end > start {
            return (&audio.samples[start..end], sr);
        }
    }
    (&audio.samples, sr)
}

fn apply_bandpass(samples: &[f32], sample_rate: u32, freq_low: f64, freq_high: f64) -> Vec<f32> {
    let mut result = samples.to_vec();
    if freq_low > 0.0 {
        let lp = cascaded_lowpass(&result, freq_low, sample_rate, 4);
        for (r, l) in result.iter_mut().zip(lp.iter()) {
            *r -= l;
        }
    }
    if freq_high < (sample_rate as f64 / 2.0) {
        result = cascaded_lowpass(&result, freq_high, sample_rate, 4);
    }
    result
}

fn lowpass_filter_inplace(buf: &mut [f32], cutoff_hz: f64, sample_rate: u32) {
    if buf.is_empty() {
        return;
    }
    let dt = 1.0 / sample_rate as f64;
    let rc = 1.0 / (2.0 * std::f64::consts::PI * cutoff_hz);
    let alpha = (dt / (rc + dt)) as f32;
    let mut prev = buf[0];
    for s in buf[1..].iter_mut() {
        let y = alpha * *s + (1.0 - alpha) * prev;
        *s = y;
        prev = y;
    }
}

fn cascaded_lowpass(samples: &[f32], cutoff: f64, sample_rate: u32, passes: usize) -> Vec<f32> {
    let mut result = samples.to_vec();
    for _ in 0..passes {
        lowpass_filter_inplace(&mut result, cutoff, sample_rate);
    }
    result
}

fn apply_gain(samples: &mut [f32], gain_db: f64) {
    if gain_db.abs() < 0.001 {
        return;
    }
    let gain_linear = 10.0_f64.powf(gain_db / 20.0) as f32;
    for s in samples.iter_mut() {
        *s *= gain_linear;
    }
}

fn auto_gain_db(samples: &[f32]) -> f64 {
    let peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    if peak < 1e-10 {
        return 0.0;
    }
    let peak_db = 20.0 * (peak as f64).log10();
    -3.0 - peak_db
}

fn play_samples(samples: &[f32], sample_rate: u32) {
    let opts = AudioContextOptions::new();
    opts.set_sample_rate(sample_rate as f32);
    let ctx = AudioContext::new_with_context_options(&opts)
        .or_else(|_| AudioContext::new())
        .unwrap();

    let buffer = ctx
        .create_buffer(1, samples.len() as u32, sample_rate as f32)
        .unwrap();
    let _ = buffer.copy_to_channel(samples, 0);

    let source = ctx.create_buffer_source().unwrap();
    source.set_buffer(Some(&buffer));
    let _ = source.connect_with_audio_node(&ctx.destination());
    let _ = source.start();

    CURRENT_SOURCE.with(|s| {
        *s.borrow_mut() = Some(source);
    });
    CURRENT_CTX.with(|c| {
        *c.borrow_mut() = Some(ctx);
    });
}

/// Animate the playhead using requestAnimationFrame
fn start_playhead(state: AppState, start_time: f64, duration: f64, speed: f64) {
    let window = web_sys::window().unwrap();
    let perf = window.performance().unwrap();
    let anim_start = perf.now();
    let end_time = start_time + duration;

    // Use a recursive rAF loop via Closure
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;

    let cb: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
    let cb_clone = cb.clone();

    *cb.borrow_mut() = Some(Closure::new(move || {
        if !state.is_playing.get_untracked() {
            return;
        }
        let window = web_sys::window().unwrap();
        let perf = window.performance().unwrap();
        let elapsed_ms = perf.now() - anim_start;
        let elapsed_real = elapsed_ms / 1000.0;
        let current = start_time + elapsed_real * speed;

        if current >= end_time {
            state.playhead_time.set(end_time);
            // Always restore scroll to pre-play position when playback finishes
            state.scroll_offset.set(state.pre_play_scroll.get_untracked());
            state.is_playing.set(false);
            // Show bookmark popup briefly if any bookmarks were made during playback
            if !state.bookmarks.get_untracked().is_empty() {
                state.show_bookmark_popup.set(true);
                // Auto-dismiss after 6 seconds
                let state_bm = state.clone();
                let cb = wasm_bindgen::closure::Closure::once(move || {
                    state_bm.show_bookmark_popup.set(false);
                });
                let _ = web_sys::window().unwrap()
                    .set_timeout_with_callback_and_timeout_and_arguments_0(
                        cb.as_ref().unchecked_ref(), 6000,
                    );
                cb.forget();
            }
            return;
        }

        state.playhead_time.set(current);

        let handle = window
            .request_animation_frame(
                cb_clone.borrow().as_ref().unwrap().as_ref().unchecked_ref(),
            )
            .unwrap();
        PLAYHEAD_HANDLE.with(|h| {
            *h.borrow_mut() = Some(handle);
        });
    }));

    let handle = window
        .request_animation_frame(cb.borrow().as_ref().unwrap().as_ref().unchecked_ref())
        .unwrap();
    PLAYHEAD_HANDLE.with(|h| {
        *h.borrow_mut() = Some(handle);
    });
}

fn cancel_playhead() {
    PLAYHEAD_HANDLE.with(|h| {
        if let Some(handle) = h.borrow_mut().take() {
            let _ = web_sys::window().unwrap().cancel_animation_frame(handle);
        }
    });
}
