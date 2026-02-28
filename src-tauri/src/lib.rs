mod audio_decode;
mod native_playback;
mod recording;
mod xc;

use audio_decode::{AudioFileInfo, FullDecodeResult};
use native_playback::{NativePlayParams, PlaybackState, PlaybackStatus};
use recording::{MicInfo, MicState, MicStatus, RecordingResult};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::Manager;

type MicMutex = Mutex<Option<MicState>>;
type PlaybackMutex = Mutex<Option<PlaybackState>>;

#[tauri::command]
fn save_recording(
    app: tauri::AppHandle,
    filename: String,
    data: Vec<u8>,
) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("recordings");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(&filename);
    std::fs::write(&path, &data).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn mic_open(app: tauri::AppHandle, state: tauri::State<MicMutex>) -> Result<MicInfo, String> {
    let mut mic = state.lock().map_err(|e| e.to_string())?;
    if mic.is_some() {
        // Already open — return current info
        let m = mic.as_ref().unwrap();
        return Ok(MicInfo {
            device_name: m.device_name.clone(),
            sample_rate: m.sample_rate,
            bits_per_sample: m.format.bits_per_sample(),
            is_float: m.format.is_float(),
            format: format!("{:?}", m.format),
        });
    }

    let m = recording::open_mic()?;
    let info = MicInfo {
        device_name: m.device_name.clone(),
        sample_rate: m.sample_rate,
        bits_per_sample: m.format.bits_per_sample(),
        is_float: m.format.is_float(),
        format: format!("{:?}", m.format),
    };

    // Start the emitter thread for streaming audio chunks to the frontend
    recording::start_emitter(app, m.buffer.clone(), m.emitter_stop.clone());

    *mic = Some(m);
    Ok(info)
}

#[tauri::command]
fn mic_close(state: tauri::State<MicMutex>) -> Result<(), String> {
    let mut mic = state.lock().map_err(|e| e.to_string())?;
    if let Some(m) = mic.take() {
        m.emitter_stop.store(true, Ordering::Relaxed);
        m.is_recording.store(false, Ordering::Relaxed);
        m.is_streaming.store(false, Ordering::Relaxed);
        drop(m); // drops the cpal::Stream, closing the mic
    }
    Ok(())
}

#[tauri::command]
fn mic_start_recording(state: tauri::State<MicMutex>) -> Result<(), String> {
    let mic = state.lock().map_err(|e| e.to_string())?;
    let m = mic.as_ref().ok_or("Microphone not open")?;
    {
        let mut buf = m.buffer.lock().unwrap();
        buf.clear();
    }
    m.is_recording.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn mic_stop_recording(
    app: tauri::AppHandle,
    state: tauri::State<MicMutex>,
) -> Result<RecordingResult, String> {
    let mic = state.lock().map_err(|e| e.to_string())?;
    let m = mic.as_ref().ok_or("Microphone not open")?;
    m.is_recording.store(false, Ordering::Relaxed);

    let buf = m.buffer.lock().unwrap();
    let num_samples = buf.total_samples;
    if num_samples == 0 {
        return Err("No samples recorded".into());
    }

    let sample_rate = buf.sample_rate;
    let duration_secs = num_samples as f64 / sample_rate as f64;

    // Generate filename
    let now = chrono::Local::now();
    let filename = now.format("batcap_%Y-%m-%d_%H%M%S.wav").to_string();

    // Encode WAV at native bit depth
    let wav_data = recording::encode_native_wav(&buf)?;

    // Get f32 samples for frontend display
    let samples_f32 = recording::get_samples_f32(&buf);

    let bits_per_sample = buf.format.bits_per_sample();
    let is_float = buf.format.is_float();

    drop(buf);

    // Save to disk
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("recordings");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(&filename);
    std::fs::write(&path, &wav_data).map_err(|e| e.to_string())?;
    let saved_path = path.to_string_lossy().to_string();

    Ok(RecordingResult {
        filename,
        saved_path,
        sample_rate,
        bits_per_sample,
        is_float,
        duration_secs,
        num_samples,
        samples_f32,
    })
}

#[tauri::command]
fn mic_set_listening(state: tauri::State<MicMutex>, listening: bool) -> Result<(), String> {
    let mic = state.lock().map_err(|e| e.to_string())?;
    let m = mic.as_ref().ok_or("Microphone not open")?;
    m.is_streaming.store(listening, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn mic_get_status(state: tauri::State<MicMutex>) -> MicStatus {
    let mic = state.lock().unwrap_or_else(|e| e.into_inner());
    match mic.as_ref() {
        Some(m) => {
            let samples = m.buffer.lock().map(|b| b.total_samples).unwrap_or(0);
            MicStatus {
                is_open: true,
                is_recording: m.is_recording.load(Ordering::Relaxed),
                is_streaming: m.is_streaming.load(Ordering::Relaxed),
                samples_recorded: samples,
                sample_rate: m.sample_rate,
            }
        }
        None => MicStatus {
            is_open: false,
            is_recording: false,
            is_streaming: false,
            samples_recorded: 0,
            sample_rate: 0,
        },
    }
}

// ── Audio file decoding commands ─────────────────────────────────────

#[tauri::command]
fn audio_file_info(path: String) -> Result<AudioFileInfo, String> {
    audio_decode::file_info(&path)
}

#[tauri::command]
fn audio_decode_full(path: String) -> Result<FullDecodeResult, String> {
    audio_decode::decode_full(&path)
}

/// Read raw file bytes — returns binary data via efficient IPC (no JSON serialization).
#[tauri::command]
fn read_file_bytes(path: String) -> Result<tauri::ipc::Response, String> {
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("Failed to read file '{}': {}", path, e))?;
    Ok(tauri::ipc::Response::new(bytes))
}

// ── Native playback commands ────────────────────────────────────────

#[tauri::command]
fn native_play(
    app: tauri::AppHandle,
    state: tauri::State<PlaybackMutex>,
    params: NativePlayParams,
) -> Result<(), String> {
    let mut pb = state.lock().map_err(|e| e.to_string())?;
    // Stop existing playback
    native_playback::stop(&mut pb);
    // Start new playback
    let new_state = native_playback::start(params, app)?;
    *pb = Some(new_state);
    Ok(())
}

#[tauri::command]
fn native_stop(state: tauri::State<PlaybackMutex>) -> Result<(), String> {
    let mut pb = state.lock().map_err(|e| e.to_string())?;
    native_playback::stop(&mut pb);
    Ok(())
}

#[tauri::command]
fn native_playback_status(state: tauri::State<PlaybackMutex>) -> PlaybackStatus {
    let pb = state.lock().unwrap_or_else(|e| e.into_inner());
    match pb.as_ref() {
        Some(s) => PlaybackStatus {
            is_playing: s.is_playing(),
            playhead_secs: s.playhead_secs(),
        },
        None => PlaybackStatus {
            is_playing: false,
            playhead_secs: 0.0,
        },
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(None::<MicState>))
        .manage(Mutex::new(None::<PlaybackState>))
        .setup(|app| {
            let cache_root = app
                .path()
                .app_data_dir()
                .map(|d| d.join("xc-cache"))
                .unwrap_or_else(|_| std::path::PathBuf::from("xc-cache"));
            let _ = std::fs::create_dir_all(&cache_root);
            app.manage(Mutex::new(xc::XcState {
                client: reqwest::Client::new(),
                cache_root,
            }));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            save_recording,
            mic_open,
            mic_close,
            mic_start_recording,
            mic_stop_recording,
            mic_set_listening,
            mic_get_status,
            audio_file_info,
            audio_decode_full,
            read_file_bytes,
            native_play,
            native_stop,
            native_playback_status,
            xc::xc_set_api_key,
            xc::xc_get_api_key,
            xc::xc_browse_group,
            xc::xc_refresh_taxonomy,
            xc::xc_taxonomy_age,
            xc::xc_search,
            xc::xc_species_recordings,
            xc::xc_download,
            xc::xc_is_cached,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
