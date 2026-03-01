use crate::canvas::colors::magnitude_to_greyscale;
use crate::types::{AudioData, PreviewImage, SpectrogramColumn, SpectrogramData};
use realfft::RealFftPlanner;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

thread_local! {
    static FFT_PLANNER: RefCell<RealFftPlanner<f32>> = RefCell::new(RealFftPlanner::new());
    static HANN_CACHE: RefCell<HashMap<usize, Vec<f32>>> = RefCell::new(HashMap::new());
}

fn hann_window(size: usize) -> Vec<f32> {
    HANN_CACHE.with(|cache| {
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

/// Compute a spectrogram from audio data using a Short-Time Fourier Transform (STFT).
///
/// Uses a Hann window for spectral leakage reduction.
pub fn compute_spectrogram(
    audio: &AudioData,
    fft_size: usize,
    hop_size: usize,
) -> SpectrogramData {
    let fft = FFT_PLANNER.with(|p| p.borrow_mut().plan_fft_forward(fft_size));

    let mut columns = Vec::new();

    let window = hann_window(fft_size);

    // Pre-allocate FFT buffers once and reuse across frames
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();

    let mut pos = 0;
    while pos + fft_size <= audio.samples.len() {
        // Fill input in-place (no allocation per frame)
        for (inp, (&s, &w)) in input
            .iter_mut()
            .zip(audio.samples[pos..pos + fft_size].iter().zip(window.iter()))
        {
            *inp = s * w;
        }

        fft.process(&mut input, &mut spectrum).expect("FFT failed");

        let magnitudes: Vec<f32> = spectrum.iter().map(|c| c.norm()).collect();

        let time_offset = pos as f64 / audio.sample_rate as f64;
        columns.push(SpectrogramColumn {
            magnitudes,
            time_offset,
        });

        pos += hop_size;
    }

    let freq_resolution = audio.sample_rate as f64 / fft_size as f64;
    let time_resolution = hop_size as f64 / audio.sample_rate as f64;
    let max_freq = audio.sample_rate as f64 / 2.0;

    let total_columns = columns.len();
    SpectrogramData {
        columns: Arc::new(columns),
        total_columns,
        freq_resolution,
        time_resolution,
        max_freq,
        sample_rate: audio.sample_rate,
    }
}

/// Compute a partial spectrogram — only columns `col_start .. col_start + col_count`.
///
/// Identical FFT parameters to `compute_spectrogram`.  Used for chunked async
/// computation so the browser stays responsive between chunks.
pub fn compute_spectrogram_partial(
    audio: &AudioData,
    fft_size: usize,
    hop_size: usize,
    col_start: usize,
    col_count: usize,
) -> Vec<SpectrogramColumn> {
    if audio.samples.len() < fft_size || col_count == 0 {
        return vec![];
    }

    let fft = FFT_PLANNER.with(|p| p.borrow_mut().plan_fft_forward(fft_size));
    let window = hann_window(fft_size);
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();

    let total_cols = (audio.samples.len() - fft_size) / hop_size + 1;
    let col_end = (col_start + col_count).min(total_cols);

    let mut columns = Vec::with_capacity(col_end.saturating_sub(col_start));
    for col_i in col_start..col_end {
        let pos = col_i * hop_size;
        if pos + fft_size > audio.samples.len() {
            break;
        }
        for (inp, (&s, &w)) in input
            .iter_mut()
            .zip(audio.samples[pos..pos + fft_size].iter().zip(window.iter()))
        {
            *inp = s * w;
        }
        fft.process(&mut input, &mut spectrum).expect("FFT failed");
        let magnitudes: Vec<f32> = spectrum.iter().map(|c| c.norm()).collect();
        let time_offset = pos as f64 / audio.sample_rate as f64;
        columns.push(SpectrogramColumn { magnitudes, time_offset });
    }
    columns
}

/// Compute STFT columns directly from a sample slice.
///
/// Like `compute_spectrogram_partial` but works on a raw `&[f32]` slice instead
/// of `&AudioData`, avoiding the need to wrap live recording buffers in `Arc<Vec<f32>>`.
pub fn compute_stft_columns(
    samples: &[f32],
    sample_rate: u32,
    fft_size: usize,
    hop_size: usize,
    col_start: usize,
    col_count: usize,
) -> Vec<SpectrogramColumn> {
    if samples.len() < fft_size || col_count == 0 {
        return vec![];
    }

    let fft = FFT_PLANNER.with(|p| p.borrow_mut().plan_fft_forward(fft_size));
    let window = hann_window(fft_size);
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();

    let total_cols = (samples.len() - fft_size) / hop_size + 1;
    let col_end = (col_start + col_count).min(total_cols);

    let mut columns = Vec::with_capacity(col_end.saturating_sub(col_start));
    for col_i in col_start..col_end {
        let pos = col_i * hop_size;
        if pos + fft_size > samples.len() {
            break;
        }
        for (inp, (&s, &w)) in input
            .iter_mut()
            .zip(samples[pos..pos + fft_size].iter().zip(window.iter()))
        {
            *inp = s * w;
        }
        fft.process(&mut input, &mut spectrum).expect("FFT failed");
        let magnitudes: Vec<f32> = spectrum.iter().map(|c| c.norm()).collect();
        let time_offset = pos as f64 / sample_rate as f64;
        columns.push(SpectrogramColumn { magnitudes, time_offset });
    }
    columns
}

/// Compute a fast low-resolution preview spectrogram as an RGBA pixel buffer.
/// Uses FFT=256 with a dynamic hop to produce roughly `target_width` columns.
pub fn compute_preview(audio: &AudioData, target_width: u32, target_height: u32) -> PreviewImage {
    if audio.samples.len() < 256 {
        // Too short for even one FFT frame
        return PreviewImage {
            width: 1,
            height: 1,
            pixels: Arc::new(vec![0, 0, 0, 255]),
        };
    }

    let fft_size = 256;
    let hop = (audio.samples.len() / target_width as usize).max(fft_size);
    let spec = compute_spectrogram(audio, fft_size, hop);

    if spec.columns.is_empty() {
        return PreviewImage {
            width: 1,
            height: 1,
            pixels: Arc::new(vec![0, 0, 0, 255]),
        };
    }

    let src_w = spec.columns.len();
    let src_h = spec.columns[0].magnitudes.len();
    let out_w = (src_w as u32).min(target_width);
    let out_h = (src_h as u32).min(target_height);

    // Find global max magnitude for normalization
    let max_mag = spec
        .columns
        .iter()
        .flat_map(|c| c.magnitudes.iter())
        .copied()
        .fold(0.0f32, f32::max);

    let mut pixels = vec![0u8; (out_w * out_h * 4) as usize];

    for x in 0..out_w {
        let src_col = (x as usize * src_w) / out_w as usize;
        let col = &spec.columns[src_col.min(src_w - 1)];
        for y in 0..out_h {
            // Map output row to source bin (row 0 = highest freq)
            let src_bin = src_h - 1 - ((y as usize * src_h) / out_h as usize).min(src_h - 1);
            let mag = col.magnitudes[src_bin];
            let grey = magnitude_to_greyscale(mag, max_mag);
            let idx = (y * out_w + x) as usize * 4;
            pixels[idx] = grey;
            pixels[idx + 1] = grey;
            pixels[idx + 2] = grey;
            pixels[idx + 3] = 255;
        }
    }

    PreviewImage {
        width: out_w,
        height: out_h,
        pixels: Arc::new(pixels),
    }
}

/// Compute a higher-resolution overview image by downsampling existing SpectrogramData.
/// Produces a ~1024×256 greyscale RGBA image (same format as PreviewImage).
pub fn compute_overview_from_spectrogram(data: &SpectrogramData) -> Option<PreviewImage> {
    if data.columns.is_empty() {
        return None;
    }

    let src_w = data.columns.len();
    let src_h = data.columns[0].magnitudes.len();
    if src_h == 0 { return None; }

    let out_w = (src_w as u32).min(1024);
    let out_h = (src_h as u32).min(256);

    let max_mag = data.columns.iter()
        .flat_map(|c| c.magnitudes.iter())
        .copied()
        .fold(0.0f32, f32::max);
    if max_mag <= 0.0 { return None; }

    let mut pixels = vec![0u8; (out_w * out_h * 4) as usize];

    for x in 0..out_w {
        let src_col = (x as usize * src_w) / out_w as usize;
        let col = &data.columns[src_col.min(src_w - 1)];
        for y in 0..out_h {
            let src_bin = src_h - 1 - ((y as usize * src_h) / out_h as usize).min(src_h - 1);
            let mag = col.magnitudes[src_bin];
            let grey = magnitude_to_greyscale(mag, max_mag);
            let idx = (y * out_w + x) as usize * 4;
            pixels[idx] = grey;
            pixels[idx + 1] = grey;
            pixels[idx + 2] = grey;
            pixels[idx + 3] = 255;
        }
    }

    Some(PreviewImage {
        width: out_w,
        height: out_h,
        pixels: Arc::new(pixels),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AudioData, FileMetadata};

    fn test_audio(samples: Vec<f32>, sample_rate: u32) -> AudioData {
        AudioData {
            duration_secs: samples.len() as f64 / sample_rate as f64,
            samples: Arc::new(samples),
            sample_rate,
            channels: 1,
            metadata: FileMetadata {
                file_size: 0,
                format: "test",
                bits_per_sample: 32,
                is_float: true,
                guano: None,
            },
        }
    }

    #[test]
    fn test_spectrogram_basic() {
        let sample_rate = 44100u32;
        let freq = 1000.0f64;
        let num_samples = 4096;

        let samples: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * std::f64::consts::PI * freq * t).sin() as f32
            })
            .collect();

        let audio = test_audio(samples, sample_rate);

        let result = compute_spectrogram(&audio, 1024, 512);
        assert!(!result.columns.is_empty());
        assert_eq!(result.sample_rate, sample_rate);

        // The peak bin should be near 1000 Hz
        let col = &result.columns[1]; // skip first column (edge effects)
        let peak_bin = col
            .magnitudes
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        let peak_freq = peak_bin as f64 * result.freq_resolution;
        let error = (peak_freq - freq).abs();
        assert!(
            error < result.freq_resolution * 2.0,
            "Peak at {peak_freq} Hz, expected ~{freq} Hz"
        );
    }
}
