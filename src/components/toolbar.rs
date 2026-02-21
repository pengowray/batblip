use leptos::prelude::*;
use crate::state::AppState;

#[component]
pub fn Toolbar() -> impl IntoView {
    let state = expect_context::<AppState>();
    let show_about = RwSignal::new(false);

    view! {
        <div class="toolbar">
            <span
                class="toolbar-brand"
                style=move || if state.sidebar_collapsed.get() { "margin-left: 24px; cursor: pointer" } else { "cursor: pointer" }
                on:click=move |_| show_about.set(true)
                title="About Batchi"
            >"Batchi"</span>

            {move || show_about.get().then(|| view! {
                <div class="about-overlay" on:click=move |_| show_about.set(false)>
                    <div class="about-dialog" on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()>
                        <div class="about-header">
                            <span class="about-title">"Batchi"</span>
                            <span class="about-version">"v0.1.2"</span>
                        </div>
                        <p class="about-desc">"Bat call viewer and acoustic analysis tool for WAV and FLAC recordings."</p>
                        <div class="about-modes">
                            <div class="about-mode"><span class="about-mode-tag">"HET"</span>" Heterodyne — mix with a local oscillator to shift ultrasonic calls into audible range"</div>
                            <div class="about-mode"><span class="about-mode-tag">"TE"</span>" Time Expansion — slow playback to lower pitch proportionally"</div>
                            <div class="about-mode"><span class="about-mode-tag">"PS"</span>" Pitch Shift — lower pitch while preserving original duration"</div>
                            <div class="about-mode"><span class="about-mode-tag">"ZC"</span>" Zero Crossing — frequency division via zero-crossing detection"</div>
                        </div>
                        <button class="about-close" on:click=move |_| show_about.set(false)>"Close"</button>
                    </div>
                </div>
            })}
        </div>
    }
}
