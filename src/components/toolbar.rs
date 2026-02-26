use leptos::prelude::*;
use crate::state::AppState;
use crate::audio::microphone;

#[component]
pub fn Toolbar() -> impl IntoView {
    let state = expect_context::<AppState>();
    let show_about = RwSignal::new(false);

    let is_mobile = state.is_mobile.get_untracked();

    view! {
        <div class="toolbar">
            {if is_mobile {
                Some(view! {
                    <button
                        class="toolbar-menu-btn"
                        on:click=move |ev: web_sys::MouseEvent| {
                            ev.stop_propagation();
                            state.sidebar_collapsed.update(|c| *c = !*c);
                        }
                        title="Menu"
                    >"\u{2630}"</button>
                })
            } else {
                None
            }}
            <span
                class="toolbar-brand"
                style=move || if !is_mobile && state.sidebar_collapsed.get() { "margin-left: 24px; cursor: pointer" } else { "cursor: pointer" }
                on:click=move |_| show_about.set(true)
                title="About Batchi"
            >"Batchi"</span>

            // Spacer
            <div style="flex: 1;"></div>

            // Listen button
            <button
                class=move || if state.mic_listening.get() { "toolbar-listen-btn active" } else { "toolbar-listen-btn" }
                on:click=move |_| {
                    let st = state;
                    wasm_bindgen_futures::spawn_local(async move {
                        microphone::toggle_listen(&st).await;
                    });
                }
                title="Toggle live listening (L)"
            >"Listen"</button>

            // Settings button (opens right sidebar on mobile)
            {if is_mobile {
                Some(view! {
                    <button
                        class="toolbar-menu-btn"
                        on:click=move |ev: web_sys::MouseEvent| {
                            ev.stop_propagation();
                            state.right_sidebar_collapsed.update(|c| *c = !*c);
                            // Close left sidebar when opening right
                            if !state.right_sidebar_collapsed.get_untracked() {
                                state.sidebar_collapsed.set(true);
                            }
                        }
                        title="Settings"
                    >{"\u{2699}"}</button>
                })
            } else {
                None
            }}

            // Record button
            <button
                class=move || if state.mic_recording.get() { "toolbar-record-btn active" } else { "toolbar-record-btn" }
                on:click=move |_| {
                    let st = state;
                    wasm_bindgen_futures::spawn_local(async move {
                        microphone::toggle_record(&st).await;
                    });
                }
                title="Toggle recording (R)"
            >
                {move || if state.mic_recording.get() {
                    let n = state.mic_samples_recorded.get();
                    let sr = state.mic_sample_rate.get_untracked().max(1);
                    let secs = n as f64 / sr as f64;
                    format!("Rec {:.1}s", secs)
                } else {
                    "Record".to_string()
                }}
            </button>

            {move || show_about.get().then(|| view! {
                <div class="about-overlay" on:click=move |_| show_about.set(false)>
                    <div class="about-dialog" on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()>
                        <div class="about-header">
                            <span class="about-title">"Batchi by Pengo Wray"</span>
                            <span class="about-version">"v0.1.5"</span>
                        </div>
                        <p class="about-desc">"Bat call viewer and acoustic analysis tool."</p>
                        <div class="about-modes">
                            <div class="about-mode"><span class="about-mode-tag">"HET"</span>" Heterodyne — mix with a local oscillator to shift ultrasonic calls into audible range"</div>
                            <div class="about-mode"><span class="about-mode-tag">"TE"</span>" Time Expansion — slow playback to lower pitch proportionally"</div>
                            <div class="about-mode"><span class="about-mode-tag">"PS"</span>" Pitch Shift — lower pitch while preserving original duration"</div>
                            <div class="about-mode"><span class="about-mode-tag">"ZC"</span>" Zero Crossing — frequency division via zero-crossing detection"</div>
                        </div>
                        <div style="margin-top: 12px; font-size: 11px; color: #666; line-height: 1.5;">
                            "Thanks to "
                            <a href="https://twilighttravels.org/batgizmo-app/"
                               target="_blank"
                               style="color: #8cf; text-decoration: none;"
                            >"John Mears"</a>
                            " ("
                            <a href="https://github.com/jmears63/batgizmo-app-public"
                               target="_blank"
                               style="color: #8cf; text-decoration: none;"
                            >"batgizmo-app"</a>
                            ", MIT)"
                        </div>
                        <button class="about-close" on:click=move |_| show_about.set(false)>"Close"</button>
                    </div>
                </div>
            })}
        </div>
    }
}
