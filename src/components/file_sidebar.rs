use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{DragEvent, File, FileReader};
use crate::audio::loader::load_audio;
use crate::dsp::fft::compute_spectrogram;
use crate::state::{AppState, LoadedFile};

#[component]
pub fn FileSidebar() -> impl IntoView {
    let state = expect_context::<AppState>();
    let drag_over = RwSignal::new(false);
    let files = state.files;
    let current_idx = state.current_file_index;

    let on_dragover = move |ev: DragEvent| {
        ev.prevent_default();
        drag_over.set(true);
    };

    let on_dragleave = move |_: DragEvent| {
        drag_over.set(false);
    };

    let state_for_drop = state.clone();
    let on_drop = move |ev: DragEvent| {
        ev.prevent_default();
        drag_over.set(false);

        let Some(dt) = ev.data_transfer() else { return };
        let Some(file_list) = dt.files() else { return };

        for i in 0..file_list.length() {
            let Some(file) = file_list.get(i) else { continue };
            let state = state_for_drop.clone();
            spawn_local(async move {
                match read_and_load_file(file, state).await {
                    Ok(()) => {}
                    Err(e) => log::error!("Failed to load file: {e}"),
                }
            });
        }
    };

    view! {
        <div class="sidebar">
            <div class="sidebar-header">"Files"</div>
            <div
                class=move || if drag_over.get() { "drop-zone drag-over" } else { "drop-zone" }
                on:dragover=on_dragover
                on:dragleave=on_dragleave
                on:drop=on_drop
            >
                {move || {
                    let file_vec = files.get();
                    if file_vec.is_empty() {
                        view! {
                            <div class="drop-hint">"Drop WAV/FLAC files here"</div>
                        }.into_any()
                    } else {
                        let items: Vec<_> = file_vec.iter().enumerate().map(|(i, f)| {
                            let name = f.name.clone();
                            let is_active = move || current_idx.get() == Some(i);
                            let on_click = move |_| {
                                current_idx.set(Some(i));
                            };
                            view! {
                                <div
                                    class=move || if is_active() { "file-item active" } else { "file-item" }
                                    on:click=on_click
                                >
                                    {name}
                                </div>
                            }
                        }).collect();
                        view! {
                            <div class="file-list">{items}</div>
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}

async fn read_and_load_file(file: File, state: AppState) -> Result<(), String> {
    let name = file.name();
    let bytes = read_file_bytes(&file).await?;

    let audio = load_audio(&bytes)?;
    log::info!(
        "Loaded {}: {} samples, {} Hz, {:.2}s",
        name,
        audio.samples.len(),
        audio.sample_rate,
        audio.duration_secs
    );

    let spectrogram = compute_spectrogram(&audio, 2048, 512);
    log::info!(
        "Spectrogram: {} columns, freq_res={:.1} Hz, time_res={:.4}s",
        spectrogram.columns.len(),
        spectrogram.freq_resolution,
        spectrogram.time_resolution
    );

    let loaded = LoadedFile {
        name,
        audio,
        spectrogram,
    };

    state.files.update(|files| {
        files.push(loaded);
        if files.len() == 1 {
            state.current_file_index.set(Some(0));
        }
    });

    Ok(())
}

async fn read_file_bytes(file: &File) -> Result<Vec<u8>, String> {
    let reader = FileReader::new().map_err(|e| format!("FileReader: {e:?}"))?;
    let reader_clone = reader.clone();

    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let resolve_clone = resolve.clone();
        let reject_clone = reject.clone();

        let onload = Closure::once(move |_: web_sys::Event| {
            resolve_clone.call0(&JsValue::NULL).unwrap();
        });
        let onerror = Closure::once(move |_: web_sys::Event| {
            reject_clone.call0(&JsValue::NULL).unwrap();
        });

        reader_clone.set_onloadend(Some(onload.as_ref().unchecked_ref()));
        reader_clone.set_onerror(Some(onerror.as_ref().unchecked_ref()));

        // Prevent closures from being dropped
        onload.forget();
        onerror.forget();
    });

    reader
        .read_as_array_buffer(file)
        .map_err(|e| format!("read_as_array_buffer: {e:?}"))?;

    JsFuture::from(promise)
        .await
        .map_err(|e| format!("FileReader await: {e:?}"))?;

    let result = reader.result().map_err(|e| format!("result: {e:?}"))?;
    let array_buffer = result
        .dyn_into::<js_sys::ArrayBuffer>()
        .map_err(|_| "Expected ArrayBuffer".to_string())?;
    let uint8_array = js_sys::Uint8Array::new(&array_buffer);
    Ok(uint8_array.to_vec())
}
