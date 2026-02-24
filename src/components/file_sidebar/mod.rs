mod files_panel;
pub mod settings_panel;
pub mod analysis;
pub mod metadata_panel;
pub mod harmonics;
mod loading;

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use js_sys;
use crate::state::AppState;

use files_panel::FilesPanel;
pub(crate) use settings_panel::{SpectrogramSettingsPanel, SelectionPanel};
pub(crate) use analysis::AnalysisPanel as SidebarAnalysisPanel;
pub(crate) use metadata_panel::MetadataPanel;
pub(crate) use harmonics::HarmonicsPanel;
pub(crate) use loading::load_named_bytes;

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

    let is_mobile = state.is_mobile.get_untracked();

    let sidebar_class = move || {
        let mut cls = String::from("sidebar");
        if state.sidebar_collapsed.get() {
            cls.push_str(" collapsed");
        }
        if is_mobile {
            cls.push_str(" mobile-overlay");
        }
        cls
    };

    view! {
        <div class=sidebar_class>
            <div class="sidebar-tabs">
                {if !is_mobile {
                    Some(view! {
                        <button
                            class="sidebar-tab sidebar-collapse-btn"
                            on:click=move |_| {
                                state.sidebar_collapsed.update(|c| *c = !*c);
                            }
                            title=move || if state.sidebar_collapsed.get() { "Show sidebar" } else { "Hide sidebar" }
                        >
                            {move || if state.sidebar_collapsed.get() { "\u{25B6}" } else { "\u{25C0}" }}
                        </button>
                    })
                } else {
                    None
                }}
                <div class="sidebar-header-label">"Files"</div>
            </div>
            <FilesPanel />
            {if !is_mobile {
                Some(view! { <div class="sidebar-resize-handle" on:mousedown=on_resize_start></div> })
            } else {
                None
            }}
        </div>
    }
}
