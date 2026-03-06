//! Native audio file decoding for Tauri.
//!
//! Provides Tauri commands to read audio file metadata and decode files
//! to mono f32 samples natively on a background thread, avoiding the
//! need to pass entire file bytes through the WASM boundary.

use serde::Serialize;
use std::io::Cursor;
use std::path::Path;

#[derive(Serialize, Clone, Debug)]
pub struct AudioFileInfo {
    pub sample_rate: u32,
    pub channels: u32,
    pub duration_secs: f64,
    pub total_mono_samples: usize,
    pub bits_per_sample: u16,
    pub is_float: bool,
    pub format: String,
    pub file_size: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct FullDecodeResult {
    pub info: AudioFileInfo,
    pub samples: Vec<f32>,
}

/// Read audio file metadata without decoding samples.
pub fn file_info(path: &str) -> Result<AudioFileInfo, String> {
    let path = Path::new(path);
    let bytes = std::fs::read(path).map_err(|e| format!("Failed to read file: {e}"))?;
    let file_size = bytes.len();

    if bytes.len() < 4 {
        return Err("File too small".into());
    }

    match &bytes[0..4] {
        b"RIFF" => wav_info(&bytes, file_size),
        b"fLaC" => flac_info(&bytes, file_size),
        b"OggS" => ogg_info(&bytes, file_size),
        _ if is_mp3(&bytes) => mp3_info(&bytes, file_size),
        _ => Err("Unknown audio format (expected WAV, FLAC, OGG, or MP3)".into()),
    }
}

/// Decode entire audio file to mono f32 samples.
pub fn decode_full(path: &str) -> Result<FullDecodeResult, String> {
    let path = Path::new(path);
    let bytes = std::fs::read(path).map_err(|e| format!("Failed to read file: {e}"))?;
    let file_size = bytes.len();

    if bytes.len() < 4 {
        return Err("File too small".into());
    }

    match &bytes[0..4] {
        b"RIFF" => decode_wav(&bytes, file_size),
        b"fLaC" => decode_flac(&bytes, file_size),
        b"OggS" => decode_ogg(&bytes, file_size),
        _ if is_mp3(&bytes) => decode_mp3(&bytes, file_size),
        _ => Err("Unknown audio format".into()),
    }
}

fn is_mp3(bytes: &[u8]) -> bool {
    if bytes.len() >= 3 && &bytes[0..3] == b"ID3" {
        return true;
    }
    if bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0 {
        return true;
    }
    false
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

// ── WAV ─────────────────────────────────────────────────────────────

fn wav_info(bytes: &[u8], file_size: usize) -> Result<AudioFileInfo, String> {
    let cursor = Cursor::new(bytes);
    let reader = hound::WavReader::new(cursor).map_err(|e| format!("WAV error: {e}"))?;
    let spec = reader.spec();
    let total_samples = reader.len() as usize;
    let channels = spec.channels as u32;
    let mono_samples = total_samples / channels as usize;
    Ok(AudioFileInfo {
        sample_rate: spec.sample_rate,
        channels,
        duration_secs: mono_samples as f64 / spec.sample_rate as f64,
        total_mono_samples: mono_samples,
        bits_per_sample: spec.bits_per_sample,
        is_float: matches!(spec.sample_format, hound::SampleFormat::Float),
        format: "WAV".into(),
        file_size,
    })
}

fn decode_wav(bytes: &[u8], file_size: usize) -> Result<FullDecodeResult, String> {
    let cursor = Cursor::new(bytes);
    let reader = hound::WavReader::new(cursor).map_err(|e| format!("WAV error: {e}"))?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let channels = spec.channels as u32;
    let bits_per_sample = spec.bits_per_sample;
    let is_float = matches!(spec.sample_format, hound::SampleFormat::Float);

    let all_samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("WAV sample error: {e}"))?,
        hound::SampleFormat::Int => {
            let max_val = (1u32 << (bits_per_sample - 1)) as f32;
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

    Ok(FullDecodeResult {
        info: AudioFileInfo {
            sample_rate,
            channels,
            duration_secs,
            total_mono_samples: samples.len(),
            bits_per_sample,
            is_float,
            format: "WAV".into(),
            file_size,
        },
        samples,
    })
}

// ── FLAC ────────────────────────────────────────────────────────────

fn flac_info(bytes: &[u8], file_size: usize) -> Result<AudioFileInfo, String> {
    let cursor = Cursor::new(bytes);
    let reader = claxon::FlacReader::new(cursor).map_err(|e| format!("FLAC error: {e}"))?;
    let info = reader.streaminfo();
    // info.samples is Option<u64> — total inter-channel frames. Use u64 arithmetic
    // to avoid overflow on 32-bit targets for files with > 2^32 frames.
    let total_frames = info.samples.unwrap_or(0);
    let mono_samples = total_frames as usize; // safe on 64-bit Tauri targets
    Ok(AudioFileInfo {
        sample_rate: info.sample_rate,
        channels: info.channels,
        duration_secs: total_frames as f64 / info.sample_rate as f64,
        total_mono_samples: mono_samples,
        bits_per_sample: info.bits_per_sample as u16,
        is_float: false,
        format: "FLAC".into(),
        file_size,
    })
}

fn decode_flac(bytes: &[u8], file_size: usize) -> Result<FullDecodeResult, String> {
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

    Ok(FullDecodeResult {
        info: AudioFileInfo {
            sample_rate,
            channels,
            duration_secs,
            total_mono_samples: samples.len(),
            bits_per_sample: bits as u16,
            is_float: false,
            format: "FLAC".into(),
            file_size,
        },
        samples,
    })
}

// ── OGG ─────────────────────────────────────────────────────────────

fn ogg_info(bytes: &[u8], file_size: usize) -> Result<AudioFileInfo, String> {
    // OGG requires full decode to know exact sample count
    let result = decode_ogg(bytes, file_size)?;
    Ok(result.info)
}

fn decode_ogg(bytes: &[u8], file_size: usize) -> Result<FullDecodeResult, String> {
    use lewton::inside_ogg::OggStreamReader;

    let cursor = Cursor::new(bytes);
    let mut reader = OggStreamReader::new(cursor).map_err(|e| format!("OGG error: {e}"))?;
    let sample_rate = reader.ident_hdr.audio_sample_rate;
    let channels = reader.ident_hdr.audio_channels as u32;

    let mut all_samples: Vec<f32> = Vec::new();
    loop {
        match reader.read_dec_packet_itl() {
            Ok(Some(packet)) => {
                all_samples.extend(packet.iter().map(|&s| s as f32 / 32768.0));
            }
            Ok(None) => break,
            Err(e) => return Err(format!("OGG decode error: {e}")),
        }
    }

    let samples = mix_to_mono(&all_samples, channels);
    let duration_secs = samples.len() as f64 / sample_rate as f64;

    Ok(FullDecodeResult {
        info: AudioFileInfo {
            sample_rate,
            channels,
            duration_secs,
            total_mono_samples: samples.len(),
            bits_per_sample: 16,
            is_float: false,
            format: "OGG".into(),
            file_size,
        },
        samples,
    })
}

// ── MP3 ─────────────────────────────────────────────────────────────

fn mp3_info(bytes: &[u8], file_size: usize) -> Result<AudioFileInfo, String> {
    // MP3 requires full decode to know exact sample count
    let result = decode_mp3(bytes, file_size)?;
    Ok(result.info)
}

fn decode_mp3(bytes: &[u8], file_size: usize) -> Result<FullDecodeResult, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error as SymphoniaError;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let cursor = Cursor::new(bytes.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let hint = Hint::new();
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("MP3 probe error: {e}"))?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("No audio track found in MP3")?;

    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or("MP3 missing sample rate")?;
    let channels = track
        .codec_params
        .channels
        .ok_or("MP3 missing channel info")?
        .count() as u32;
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("MP3 decoder error: {e}"))?;

    let mut all_samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(SymphoniaError::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(format!("MP3 packet error: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                buf.copy_interleaved_ref(decoded);
                all_samples.extend_from_slice(buf.samples());
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(format!("MP3 decode error: {e}")),
        }
    }

    let samples = mix_to_mono(&all_samples, channels);
    let duration_secs = samples.len() as f64 / sample_rate as f64;

    Ok(FullDecodeResult {
        info: AudioFileInfo {
            sample_rate,
            channels,
            duration_secs,
            total_mono_samples: samples.len(),
            bits_per_sample: 16,
            is_float: false,
            format: "MP3".into(),
            file_size,
        },
        samples,
    })
}
