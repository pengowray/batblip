//! Power Spectral Density estimation using Welch's method.
//!
//! Computes averaged periodograms over overlapping Hann-windowed segments,
//! with peak detection and bandwidth analysis (-6 dB, -10 dB).

use leptos::prelude::GetUntracked;
use realfft::RealFftPlanner;
use std::cell::RefCell;
use std::collections::HashMap;

// ── Thread-local caches ─────────────────────────────────────────────────────

thread_local! {
    static PSD_FFT_PLANNER: RefCell<RealFftPlanner<f32>> = RefCell::new(RealFftPlanner::new());
    static PSD_HANN_CACHE: RefCell<HashMap<usize, Vec<f32>>> = RefCell::new(HashMap::new());
}

fn hann_window(size: usize) -> Vec<f32> {
    PSD_HANN_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .entry(size)
            .or_insert_with(|| {
                (0..size)
                    .map(|i| {
                        0.5 * (1.0
                            - (2.0 * std::f32::consts::PI * i as f32 / (size - 1) as f32).cos())
                    })
                    .collect()
            })
            .clone()
    })
}

/// Hann window power correction factor: sum of squared window values.
fn hann_power_sum(size: usize) -> f64 {
    let w = hann_window(size);
    w.iter().map(|&v| (v as f64) * (v as f64)).sum()
}

// ── Result types ────────────────────────────────────────────────────────────

/// Result of a PSD computation.
#[derive(Clone, Debug)]
pub struct PsdResult {
    /// Power spectral density in dB per bin (length = nfft/2 + 1).
    /// Bin 0 = DC, bin N = Nyquist.
    pub power_db: Vec<f64>,
    /// Frequency resolution in Hz per bin.
    pub freq_resolution: f64,
    /// Sample rate of the source audio.
    pub sample_rate: u32,
    /// NFFT size used.
    pub nfft: usize,
    /// Number of frames averaged.
    pub frame_count: usize,
    /// Peak analysis results.
    pub peak: PsdPeak,
}

/// Peak frequency and bandwidth analysis from a PSD.
#[derive(Clone, Debug)]
pub struct PsdPeak {
    /// Peak frequency in Hz.
    pub freq_hz: f64,
    /// Peak power in dB.
    pub power_db: f64,
    /// Bin index of the peak.
    pub bin_index: usize,
    /// -6 dB bandwidth: (low_hz, high_hz). None if the peak doesn't drop 6 dB.
    pub bw_6db: Option<(f64, f64)>,
    /// -10 dB bandwidth: (low_hz, high_hz). None if the peak doesn't drop 10 dB.
    pub bw_10db: Option<(f64, f64)>,
}

// ── Computation ─────────────────────────────────────────────────────────────

/// Compute PSD using Welch's method (synchronous).
///
/// - `samples`: mono f32 audio
/// - `sample_rate`: Hz
/// - `nfft`: FFT size (e.g. 256, 512, 1024, 2048, 4096)
///
/// Uses 50% overlap and Hann window.
pub fn compute_psd(samples: &[f32], sample_rate: u32, nfft: usize) -> PsdResult {
    let n_bins = nfft / 2 + 1;
    let hop = nfft / 2;
    let window = hann_window(nfft);
    let power_norm = hann_power_sum(nfft) * sample_rate as f64;

    let fft = PSD_FFT_PLANNER.with(|p| p.borrow_mut().plan_fft_forward(nfft));
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();

    let mut accum = vec![0.0f64; n_bins];
    let mut frame_count = 0usize;

    let mut pos = 0usize;
    while pos + nfft <= samples.len() {
        let frame = &samples[pos..pos + nfft];

        // Apply window
        for (inp, (&s, &w)) in input.iter_mut().zip(frame.iter().zip(window.iter())) {
            *inp = s * w;
        }

        // FFT
        fft.process(&mut input, &mut spectrum).expect("FFT failed");

        // Accumulate |X[k]|²
        for (acc, c) in accum.iter_mut().zip(spectrum.iter()) {
            *acc += (c.re as f64) * (c.re as f64) + (c.im as f64) * (c.im as f64);
        }

        frame_count += 1;
        pos += hop;
    }

    // Average and normalize to PSD, convert to dB
    let power_db: Vec<f64> = if frame_count > 0 {
        accum
            .iter()
            .enumerate()
            .map(|(i, &sum)| {
                let mut psd = sum / (frame_count as f64 * power_norm);
                // Double non-DC, non-Nyquist bins (one-sided spectrum)
                if i > 0 && i < n_bins - 1 {
                    psd *= 2.0;
                }
                if psd > 0.0 {
                    10.0 * psd.log10()
                } else {
                    -200.0
                }
            })
            .collect()
    } else {
        vec![-200.0; n_bins]
    };

    let freq_resolution = sample_rate as f64 / nfft as f64;
    let peak = find_peak(&power_db, freq_resolution);

    PsdResult {
        power_db,
        freq_resolution,
        sample_rate,
        nfft,
        frame_count,
        peak,
    }
}

/// Async version that yields to the browser every `yield_interval` frames.
pub async fn compute_psd_async(
    samples: &[f32],
    sample_rate: u32,
    nfft: usize,
    generation: u32,
    gen_signal: leptos::prelude::RwSignal<u32>,
) -> Option<PsdResult> {
    use wasm_bindgen::prelude::*;

    let n_bins = nfft / 2 + 1;
    let hop = nfft / 2;
    let window = hann_window(nfft);
    let power_norm = hann_power_sum(nfft) * sample_rate as f64;

    let fft = PSD_FFT_PLANNER.with(|p| p.borrow_mut().plan_fft_forward(nfft));
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();

    let mut accum = vec![0.0f64; n_bins];
    let mut frame_count = 0usize;

    let yield_interval = 64;
    let mut pos = 0usize;
    while pos + nfft <= samples.len() {
        let frame = &samples[pos..pos + nfft];

        for (inp, (&s, &w)) in input.iter_mut().zip(frame.iter().zip(window.iter())) {
            *inp = s * w;
        }
        fft.process(&mut input, &mut spectrum).expect("FFT failed");

        for (acc, c) in accum.iter_mut().zip(spectrum.iter()) {
            *acc += (c.re as f64) * (c.re as f64) + (c.im as f64) * (c.im as f64);
        }

        frame_count += 1;
        pos += hop;

        if frame_count % yield_interval == 0 {
            // Yield to browser
            let promise = js_sys::Promise::new(&mut |resolve, _| {
                let win = web_sys::window().unwrap();
                let cb = Closure::once_into_js(move || {
                    let _ = resolve.call0(&JsValue::NULL);
                });
                let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                    cb.unchecked_ref(),
                    0,
                );
            });
            let _ = wasm_bindgen_futures::JsFuture::from(promise).await;

            // Check cancellation
            if gen_signal.get_untracked() != generation {
                return None;
            }
        }
    }

    let power_db: Vec<f64> = if frame_count > 0 {
        accum
            .iter()
            .enumerate()
            .map(|(i, &sum)| {
                let mut psd = sum / (frame_count as f64 * power_norm);
                if i > 0 && i < n_bins - 1 {
                    psd *= 2.0;
                }
                if psd > 0.0 {
                    10.0 * psd.log10()
                } else {
                    -200.0
                }
            })
            .collect()
    } else {
        vec![-200.0; n_bins]
    };

    let freq_resolution = sample_rate as f64 / nfft as f64;
    let peak = find_peak(&power_db, freq_resolution);

    Some(PsdResult {
        power_db,
        freq_resolution,
        sample_rate,
        nfft,
        frame_count,
        peak,
    })
}

// ── Peak detection ──────────────────────────────────────────────────────────

fn find_peak(power_db: &[f64], freq_resolution: f64) -> PsdPeak {
    if power_db.is_empty() {
        return PsdPeak {
            freq_hz: 0.0,
            power_db: -200.0,
            bin_index: 0,
            bw_6db: None,
            bw_10db: None,
        };
    }

    // Find peak bin (skip DC bin 0)
    let start_bin = 1;
    let (peak_bin, &peak_power) = power_db[start_bin..]
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, v)| (i + start_bin, v))
        .unwrap_or((0, &-200.0));

    let peak_freq = peak_bin as f64 * freq_resolution;

    let bw_6db = find_bandwidth(power_db, peak_bin, peak_power, 6.0, freq_resolution);
    let bw_10db = find_bandwidth(power_db, peak_bin, peak_power, 10.0, freq_resolution);

    PsdPeak {
        freq_hz: peak_freq,
        power_db: peak_power,
        bin_index: peak_bin,
        bw_6db,
        bw_10db,
    }
}

/// Find bandwidth at `drop_db` below peak using linear interpolation.
fn find_bandwidth(
    power_db: &[f64],
    peak_bin: usize,
    peak_power: f64,
    drop_db: f64,
    freq_resolution: f64,
) -> Option<(f64, f64)> {
    let threshold = peak_power - drop_db;

    // Walk left from peak
    let low_freq = {
        let mut low_bin = None;
        for i in (1..peak_bin).rev() {
            if power_db[i] < threshold {
                // Interpolate between bin i and i+1
                let frac = if (power_db[i + 1] - power_db[i]).abs() > 1e-12 {
                    (threshold - power_db[i]) / (power_db[i + 1] - power_db[i])
                } else {
                    0.5
                };
                low_bin = Some((i as f64 + frac) * freq_resolution);
                break;
            }
        }
        low_bin
    };

    // Walk right from peak
    let high_freq = {
        let mut high_bin = None;
        for i in (peak_bin + 1)..power_db.len() {
            if power_db[i] < threshold {
                // Interpolate between bin i-1 and i
                let frac = if (power_db[i - 1] - power_db[i]).abs() > 1e-12 {
                    (threshold - power_db[i - 1]) / (power_db[i] - power_db[i - 1])
                } else {
                    0.5
                };
                high_bin = Some(((i - 1) as f64 + frac) * freq_resolution);
                break;
            }
        }
        high_bin
    };

    match (low_freq, high_freq) {
        (Some(lo), Some(hi)) => Some((lo, hi)),
        _ => None,
    }
}
