use realfft::RealFftPlanner;

/// Apply a multi-band EQ filter in the frequency domain using overlap-add FFT processing.
///
/// Bands are defined relative to the "selected" frequency range [freq_low, freq_high]:
/// - Below: 0 to freq_low
/// - Selected: freq_low to freq_high
/// - Harmonics: freq_high to freq_high*2 (only band_mode==4 and selection < 1 octave)
/// - Above: everything above (band_mode >= 3)
///
/// In 2-band mode, everything at or above freq_low uses db_selected.
pub fn apply_eq_filter(
    samples: &[f32],
    sample_rate: u32,
    freq_low: f64,
    freq_high: f64,
    db_below: f64,
    db_selected: f64,
    db_harmonics: f64,
    db_above: f64,
    band_mode: u8,
) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }

    let fft_size = 4096;
    let hop_size = fft_size / 2;
    let len = samples.len();

    // Hann window
    let window: Vec<f32> = (0..fft_size)
        .map(|i| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (fft_size - 1) as f32).cos())
        })
        .collect();

    // Build per-bin gain table
    let num_bins = fft_size / 2 + 1;
    let freq_per_bin = sample_rate as f64 / fft_size as f64;
    let harmonics_active = band_mode >= 4 && freq_high > 0.0 && freq_low > 0.0 && freq_high / freq_low < 2.0;
    let harmonics_upper = freq_high * 2.0;

    let gains: Vec<f32> = (0..num_bins)
        .map(|i| {
            let freq = i as f64 * freq_per_bin;
            let db = if freq < freq_low {
                db_below
            } else if freq <= freq_high {
                db_selected
            } else if band_mode <= 2 {
                // 2-band: everything above uses selected
                db_selected
            } else if harmonics_active && freq <= harmonics_upper {
                db_harmonics
            } else {
                // 3-band or 4-band above region
                db_above
            };
            10.0_f64.powf(db / 20.0) as f32
        })
        .collect();

    let mut planner = RealFftPlanner::<f32>::new();
    let fft_fwd = planner.plan_fft_forward(fft_size);
    let fft_inv = planner.plan_fft_inverse(fft_size);

    // Overlap-add output buffer
    let mut output = vec![0.0f32; len];
    let mut window_sum = vec![0.0f32; len];

    let mut pos = 0;
    while pos < len {
        // Extract frame, zero-pad if needed
        let mut frame = vec![0.0f32; fft_size];
        for (i, &w) in window.iter().enumerate() {
            if pos + i < len {
                frame[i] = samples[pos + i] * w;
            }
        }

        // Forward FFT
        let mut spectrum = fft_fwd.make_output_vec();
        fft_fwd.process(&mut frame, &mut spectrum).expect("FFT forward failed");

        // Apply per-bin gains
        for (bin, gain) in gains.iter().enumerate() {
            if bin < spectrum.len() {
                spectrum[bin] *= *gain;
            }
        }

        // Inverse FFT
        let mut time_out = fft_inv.make_output_vec();
        fft_inv.process(&mut spectrum, &mut time_out).expect("FFT inverse failed");

        // Normalize (realfft inverse doesn't normalize)
        let norm = 1.0 / fft_size as f32;

        // Overlap-add with window
        for i in 0..fft_size {
            if pos + i < len {
                output[pos + i] += time_out[i] * norm * window[i];
                window_sum[pos + i] += window[i] * window[i];
            }
        }

        pos += hop_size;
    }

    // Normalize by window sum to avoid amplitude changes
    for i in 0..len {
        if window_sum[i] > 1e-6 {
            output[i] /= window_sum[i];
        }
    }

    output
}

/// Fast IIR-based multi-band EQ using cascaded lowpass band-splitting.
/// Lower latency than FFT version, suitable for live parameter changes.
/// Band transitions are softer (~24 dB/octave) compared to FFT's brick-wall.
pub fn apply_eq_filter_fast(
    samples: &[f32],
    sample_rate: u32,
    freq_low: f64,
    freq_high: f64,
    db_below: f64,
    db_selected: f64,
    db_harmonics: f64,
    db_above: f64,
    band_mode: u8,
) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }

    let gain_below = 10.0_f64.powf(db_below / 20.0) as f32;
    let gain_selected = 10.0_f64.powf(db_selected / 20.0) as f32;
    let harmonics_active = band_mode >= 4 && freq_low > 0.0 && freq_high / freq_low < 2.0;
    let gain_harmonics = if harmonics_active { 10.0_f64.powf(db_harmonics / 20.0) as f32 } else { 0.0 };
    let gain_above = if band_mode >= 3 { 10.0_f64.powf(db_above / 20.0) as f32 } else { gain_selected };

    // Split at freq_low: below vs rest
    let lp_low = cascaded_lowpass(samples, freq_low, sample_rate, 4);
    let hp_low: Vec<f32> = samples.iter().zip(lp_low.iter()).map(|(s, l)| s - l).collect();

    // Split rest at freq_high: selected vs upper
    let lp_high = cascaded_lowpass(&hp_low, freq_high, sample_rate, 4);
    let hp_high: Vec<f32> = hp_low.iter().zip(lp_high.iter()).map(|(s, l)| s - l).collect();

    let len = samples.len();
    let mut output = vec![0.0f32; len];

    // Below band
    for i in 0..len {
        output[i] += lp_low[i] * gain_below;
    }

    // Selected band
    for i in 0..len {
        output[i] += lp_high[i] * gain_selected;
    }

    if harmonics_active {
        // Split upper at harmonics boundary
        let harmonics_upper = freq_high * 2.0;
        let lp_harm = cascaded_lowpass(&hp_high, harmonics_upper, sample_rate, 4);
        let hp_harm: Vec<f32> = hp_high.iter().zip(lp_harm.iter()).map(|(s, l)| s - l).collect();

        for i in 0..len {
            output[i] += lp_harm[i] * gain_harmonics;
        }
        for i in 0..len {
            output[i] += hp_harm[i] * gain_above;
        }
    } else {
        // Above band (or selected dB in 2-band mode)
        for i in 0..len {
            output[i] += hp_high[i] * gain_above;
        }
    }

    output
}

fn cascaded_lowpass(samples: &[f32], cutoff: f64, sample_rate: u32, passes: usize) -> Vec<f32> {
    let mut result = samples.to_vec();
    for _ in 0..passes {
        result = lowpass_filter(&result, cutoff, sample_rate);
    }
    result
}

/// Simple single-pole IIR low-pass filter (first-order exponential moving average).
///
/// Transfer function: y[n] = alpha * x[n] + (1 - alpha) * y[n-1]
/// where alpha = dt / (rc + dt), rc = 1 / (2 * PI * cutoff_hz)
///
/// For production use, upgrade to a higher-order Butterworth or Chebyshev filter
/// for sharper rolloff.
pub fn lowpass_filter(samples: &[f32], cutoff_hz: f64, sample_rate: u32) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }

    let dt = 1.0 / sample_rate as f64;
    let rc = 1.0 / (2.0 * std::f64::consts::PI * cutoff_hz);
    let alpha = (dt / (rc + dt)) as f32;

    let mut output = Vec::with_capacity(samples.len());
    let mut prev = samples[0];
    output.push(prev);

    for &sample in &samples[1..] {
        let filtered = alpha * sample + (1.0 - alpha) * prev;
        output.push(filtered);
        prev = filtered;
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lowpass_attenuates_high_frequency() {
        let sample_rate = 192_000u32;
        let num_samples = 19200; // 100ms

        // Generate a 50 kHz signal
        let high: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * std::f64::consts::PI * 50_000.0 * t).sin() as f32
            })
            .collect();

        let filtered = lowpass_filter(&high, 10_000.0, sample_rate);

        let rms_in: f64 =
            (high.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / high.len() as f64).sqrt();
        let rms_out: f64 = (filtered.iter().map(|s| (*s as f64).powi(2)).sum::<f64>()
            / filtered.len() as f64)
            .sqrt();

        assert!(
            rms_out < rms_in * 0.3,
            "50 kHz should be attenuated by LPF at 10 kHz: in={rms_in}, out={rms_out}"
        );
    }

    #[test]
    fn test_lowpass_passes_low_frequency() {
        let sample_rate = 192_000u32;
        let num_samples = 19200;

        // Generate a 1 kHz signal (well below 10 kHz cutoff)
        let low: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * std::f64::consts::PI * 1_000.0 * t).sin() as f32
            })
            .collect();

        let filtered = lowpass_filter(&low, 10_000.0, sample_rate);

        let rms_in: f64 =
            (low.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / low.len() as f64).sqrt();
        let rms_out: f64 = (filtered.iter().map(|s| (*s as f64).powi(2)).sum::<f64>()
            / filtered.len() as f64)
            .sqrt();

        assert!(
            rms_out > rms_in * 0.8,
            "1 kHz should pass through LPF at 10 kHz: in={rms_in}, out={rms_out}"
        );
    }

    #[test]
    fn test_empty_input() {
        let result = lowpass_filter(&[], 1000.0, 44100);
        assert!(result.is_empty());
    }
}
