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
                title="About batblip"
            ><b>"bat"</b><i>"blip"</i></span>

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
                            <span class="about-title"><b>"bat"</b><i>"blip"</i>" by Pengo Wray"</span>
                            <span class="about-version">{concat!("v", env!("CARGO_PKG_VERSION"))}</span>
                        </div>
                        <p class="about-desc">"Bat call viewer and acoustic analysis tool."</p>
                        <div style="margin-top: 12px; font-size: 11px; color: #999; line-height: 1.8;">
                            "Thanks to the libraries that make this possible:"
                            <div style="margin-top: 6px; columns: 2; column-gap: 16px;">
                                <div><a href="https://leptos.dev" target="_blank" style="color: #8cf; text-decoration: none;">"Leptos"</a>""</div>
                                <div><a href="https://crates.io/crates/realfft" target="_blank" style="color: #8cf; text-decoration: none;">"RealFFT"</a></div>
                                <div><a href="https://crates.io/crates/hound" target="_blank" style="color: #8cf; text-decoration: none;">"Hound"</a></div>
                                <div><a href="https://crates.io/crates/claxon" target="_blank" style="color: #8cf; text-decoration: none;">"Claxon"</a></div>
                                <div><a href="https://crates.io/crates/lewton" target="_blank" style="color: #8cf; text-decoration: none;">"Lewton"</a></div>
                                <div><a href="https://crates.io/crates/symphonia" target="_blank" style="color: #8cf; text-decoration: none;">"Symphonia"</a></div>
                                <div><a href="https://github.com/jmears63/batgizmo-app-public" target="_blank" style="color: #8cf; text-decoration: none;">"batgizmo-app"</a></div>
                            </div>
                        </div>
                        <button class="about-close" on:click=move |_| show_about.set(false)>"Close"</button>
                    </div>
                </div>
            })}
        </div>
    }
}
