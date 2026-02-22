mod files_panel;
mod settings_panel;
mod analysis;
mod metadata_panel;
mod harmonics;
mod loading;

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use js_sys;
use crate::state::{AppState, SidebarTab};

use files_panel::FilesPanel;
use settings_panel::{SpectrogramSettingsPanel, SelectionPanel};
use analysis::AnalysisPanel;
use metadata_panel::MetadataPanel;
use harmonics::HarmonicsPanel;

fn copy_to_clipboard(text: &str) {
    if let Some(window) = web_sys::window() {
        let clipboard = window.navigator().clipboard();
        let _ = clipboard.write_text(text);
    }
}

#[component]
pub fn FileSidebar() -> impl IntoView {
    let state = expect_context::<AppState>();

    // Resize drag logic
    let on_resize_start = move |ev: web_sys::MouseEvent| {
        ev.prevent_default();
        let start_x = ev.client_x() as f64;
        let start_width = state.sidebar_width.get_untracked();
        let doc = web_sys::window().unwrap().document().unwrap();
        let body = doc.body().unwrap();
        let _ = body.class_list().add_1("sidebar-resizing");

        let on_move = Closure::<dyn Fn(web_sys::MouseEvent)>::new(move |ev: web_sys::MouseEvent| {
            let dx = ev.client_x() as f64 - start_x;
            let new_width = (start_width + dx).clamp(140.0, 500.0);
            state.sidebar_width.set(new_width);
        });

        let on_move_fn = on_move.as_ref().unchecked_ref::<js_sys::Function>().clone();
        let on_move_fn2 = on_move_fn.clone();
        let _ = doc.add_event_listener_with_callback("mousemove", &on_move_fn);

        let on_up = Closure::<dyn FnMut(web_sys::MouseEvent)>::once_into_js(move |_: web_sys::MouseEvent| {
            let doc = web_sys::window().unwrap().document().unwrap();
            let body = doc.body().unwrap();
            let _ = body.class_list().remove_1("sidebar-resizing");
            let _ = doc.remove_event_listener_with_callback("mousemove", &on_move_fn2);
            drop(on_move);
        });

        let _ = doc.add_event_listener_with_callback_and_bool("mouseup", on_up.unchecked_ref(), true);
    };

    let sidebar_class = move || {
        if state.sidebar_collapsed.get() { "sidebar collapsed" } else { "sidebar" }
    };

    let dropdown_open = state.sidebar_dropdown_open;

    let on_dropdown_toggle = move |_: web_sys::MouseEvent| {
        if state.sidebar_collapsed.get_untracked() {
            state.sidebar_collapsed.set(false);
        } else {
            dropdown_open.update(|v| *v = !*v);
        }
    };

    let select_tab = move |tab: SidebarTab| {
        state.sidebar_collapsed.set(false);
        state.sidebar_tab.set(tab);
        dropdown_open.set(false);
    };

    // Close dropdown when clicking outside
    let on_dropdown_blur = move |_: web_sys::FocusEvent| {
        // Small delay to allow click on menu items to register first
        let handle = wasm_bindgen::closure::Closure::once(move || {
            dropdown_open.set(false);
        });
        let _ = web_sys::window().unwrap().set_timeout_with_callback_and_timeout_and_arguments_0(
            handle.as_ref().unchecked_ref(),
            150,
        );
        handle.forget();
    };

    view! {
        <div class=sidebar_class>
            <div class="sidebar-tabs">
                <button
                    class="sidebar-tab sidebar-collapse-btn"
                    on:click=move |_| {
                        state.sidebar_collapsed.update(|c| *c = !*c);
                        dropdown_open.set(false);
                    }
                    title=move || if state.sidebar_collapsed.get() { "Show sidebar" } else { "Hide sidebar" }
                >
                    {move || if state.sidebar_collapsed.get() { "\u{25B6}" } else { "\u{25C0}" }}
                </button>
                <div class="sidebar-tab-dropdown-wrap" tabindex="-1" on:focusout=on_dropdown_blur>
                    <button class="sidebar-tab-dropdown" on:click=on_dropdown_toggle>
                        {move || state.sidebar_tab.get().label()}
                        <span class="dropdown-arrow">{move || if dropdown_open.get() { "\u{25B4}" } else { "\u{25BE}" }}</span>
                    </button>
                    {move || {
                        if dropdown_open.get() {
                            let items: Vec<_> = SidebarTab::ALL.iter().map(|&tab| {
                                let is_active = move || state.sidebar_tab.get() == tab;
                                let label = tab.label();
                                view! {
                                    <button
                                        class=move || if is_active() { "sidebar-tab-option active" } else { "sidebar-tab-option" }
                                        on:mousedown=move |ev: web_sys::MouseEvent| {
                                            ev.prevent_default();
                                            select_tab(tab);
                                        }
                                    >
                                        {label}
                                    </button>
                                }
                            }).collect();
                            view! {
                                <div class="sidebar-tab-menu">{items}</div>
                            }.into_any()
                        } else {
                            view! { <span></span> }.into_any()
                        }
                    }}
                </div>
            </div>
            {move || match state.sidebar_tab.get() {
                SidebarTab::Files => view! { <FilesPanel /> }.into_any(),
                SidebarTab::Spectrogram => view! { <SpectrogramSettingsPanel /> }.into_any(),
                SidebarTab::Selection => view! { <SelectionPanel /> }.into_any(),
                SidebarTab::Analysis => view! { <AnalysisPanel /> }.into_any(),
                SidebarTab::Harmonics => view! { <HarmonicsPanel /> }.into_any(),
                SidebarTab::Metadata => view! { <MetadataPanel /> }.into_any(),
            }}
            <div class="sidebar-resize-handle" on:mousedown=on_resize_start></div>
        </div>
    }
}
