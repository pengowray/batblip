use crate::audio::guano::parse_guano;
use crate::types::{AudioData, FileMetadata};
use std::io::Cursor;

/// Load audio from raw file bytes. Detects WAV, FLAC, OGG, or MP3 by header magic bytes.
pub fn load_audio(bytes: &[u8]) -> Result<AudioData, String> {
    if bytes.len() < 4 {
        return Err("File too small".into());
    }

    match &bytes[0..4] {
        b"RIFF" => load_wav(bytes),
        b"fLaC" => load_flac(bytes),
        b"OggS" => load_ogg(bytes),
        _ if is_mp3(bytes) => load_mp3(bytes),
        _ => Err("Unknown file format (expected WAV, FLAC, OGG, or MP3)".into()),
    }
}

fn is_mp3(bytes: &[u8]) -> bool {
    // ID3v2 tag header
    if bytes.len() >= 3 && &bytes[0..3] == b"ID3" {
        return true;
    }
    // MPEG sync word: 0xFF followed by 0xE0–0xFF
    if bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0 {
        return true;
    }
    false
}

fn load_wav(bytes: &[u8]) -> Result<AudioData, String> {
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

    let guano = parse_guano(bytes);

    let samples = mix_to_mono(&all_samples, channels);
    let duration_secs = samples.len() as f64 / sample_rate as f64;

    Ok(AudioData {
        samples,
        sample_rate,
        channels,
        duration_secs,
        metadata: FileMetadata {
            file_size: bytes.len(),
            format: "WAV",
            bits_per_sample,
            is_float,
            guano,
        },
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
        metadata: FileMetadata {
            file_size: bytes.len(),
            format: "FLAC",
            bits_per_sample: bits as u16,
            is_float: false,
            guano: None,
        },
    })
}

fn load_ogg(bytes: &[u8]) -> Result<AudioData, String> {
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

    Ok(AudioData {
        samples,
        sample_rate,
        channels,
        duration_secs,
        metadata: FileMetadata {
            file_size: bytes.len(),
            format: "OGG",
            bits_per_sample: 16,
            is_float: false,
            guano: None,
        },
    })
}

fn load_mp3(bytes: &[u8]) -> Result<AudioData, String> {
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
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
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

    Ok(AudioData {
        samples,
        sample_rate,
        channels,
        duration_secs,
        metadata: FileMetadata {
            file_size: bytes.len(),
            format: "MP3",
            bits_per_sample: 16,
            is_float: false,
            guano: None,
        },
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
