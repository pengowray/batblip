use crate::types::AudioData;
use std::io::Cursor;

/// Load audio from raw file bytes. Detects WAV or FLAC by header magic bytes.
pub fn load_audio(bytes: &[u8]) -> Result<AudioData, String> {
    if bytes.len() < 4 {
        return Err("File too small".into());
    }

    match &bytes[0..4] {
        b"RIFF" => load_wav(bytes),
        b"fLaC" => load_flac(bytes),
        _ => Err("Unknown file format (expected WAV or FLAC)".into()),
    }
}

fn load_wav(bytes: &[u8]) -> Result<AudioData, String> {
    let cursor = Cursor::new(bytes);
    let reader = hound::WavReader::new(cursor).map_err(|e| format!("WAV error: {e}"))?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let channels = spec.channels as u32;

    let all_samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("WAV sample error: {e}"))?,
        hound::SampleFormat::Int => {
            let bits = spec.bits_per_sample;
            let max_val = (1u32 << (bits - 1)) as f32;
            reader
                .into_samples::<i32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("WAV sample error: {e}"))?
                .into_iter()
                .map(|s| s as f32 / max_val)
                .collect()
        }
    };

    let samples = mix_to_mono(&all_samples, channels);
    let duration_secs = samples.len() as f64 / sample_rate as f64;

    Ok(AudioData {
        samples,
        sample_rate,
        channels,
        duration_secs,
    })
}

fn load_flac(bytes: &[u8]) -> Result<AudioData, String> {
    let cursor = Cursor::new(bytes);
    let mut reader = claxon::FlacReader::new(cursor).map_err(|e| format!("FLAC error: {e}"))?;
    let info = reader.streaminfo();
    let sample_rate = info.sample_rate;
    let channels = info.channels;
    let bits = info.bits_per_sample;
    let max_val = (1u32 << (bits - 1)) as f32;

    let all_samples: Vec<f32> = reader
        .samples()
        .map(|s| s.map(|v| v as f32 / max_val))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("FLAC sample error: {e}"))?;

    let samples = mix_to_mono(&all_samples, channels);
    let duration_secs = samples.len() as f64 / sample_rate as f64;

    Ok(AudioData {
        samples,
        sample_rate,
        channels,
        duration_secs,
    })
}

fn mix_to_mono(samples: &[f32], channels: u32) -> Vec<f32> {
    if channels == 1 {
        return samples.to_vec();
    }
    let ch = channels as usize;
    samples
        .chunks_exact(ch)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}
