/// Pitch-shift audio by `factor` while preserving original duration.
///
/// - `factor > 1.0`: shift DOWN (divide frequencies). E.g. factor=10 shifts 50 kHz → 5 kHz.
/// - `factor < -1.0`: shift UP (multiply frequencies). E.g. factor=-10 shifts 5 Hz → 50 Hz.
/// - `|factor| <= 1.0`: bypass (returns input unchanged).
pub fn pitch_shift_realtime(samples: &[f32], factor: f64) -> Vec<f32> {
    if samples.is_empty() {
        return samples.to_vec();
    }

    let abs_factor = factor.abs();
    if abs_factor <= 1.0 {
        return samples.to_vec();
    }

    let shift_up = factor < 0.0;

    // Step 1: resample to change frequencies
    let resampled = if shift_up {
        resample_compress(samples, abs_factor) // shorter, higher freq
    } else {
        resample_stretch(samples, abs_factor) // longer, lower freq
    };

    // Step 2: OLA to restore original duration
    // Shift down: resampled is longer → compress with analysis_hop > synthesis_hop
    // Shift up:   resampled is shorter → stretch with analysis_hop < synthesis_hop
    let window_size: usize = 2048;
    let synthesis_hop = window_size / 2;
    let analysis_hop = if shift_up {
        (synthesis_hop as f64 / abs_factor).max(1.0) as usize
    } else {
        (synthesis_hop as f64 * abs_factor) as usize
    };

    let out_len = samples.len();
    let mut output = vec![0.0f32; out_len];
    let mut window_sum = vec![0.0f32; out_len];

    // Hann window
    let hann: Vec<f32> = (0..window_size)
        .map(|i| {
            let x = std::f32::consts::PI * i as f32 / window_size as f32;
            x.sin().powi(2)
        })
        .collect();

    let mut read_pos = 0usize;
    let mut write_pos = 0usize;

    while read_pos + window_size <= resampled.len() && write_pos + window_size <= out_len {
        for i in 0..window_size {
            output[write_pos + i] += resampled[read_pos + i] * hann[i];
            window_sum[write_pos + i] += hann[i];
        }
        read_pos += analysis_hop;
        write_pos += synthesis_hop;
    }

    // Normalize by window overlap sum
    for i in 0..out_len {
        if window_sum[i] > 0.001 {
            output[i] /= window_sum[i];
        }
    }

    output
}

/// Resample by stretching: output is longer, frequencies lower.
fn resample_stretch(samples: &[f32], factor: f64) -> Vec<f32> {
    let out_len = (samples.len() as f64 * factor) as usize;
    let mut output = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let src_pos = i as f64 / factor;
        let idx = src_pos as usize;
        let frac = (src_pos - idx as f64) as f32;

        let s0 = samples[idx.min(samples.len() - 1)];
        let s1 = samples[(idx + 1).min(samples.len() - 1)];
        output.push(s0 + frac * (s1 - s0));
    }

    output
}

/// Resample by compressing: output is shorter, frequencies higher.
fn resample_compress(samples: &[f32], factor: f64) -> Vec<f32> {
    let out_len = (samples.len() as f64 / factor) as usize;
    let mut output = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let src_pos = i as f64 * factor;
        let idx = src_pos as usize;
        let frac = (src_pos - idx as f64) as f32;

        let s0 = samples[idx.min(samples.len() - 1)];
        let s1 = samples[(idx + 1).min(samples.len() - 1)];
        output.push(s0 + frac * (s1 - s0));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resample_stretch_doubles_length() {
        let input: Vec<f32> = (0..100).map(|i| (i as f32 / 100.0).sin()).collect();
        let output = resample_stretch(&input, 2.0);
        assert_eq!(output.len(), 200);
    }

    #[test]
    fn test_resample_compress_halves_length() {
        let input: Vec<f32> = (0..100).map(|i| (i as f32 / 100.0).sin()).collect();
        let output = resample_compress(&input, 2.0);
        assert_eq!(output.len(), 50);
    }

    #[test]
    fn test_pitch_shift_down_preserves_length() {
        let input: Vec<f32> = (0..4096).map(|i| (i as f32 / 100.0).sin()).collect();
        let output = pitch_shift_realtime(&input, 10.0);
        assert_eq!(output.len(), input.len());
    }

    #[test]
    fn test_pitch_shift_up_preserves_length() {
        let input: Vec<f32> = (0..4096).map(|i| (i as f32 / 100.0).sin()).collect();
        let output = pitch_shift_realtime(&input, -10.0);
        assert_eq!(output.len(), input.len());
    }

    #[test]
    fn test_pitch_shift_bypass() {
        let input: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(pitch_shift_realtime(&input, 0.0), input);
        assert_eq!(pitch_shift_realtime(&input, 1.0), input);
        assert_eq!(pitch_shift_realtime(&input, -1.0), input);
    }

    #[test]
    fn test_pitch_shift_empty() {
        assert!(pitch_shift_realtime(&[], 10.0).is_empty());
        assert!(pitch_shift_realtime(&[], -10.0).is_empty());
    }
}
