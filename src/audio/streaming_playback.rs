//! Streaming playback engine.
//!
//! Instead of processing the entire selection through DSP before any audio
//! plays, this module processes and schedules audio in small chunks (~0.5s).
//! The user hears sound almost immediately while subsequent chunks are
//! processed in the background.

use web_sys::{AudioContext, AudioContextOptions};
use wasm_bindgen_futures::JsFuture;
use std::cell::RefCell;
use std::sync::Arc;

use crate::state::{PlaybackMode, FilterQuality};
use crate::dsp::heterodyne::heterodyne_mix;
use crate::dsp::pitch_shift::pitch_shift_realtime;
use crate::dsp::zc_divide::zc_divide;
use crate::dsp::filters::{apply_eq_filter, apply_eq_filter_fast};
use crate::audio::playback::{apply_bandpass, apply_gain, auto_gain_db};

/// Number of source samples per chunk. ~0.5s at 192kHz, ~2s at 44.1kHz.
const CHUNK_SAMPLES: usize = 96_000;

/// Extra overlap samples prepended to each chunk for IIR filter warmup.
/// This lets filters (heterodyne lowpass, bandpass) settle before the
/// actual chunk data, avoiding clicks at boundaries.
const FILTER_WARMUP: usize = 4096;

/// How far ahead (in seconds) to stay buffered beyond current playback time.
const LOOKAHEAD_SECS: f64 = 1.5;

thread_local! {
    static STREAM_CTX: RefCell<Option<AudioContext>> = RefCell::new(None);
    /// Monotonically increasing generation counter to detect stale streams.
    static STREAM_GEN: RefCell<u32> = RefCell::new(0);
}

/// Snapshot of all playback parameters, frozen at play start so that
/// parameter changes mid-playback don't cause glitches.
pub(crate) struct PlaybackParams {
    pub mode: PlaybackMode,
    pub het_freq: f64,
    pub het_cutoff: f64,
    pub te_factor: f64,
    pub ps_factor: f64,
    pub zc_factor: f64,
    pub gain_db: f64,
    pub auto_gain: bool,
    pub filter_enabled: bool,
    pub filter_freq_low: f64,
    pub filter_freq_high: f64,
    pub filter_db_below: f64,
    pub filter_db_selected: f64,
    pub filter_db_harmonics: f64,
    pub filter_db_above: f64,
    pub filter_band_mode: u8,
    pub filter_quality: FilterQuality,
    pub sel_freq_low: f64,
    pub sel_freq_high: f64,
    pub has_selection: bool,
}

/// Stop any active streaming playback.
pub(crate) fn stop_stream() {
    STREAM_GEN.with(|g| {
        let mut gen = g.borrow_mut();
        *gen = gen.wrapping_add(1);
    });
    STREAM_CTX.with(|c| {
        if let Some(ctx) = c.borrow_mut().take() {
            let _ = ctx.close();
        }
    });
}

/// Returns true if streaming playback is currently active.
#[allow(dead_code)]
pub(crate) fn is_streaming() -> bool {
    STREAM_CTX.with(|c| c.borrow().is_some())
}

/// Start streaming playback of a sample range.
///
/// Returns the final playback sample rate (may differ from source for TE mode).
pub(crate) fn start_stream(
    source_samples: Arc<Vec<f32>>,
    sample_rate: u32,
    start_sample: usize,
    end_sample: usize,
    params: PlaybackParams,
) -> u32 {
    stop_stream();

    let final_rate = match params.mode {
        PlaybackMode::TimeExpansion => {
            let abs_f = params.te_factor.abs().max(1.0);
            let rate = if params.te_factor > 0.0 {
                sample_rate as f64 / abs_f
            } else {
                sample_rate as f64 * abs_f
            };
            (rate as u32).clamp(8000, 384_000)
        }
        _ => sample_rate,
    };

    // Create AudioContext at the final playback rate
    let opts = AudioContextOptions::new();
    opts.set_sample_rate(final_rate as f32);
    let ctx = AudioContext::new_with_context_options(&opts)
        .or_else(|_| AudioContext::new())
        .unwrap();

    let generation = STREAM_GEN.with(|g| *g.borrow());
    STREAM_CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));

    // Spawn the async chunk-processing loop
    wasm_bindgen_futures::spawn_local(chunk_loop(
        ctx,
        generation,
        source_samples,
        sample_rate,
        final_rate,
        start_sample,
        end_sample,
        params,
    ));

    final_rate
}

/// Async loop that processes and schedules audio chunks.
async fn chunk_loop(
    ctx: AudioContext,
    generation: u32,
    source: Arc<Vec<f32>>,
    source_rate: u32,
    final_rate: u32,
    start_sample: usize,
    end_sample: usize,
    params: PlaybackParams,
) {
    let mut pos = start_sample;
    // Small initial delay so the first chunk has time to be created
    let mut scheduled_time = ctx.current_time() + 0.02;

    // For auto-gain: compute gain from first chunk and reuse for all
    let mut cached_gain: Option<f64> = None;

    while pos < end_sample {
        // Check if this stream has been cancelled
        let current_gen = STREAM_GEN.with(|g| *g.borrow());
        if current_gen != generation {
            break;
        }

        // Determine chunk boundaries with filter warmup overlap
        let warmup_start = if pos > start_sample {
            pos.saturating_sub(FILTER_WARMUP)
        } else {
            pos
        };
        let chunk_end = (pos + CHUNK_SAMPLES).min(end_sample);
        let warmup_len = pos - warmup_start;

        let chunk_with_warmup = &source[warmup_start..chunk_end];

        // Apply EQ/bandpass filter
        let filtered = apply_filters(chunk_with_warmup, source_rate, &params);

        // Apply DSP mode transform
        let processed = apply_dsp_mode(&filtered, source_rate, &params);

        // Trim warmup samples from the output.
        // For most modes, output length == input length, so trimming by
        // warmup_len is correct. For PitchShift, the output length differs
        // proportionally, so we scale.
        let trim = match params.mode {
            PlaybackMode::PitchShift if params.ps_factor > 1.0 => {
                // Shift-down: warmup maps to fewer output samples
                ((warmup_len as f64) / params.ps_factor) as usize
            }
            PlaybackMode::ZeroCrossing => {
                // zc_divide output matches input length
                warmup_len
            }
            _ => warmup_len,
        };
        let trimmed = if trim > 0 && trim < processed.len() {
            &processed[trim..]
        } else {
            &processed[..]
        };

        // Apply gain
        let mut final_samples = trimmed.to_vec();
        let gain = if params.auto_gain {
            match cached_gain {
                Some(g) => g,
                None => {
                    let g = auto_gain_db(&final_samples);
                    cached_gain = Some(g);
                    g
                }
            }
        } else {
            params.gain_db
        };
        apply_gain(&mut final_samples, gain);

        // Schedule this chunk in Web Audio
        if !final_samples.is_empty() {
            schedule_buffer(&ctx, &final_samples, final_rate, scheduled_time);
            let chunk_duration = final_samples.len() as f64 / final_rate as f64;
            scheduled_time += chunk_duration;
        }

        pos = chunk_end;

        // Yield to browser so UI stays responsive
        yield_to_browser().await;

        // If we're well ahead of playback, sleep before processing next chunk
        let now = ctx.current_time();
        if scheduled_time - now > LOOKAHEAD_SECS {
            let sleep_ms = ((scheduled_time - now - LOOKAHEAD_SECS * 0.5) * 1000.0) as u32;
            if sleep_ms > 10 {
                sleep(sleep_ms).await;
            }
        }
    }
}

fn apply_filters(samples: &[f32], sample_rate: u32, params: &PlaybackParams) -> Vec<f32> {
    if params.filter_enabled {
        match params.filter_quality {
            FilterQuality::Fast => apply_eq_filter_fast(
                samples, sample_rate,
                params.filter_freq_low, params.filter_freq_high,
                params.filter_db_below, params.filter_db_selected,
                params.filter_db_harmonics, params.filter_db_above,
                params.filter_band_mode,
            ),
            FilterQuality::HQ => apply_eq_filter(
                samples, sample_rate,
                params.filter_freq_low, params.filter_freq_high,
                params.filter_db_below, params.filter_db_selected,
                params.filter_db_harmonics, params.filter_db_above,
                params.filter_band_mode,
            ),
        }
    } else if params.has_selection
        && matches!(
            params.mode,
            PlaybackMode::Normal
                | PlaybackMode::TimeExpansion
                | PlaybackMode::PitchShift
                | PlaybackMode::ZeroCrossing
        )
        && (params.sel_freq_low > 0.0
            || params.sel_freq_high < (sample_rate as f64 / 2.0))
    {
        apply_bandpass(samples, sample_rate, params.sel_freq_low, params.sel_freq_high)
    } else {
        samples.to_vec()
    }
}

fn apply_dsp_mode(samples: &[f32], sample_rate: u32, params: &PlaybackParams) -> Vec<f32> {
    match params.mode {
        PlaybackMode::Normal => samples.to_vec(),
        PlaybackMode::Heterodyne => {
            let effective_lo =
                if params.has_selection && (params.sel_freq_low > 0.0 || params.sel_freq_high > 0.0)
                {
                    (params.sel_freq_low + params.sel_freq_high) / 2.0
                } else {
                    params.het_freq
                };
            heterodyne_mix(samples, sample_rate, effective_lo, params.het_cutoff)
        }
        PlaybackMode::TimeExpansion => {
            // Rate change handled by AudioContext sample rate, not sample transform
            samples.to_vec()
        }
        PlaybackMode::PitchShift => pitch_shift_realtime(samples, params.ps_factor),
        PlaybackMode::ZeroCrossing => {
            zc_divide(samples, sample_rate, params.zc_factor as u32, params.filter_enabled)
        }
    }
}

fn schedule_buffer(ctx: &AudioContext, samples: &[f32], sample_rate: u32, when: f64) {
    let Ok(buffer) = ctx.create_buffer(1, samples.len() as u32, sample_rate as f32) else {
        return;
    };
    let _ = buffer.copy_to_channel(samples, 0);
    let Ok(source) = ctx.create_buffer_source() else {
        return;
    };
    source.set_buffer(Some(&buffer));
    let _ = source.connect_with_audio_node(&ctx.destination());
    let _ = source.start_with_when(when);
}

async fn yield_to_browser() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback(&resolve);
        }
    });
    let _ = JsFuture::from(promise).await;
}

async fn sleep(ms: u32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32);
        }
    });
    let _ = JsFuture::from(promise).await;
}
