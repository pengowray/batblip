use leptos::prelude::*;
use web_sys::{AudioContext, AudioContextOptions, AudioBufferSourceNode};
use crate::types::AudioData;
use crate::state::{AppState, Selection, PlaybackMode};
use crate::dsp::heterodyne::heterodyne_mix;
use crate::dsp::pitch_shift::pitch_shift_down;
use crate::dsp::filters::lowpass_filter;
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
    state.is_playing.set(false);
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

    let (samples, sample_rate) = extract_selection(&file.audio, selection);

    // Apply bandpass if selection has frequency bounds (Normal/TE modes)
    let samples = if let Some(sel) = selection {
        if matches!(mode, PlaybackMode::Normal | PlaybackMode::TimeExpansion | PlaybackMode::PitchShift)
            && (sel.freq_low > 0.0 || sel.freq_high < (sample_rate as f64 / 2.0))
        {
            apply_bandpass(&samples, sample_rate, sel.freq_low, sel.freq_high)
        } else {
            samples
        }
    } else {
        samples
    };

    // Determine playback start time (in the original audio timeline)
    let play_start_time = selection.map(|s| s.time_start).unwrap_or(0.0);
    let play_duration_orig = samples.len() as f64 / sample_rate as f64;

    match mode {
        PlaybackMode::Normal => {
            play_samples(&samples, sample_rate);
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
            let processed = heterodyne_mix(&samples, sample_rate, effective_lo);
            play_samples(&processed, sample_rate);
        }
        PlaybackMode::TimeExpansion => {
            // TE: play at reduced sample rate = original_rate / factor
            // This stretches time by the factor, shifting frequencies down
            let te_rate = (sample_rate as f64 / te_factor) as u32;
            let te_rate = te_rate.max(8000); // browser minimum
            play_samples(&samples, te_rate);
        }
        PlaybackMode::PitchShift => {
            // PS: resample to stretch by factor, then play at original rate
            // Frequencies shift down by factor, duration increases by factor
            let shifted = pitch_shift_down(&samples, ps_factor);
            play_samples(&shifted, sample_rate);
        }
    }

    // Start playhead animation
    let playback_speed = match mode {
        PlaybackMode::Normal => 1.0,
        PlaybackMode::Heterodyne => 1.0,
        PlaybackMode::TimeExpansion => 1.0 / te_factor,
        PlaybackMode::PitchShift => 1.0 / ps_factor,
    };

    state.is_playing.set(true);
    state.playhead_time.set(play_start_time);
    start_playhead(state.clone(), play_start_time, play_duration_orig, playback_speed);
}

fn extract_selection(audio: &AudioData, selection: Option<Selection>) -> (Vec<f32>, u32) {
    let sr = audio.sample_rate;
    if let Some(sel) = selection {
        let start = (sel.time_start * sr as f64) as usize;
        let end = (sel.time_end * sr as f64) as usize;
        let start = start.min(audio.samples.len());
        let end = end.min(audio.samples.len());
        if end > start {
            return (audio.samples[start..end].to_vec(), sr);
        }
    }
    (audio.samples.clone(), sr)
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

fn cascaded_lowpass(samples: &[f32], cutoff: f64, sample_rate: u32, passes: usize) -> Vec<f32> {
    let mut result = samples.to_vec();
    for _ in 0..passes {
        result = lowpass_filter(&result, cutoff, sample_rate);
    }
    result
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
            state.is_playing.set(false);
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
