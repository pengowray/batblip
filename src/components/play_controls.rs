use leptos::prelude::*;
use crate::state::{AppState, MicState};
use crate::audio::playback;
use crate::audio::microphone;

#[component]
pub fn PlayControls() -> impl IntoView {
    let state = expect_context::<AppState>();
    let has_file = move || state.current_file_index.get().is_some();
    let is_playing = move || state.is_playing.get();

    let state_play = state.clone();
    let on_play_start = move |_| {
        playback::play_from_start(&state_play);
    };

    let state_here = state.clone();
    let on_play_here = move |_| {
        playback::play_from_here(&state_here);
    };

    let state_stop = state.clone();
    let on_stop = move |_| {
        playback::stop(&state_stop);
    };

    view! {
        <div class="play-controls"
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
        >
            // Mic button (always visible)
            <button
                class=move || match state.mic_state.get() {
                    MicState::Off => "layer-btn".to_string(),
                    MicState::Armed => "layer-btn mic-armed".to_string(),
                    MicState::Recording => "layer-btn mic-recording".to_string(),
                }
                on:click=move |_| {
                    let st = state;
                    match st.mic_state.get_untracked() {
                        MicState::Off => {
                            wasm_bindgen_futures::spawn_local(async move {
                                microphone::arm(&st).await;
                            });
                        }
                        MicState::Armed => {
                            microphone::start_recording(&st);
                        }
                        MicState::Recording => {
                            if let Some((samples, sr)) = microphone::stop_recording(&st) {
                                microphone::finalize_recording(samples, sr, st);
                            }
                        }
                    }
                }
                title=move || match state.mic_state.get() {
                    MicState::Off => "Arm microphone (M)",
                    MicState::Armed => "Start recording (M)",
                    MicState::Recording => "Stop recording (M)",
                }
            >
                <span class="layer-btn-category">"Mic"</span>
                <span class="layer-btn-value">{move || match state.mic_state.get() {
                    MicState::Off => "Off".to_string(),
                    MicState::Armed => "Armed".to_string(),
                    MicState::Recording => {
                        let n = state.mic_samples_recorded.get();
                        let sr = state.mic_sample_rate.get_untracked().max(1);
                        let secs = n as f64 / sr as f64;
                        format!("{:.1}s", secs)
                    }
                }}</span>
            </button>

            // Disarm button (only when armed or recording)
            {move || (state.mic_state.get() != MicState::Off).then(|| {
                view! {
                    <button
                        class="layer-btn"
                        on:click=move |_| microphone::disarm(&state)
                        title="Disarm microphone (Esc)"
                    >
                        <span class="layer-btn-category">"Mic"</span>
                        <span class="layer-btn-value">"Off"</span>
                    </button>
                }
            })}

            // Play/Stop buttons (when a file is loaded)
            {move || if !has_file() {
                view! { <span></span> }.into_any()
            } else if is_playing() {
                view! {
                    <button class="layer-btn" on:click=on_stop.clone()>"Stop"</button>
                }.into_any()
            } else {
                view! {
                    <button class="layer-btn" on:click=on_play_start.clone()
                        title="Play from start of file"
                    >"Play start"</button>
                    <button class="layer-btn" on:click=on_play_here.clone()
                        title="Play from current position"
                    >"Play here"</button>
                }.into_any()
            }}

            // Bookmark popup
            {move || state.show_bookmark_popup.get().then(|| {
                let bms = state.bookmarks.get();
                let recent: Vec<_> = bms.iter().rev().take(8).cloned().collect();
                view! {
                    <div class="bookmark-popup"
                        on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
                    >
                        <div class="bookmark-popup-title">"Bookmarks"</div>
                        {recent.into_iter().map(|bm| {
                            let t = bm.time;
                            let state2 = state.clone();
                            view! {
                                <button class="bookmark-item"
                                    on:click=move |_| {
                                        // Jump to just before the bookmark so it's visible
                                        let zoom = state2.zoom_level.get_untracked();
                                        let files = state2.files.get_untracked();
                                        let idx = state2.current_file_index.get_untracked();
                                        let time_res = idx.and_then(|i| files.get(i))
                                            .map(|f| f.spectrogram.time_resolution)
                                            .unwrap_or(0.001);
                                        let canvas_w = 800.0_f64; // approximate
                                        let visible_time = (canvas_w / zoom) * time_res;
                                        let new_scroll = (t - visible_time * 0.1).max(0.0);
                                        state2.scroll_offset.set(new_scroll);
                                        state2.show_bookmark_popup.set(false);
                                    }
                                >{format!("{:.2}s", t)}</button>
                            }
                        }).collect_view()}
                        <button class="bookmark-popup-close"
                            on:click=move |_| state.show_bookmark_popup.set(false)
                        >"Dismiss"</button>
                    </div>
                }
            })}
        </div>
    }
}
