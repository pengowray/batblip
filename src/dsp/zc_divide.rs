/// Simulate a zero-crossing frequency division bat detector.
///
/// Old-style bat detectors work by detecting zero crossings in the ultrasonic
/// input and producing an audible click every Nth crossing. The result is a
/// characteristic rhythmic clicking whose rate equals the input frequency
/// divided by `division_factor`.
pub fn zc_divide(samples: &[f32], sample_rate: u32, division_factor: u32) -> Vec<f32> {
    if samples.len() < 2 || division_factor == 0 {
        return vec![0.0; samples.len()];
    }

    let mut output = vec![0.0f32; samples.len()];
    let mut crossing_count: u32 = 0;

    // Click duration: ~0.1ms worth of samples
    let click_len = (sample_rate as f64 * 0.0001) as usize;
    let click_len = click_len.max(1);

    for i in 1..samples.len() {
        let prev_positive = samples[i - 1] >= 0.0;
        let curr_positive = samples[i] >= 0.0;

        if prev_positive != curr_positive {
            crossing_count += 1;
            if crossing_count >= division_factor {
                crossing_count = 0;
                // Emit a short click (half-sine pulse)
                let end = (i + click_len).min(samples.len());
                for j in i..end {
                    let phase = (j - i) as f64 / click_len as f64 * std::f64::consts::PI;
                    output[j] = phase.sin() as f32;
                }
            }
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn test_known_sine_produces_clicks() {
        let sample_rate = 192_000u32;
        let freq = 45_000.0f64;
        let duration = 0.01; // 10ms
        let num_samples = (sample_rate as f64 * duration) as usize;
        let division = 10u32;

        let input: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * PI * freq * t).sin() as f32
            })
            .collect();

        let output = zc_divide(&input, sample_rate, division);
        assert_eq!(output.len(), input.len());

        // Count output clicks (non-zero regions)
        let mut click_count = 0usize;
        let mut in_click = false;
        for &s in &output {
            if s.abs() > 0.01 {
                if !in_click {
                    click_count += 1;
                    in_click = true;
                }
            } else {
                in_click = false;
            }
        }

        // 45 kHz sine has 90,000 zero crossings/sec. Over 10ms = 900 crossings.
        // Dividing by 10 → ~90 clicks. Allow some tolerance.
        let expected = (freq * 2.0 * duration / division as f64) as usize;
        let diff = (click_count as isize - expected as isize).unsigned_abs();
        assert!(
            diff <= 5,
            "Expected ~{expected} clicks, got {click_count}"
        );
    }

    #[test]
    fn test_empty_input() {
        let output = zc_divide(&[], 192_000, 10);
        assert!(output.is_empty());
    }

    #[test]
    fn test_dc_signal_no_clicks() {
        let input = vec![1.0f32; 1000];
        let output = zc_divide(&input, 44100, 10);
        assert!(output.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn test_division_factor_one() {
        // Every crossing produces a click
        let sample_rate = 192_000u32;
        let freq = 1000.0f64;
        let duration = 0.01;
        let num_samples = (sample_rate as f64 * duration) as usize;

        let input: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * PI * freq * t).sin() as f32
            })
            .collect();

        let output = zc_divide(&input, sample_rate, 1);
        let has_energy = output.iter().any(|&s| s.abs() > 0.01);
        assert!(has_energy, "Division by 1 should produce clicks");
    }
}
