use leptos::prelude::*;
use wasm_bindgen::JsCast;
use crate::state::{AppState, MainView, SpectrogramDisplay};
use crate::dsp::zero_crossing::zero_crossing_frequency;

#[component]
pub(crate) fn SpectrogramSettingsPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    let on_display_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let select: web_sys::HtmlSelectElement = target.unchecked_into();
        let mode = match select.value().as_str() {
            "centroid" => SpectrogramDisplay::MovementCentroid,
            "gradient" => SpectrogramDisplay::MovementGradient,
            _ => SpectrogramDisplay::MovementFlow,
        };
        state.spectrogram_display.set(mode);
    };

    let on_intensity_gate_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        if let Ok(val) = input.value().parse::<f32>() {
            state.mv_intensity_gate.set(val / 100.0);
        }
    };

    let on_movement_gate_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        if let Ok(val) = input.value().parse::<f32>() {
            state.mv_movement_gate.set(val / 100.0);
        }
    };

    let on_opacity_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        if let Ok(val) = input.value().parse::<f32>() {
            state.mv_opacity.set(val / 100.0);
        }
    };

    let on_max_freq_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let select: web_sys::HtmlSelectElement = target.unchecked_into();
        let freq = match select.value().as_str() {
            "auto" => None,
            v => v.parse::<f64>().ok().map(|khz| khz * 1000.0),
        };
        state.max_display_freq.set(freq);
        state.min_display_freq.set(None); // reset to 0 Hz on manual override
    };

    view! {
        <div class="sidebar-panel">
            // Max display frequency section (first)
            <div class="setting-group">
                <div class="setting-group-title">"Display"</div>
                <div class="setting-row">
                    <span class="setting-label">"Max freq"</span>
                    <select
                        class="setting-select"
                        on:change=on_max_freq_change
                        prop:value=move || match state.max_display_freq.get() {
                            None => "auto".to_string(),
                            Some(hz) => format!("{}", (hz / 1000.0) as u32),
                        }
                    >
                        <option value="auto">"Auto"</option>
                        <option value="50">"50 kHz"</option>
                        <option value="100">"100 kHz"</option>
                        <option value="150">"150 kHz"</option>
                        <option value="200">"200 kHz"</option>
                        <option value="250">"250 kHz"</option>
                    </select>
                </div>
            </div>

            // Movement settings (shown when Movement view is active)
            {move || {
                if state.main_view.get() == MainView::Movement {
                    view! {
                        <div class="setting-group">
                            <div class="setting-group-title">"Movement"</div>
                            <div class="setting-row">
                                <span class="setting-label">"Algorithm"</span>
                                <select
                                    class="setting-select"
                                    on:change=on_display_change
                                    prop:value=move || match state.spectrogram_display.get() {
                                        SpectrogramDisplay::MovementCentroid => "centroid",
                                        SpectrogramDisplay::MovementGradient => "gradient",
                                        SpectrogramDisplay::MovementFlow => "flow",
                                    }
                                >
                                    <option value="flow">"Flow"</option>
                                    <option value="centroid">"Centroid"</option>
                                    <option value="gradient">"Gradient"</option>
                                </select>
                            </div>
                            <div class="setting-row">
                                <span class="setting-label">"Intensity gate"</span>
                                <div class="setting-slider-row">
                                    <input
                                        type="range"
                                        class="setting-range"
                                        min="0"
                                        max="100"
                                        step="1"
                                        prop:value=move || (state.mv_intensity_gate.get() * 100.0).round().to_string()
                                        on:input=on_intensity_gate_change
                                    />
                                    <span class="setting-value">{move || format!("{}%", (state.mv_intensity_gate.get() * 100.0).round() as u32)}</span>
                                </div>
                            </div>
                            <div class="setting-row">
                                <span class="setting-label">"Movement gate"</span>
                                <div class="setting-slider-row">
                                    <input
                                        type="range"
                                        class="setting-range"
                                        min="0"
                                        max="100"
                                        step="1"
                                        prop:value=move || (state.mv_movement_gate.get() * 100.0).round().to_string()
                                        on:input=on_movement_gate_change
                                    />
                                    <span class="setting-value">{move || format!("{}%", (state.mv_movement_gate.get() * 100.0).round() as u32)}</span>
                                </div>
                            </div>
                            <div class="setting-row">
                                <span class="setting-label">"Opacity"</span>
                                <div class="setting-slider-row">
                                    <input
                                        type="range"
                                        class="setting-range"
                                        min="0"
                                        max="100"
                                        step="1"
                                        prop:value=move || (state.mv_opacity.get() * 100.0).to_string()
                                        on:input=on_opacity_change
                                    />
                                    <span class="setting-value">{move || format!("{}%", (state.mv_opacity.get() * 100.0) as u32)}</span>
                                </div>
                            </div>
                        </div>
                    }.into_any()
                } else {
                    view! { <span></span> }.into_any()
                }
            }}
        </div>
    }
}

#[component]
pub(crate) fn SelectionPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    let analysis = move || {
        let selection = state.selection.get()?;
        let dragging = state.is_dragging.get();
        let files = state.files.get();
        let idx = state.current_file_index.get()?;
        let file = files.get(idx)?;

        let sr = file.audio.sample_rate;
        let start = ((selection.time_start * sr as f64) as usize).min(file.audio.samples.len());
        let end = ((selection.time_end * sr as f64) as usize).min(file.audio.samples.len());

        if end <= start {
            return None;
        }

        let duration = selection.time_end - selection.time_start;
        let frames = end - start;

        let (crossing_count, estimated_freq) = if dragging {
            (None, None)
        } else {
            let slice = &file.audio.samples[start..end];
            let zc = zero_crossing_frequency(slice, sr);
            (Some(zc.crossing_count), Some(zc.estimated_frequency_hz))
        };

        Some((duration, frames, crossing_count, estimated_freq, selection.freq_low, selection.freq_high))
    };

    view! {
        <div class="sidebar-panel">
            {move || {
                match analysis() {
                    Some((duration, frames, crossing_count, estimated_freq, freq_low, freq_high)) => {
                        view! {
                            <div class="setting-group">
                                <div class="setting-group-title">"Selection"</div>
                                <div class="setting-row">
                                    <span class="setting-label">"Duration"</span>
                                    <span class="setting-value">{format!("{:.3} s", duration)}</span>
                                </div>
                                <div class="setting-row">
                                    <span class="setting-label">"Frames"</span>
                                    <span class="setting-value">{format!("{}", frames)}</span>
                                </div>
                                <div class="setting-row">
                                    <span class="setting-label">"Freq range"</span>
                                    <span class="setting-value">{format!("{:.0} – {:.0} kHz", freq_low / 1000.0, freq_high / 1000.0)}</span>
                                </div>
                                <div class="setting-row">
                                    <span class="setting-label">"ZC count"</span>
                                    <span class="setting-value">{match crossing_count { Some(c) => format!("{c}"), None => "...".into() }}</span>
                                </div>
                                <div class="setting-row">
                                    <span class="setting-label">"ZC est. freq"</span>
                                    <span class="setting-value">{match estimated_freq { Some(f) => format!("~{:.1} kHz", f / 1000.0), None => "...".into() }}</span>
                                </div>
                            </div>
                        }.into_any()
                    }
                    None => {
                        view! {
                            <div class="sidebar-panel-empty">"No selection"</div>
                        }.into_any()
                    }
                }
            }}
        </div>
    }
}
