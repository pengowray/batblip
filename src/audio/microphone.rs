use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::AudioContext;
use crate::state::{AppState, MicState, LoadedFile};
use crate::types::{AudioData, FileMetadata, SpectrogramData, SpectrogramColumn};
use crate::dsp::fft::{compute_preview, compute_spectrogram_partial};
use std::cell::RefCell;

thread_local! {
    static MIC_CTX: RefCell<Option<AudioContext>> = RefCell::new(None);
    static MIC_STREAM: RefCell<Option<web_sys::MediaStream>> = RefCell::new(None);
    static MIC_PROCESSOR: RefCell<Option<web_sys::ScriptProcessorNode>> = RefCell::new(None);
    static MIC_BUFFER: RefCell<Vec<f32>> = RefCell::new(Vec::new());
    static MIC_HANDLER: RefCell<Option<Closure<dyn FnMut(web_sys::AudioProcessingEvent)>>> = RefCell::new(None);
}

/// Request microphone permission and start monitoring (passthrough to speakers).
pub async fn arm(state: &AppState) {
    let window = match web_sys::window() {
        Some(w) => w,
        None => {
            log::error!("No window object");
            return;
        }
    };
    let navigator = window.navigator();
    let media_devices = match navigator.media_devices() {
        Ok(md) => md,
        Err(e) => {
            log::error!("No media devices: {:?}", e);
            return;
        }
    };

    // Request audio-only stream
    let constraints = web_sys::MediaStreamConstraints::new();
    constraints.set_audio(&JsValue::TRUE);

    let promise = match media_devices.get_user_media_with_constraints(&constraints) {
        Ok(p) => p,
        Err(e) => {
            log::error!("getUserMedia failed: {:?}", e);
            return;
        }
    };

    let stream_js = match JsFuture::from(promise).await {
        Ok(s) => s,
        Err(e) => {
            log::error!("Mic permission denied: {:?}", e);
            return;
        }
    };

    let stream: web_sys::MediaStream = match stream_js.dyn_into() {
        Ok(s) => s,
        Err(_) => {
            log::error!("Failed to cast MediaStream");
            return;
        }
    };

    // Create AudioContext for mic (separate from playback)
    let ctx = match AudioContext::new() {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to create AudioContext: {:?}", e);
            return;
        }
    };

    let sample_rate = ctx.sample_rate() as u32;
    state.mic_sample_rate.set(sample_rate);

    let source = match ctx.create_media_stream_source(&stream) {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to create MediaStreamSource: {:?}", e);
            return;
        }
    };

    // ScriptProcessorNode: buffer 4096, 1 input channel, 1 output channel
    let processor = match ctx.create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(4096, 1, 1) {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to create ScriptProcessorNode: {:?}", e);
            return;
        }
    };

    // Wire up: source -> processor -> destination
    if let Err(e) = source.connect_with_audio_node(&processor) {
        log::error!("Failed to connect source -> processor: {:?}", e);
        return;
    }
    if let Err(e) = processor.connect_with_audio_node(&ctx.destination()) {
        log::error!("Failed to connect processor -> destination: {:?}", e);
        return;
    }

    // Set up onaudioprocess callback
    let state_cb = *state;
    let handler = Closure::<dyn FnMut(web_sys::AudioProcessingEvent)>::new(move |ev: web_sys::AudioProcessingEvent| {
        let input_buffer = match ev.input_buffer() {
            Ok(b) => b,
            Err(_) => return,
        };
        let output_buffer = match ev.output_buffer() {
            Ok(b) => b,
            Err(_) => return,
        };

        // Get input samples
        let input_data = match input_buffer.get_channel_data(0) {
            Ok(d) => d,
            Err(_) => return,
        };

        // Copy input to output for monitoring (passthrough)
        let _ = output_buffer.copy_to_channel(&input_data, 0);

        // If recording, accumulate samples
        if state_cb.mic_state.get_untracked() == MicState::Recording {
            MIC_BUFFER.with(|buf| {
                buf.borrow_mut().extend_from_slice(&input_data);
                state_cb.mic_samples_recorded.set(buf.borrow().len());
            });
        }
    });

    processor.set_onaudioprocess(Some(handler.as_ref().unchecked_ref()));

    // Store everything in thread-locals
    MIC_CTX.with(|c| *c.borrow_mut() = Some(ctx));
    MIC_STREAM.with(|s| *s.borrow_mut() = Some(stream));
    MIC_PROCESSOR.with(|p| *p.borrow_mut() = Some(processor));
    MIC_HANDLER.with(|h| *h.borrow_mut() = Some(handler));

    state.mic_state.set(MicState::Armed);
    log::info!("Mic armed at {} Hz", sample_rate);
}

/// Start recording (mic must be armed).
pub fn start_recording(state: &AppState) {
    MIC_BUFFER.with(|buf| buf.borrow_mut().clear());
    state.mic_samples_recorded.set(0);
    state.mic_state.set(MicState::Recording);
    log::info!("Recording started");
}

/// Stop recording, return accumulated samples. Mic stays armed.
pub fn stop_recording(state: &AppState) -> Option<(Vec<f32>, u32)> {
    state.mic_state.set(MicState::Armed);
    let sample_rate = state.mic_sample_rate.get_untracked();
    let samples = MIC_BUFFER.with(|buf| std::mem::take(&mut *buf.borrow_mut()));
    state.mic_samples_recorded.set(0);

    if samples.is_empty() || sample_rate == 0 {
        log::warn!("No samples recorded");
        return None;
    }

    log::info!("Recording stopped: {} samples ({:.2}s at {} Hz)",
        samples.len(), samples.len() as f64 / sample_rate as f64, sample_rate);
    Some((samples, sample_rate))
}

/// Disarm microphone completely: stop all tracks, close context.
pub fn disarm(state: &AppState) {
    // Stop media tracks
    MIC_STREAM.with(|s| {
        if let Some(stream) = s.borrow_mut().take() {
            let tracks = stream.get_tracks();
            for i in 0..tracks.length() {
                let track_js = tracks.get(i);
                if let Ok(track) = track_js.dyn_into::<web_sys::MediaStreamTrack>() {
                    track.stop();
                }
            }
        }
    });

    // Disconnect and drop processor
    MIC_PROCESSOR.with(|p| {
        if let Some(proc) = p.borrow_mut().take() {
            proc.set_onaudioprocess(None);
            let _ = proc.disconnect();
        }
    });

    // Drop handler closure
    MIC_HANDLER.with(|h| { h.borrow_mut().take(); });

    // Close audio context
    MIC_CTX.with(|c| {
        if let Some(ctx) = c.borrow_mut().take() {
            let _ = ctx.close();
        }
    });

    // Clear buffer
    MIC_BUFFER.with(|buf| buf.borrow_mut().clear());

    state.mic_state.set(MicState::Off);
    state.mic_sample_rate.set(0);
    state.mic_samples_recorded.set(0);
    log::info!("Mic disarmed");
}

/// Convert recorded samples into a LoadedFile and add to state, then compute spectrogram.
pub fn finalize_recording(samples: Vec<f32>, sample_rate: u32, state: AppState) {
    let duration_secs = samples.len() as f64 / sample_rate as f64;
    let now = js_sys::Date::new_0();
    let name = format!(
        "rec_{:04}-{:02}-{:02}_{:02}{:02}{:02}.wav",
        now.get_full_year(),
        now.get_month() + 1,
        now.get_date(),
        now.get_hours(),
        now.get_minutes(),
        now.get_seconds(),
    );

    let audio = AudioData {
        samples,
        sample_rate,
        channels: 1,
        duration_secs,
        metadata: FileMetadata {
            file_size: 0,
            format: "REC",
            bits_per_sample: 32,
            is_float: true,
            guano: None,
        },
    };

    // Phase 1: fast preview
    let preview = compute_preview(&audio, 256, 128);
    let audio_for_stft = audio.clone();
    let name_check = name.clone();

    let placeholder_spec = SpectrogramData {
        columns: Vec::new(),
        freq_resolution: 0.0,
        time_resolution: 0.0,
        max_freq: sample_rate as f64 / 2.0,
        sample_rate,
    };

    let file_index;
    {
        let mut idx = 0;
        state.files.update(|files| {
            idx = files.len();
            files.push(LoadedFile {
                name,
                audio,
                spectrogram: placeholder_spec,
                preview: Some(preview),
                xc_metadata: None,
            });
        });
        file_index = idx;
    }
    state.current_file_index.set(Some(file_index));

    // Phase 2: async chunked spectrogram computation
    wasm_bindgen_futures::spawn_local(async move {
        // Yield to let UI render the preview
        let yield_promise = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback(&resolve)
                .unwrap();
        });
        JsFuture::from(yield_promise).await.ok();

        const FFT_SIZE: usize = 2048;
        const HOP_SIZE: usize = 512;
        const CHUNK_COLS: usize = 32;

        let total_cols = if audio_for_stft.samples.len() >= FFT_SIZE {
            (audio_for_stft.samples.len() - FFT_SIZE) / HOP_SIZE + 1
        } else {
            0
        };

        let mut all_columns: Vec<SpectrogramColumn> = Vec::with_capacity(total_cols);
        let mut chunk_start = 0;

        while chunk_start < total_cols {
            let still_present = state.files.get_untracked()
                .get(file_index)
                .map(|f| f.name == name_check)
                .unwrap_or(false);
            if !still_present { return; }

            let chunk = compute_spectrogram_partial(
                &audio_for_stft,
                FFT_SIZE,
                HOP_SIZE,
                chunk_start,
                CHUNK_COLS,
            );
            all_columns.extend(chunk);
            chunk_start += CHUNK_COLS;

            let p = js_sys::Promise::new(&mut |resolve, _| {
                web_sys::window().unwrap().set_timeout_with_callback(&resolve).unwrap();
            });
            JsFuture::from(p).await.ok();
        }

        let freq_resolution = audio_for_stft.sample_rate as f64 / FFT_SIZE as f64;
        let time_resolution = HOP_SIZE as f64 / audio_for_stft.sample_rate as f64;
        let max_freq = audio_for_stft.sample_rate as f64 / 2.0;

        let spectrogram = SpectrogramData {
            columns: all_columns,
            freq_resolution,
            time_resolution,
            max_freq,
            sample_rate: audio_for_stft.sample_rate,
        };

        log::info!(
            "Recording spectrogram: {} columns, freq_res={:.1} Hz, time_res={:.4}s",
            spectrogram.columns.len(),
            spectrogram.freq_resolution,
            spectrogram.time_resolution
        );

        state.files.update(|files| {
            if let Some(f) = files.get_mut(file_index) {
                if f.name == name_check {
                    f.spectrogram = spectrogram;
                }
            }
        });

        // Trigger spectrogram redraw
        state.tile_ready_signal.update(|n| *n += 1);
    });
}
