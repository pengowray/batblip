use leptos::prelude::*;
use crate::state::{AppState, CanvasTool, LayerPanel};

fn layer_opt_class(active: bool) -> &'static str {
    if active { "layer-panel-opt sel" } else { "layer-panel-opt" }
}

fn toggle_panel(state: &AppState, panel: LayerPanel) {
    state.layer_panel_open.update(|p| {
        *p = if *p == Some(panel) { None } else { Some(panel) };
    });
}

#[component]
pub fn ToolButton() -> impl IntoView {
    let state = expect_context::<AppState>();
    let is_open = move || state.layer_panel_open.get() == Some(LayerPanel::Tool);

    view! {
        // Anchored bottom-right of main-overlays
        <div
            style="position: absolute; bottom: 50px; right: 12px; pointer-events: none;"
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
        >
            <div style="position: relative; pointer-events: auto;">
                <button
                    class=move || if is_open() { "layer-btn open" } else { "layer-btn" }
                    on:click=move |_| toggle_panel(&state, LayerPanel::Tool)
                    title="Tool"
                >
                    {move || match state.canvas_tool.get() {
                        CanvasTool::Hand => "Hand",
                        CanvasTool::Selection => "Select",
                    }}
                </button>
                {move || is_open().then(|| view! {
                    <div class="layer-panel" style="bottom: 34px; right: 0;">
                        <div class="layer-panel-title">"Tool"</div>
                        <button
                            class=move || layer_opt_class(state.canvas_tool.get() == CanvasTool::Hand)
                            on:click=move |_| {
                                state.canvas_tool.set(CanvasTool::Hand);
                                state.layer_panel_open.set(None);
                            }
                        >"Hand (pan)"</button>
                        <button
                            class=move || layer_opt_class(state.canvas_tool.get() == CanvasTool::Selection)
                            on:click=move |_| {
                                state.canvas_tool.set(CanvasTool::Selection);
                                state.layer_panel_open.set(None);
                            }
                        >"Selection"</button>
                    </div>
                })}
            </div>
        </div>
    }
}
