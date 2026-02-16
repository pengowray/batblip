/// Pitch-shift audio down by a given factor using linear interpolation resampling.
///
/// This stretches the sample data by `factor` (inserting interpolated samples),
/// then the caller plays it at the original sample rate. The result is that all
/// frequencies are divided by `factor` while playback duration is multiplied by
/// `factor`.
///
/// For bat audio: a factor of 10 shifts 50 kHz → 5 kHz (audible), and a 1-second
/// recording becomes 10 seconds — like time expansion, but conceptually framed as
/// pitch shifting.
pub fn pitch_shift_down(samples: &[f32], factor: f64) -> Vec<f32> {
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
