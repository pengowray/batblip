use leptos::prelude::*;
use crate::state::AppState;
use crate::components::file_sidebar::FileSidebar;
use crate::components::spectrogram::Spectrogram;

#[component]
pub fn App() -> impl IntoView {
    let state = AppState::new();
    provide_context(state);

    view! {
        <div class="app">
            <FileSidebar />
            <MainArea />
        </div>
    }
}

#[component]
fn MainArea() -> impl IntoView {
    let state = expect_context::<AppState>();
    let has_file = move || state.current_file_index.get().is_some();

    view! {
        <div class="main">
            <div class="toolbar">
                <span style="color: #666">"Batgram"</span>
            </div>
            {move || {
                if has_file() {
                    view! {
                        <Spectrogram />
                        <div class="waveform-container"></div>
                        <div class="analysis-panel">
                            <span>"No selection"</span>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class="empty-state">
                            "Drop WAV or FLAC files into the sidebar"
                        </div>
                    }.into_any()
                }
            }}
        </div>
    }
}
