use leptos::prelude::*;
use crate::state::AppState;
use crate::audio::playback;

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
