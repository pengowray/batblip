use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// Reusable slider row: static label + range input + fixed-width value display.
/// Double-click the slider to reset to its default value.
#[component]
pub fn SliderRow(
    /// Static label text (e.g., "Gain", "Range")
    label: &'static str,
    /// The signal to bind to
    signal: RwSignal<f32>,
    /// Minimum slider value
    min: f32,
    /// Maximum slider value
    max: f32,
    /// Slider step
    step: f32,
    /// Default value for double-click reset
    default: f32,
    /// Formats the current value for the display span
    #[prop(into)]
    format_value: Callback<f32, String>,
    /// Optional callback fired after each input event (receives new value)
    #[prop(optional, into)]
    on_change: Option<Callback<f32>>,
) -> impl IntoView {
    let min_s = min.to_string();
    let max_s = max.to_string();
    let step_s = step.to_string();

    view! {
        <div class="setting-row">
            <span class="setting-label">{label}</span>
            <div class="setting-slider-row">
                <input
                    type="range"
                    class="setting-range"
                    min=min_s
                    max=max_s
                    step=step_s
                    prop:value=move || signal.get().to_string()
                    on:input=move |ev: web_sys::Event| {
                        let target = ev.target().unwrap();
                        let input: web_sys::HtmlInputElement = target.unchecked_into();
                        if let Ok(v) = input.value().parse::<f32>() {
                            signal.set(v);
                            if let Some(cb) = on_change {
                                cb.run(v);
                            }
                        }
                    }
                    on:dblclick=move |_: web_sys::MouseEvent| {
                        signal.set(default);
                        if let Some(cb) = on_change {
                            cb.run(default);
                        }
                    }
                />
                <span class="setting-value">{move || format_value.run(signal.get())}</span>
            </div>
        </div>
    }
}
