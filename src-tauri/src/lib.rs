mod recording;

use recording::{MicInfo, MicState, MicStatus, RecordingResult};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::Manager;

type MicMutex = Mutex<Option<MicState>>;

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
    let filename = now.format("rec_%Y-%m-%d_%H%M%S.wav").to_string();

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(None::<MicState>))
        .invoke_handler(tauri::generate_handler![
            save_recording,
            mic_open,
            mic_close,
            mic_start_recording,
            mic_stop_recording,
            mic_set_listening,
            mic_get_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
