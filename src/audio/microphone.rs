use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::AudioContext;
use crate::state::{AppState, LoadedFile};
use crate::types::{AudioData, FileMetadata, SpectrogramData, SpectrogramColumn};
use crate::dsp::fft::{compute_preview, compute_spectrogram_partial};
use crate::dsp::heterodyne::RealtimeHet;
use std::cell::RefCell;

// ── Thread-local state: Web Audio mode ──────────────────────────────────

thread_local! {
    static MIC_CTX: RefCell<Option<AudioContext>> = RefCell::new(None);
    static MIC_STREAM: RefCell<Option<web_sys::MediaStream>> = RefCell::new(None);
    static MIC_PROCESSOR: RefCell<Option<web_sys::ScriptProcessorNode>> = RefCell::new(None);
    static MIC_BUFFER: RefCell<Vec<f32>> = RefCell::new(Vec::new());
    static MIC_HANDLER: RefCell<Option<Closure<dyn FnMut(web_sys::AudioProcessingEvent)>>> = RefCell::new(None);
    static RT_HET: RefCell<RealtimeHet> = RefCell::new(RealtimeHet::new());
}

// ── Thread-local state: Tauri native mode ───────────────────────────────

thread_local! {
    /// Whether the Tauri native mic is currently open
    static TAURI_MIC_OPEN: RefCell<bool> = RefCell::new(false);
    /// AudioContext for HET playback (output only, no mic input)
    static HET_CTX: RefCell<Option<AudioContext>> = RefCell::new(None);
    /// Next scheduled playback time for HET audio buffers
    static HET_NEXT_TIME: RefCell<f64> = RefCell::new(0.0);
    /// Keep the event listener closure alive
    static TAURI_EVENT_CLOSURE: RefCell<Option<Closure<dyn FnMut(JsValue)>>> = RefCell::new(None);
    /// Unlisten function returned by Tauri event subscription
    static TAURI_UNLISTEN: RefCell<Option<js_sys::Function>> = RefCell::new(None);
}

// ── Tauri IPC helpers ───────────────────────────────────────────────────

/// Get the Tauri internals object, if running in Tauri.
fn get_tauri_internals() -> Option<JsValue> {
    let window = web_sys::window()?;
    let tauri = js_sys::Reflect::get(&window, &JsValue::from_str("__TAURI_INTERNALS__")).ok()?;
    if tauri.is_undefined() { None } else { Some(tauri) }
}

/// Invoke a Tauri command and return the result as a JsValue.
async fn tauri_invoke(cmd: &str, args: &JsValue) -> Result<JsValue, String> {
    let tauri = get_tauri_internals().ok_or("Not running in Tauri")?;
    let invoke = js_sys::Reflect::get(&tauri, &JsValue::from_str("invoke"))
        .map_err(|_| "No invoke function")?;
    let invoke_fn = js_sys::Function::from(invoke);

    let promise_val = invoke_fn
        .call2(&tauri, &JsValue::from_str(cmd), args)
        .map_err(|e| format!("Invoke call failed: {:?}", e))?;

    let promise: js_sys::Promise = promise_val
        .dyn_into()
        .map_err(|_| "Result is not a Promise")?;

    JsFuture::from(promise)
        .await
        .map_err(|e| format!("Command '{}' failed: {:?}", cmd, e))
}

/// Invoke a Tauri command with no arguments.
async fn tauri_invoke_no_args(cmd: &str) -> Result<JsValue, String> {
    tauri_invoke(cmd, &js_sys::Object::new().into()).await
}

/// Subscribe to a Tauri event. Returns an unlisten function.
fn tauri_listen(event_name: &str, callback: Closure<dyn FnMut(JsValue)>) -> Option<()> {
    let tauri = get_tauri_internals()?;

    let transform_fn = js_sys::Reflect::get(&tauri, &JsValue::from_str("transformCallback")).ok()?;
    let transform_fn = js_sys::Function::from(transform_fn);
    let handler_id = transform_fn.call1(&tauri, callback.as_ref().unchecked_ref()).ok()?;

    let invoke_fn = js_sys::Reflect::get(&tauri, &JsValue::from_str("invoke")).ok()?;
    let invoke_fn = js_sys::Function::from(invoke_fn);

    let args = js_sys::Object::new();
    js_sys::Reflect::set(&args, &"event".into(), &JsValue::from_str(event_name)).ok();
    let target = js_sys::Object::new();
    js_sys::Reflect::set(&target, &"kind".into(), &JsValue::from_str("Any")).ok();
    js_sys::Reflect::set(&args, &"target".into(), &target).ok();
    js_sys::Reflect::set(&args, &"handler".into(), &handler_id).ok();

    invoke_fn
        .call2(&tauri, &JsValue::from_str("plugin:event|listen"), &args)
        .ok();

    // Store the closure so it's not dropped
    TAURI_EVENT_CLOSURE.with(|c| *c.borrow_mut() = Some(callback));

    Some(())
}

// ── Web Audio mode (existing implementation) ────────────────────────────

fn web_mic_is_open() -> bool {
    MIC_CTX.with(|c| c.borrow().is_some())
}

async fn ensure_mic_open_web(state: &AppState) -> bool {
    if web_mic_is_open() {
        return true;
    }

    let window = match web_sys::window() {
        Some(w) => w,
        None => {
            log::error!("No window object");
            return false;
        }
    };
    let navigator = window.navigator();
    let media_devices = match navigator.media_devices() {
        Ok(md) => md,
        Err(e) => {
            log::error!("No media devices: {:?}", e);
            state.status_message.set(Some("Microphone not available on this device".into()));
            return false;
        }
    };

    let constraints = web_sys::MediaStreamConstraints::new();
    constraints.set_audio(&JsValue::TRUE);

    let promise = match media_devices.get_user_media_with_constraints(&constraints) {
        Ok(p) => p,
        Err(e) => {
            log::error!("getUserMedia failed: {:?}", e);
            state.status_message.set(Some("Microphone not available".into()));
            return false;
        }
    };

    let stream_js = match JsFuture::from(promise).await {
        Ok(s) => s,
        Err(e) => {
            log::error!("Mic permission denied: {:?}", e);
            state.status_message.set(Some("Microphone permission denied".into()));
            return false;
        }
    };

    let stream: web_sys::MediaStream = match stream_js.dyn_into() {
        Ok(s) => s,
        Err(_) => {
            log::error!("Failed to cast MediaStream");
            return false;
        }
    };

    let ctx = match AudioContext::new() {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to create AudioContext: {:?}", e);
            state.status_message.set(Some("Failed to initialize audio".into()));
            return false;
        }
    };

    let sample_rate = ctx.sample_rate() as u32;
    state.mic_sample_rate.set(sample_rate);

    let source = match ctx.create_media_stream_source(&stream) {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to create MediaStreamSource: {:?}", e);
            return false;
        }
    };

    let processor = match ctx.create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(4096, 1, 1) {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to create ScriptProcessorNode: {:?}", e);
            return false;
        }
    };

    if let Err(e) = source.connect_with_audio_node(&processor) {
        log::error!("Failed to connect source -> processor: {:?}", e);
        return false;
    }
    if let Err(e) = processor.connect_with_audio_node(&ctx.destination()) {
        log::error!("Failed to connect processor -> destination: {:?}", e);
        return false;
    }

    RT_HET.with(|h| h.borrow_mut().reset());

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

        let input_data = match input_buffer.get_channel_data(0) {
            Ok(d) => d,
            Err(_) => return,
        };

        if state_cb.mic_listening.get_untracked() {
            let sr = state_cb.mic_sample_rate.get_untracked();
            let het_freq = state_cb.het_frequency.get_untracked();
            let het_cutoff = state_cb.het_cutoff.get_untracked();
            let mut out_data = vec![0.0f32; input_data.len()];
            RT_HET.with(|h| {
                h.borrow_mut().process(&input_data, &mut out_data, sr, het_freq, het_cutoff);
            });
            let _ = output_buffer.copy_to_channel(&out_data, 0);
        } else {
            let zeros = vec![0.0f32; input_data.len()];
            let _ = output_buffer.copy_to_channel(&zeros, 0);
        }

        if state_cb.mic_recording.get_untracked() {
            MIC_BUFFER.with(|buf| {
                buf.borrow_mut().extend_from_slice(&input_data);
                state_cb.mic_samples_recorded.set(buf.borrow().len());
            });
        }
    });

    processor.set_onaudioprocess(Some(handler.as_ref().unchecked_ref()));

    MIC_CTX.with(|c| *c.borrow_mut() = Some(ctx));
    MIC_STREAM.with(|s| *s.borrow_mut() = Some(stream));
    MIC_PROCESSOR.with(|p| *p.borrow_mut() = Some(processor));
    MIC_HANDLER.with(|h| *h.borrow_mut() = Some(handler));

    log::info!("Web mic opened at {} Hz", sample_rate);
    true
}

fn close_mic_web(state: &AppState) {
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

    MIC_PROCESSOR.with(|p| {
        if let Some(proc) = p.borrow_mut().take() {
            proc.set_onaudioprocess(None);
            let _ = proc.disconnect();
        }
    });

    MIC_HANDLER.with(|h| { h.borrow_mut().take(); });

    MIC_CTX.with(|c| {
        if let Some(ctx) = c.borrow_mut().take() {
            let _ = ctx.close();
        }
    });

    MIC_BUFFER.with(|buf| buf.borrow_mut().clear());
    RT_HET.with(|h| h.borrow_mut().reset());

    state.mic_sample_rate.set(0);
    state.mic_samples_recorded.set(0);
    state.mic_bits_per_sample.set(16);
    log::info!("Web mic closed");
}

fn maybe_close_mic_web(state: &AppState) {
    if !state.mic_listening.get_untracked() && !state.mic_recording.get_untracked() {
        close_mic_web(state);
    }
}

fn stop_recording_web(state: &AppState) -> Option<(Vec<f32>, u32)> {
    state.mic_recording.set(false);
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

// ── Tauri native mode ───────────────────────────────────────────────────

fn tauri_mic_is_open() -> bool {
    TAURI_MIC_OPEN.with(|o| *o.borrow())
}

/// Open the mic via cpal in the Tauri backend.
async fn ensure_mic_open_tauri(state: &AppState) -> bool {
    if tauri_mic_is_open() {
        return true;
    }

    let result = match tauri_invoke_no_args("mic_open").await {
        Ok(v) => v,
        Err(e) => {
            log::warn!("Native mic failed ({}), falling back to Web Audio", e);
            state.status_message.set(Some(format!("Native mic unavailable: {}", e)));
            return ensure_mic_open_web(state).await;
        }
    };

    // Parse MicInfo from the response
    let sample_rate = js_sys::Reflect::get(&result, &JsValue::from_str("sample_rate"))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(48000.0) as u32;
    let bits_per_sample = js_sys::Reflect::get(&result, &JsValue::from_str("bits_per_sample"))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(16.0) as u16;
    let device_name = js_sys::Reflect::get(&result, &JsValue::from_str("device_name"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| "Unknown".into());

    state.mic_sample_rate.set(sample_rate);
    state.mic_bits_per_sample.set(bits_per_sample);

    // Setup HET playback AudioContext (output only)
    let het_ctx = match AudioContext::new() {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to create HET AudioContext: {:?}", e);
            state.status_message.set(Some("Failed to initialize audio output".into()));
            return false;
        }
    };
    HET_CTX.with(|c| *c.borrow_mut() = Some(het_ctx));
    HET_NEXT_TIME.with(|t| *t.borrow_mut() = 0.0);
    RT_HET.with(|h| h.borrow_mut().reset());

    // Setup event listener for audio chunks from the backend
    let state_cb = *state;
    let chunk_handler = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
        let payload = match js_sys::Reflect::get(&event, &JsValue::from_str("payload")) {
            Ok(p) => p,
            Err(_) => return,
        };

        let array = js_sys::Array::from(&payload);
        let len = array.length() as usize;
        if len == 0 {
            return;
        }

        let input_data: Vec<f32> = (0..len)
            .map(|i| array.get(i as u32).as_f64().unwrap_or(0.0) as f32)
            .collect();

        // Update sample count for recording UI
        if state_cb.mic_recording.get_untracked() {
            state_cb.mic_samples_recorded.update(|n| *n += len);
        }

        // HET listening: process and play through speakers
        if state_cb.mic_listening.get_untracked() {
            let sr = state_cb.mic_sample_rate.get_untracked();
            let het_freq = state_cb.het_frequency.get_untracked();
            let het_cutoff = state_cb.het_cutoff.get_untracked();
            let mut out_data = vec![0.0f32; len];
            RT_HET.with(|h| {
                h.borrow_mut().process(&input_data, &mut out_data, sr, het_freq, het_cutoff);
            });

            // Schedule playback via AudioBuffer
            HET_CTX.with(|ctx_cell| {
                let ctx_ref = ctx_cell.borrow();
                let Some(ctx) = ctx_ref.as_ref() else { return };
                let Ok(buffer) = ctx.create_buffer(1, len as u32, sr as f32) else { return };
                let _ = buffer.copy_to_channel(&out_data, 0);
                let Ok(source) = ctx.create_buffer_source() else { return };
                source.set_buffer(Some(&buffer));
                let _ = source.connect_with_audio_node(&ctx.destination());

                let current_time = ctx.current_time();
                let next_time = HET_NEXT_TIME.with(|t| *t.borrow());
                let start = if next_time > current_time { next_time } else { current_time };
                let _ = source.start_with_when(start);

                let duration = len as f64 / sr as f64;
                HET_NEXT_TIME.with(|t| *t.borrow_mut() = start + duration);
            });
        }
    });

    tauri_listen("mic-audio-chunk", chunk_handler);

    TAURI_MIC_OPEN.with(|o| *o.borrow_mut() = true);
    log::info!("Native mic opened: {} at {} Hz, {}-bit", device_name, sample_rate, bits_per_sample);
    true
}

async fn close_mic_tauri(state: &AppState) {
    // Tell backend to stop streaming and close mic
    if let Err(e) = tauri_invoke_no_args("mic_close").await {
        log::error!("mic_close failed: {}", e);
    }

    // Clean up event listener
    TAURI_EVENT_CLOSURE.with(|c| { c.borrow_mut().take(); });
    TAURI_UNLISTEN.with(|u| { u.borrow_mut().take(); });

    // Close HET playback context
    HET_CTX.with(|c| {
        if let Some(ctx) = c.borrow_mut().take() {
            let _ = ctx.close();
        }
    });

    RT_HET.with(|h| h.borrow_mut().reset());
    TAURI_MIC_OPEN.with(|o| *o.borrow_mut() = false);

    state.mic_sample_rate.set(0);
    state.mic_samples_recorded.set(0);
    state.mic_bits_per_sample.set(16);
    log::info!("Native mic closed");
}

async fn maybe_close_mic_tauri(state: &AppState) {
    if !state.mic_listening.get_untracked() && !state.mic_recording.get_untracked() {
        close_mic_tauri(state).await;
    }
}

/// Toggle listening in Tauri mode.
async fn toggle_listen_tauri(state: &AppState) {
    if state.mic_listening.get_untracked() {
        state.mic_listening.set(false);
        // Tell backend to stop streaming audio chunks
        let args = js_sys::Object::new();
        js_sys::Reflect::set(&args, &"listening".into(), &JsValue::FALSE).ok();
        let _ = tauri_invoke("mic_set_listening", &args.into()).await;
        maybe_close_mic_tauri(state).await;
    } else {
        if ensure_mic_open_tauri(state).await {
            // Tell backend to start streaming audio chunks
            let args = js_sys::Object::new();
            js_sys::Reflect::set(&args, &"listening".into(), &JsValue::TRUE).ok();
            let _ = tauri_invoke("mic_set_listening", &args.into()).await;
            state.mic_listening.set(true);
        }
    }
}

/// Toggle recording in Tauri mode.
async fn toggle_record_tauri(state: &AppState) {
    if state.mic_recording.get_untracked() {
        // Stop recording
        state.mic_recording.set(false);
        state.mic_samples_recorded.set(0);

        match tauri_invoke_no_args("mic_stop_recording").await {
            Ok(result) => {
                finalize_recording_tauri(result, *state);
            }
            Err(e) => {
                log::error!("mic_stop_recording failed: {}", e);
                state.status_message.set(Some(format!("Recording failed: {}", e)));
            }
        }

        maybe_close_mic_tauri(state).await;
    } else {
        // Start recording
        if ensure_mic_open_tauri(state).await {
            match tauri_invoke_no_args("mic_start_recording").await {
                Ok(_) => {
                    state.mic_samples_recorded.set(0);
                    state.mic_recording.set(true);
                    log::info!("Native recording started");
                }
                Err(e) => {
                    log::error!("mic_start_recording failed: {}", e);
                    state.status_message.set(Some(format!("Failed to start recording: {}", e)));
                }
            }
        }
    }
}

/// Stop all in Tauri mode.
async fn stop_all_tauri(state: &AppState) {
    if state.mic_recording.get_untracked() {
        state.mic_recording.set(false);
        match tauri_invoke_no_args("mic_stop_recording").await {
            Ok(result) => {
                finalize_recording_tauri(result, *state);
            }
            Err(e) => {
                log::error!("mic_stop_recording failed: {}", e);
            }
        }
    }
    state.mic_listening.set(false);
    close_mic_tauri(state).await;
}

/// Build a LoadedFile from the Tauri RecordingResult and add to state.
fn finalize_recording_tauri(result: JsValue, state: AppState) {
    let filename = js_sys::Reflect::get(&result, &JsValue::from_str("filename"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| "recording.wav".into());
    let sample_rate = js_sys::Reflect::get(&result, &JsValue::from_str("sample_rate"))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(48000.0) as u32;
    let bits_per_sample = js_sys::Reflect::get(&result, &JsValue::from_str("bits_per_sample"))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(16.0) as u16;
    let is_float = js_sys::Reflect::get(&result, &JsValue::from_str("is_float"))
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let duration_secs = js_sys::Reflect::get(&result, &JsValue::from_str("duration_secs"))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let saved_path = js_sys::Reflect::get(&result, &JsValue::from_str("saved_path"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();

    // Extract f32 samples for frontend display
    let samples_js = js_sys::Reflect::get(&result, &JsValue::from_str("samples_f32"))
        .unwrap_or(JsValue::NULL);
    let samples_array = js_sys::Array::from(&samples_js);
    let samples: Vec<f32> = (0..samples_array.length())
        .map(|i| samples_array.get(i).as_f64().unwrap_or(0.0) as f32)
        .collect();

    if samples.is_empty() {
        log::warn!("No samples in recording result");
        return;
    }

    log::info!("Native recording: {} samples ({:.2}s at {} Hz, {}-bit{}), saved to {}",
        samples.len(), duration_secs, sample_rate, bits_per_sample,
        if is_float { " float" } else { "" }, saved_path);

    let audio = AudioData {
        samples,
        sample_rate,
        channels: 1,
        duration_secs,
        metadata: FileMetadata {
            file_size: 0,
            format: "REC",
            bits_per_sample,
            is_float,
            guano: None,
        },
    };

    // Phase 1: fast preview
    let preview = compute_preview(&audio, 256, 128);
    let audio_for_stft = audio.clone();
    let name_check = filename.clone();

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
                name: filename,
                audio,
                spectrogram: placeholder_spec,
                preview: Some(preview),
                xc_metadata: None,
                is_recording: false, // Already saved by backend
            });
        });
        file_index = idx;
    }
    state.current_file_index.set(Some(file_index));

    // Phase 2: async chunked spectrogram computation
    spawn_spectrogram_computation(audio_for_stft, name_check, file_index, state);
}

// ── Public API (routes by is_tauri) ─────────────────────────────────────

/// Toggle live HET listening on/off.
pub async fn toggle_listen(state: &AppState) {
    if state.is_tauri {
        toggle_listen_tauri(state).await;
    } else {
        if state.mic_listening.get_untracked() {
            state.mic_listening.set(false);
            maybe_close_mic_web(state);
        } else {
            if ensure_mic_open_web(state).await {
                state.mic_listening.set(true);
            }
        }
    }
}

/// Toggle recording on/off. When stopping, finalizes the recording.
pub async fn toggle_record(state: &AppState) {
    if state.is_tauri {
        toggle_record_tauri(state).await;
    } else {
        if state.mic_recording.get_untracked() {
            if let Some((samples, sr)) = stop_recording_web(state) {
                finalize_recording(samples, sr, *state);
            }
            maybe_close_mic_web(state);
        } else {
            if ensure_mic_open_web(state).await {
                MIC_BUFFER.with(|buf| buf.borrow_mut().clear());
                state.mic_samples_recorded.set(0);
                state.mic_recording.set(true);
                log::info!("Recording started");
            }
        }
    }
}

/// Stop both listening and recording, close mic.
pub fn stop_all(state: &AppState) {
    if state.is_tauri {
        let state = *state;
        wasm_bindgen_futures::spawn_local(async move {
            stop_all_tauri(&state).await;
        });
    } else {
        if state.mic_recording.get_untracked() {
            if let Some((samples, sr)) = stop_recording_web(state) {
                finalize_recording(samples, sr, *state);
            }
        }
        state.mic_listening.set(false);
        state.mic_recording.set(false);
        close_mic_web(state);
    }
}

// ── Common: WAV encoding, download, finalization ────────────────────────

/// Encode f32 samples as a 16-bit PCM WAV file (web mode fallback).
pub fn encode_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let num_samples = samples.len();
    let data_size = num_samples * 2;
    let file_size = 36 + data_size;

    let mut buf = Vec::with_capacity(44 + data_size);

    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(file_size as u32).to_le_bytes());
    buf.extend_from_slice(b"WAVE");

    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());  // PCM
    buf.extend_from_slice(&1u16.to_le_bytes());  // mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes());  // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&(data_size as u32).to_le_bytes());
    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let val = (clamped * 32767.0) as i16;
        buf.extend_from_slice(&val.to_le_bytes());
    }

    buf
}

/// Trigger a browser download of WAV data.
pub fn download_wav(samples: &[f32], sample_rate: u32, filename: &str) {
    let wav_data = encode_wav(samples, sample_rate);

    let array = js_sys::Uint8Array::new_with_length(wav_data.len() as u32);
    array.copy_from(&wav_data);

    let parts = js_sys::Array::new();
    parts.push(&array.buffer());

    let blob = match web_sys::Blob::new_with_u8_array_sequence(&parts) {
        Ok(b) => b,
        Err(e) => {
            log::error!("Failed to create Blob: {:?}", e);
            return;
        }
    };

    let url = match web_sys::Url::create_object_url_with_blob(&blob) {
        Ok(u) => u,
        Err(e) => {
            log::error!("Failed to create object URL: {:?}", e);
            return;
        }
    };

    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    let a: web_sys::HtmlAnchorElement = document
        .create_element("a").unwrap()
        .dyn_into().unwrap();
    a.set_href(&url);
    a.set_download(filename);
    a.set_attribute("style", "display:none").ok();
    document.body().unwrap().append_child(&a).ok();
    a.click();
    document.body().unwrap().remove_child(&a).ok();
    web_sys::Url::revoke_object_url(&url).ok();
}

/// Try to save recording via Tauri IPC (web mode). Returns true on success.
async fn try_tauri_save(wav_data: &[u8], filename: &str) -> bool {
    let tauri = match get_tauri_internals() {
        Some(t) => t,
        None => return false,
    };

    let invoke = match js_sys::Reflect::get(&tauri, &JsValue::from_str("invoke")) {
        Ok(f) if f.is_function() => js_sys::Function::from(f),
        _ => return false,
    };

    let args = js_sys::Object::new();
    js_sys::Reflect::set(&args, &JsValue::from_str("filename"), &JsValue::from_str(filename)).ok();

    let array = js_sys::Uint8Array::new_with_length(wav_data.len() as u32);
    array.copy_from(wav_data);
    js_sys::Reflect::set(&args, &JsValue::from_str("data"), &array).ok();

    let result = invoke.call2(&tauri, &JsValue::from_str("save_recording"), &args);
    match result {
        Ok(promise_val) => {
            if let Ok(promise) = promise_val.dyn_into::<js_sys::Promise>() {
                match JsFuture::from(promise).await {
                    Ok(path) => {
                        log::info!("Saved recording to: {:?}", path.as_string());
                        true
                    }
                    Err(e) => {
                        log::error!("Tauri save failed: {:?}", e);
                        false
                    }
                }
            } else {
                false
            }
        }
        Err(e) => {
            log::error!("Tauri invoke failed: {:?}", e);
            false
        }
    }
}

/// Convert recorded samples into a LoadedFile and add to state (web mode).
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

    let preview = compute_preview(&audio, 256, 128);
    let audio_for_stft = audio.clone();
    let name_check = name.clone();
    let name_for_save = name.clone();
    let is_tauri = state.is_tauri;

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
                is_recording: true,
            });
        });
        file_index = idx;
    }
    state.current_file_index.set(Some(file_index));

    // Try Tauri auto-save in background (web mode path for old save_recording command)
    if is_tauri {
        let samples_ref = state.files.get_untracked();
        if let Some(file) = samples_ref.get(file_index) {
            let wav_data = encode_wav(&file.audio.samples, file.audio.sample_rate);
            let filename = name_for_save;
            wasm_bindgen_futures::spawn_local(async move {
                if try_tauri_save(&wav_data, &filename).await {
                    state.files.update(|files| {
                        if let Some(f) = files.get_mut(file_index) {
                            f.is_recording = false;
                        }
                    });
                }
            });
        }
    }

    spawn_spectrogram_computation(audio_for_stft, name_check, file_index, state);
}

/// Shared async spectrogram computation (used by both web and Tauri modes).
fn spawn_spectrogram_computation(
    audio: AudioData,
    name_check: String,
    file_index: usize,
    state: AppState,
) {
    wasm_bindgen_futures::spawn_local(async move {
        let yield_promise = js_sys::Promise::new(&mut |resolve, _| {
            if let Some(w) = web_sys::window() {
                let _ = w.set_timeout_with_callback(&resolve);
            }
        });
        JsFuture::from(yield_promise).await.ok();

        const FFT_SIZE: usize = 2048;
        const HOP_SIZE: usize = 512;
        const CHUNK_COLS: usize = 32;

        let total_cols = if audio.samples.len() >= FFT_SIZE {
            (audio.samples.len() - FFT_SIZE) / HOP_SIZE + 1
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
                &audio,
                FFT_SIZE,
                HOP_SIZE,
                chunk_start,
                CHUNK_COLS,
            );
            all_columns.extend(chunk);
            chunk_start += CHUNK_COLS;

            let p = js_sys::Promise::new(&mut |resolve, _| {
                if let Some(w) = web_sys::window() {
                    let _ = w.set_timeout_with_callback(&resolve);
                }
            });
            JsFuture::from(p).await.ok();
        }

        let freq_resolution = audio.sample_rate as f64 / FFT_SIZE as f64;
        let time_resolution = HOP_SIZE as f64 / audio.sample_rate as f64;
        let max_freq = audio.sample_rate as f64 / 2.0;

        let spectrogram = SpectrogramData {
            columns: all_columns,
            freq_resolution,
            time_resolution,
            max_freq,
            sample_rate: audio.sample_rate,
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

        state.tile_ready_signal.update(|n| *n += 1);
    });
}
