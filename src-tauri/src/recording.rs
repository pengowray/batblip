use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Serialize)]
#[allow(dead_code)]
pub enum NativeSampleFormat {
    I16,
    I24,
    I32,
    F32,
}

impl NativeSampleFormat {
    pub fn bits_per_sample(self) -> u16 {
        match self {
            Self::I16 => 16,
            Self::I24 => 24,
            Self::I32 => 32,
            Self::F32 => 32,
        }
    }

    pub fn is_float(self) -> bool {
        matches!(self, Self::F32)
    }
}

/// Thread-safe sample storage that keeps raw samples in their native format.
pub struct RecordingBuffer {
    pub format: NativeSampleFormat,
    pub sample_rate: u32,
    // Native-format storage (only one is active, based on format)
    pub samples_i16: Vec<i16>,
    pub samples_i32: Vec<i32>, // for I24 and I32
    pub samples_f32: Vec<f32>, // for F32 format (native)
    // f32 copies for streaming to frontend
    pub pending_f32: Vec<f32>,
    pub total_samples: usize,
}

impl RecordingBuffer {
    pub fn new(format: NativeSampleFormat, sample_rate: u32) -> Self {
        Self {
            format,
            sample_rate,
            samples_i16: Vec::new(),
            samples_i32: Vec::new(),
            samples_f32: Vec::new(),
            pending_f32: Vec::new(),
            total_samples: 0,
        }
    }

    pub fn clear(&mut self) {
        self.samples_i16.clear();
        self.samples_i32.clear();
        self.samples_f32.clear();
        self.pending_f32.clear();
        self.total_samples = 0;
    }

    /// Drain pending f32 samples for streaming to frontend.
    pub fn drain_pending(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.pending_f32)
    }
}

/// Wrapper to allow cpal::Stream in Tauri managed state.
/// Safe because we only store/drop the stream; we never access its internals
/// from multiple threads simultaneously.
pub(crate) struct SendStream(#[allow(dead_code)] cpal::Stream);
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}

/// Holds the active cpal stream and shared state.
pub struct MicState {
    #[allow(dead_code)]
    pub stream: SendStream,
    pub buffer: Arc<Mutex<RecordingBuffer>>,
    pub is_recording: Arc<AtomicBool>,
    pub is_streaming: Arc<AtomicBool>,
    pub emitter_stop: Arc<AtomicBool>,
    pub format: NativeSampleFormat,
    pub sample_rate: u32,
    pub device_name: String,
}

#[derive(Serialize)]
pub struct MicInfo {
    pub device_name: String,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub is_float: bool,
    pub format: String,
}

#[derive(Serialize)]
pub struct RecordingResult {
    pub filename: String,
    pub saved_path: String,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub is_float: bool,
    pub duration_secs: f64,
    pub num_samples: usize,
    pub samples_f32: Vec<f32>,
}

#[derive(Serialize)]
pub struct MicStatus {
    pub is_open: bool,
    pub is_recording: bool,
    pub is_streaming: bool,
    pub samples_recorded: usize,
    pub sample_rate: u32,
}

fn detect_format(config: &cpal::SupportedStreamConfig) -> NativeSampleFormat {
    match config.sample_format() {
        cpal::SampleFormat::I16 => NativeSampleFormat::I16,
        cpal::SampleFormat::I32 => {
            // cpal reports I32 for both 24-bit and 32-bit devices.
            // Check if the config's bits per sample hint suggests 24-bit.
            // Unfortunately cpal doesn't expose this directly, so we default to I32.
            // Users with 24-bit devices will still get lossless capture since
            // 24-bit samples fit in i32.
            NativeSampleFormat::I32
        }
        cpal::SampleFormat::F32 => NativeSampleFormat::F32,
        _ => NativeSampleFormat::F32, // fallback for other formats
    }
}

/// Open the default input device and create a capture stream.
pub fn open_mic() -> Result<MicState, String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No microphone found. Check your audio settings.".to_string())?;

    let device_name = device.name().unwrap_or_else(|_| "Unknown".into());
    let config = device
        .default_input_config()
        .map_err(|e| format!("Failed to get mic config: {}", e))?;

    let format = detect_format(&config);
    let sample_rate = config.sample_rate().0;
    let stream_config: cpal::StreamConfig = config.into();

    let buffer = Arc::new(Mutex::new(RecordingBuffer::new(format, sample_rate)));
    let is_recording = Arc::new(AtomicBool::new(false));
    let is_streaming = Arc::new(AtomicBool::new(false));
    let emitter_stop = Arc::new(AtomicBool::new(false));

    let buf = buffer.clone();
    let rec = is_recording.clone();
    let strm = is_streaming.clone();

    let err_callback = |err: cpal::StreamError| {
        eprintln!("Audio stream error: {}", err);
    };

    let stream = match format {
        NativeSampleFormat::I16 => {
            device.build_input_stream(
                &stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let mut buf = buf.lock().unwrap();
                    if rec.load(Ordering::Relaxed) {
                        buf.samples_i16.extend_from_slice(data);
                        buf.total_samples += data.len();
                    }
                    if strm.load(Ordering::Relaxed) || rec.load(Ordering::Relaxed) {
                        let f32_data: Vec<f32> =
                            data.iter().map(|&s| s as f32 / 32768.0).collect();
                        buf.pending_f32.extend_from_slice(&f32_data);
                    }
                },
                err_callback,
                None,
            )
        }
        NativeSampleFormat::I24 | NativeSampleFormat::I32 => {
            device.build_input_stream(
                &stream_config,
                move |data: &[i32], _: &cpal::InputCallbackInfo| {
                    let mut buf = buf.lock().unwrap();
                    if rec.load(Ordering::Relaxed) {
                        buf.samples_i32.extend_from_slice(data);
                        buf.total_samples += data.len();
                    }
                    if strm.load(Ordering::Relaxed) || rec.load(Ordering::Relaxed) {
                        let f32_data: Vec<f32> =
                            data.iter().map(|&s| s as f32 / 2147483648.0).collect();
                        buf.pending_f32.extend_from_slice(&f32_data);
                    }
                },
                err_callback,
                None,
            )
        }
        NativeSampleFormat::F32 => {
            device.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut buf = buf.lock().unwrap();
                    if rec.load(Ordering::Relaxed) {
                        buf.samples_f32.extend_from_slice(data);
                        buf.total_samples += data.len();
                    }
                    if strm.load(Ordering::Relaxed) || rec.load(Ordering::Relaxed) {
                        buf.pending_f32.extend_from_slice(data);
                    }
                },
                err_callback,
                None,
            )
        }
    }
    .map_err(|e| format!("Failed to open microphone: {}", e))?;

    stream
        .play()
        .map_err(|e| format!("Failed to start mic stream: {}", e))?;

    Ok(MicState {
        stream: SendStream(stream),
        buffer,
        is_recording,
        is_streaming,
        emitter_stop,
        format,
        sample_rate,
        device_name,
    })
}

/// Encode the recording buffer to WAV at native bit depth.
pub fn encode_native_wav(buffer: &RecordingBuffer) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: buffer.sample_rate,
        bits_per_sample: buffer.format.bits_per_sample(),
        sample_format: if buffer.format.is_float() {
            hound::SampleFormat::Float
        } else {
            hound::SampleFormat::Int
        },
    };

    let mut cursor = Cursor::new(Vec::new());
    let mut writer =
        hound::WavWriter::new(&mut cursor, spec).map_err(|e| format!("WAV writer error: {}", e))?;

    match buffer.format {
        NativeSampleFormat::I16 => {
            for &s in &buffer.samples_i16 {
                writer
                    .write_sample(s)
                    .map_err(|e| format!("WAV write error: {}", e))?;
            }
        }
        NativeSampleFormat::I24 => {
            for &s in &buffer.samples_i32 {
                // Mask to 24-bit range
                let s24 = (s >> 8) as i32;
                writer
                    .write_sample(s24)
                    .map_err(|e| format!("WAV write error: {}", e))?;
            }
        }
        NativeSampleFormat::I32 => {
            for &s in &buffer.samples_i32 {
                writer
                    .write_sample(s)
                    .map_err(|e| format!("WAV write error: {}", e))?;
            }
        }
        NativeSampleFormat::F32 => {
            for &s in &buffer.samples_f32 {
                writer
                    .write_sample(s)
                    .map_err(|e| format!("WAV write error: {}", e))?;
            }
        }
    }

    writer
        .finalize()
        .map_err(|e| format!("WAV finalize error: {}", e))?;
    Ok(cursor.into_inner())
}

/// Get f32 version of all recorded samples (for frontend spectrogram/display).
pub fn get_samples_f32(buffer: &RecordingBuffer) -> Vec<f32> {
    match buffer.format {
        NativeSampleFormat::I16 => buffer
            .samples_i16
            .iter()
            .map(|&s| s as f32 / 32768.0)
            .collect(),
        NativeSampleFormat::I24 | NativeSampleFormat::I32 => buffer
            .samples_i32
            .iter()
            .map(|&s| s as f32 / 2147483648.0)
            .collect(),
        NativeSampleFormat::F32 => buffer.samples_f32.clone(),
    }
}

/// Start the background emitter thread that sends audio chunks to the frontend.
pub fn start_emitter(
    app: tauri::AppHandle,
    buffer: Arc<Mutex<RecordingBuffer>>,
    stop_flag: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        use tauri::Emitter;
        while !stop_flag.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(80));
            let chunks = {
                let mut buf = buffer.lock().unwrap();
                buf.drain_pending()
            };
            if !chunks.is_empty() {
                let _ = app.emit("mic-audio-chunk", &chunks);
            }
        }
    });
}
