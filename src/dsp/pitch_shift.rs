/// Pitch-shift audio down by `factor` while preserving original duration.
///
/// Two-step process:
/// 1. Resample (stretch) by `factor` — lowers all frequencies by `factor`
/// 2. Time-compress back to original length using overlap-add (OLA)
///
/// Result: frequencies divided by `factor`, duration unchanged.
/// For bat audio: factor=10 shifts 50 kHz → 5 kHz, plays back at real-time speed.
pub fn pitch_shift_realtime(samples: &[f32], factor: f64) -> Vec<f32> {
    if samples.is_empty() || factor <= 1.0 {
        return samples.to_vec();
    }

    // Step 1: resample — stretches length by `factor`, lowers frequencies
    let stretched = pitch_shift_down(samples, factor);

    // Step 2: OLA time-compression back to original length
    let window_size: usize = 2048;
    let synthesis_hop = window_size / 2;
    let analysis_hop = (synthesis_hop as f64 * factor) as usize;

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

    while read_pos + window_size <= stretched.len() && write_pos + window_size <= out_len {
        for i in 0..window_size {
            output[write_pos + i] += stretched[read_pos + i] * hann[i];
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

/// Resample audio by linear interpolation, stretching length by `factor`.
/// Frequencies are divided by `factor`, duration multiplied by `factor`.
fn pitch_shift_down(samples: &[f32], factor: f64) -> Vec<f32> {
    if samples.is_empty() || factor <= 1.0 {
        return samples.to_vec();
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pitch_shift_doubles_length() {
        let input: Vec<f32> = (0..100).map(|i| (i as f32 / 100.0).sin()).collect();
        let output = pitch_shift_down(&input, 2.0);
        assert_eq!(output.len(), 200);
    }

    #[test]
    fn test_pitch_shift_factor_one_is_identity() {
        let input: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0];
        let output = pitch_shift_down(&input, 1.0);
        assert_eq!(output, input);
    }

    #[test]
    fn test_pitch_shift_empty() {
        let output = pitch_shift_down(&[], 10.0);
        assert!(output.is_empty());
    }

    #[test]
    fn test_pitch_shift_preserves_endpoints() {
        let input: Vec<f32> = vec![0.0, 0.5, 1.0];
        let output = pitch_shift_down(&input, 3.0);
        // First sample should be 0.0
        assert!((output[0] - 0.0).abs() < 0.01);
        // Last sample should approach 1.0
        assert!((output[output.len() - 1] - 1.0).abs() < 0.1);
    }
}
