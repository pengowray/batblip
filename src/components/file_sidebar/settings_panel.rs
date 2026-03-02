use leptos::prelude::*;
use wasm_bindgen::JsCast;
use crate::state::{AppState, FlowColorScheme, MainView, SpectrogramDisplay};
use crate::dsp::zero_crossing::zero_crossing_frequency;

#[component]
pub(crate) fn SpectrogramSettingsPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    view! {
        <div class="sidebar-panel">
            // Gain/Range/Contrast — always shown (applies to all tile modes)
            <div class="setting-group">
                <div class="setting-group-title">"Intensity"</div>
                <div class="setting-row">
                    <span class="setting-label">{move || {
                        if state.display_auto_gain.get() {
                            "Gain: auto".to_string()
                        } else {
                            format!("Gain: {:+.0} dB", state.spect_gain_db.get())
                        }
                    }}</span>
                    <input
                        type="range"
                        class="setting-range"
                        min="-40"
                        max="40"
                        step="1"
                        prop:value=move || state.spect_gain_db.get().to_string()
                        on:input=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                            if let Ok(v) = input.value().parse::<f32>() {
                                state.spect_gain_db.set(v);
                                state.display_auto_gain.set(false);
                            }
                        }
                    />
                </div>
                <div class="setting-row">
                    <span class="setting-label">{move || format!("Range: {:.0} dB", state.spect_range_db.get())}</span>
                    <input
                        type="range"
                        class="setting-range"
                        min="20"
                        max="120"
                        step="5"
                        prop:value=move || state.spect_range_db.get().to_string()
                        on:input=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                            if let Ok(v) = input.value().parse::<f32>() {
                                state.spect_range_db.set(v);
                                state.spect_floor_db.set(-v);
                            }
                        }
                    />
                </div>
                <div class="setting-row">
                    <span class="setting-label">{move || {
                        let g = state.spect_gamma.get();
                        if g == 1.0 { "Contrast: linear".to_string() }
                        else { format!("Contrast: {:.2}", g) }
                    }}</span>
                    <input
                        type="range"
                        class="setting-range"
                        min="0.2"
                        max="3.0"
                        step="0.05"
                        prop:value=move || state.spect_gamma.get().to_string()
                        on:input=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                            if let Ok(v) = input.value().parse::<f32>() {
                                state.spect_gamma.set(v);
                            }
                        }
                    />
                </div>
                <div class="setting-row">
                    <label class="setting-label" style="display:flex;align-items:center;gap:4px;cursor:pointer">
                        <input
                            type="checkbox"
                            prop:checked=move || state.display_auto_gain.get()
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let input: web_sys::HtmlInputElement = target.unchecked_into();
                                state.display_auto_gain.set(input.checked());
                            }
                        />
                        "Auto gain"
                    </label>
                </div>
                <div class="setting-row">
                    <label class="setting-label" style="display:flex;align-items:center;gap:4px;cursor:pointer">
                        <input
                            type="checkbox"
                            prop:checked=move || state.display_eq.get()
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let input: web_sys::HtmlInputElement = target.unchecked_into();
                                state.display_eq.set(input.checked());
                            }
                        />
                        "Show EQ"
                    </label>
                </div>
                <div class="setting-row">
                    <label class="setting-label" style="display:flex;align-items:center;gap:4px;cursor:pointer">
                        <input
                            type="checkbox"
                            prop:checked=move || state.display_noise_filter.get()
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let input: web_sys::HtmlInputElement = target.unchecked_into();
                                state.display_noise_filter.set(input.checked());
                            }
                        />
                        "Show noise filter"
                    </label>
                </div>
                <div class="setting-row">
                    <button
                        class="setting-button"
                        on:click=move |_| {
                            state.spect_gain_db.set(0.0);
                            state.spect_floor_db.set(-80.0);
                            state.spect_range_db.set(80.0);
                            state.spect_gamma.set(1.0);
                            state.display_auto_gain.set(false);
                            state.display_eq.set(false);
                            state.display_noise_filter.set(false);
                        }
                    >"Reset"</button>
                </div>
                <div class="setting-row">
                    <span class="setting-label">"FFT size"</span>
                    <select
                        class="setting-select"
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let select: web_sys::HtmlSelectElement = target.unchecked_into();
                            if let Ok(v) = select.value().parse::<usize>() {
                                state.spect_fft_size.set(v);
                            }
                        }
                    >
                        {move || {
                            let current = state.spect_fft_size.get();
                            [256, 512, 1024, 2048, 4096, 8192].into_iter().map(|sz| {
                                let s = sz.to_string();
                                let s2 = s.clone();
                                view! { <option value={s} selected=move || sz == current>{s2}</option> }
                            }).collect::<Vec<_>>()
                        }}
                    </select>
                </div>
                <div class="setting-row">
                    <label class="setting-label" style="display:flex;align-items:center;gap:4px;cursor:pointer"
                        title="Sharpen time-frequency localization using the reassignment method (3x FFT cost)">
                        <input
                            type="checkbox"
                            prop:checked=move || state.reassign_enabled.get()
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let input: web_sys::HtmlInputElement = target.unchecked_into();
                                state.reassign_enabled.set(input.checked());
                            }
                        />
                        "Reassignment"
                    </label>
                </div>
                <div class="setting-row">
                    <label class="setting-label" style="display:flex;align-items:center;gap:4px;cursor:pointer">
                        <input
                            type="checkbox"
                            prop:checked=move || state.debug_tiles.get()
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let input: web_sys::HtmlInputElement = target.unchecked_into();
                                state.debug_tiles.set(input.checked());
                            }
                        />
                        "Debug tiles"
                    </label>
                </div>
            </div>

            // Flow-specific settings (shown only when Flow view is active)
            {move || {
                if state.main_view.get() == MainView::Flow {
                    let display = state.spectrogram_display.get();
                    let _ = display; // used for reactivity trigger above
                    view! {
                        <div class="setting-group">
                            <div class="setting-group-title">"Color"</div>
                            <div class="setting-row">
                                <span class="setting-label">"Algorithm"</span>
                                <select
                                    class="setting-select"
                                    on:change=move |ev: web_sys::Event| {
                                        let target = ev.target().unwrap();
                                        let select: web_sys::HtmlSelectElement = target.unchecked_into();
                                        let mode = match select.value().as_str() {
                                            "coherence" => SpectrogramDisplay::PhaseCoherence,
                                            "centroid" => SpectrogramDisplay::FlowCentroid,
                                            "gradient" => SpectrogramDisplay::FlowGradient,
                                            "phase" => SpectrogramDisplay::Phase,
                                            _ => SpectrogramDisplay::FlowOptical,
                                        };
                                        state.spectrogram_display.set(mode);
                                    }
                                    prop:value=move || match state.spectrogram_display.get() {
                                        SpectrogramDisplay::FlowOptical => "flow",
                                        SpectrogramDisplay::PhaseCoherence => "coherence",
                                        SpectrogramDisplay::FlowCentroid => "centroid",
                                        SpectrogramDisplay::FlowGradient => "gradient",
                                        SpectrogramDisplay::Phase => "phase",
                                    }
                                >
                                    <option value="flow">"Optical"</option>
                                    <option value="coherence">"Phase Coherence"</option>
                                    <option value="centroid">"Centroid"</option>
                                    <option value="gradient">"Gradient"</option>
                                    <option value="phase">"Phase"</option>
                                </select>
                            </div>
                            // Color scheme selector (only for flow algorithms, not phase)
                            {move || {
                                let display = state.spectrogram_display.get();
                                let is_flow_algo = matches!(display,
                                    SpectrogramDisplay::FlowOptical |
                                    SpectrogramDisplay::FlowCentroid |
                                    SpectrogramDisplay::FlowGradient
                                );
                                if is_flow_algo {
                                    view! {
                                        <div class="setting-row">
                                            <span class="setting-label">"Color scheme"</span>
                                            <select
                                                class="setting-select"
                                                on:change=move |ev: web_sys::Event| {
                                                    let target = ev.target().unwrap();
                                                    let select: web_sys::HtmlSelectElement = target.unchecked_into();
                                                    let scheme = match select.value().as_str() {
                                                        "coolwarm" => FlowColorScheme::CoolWarm,
                                                        "tealorange" => FlowColorScheme::TealOrange,
                                                        "purplegreen" => FlowColorScheme::PurpleGreen,
                                                        "spectral" => FlowColorScheme::Spectral,
                                                        _ => FlowColorScheme::RedBlue,
                                                    };
                                                    state.flow_color_scheme.set(scheme);
                                                }
                                                prop:value=move || match state.flow_color_scheme.get() {
                                                    FlowColorScheme::RedBlue => "redblue",
                                                    FlowColorScheme::CoolWarm => "coolwarm",
                                                    FlowColorScheme::TealOrange => "tealorange",
                                                    FlowColorScheme::PurpleGreen => "purplegreen",
                                                    FlowColorScheme::Spectral => "spectral",
                                                }
                                            >
                                                <option value="redblue">"Red-Blue"</option>
                                                <option value="coolwarm">"Cool-Warm"</option>
                                                <option value="tealorange">"Teal-Orange"</option>
                                                <option value="purplegreen">"Purple-Green"</option>
                                                <option value="spectral">"Spectral"</option>
                                            </select>
                                        </div>
                                    }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                            <div class="setting-row">
                                <span class="setting-label">"Intensity gate"</span>
                                <div class="setting-slider-row">
                                    <input
                                        type="range"
                                        class="setting-range"
                                        min="0"
                                        max="100"
                                        step="1"
                                        prop:value=move || (state.flow_intensity_gate.get() * 100.0).round().to_string()
                                        on:input=move |ev: web_sys::Event| {
                                            let target = ev.target().unwrap();
                                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                                            if let Ok(val) = input.value().parse::<f32>() {
                                                state.flow_intensity_gate.set(val / 100.0);
                                            }
                                        }
                                    />
                                    <span class="setting-value">{move || format!("{}%", (state.flow_intensity_gate.get() * 100.0).round() as u32)}</span>
                                </div>
                            </div>
                            <div class="setting-row">
                                <span class="setting-label">"Color gain"</span>
                                <div class="setting-slider-row">
                                    <input
                                        type="range"
                                        class="setting-range"
                                        min="0.5"
                                        max="10.0"
                                        step="0.5"
                                        prop:value=move || state.flow_shift_gain.get().to_string()
                                        on:input=move |ev: web_sys::Event| {
                                            let target = ev.target().unwrap();
                                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                                            if let Ok(val) = input.value().parse::<f32>() {
                                                state.flow_shift_gain.set(val);
                                            }
                                        }
                                    />
                                    <span class="setting-value">{move || format!("{:.1}x", state.flow_shift_gain.get())}</span>
                                </div>
                            </div>
                            <div class="setting-row">
                                <span class="setting-label">{move || {
                                    let g = state.flow_color_gamma.get();
                                    if g == 1.0 { "Color contrast: linear".to_string() }
                                    else { format!("Color contrast: {:.2}", g) }
                                }}</span>
                                <div class="setting-slider-row">
                                    <input
                                        type="range"
                                        class="setting-range"
                                        min="0.2"
                                        max="3.0"
                                        step="0.05"
                                        prop:value=move || state.flow_color_gamma.get().to_string()
                                        on:input=move |ev: web_sys::Event| {
                                            let target = ev.target().unwrap();
                                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                                            if let Ok(val) = input.value().parse::<f32>() {
                                                state.flow_color_gamma.set(val);
                                            }
                                        }
                                    />
                                </div>
                            </div>
                            // Flow gate — threshold for minimum shift/deviation magnitude to show color
                            <div class="setting-row">
                                <span class="setting-label">"Flow gate"</span>
                                <div class="setting-slider-row">
                                    <input
                                        type="range"
                                        class="setting-range"
                                        min="0"
                                        max="100"
                                        step="1"
                                        prop:value=move || (state.flow_gate.get() * 100.0).round().to_string()
                                        on:input=move |ev: web_sys::Event| {
                                            let target = ev.target().unwrap();
                                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                                            if let Ok(val) = input.value().parse::<f32>() {
                                                state.flow_gate.set(val / 100.0);
                                            }
                                        }
                                    />
                                    <span class="setting-value">{move || format!("{}%", (state.flow_gate.get() * 100.0).round() as u32)}</span>
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
