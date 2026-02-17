use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::{CanvasRenderingContext2d, DragEvent, File, FileReader, HtmlCanvasElement, ImageData};
use crate::audio::loader::load_audio;
use crate::dsp::fft::{compute_preview, compute_spectrogram};
use crate::dsp::zero_crossing::zero_crossing_frequency;
use crate::state::{AppState, LoadedFile, SidebarTab, SpectrogramDisplay};
use crate::types::{PreviewImage, SpectrogramData};

fn format_guano_key(key: &str) -> String {
    match key {
        "GUANO|Version" => "GUANO Version".into(),
        "BatGizmo|App|DeviceModel" => "Device Model".into(),
        "BatGizmo|App|Version" => "BatGizmo Version".into(),
        "BatGizmo|App|TriggerType" => "Trigger Type".into(),
        "Loc|Lat" => "Latitude".into(),
        "Loc|Lon" => "Longitude".into(),
        "Loc|Elev" => "Elevation".into(),
        "Filter|HP" => "High-pass Filter".into(),
        "Filter|LP" => "Low-pass Filter".into(),
        "Species|Auto" => "Species (Auto)".into(),
        "Species|Manual" => "Species (Manual)".into(),
        "TE" => "Time Expansion".into(),
        "Samplerate" => "Sample Rate".into(),
        "Length" => "Length".into(),
        _ => key.replace('|', " "),
    }
}

#[component]
pub fn FileSidebar() -> impl IntoView {
    let state = expect_context::<AppState>();

    view! {
        <div class="sidebar">
            <div class="sidebar-tabs">
                <button
                    class=move || if state.sidebar_tab.get() == SidebarTab::Files { "sidebar-tab active" } else { "sidebar-tab" }
                    on:click=move |_| state.sidebar_tab.set(SidebarTab::Files)
                >
                    "Files"
                </button>
                <button
                    class=move || if state.sidebar_tab.get() == SidebarTab::Spectrogram { "sidebar-tab active" } else { "sidebar-tab" }
                    on:click=move |_| state.sidebar_tab.set(SidebarTab::Spectrogram)
                >
                    "Display"
                </button>
                <button
                    class=move || if state.sidebar_tab.get() == SidebarTab::Selection { "sidebar-tab active" } else { "sidebar-tab" }
                    on:click=move |_| state.sidebar_tab.set(SidebarTab::Selection)
                >
                    "Selection"
                </button>
                <button
                    class=move || if state.sidebar_tab.get() == SidebarTab::Filter { "sidebar-tab active" } else { "sidebar-tab" }
                    on:click=move |_| state.sidebar_tab.set(SidebarTab::Filter)
                >
                    "Filter"
                </button>
                <button
                    class=move || if state.sidebar_tab.get() == SidebarTab::Metadata { "sidebar-tab active" } else { "sidebar-tab" }
                    on:click=move |_| state.sidebar_tab.set(SidebarTab::Metadata)
                >
                    "Info"
                </button>
            </div>
            {move || match state.sidebar_tab.get() {
                SidebarTab::Files => view! { <FilesPanel /> }.into_any(),
                SidebarTab::Spectrogram => view! { <SpectrogramSettingsPanel /> }.into_any(),
                SidebarTab::Selection => view! { <SelectionPanel /> }.into_any(),
                SidebarTab::Filter => view! { <FilterPanel /> }.into_any(),
                SidebarTab::Metadata => view! { <MetadataPanel /> }.into_any(),
            }}
        </div>
    }
}

#[component]
fn FilesPanel() -> impl IntoView {
    let state = expect_context::<AppState>();
    let drag_over = RwSignal::new(false);
    let files = state.files;
    let current_idx = state.current_file_index;
    let loading_count = state.loading_count;

    let on_dragover = move |ev: DragEvent| {
        ev.prevent_default();
        drag_over.set(true);
    };

    let on_dragleave = move |_: DragEvent| {
        drag_over.set(false);
    };

    let state_for_drop = state.clone();
    let on_drop = move |ev: DragEvent| {
        ev.prevent_default();
        drag_over.set(false);

        let Some(dt) = ev.data_transfer() else { return };
        let Some(file_list) = dt.files() else { return };

        for i in 0..file_list.length() {
            let Some(file) = file_list.get(i) else { continue };
            let state = state_for_drop.clone();
            state.loading_count.update(|c| *c += 1);
            spawn_local(async move {
                match read_and_load_file(file, state.clone()).await {
                    Ok(()) => {}
                    Err(e) => log::error!("Failed to load file: {e}"),
                }
                state.loading_count.update(|c| *c = c.saturating_sub(1));
            });
        }
    };

    view! {
        <div
            class=move || if drag_over.get() { "drop-zone drag-over" } else { "drop-zone" }
            on:dragover=on_dragover
            on:dragleave=on_dragleave
            on:drop=on_drop
        >
            {move || {
                let file_vec = files.get();
                let lc = loading_count.get();
                if file_vec.is_empty() && lc == 0 {
                    view! {
                        <div class="drop-hint">"Drop WAV/FLAC files here"</div>
                    }.into_any()
                } else {
                    let items: Vec<_> = file_vec.iter().enumerate().map(|(i, f)| {
                        let name = f.name.clone();
                        let dur = f.audio.duration_secs;
                        let sr = f.audio.sample_rate;
                        let preview = f.preview.clone();
                        let is_active = move || current_idx.get() == Some(i);
                        let on_click = move |_| {
                            current_idx.set(Some(i));
                        };
                        view! {
                            <div
                                class=move || if is_active() { "file-item active" } else { "file-item" }
                                on:click=on_click
                            >
                                {preview.map(|pv| view! { <PreviewCanvas preview=pv /> })}
                                <div class="file-item-name">{name}</div>
                                <div class="file-item-info">
                                    {format!("{:.1}s  {}kHz", dur, sr / 1000)}
                                </div>
                            </div>
                        }
                    }).collect();
                    view! {
                        <div class="file-list">
                            {items}
                            {move || {
                                let lc = loading_count.get();
                                if lc > 0 {
                                    view! {
                                        <div class="file-item loading">
                                            <div class="loading-spinner"></div>
                                            {format!("Loading {} file{}...", lc, if lc > 1 { "s" } else { "" })}
                                        </div>
                                    }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                        </div>
                    }.into_any()
                }
            }}
        </div>
    }
}

#[component]
fn SpectrogramSettingsPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    let on_toggle_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        state.mv_enabled.set(input.checked());
    };

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

            // Movement overlay section
            <div class="setting-group">
                <div class="setting-group-title">"Movement overlay"</div>
                <div class="setting-row">
                    <span class="setting-label">"Enabled"</span>
                    <input
                        type="checkbox"
                        class="setting-checkbox"
                        prop:checked=move || state.mv_enabled.get()
                        on:change=on_toggle_change
                    />
                </div>
                {move || {
                    if state.mv_enabled.get() {
                        view! {
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
                        }.into_any()
                    } else {
                        view! { <span></span> }.into_any()
                    }
                }}
            </div>
        </div>
    }
}

#[component]
fn SelectionPanel() -> impl IntoView {
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

#[component]
fn FilterPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    let on_enable_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        state.filter_enabled.set(input.checked());
    };

    let set_band_mode = move |mode: u8| {
        state.filter_band_mode.set(mode);
    };

    let on_set_from_selection = move |_: web_sys::MouseEvent| {
        if let Some(sel) = state.selection.get_untracked() {
            if sel.freq_low > 0.0 && sel.freq_high > sel.freq_low {
                state.filter_freq_low.set(sel.freq_low);
                state.filter_freq_high.set(sel.freq_high);
                state.filter_set_from_selection.set(true);
            }
        }
    };

    let make_db_handler = |signal: RwSignal<f64>| {
        move |ev: web_sys::Event| {
            let target = ev.target().unwrap();
            let input: web_sys::HtmlInputElement = target.unchecked_into();
            if let Ok(val) = input.value().parse::<f64>() {
                signal.set(val);
            }
        }
    };

    let on_below_change = make_db_handler(state.filter_db_below);
    let on_selected_change = make_db_handler(state.filter_db_selected);
    let on_harmonics_change = make_db_handler(state.filter_db_harmonics);
    let on_above_change = make_db_handler(state.filter_db_above);

    let hover_signal = state.filter_hovering_band;
    let make_hover_enter = move |band: u8| {
        move |_: web_sys::MouseEvent| {
            hover_signal.set(Some(band));
        }
    };
    let on_hover_leave = move |_: web_sys::MouseEvent| {
        hover_signal.set(None);
    };

    let has_selection = move || state.selection.get().is_some();
    let band_mode = move || state.filter_band_mode.get();
    let selection_under_octave = move || {
        let low = state.filter_freq_low.get();
        let high = state.filter_freq_high.get();
        low > 0.0 && high / low < 2.0
    };
    let show_harmonics = move || band_mode() >= 4 && selection_under_octave();
    let show_above = move || band_mode() >= 3;

    let quality = move || state.filter_quality.get();
    let set_quality = move |q: crate::state::FilterQuality| {
        state.filter_quality.set(q);
    };

    let on_het_cutoff_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        if let Ok(val) = input.value().parse::<f64>() {
            state.het_cutoff.set(val * 1000.0);
        }
    };

    view! {
        <div class="sidebar-panel filter-panel">
            <div class="setting-group">
                <div class="setting-row">
                    <span class="setting-label">"Enable EQ"</span>
                    <input
                        type="checkbox"
                        class="setting-checkbox"
                        prop:checked=move || state.filter_enabled.get()
                        on:change=on_enable_change
                    />
                </div>
            </div>

            {move || {
                if !state.filter_enabled.get() {
                    return view! { <span></span> }.into_any();
                }

                view! {
                    <div class="setting-group">
                        <div class="setting-group-title">"Bands"</div>
                        <div class="filter-band-mode">
                            <button
                                class=move || if band_mode() == 2 { "mode-btn active" } else { "mode-btn" }
                                on:click=move |_| set_band_mode(2)
                            >"2"</button>
                            <button
                                class=move || if band_mode() == 3 { "mode-btn active" } else { "mode-btn" }
                                on:click=move |_| set_band_mode(3)
                            >"3"</button>
                            <button
                                class=move || if band_mode() == 4 { "mode-btn active" } else { "mode-btn" }
                                on:click=move |_| set_band_mode(4)
                            >"4"</button>
                        </div>
                    </div>

                    <div class="setting-group">
                        <div class="setting-group-title">"Quality"</div>
                        <div class="filter-band-mode">
                            <button
                                class=move || if quality() == crate::state::FilterQuality::Fast { "mode-btn active" } else { "mode-btn" }
                                on:click=move |_| set_quality(crate::state::FilterQuality::Fast)
                                title="IIR band-split — low latency, softer band edges"
                            >"Fast"</button>
                            <button
                                class=move || if quality() == crate::state::FilterQuality::HQ { "mode-btn active" } else { "mode-btn" }
                                on:click=move |_| set_quality(crate::state::FilterQuality::HQ)
                                title="FFT spectral EQ — sharp band edges, higher latency"
                            >"HQ"</button>
                        </div>
                    </div>

                    <div class="setting-group">
                        <button
                            class="filter-set-btn"
                            on:click=on_set_from_selection
                            disabled=move || !has_selection()
                            title="Set frequency range from current spectrogram selection"
                        >
                            "Set from selection"
                        </button>
                        <div class="filter-freq-display">
                            {move || format!("{:.0} – {:.0} kHz",
                                state.filter_freq_low.get() / 1000.0,
                                state.filter_freq_high.get() / 1000.0
                            )}
                        </div>
                    </div>

                    <div class="setting-group">
                        <div class="setting-group-title">"EQ"</div>

                        // Above slider (3+ band) — top, highest freq
                        {move || {
                            if show_above() {
                                view! {
                                    <div class="setting-row"
                                        on:mouseenter=make_hover_enter(3)
                                        on:mouseleave=on_hover_leave
                                    >
                                        <span class="setting-label">"Above"</span>
                                        <div class="setting-slider-row">
                                            <input
                                                type="range"
                                                class="setting-range"
                                                min="-60"
                                                max="6"
                                                step="1"
                                                prop:value=move || state.filter_db_above.get().to_string()
                                                on:input=on_above_change
                                            />
                                            <span class="setting-value">{move || format!("{:.0} dB", state.filter_db_above.get())}</span>
                                        </div>
                                    </div>
                                }.into_any()
                            } else {
                                view! { <span></span> }.into_any()
                            }
                        }}

                        // Harmonics slider (4-band only, selection < 1 octave)
                        {move || {
                            if show_harmonics() {
                                view! {
                                    <div class="setting-row"
                                        on:mouseenter=make_hover_enter(2)
                                        on:mouseleave=on_hover_leave
                                    >
                                        <span class="setting-label">"Harmonics"</span>
                                        <div class="setting-slider-row">
                                            <input
                                                type="range"
                                                class="setting-range"
                                                min="-60"
                                                max="6"
                                                step="1"
                                                prop:value=move || state.filter_db_harmonics.get().to_string()
                                                on:input=on_harmonics_change
                                            />
                                            <span class="setting-value">{move || format!("{:.0} dB", state.filter_db_harmonics.get())}</span>
                                        </div>
                                    </div>
                                }.into_any()
                            } else {
                                view! { <span></span> }.into_any()
                            }
                        }}

                        // Selected slider
                        <div class="setting-row"
                            on:mouseenter=make_hover_enter(1)
                            on:mouseleave=on_hover_leave
                        >
                            <span class="setting-label">"Selected"</span>
                            <div class="setting-slider-row">
                                <input
                                    type="range"
                                    class="setting-range"
                                    min="-60"
                                    max="6"
                                    step="1"
                                    prop:value=move || state.filter_db_selected.get().to_string()
                                    on:input=on_selected_change
                                />
                                <span class="setting-value">{move || format!("{:.0} dB", state.filter_db_selected.get())}</span>
                            </div>
                        </div>

                        // Below slider — bottom, lowest freq
                        <div class="setting-row"
                            on:mouseenter=make_hover_enter(0)
                            on:mouseleave=on_hover_leave
                        >
                            <span class="setting-label">"Below"</span>
                            <div class="setting-slider-row">
                                <input
                                    type="range"
                                    class="setting-range"
                                    min="-60"
                                    max="6"
                                    step="1"
                                    prop:value=move || state.filter_db_below.get().to_string()
                                    on:input=on_below_change
                                />
                                <span class="setting-value">{move || format!("{:.0} dB", state.filter_db_below.get())}</span>
                            </div>
                        </div>
                    </div>

                    // Mode-specific filter chain
                    <div class="setting-group">
                        <div class="setting-group-title">"Mode filters"</div>
                        {move || {
                            let mode = state.playback_mode.get();
                            match mode {
                                crate::state::PlaybackMode::Heterodyne => {
                                    view! {
                                        <div class="setting-row"
                                            on:mouseenter=move |_| state.het_interacting.set(true)
                                            on:mouseleave=move |_| state.het_interacting.set(false)
                                        >
                                            <span class="setting-label">"HET LP"</span>
                                            <div class="setting-slider-row">
                                                <input
                                                    type="range"
                                                    class="setting-range"
                                                    min="1"
                                                    max="30"
                                                    step="1"
                                                    prop:value=move || (state.het_cutoff.get() / 1000.0).to_string()
                                                    on:input=on_het_cutoff_change
                                                />
                                                <span class="setting-value">{move || format!("{:.0} kHz", state.het_cutoff.get() / 1000.0)}</span>
                                            </div>
                                        </div>
                                    }.into_any()
                                }
                                crate::state::PlaybackMode::ZeroCrossing => {
                                    view! {
                                        <div class="filter-mode-info">"ZC: bandpass 15\u{2013}150 kHz"</div>
                                    }.into_any()
                                }
                                _ => view! { <span></span> }.into_any(),
                            }
                        }}
                    </div>
                }.into_any()
            }}
        </div>
    }
}

#[component]
fn MetadataPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    view! {
        <div class="sidebar-panel">
            {move || {
                let files = state.files.get();
                let idx = state.current_file_index.get();
                let file = idx.and_then(|i| files.get(i));

                match file {
                    None => view! {
                        <div class="sidebar-panel-empty">"No file selected"</div>
                    }.into_any(),
                    Some(f) => {
                        let meta = &f.audio.metadata;
                        let size_str = format_file_size(meta.file_size);
                        let guano_fields: Vec<_> = meta.guano.as_ref()
                            .map(|g| g.fields.clone())
                            .unwrap_or_default();
                        let has_guano = !guano_fields.is_empty();

                        view! {
                            <div class="setting-group">
                                <div class="setting-group-title">"File"</div>
                                <div class="setting-row">
                                    <span class="setting-label">"Name"</span>
                                    <span class="setting-value metadata-value" title=f.name.clone()>{f.name.clone()}</span>
                                </div>
                                <div class="setting-row">
                                    <span class="setting-label">"Format"</span>
                                    <span class="setting-value">{meta.format}</span>
                                </div>
                                <div class="setting-row">
                                    <span class="setting-label">"Duration"</span>
                                    <span class="setting-value">{format!("{:.3} s", f.audio.duration_secs)}</span>
                                </div>
                                <div class="setting-row">
                                    <span class="setting-label">"Sample rate"</span>
                                    <span class="setting-value">{format!("{} kHz", f.audio.sample_rate / 1000)}</span>
                                </div>
                                <div class="setting-row">
                                    <span class="setting-label">"Channels"</span>
                                    <span class="setting-value">{f.audio.channels.to_string()}</span>
                                </div>
                                <div class="setting-row">
                                    <span class="setting-label">"Bit depth"</span>
                                    <span class="setting-value">{format!("{}-bit", meta.bits_per_sample)}</span>
                                </div>
                                <div class="setting-row">
                                    <span class="setting-label">"File size"</span>
                                    <span class="setting-value">{size_str}</span>
                                </div>
                            </div>
                            {if has_guano {
                                let rows: Vec<_> = guano_fields.into_iter().map(|(k, v)| {
                                    let display_key = format_guano_key(&k);
                                    let label_title = k.clone();
                                    let value_title = v.clone();
                                    view! {
                                        <div class="setting-row">
                                            <span class="setting-label" title=label_title>{display_key}</span>
                                            <span class="setting-value metadata-value" title=value_title>{v}</span>
                                        </div>
                                    }
                                }).collect();
                                view! {
                                    <div class="setting-group">
                                        <div class="setting-group-title">"GUANO"</div>
                                        {rows}
                                    </div>
                                }.into_any()
                            } else {
                                view! { <span></span> }.into_any()
                            }}
                        }.into_any()
                    }
                }
            }}
        </div>
    }
}

fn format_file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[component]
fn PreviewCanvas(preview: PreviewImage) -> impl IntoView {
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();
    let pv = preview.clone();

    Effect::new(move || {
        let Some(el) = canvas_ref.get() else { return };
        let canvas: &HtmlCanvasElement = el.as_ref();
        canvas.set_width(pv.width);
        canvas.set_height(pv.height);
        let ctx = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into::<CanvasRenderingContext2d>()
            .unwrap();
        let clamped = Clamped(pv.pixels.as_slice());
        if let Ok(img) = ImageData::new_with_u8_clamped_array_and_sh(clamped, pv.width, pv.height) {
            let _ = ctx.put_image_data(&img, 0.0, 0.0);
        }
    });

    view! {
        <canvas
            node_ref=canvas_ref
            class="file-preview-canvas"
        />
    }
}

async fn read_and_load_file(file: File, state: AppState) -> Result<(), String> {
    let name = file.name();
    let bytes = read_file_bytes(&file).await?;

    let audio = load_audio(&bytes)?;
    log::info!(
        "Loaded {}: {} samples, {} Hz, {:.2}s",
        name,
        audio.samples.len(),
        audio.sample_rate,
        audio.duration_secs
    );

    // Phase 1: fast preview
    let preview = compute_preview(&audio, 256, 128);
    let audio_for_stft = audio.clone();
    let name_check = name.clone();

    let placeholder_spec = SpectrogramData {
        columns: Vec::new(),
        freq_resolution: 0.0,
        time_resolution: 0.0,
        max_freq: audio.sample_rate as f64 / 2.0,
        sample_rate: audio.sample_rate,
    };

    let file_index;
    {
        let mut idx = 0;
        state.files.update(|files| {
            idx = files.len();
            files.push(LoadedFile {
                name,
                audio,
                spectrogram: placeholder_spec,
                preview: Some(preview),
            });
            if files.len() == 1 {
                state.current_file_index.set(Some(0));
            }
        });
        file_index = idx;
    }

    // Yield to let the UI render the preview
    let yield_promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback(&resolve)
            .unwrap();
    });
    JsFuture::from(yield_promise).await.ok();

    // Phase 2: full spectrogram
    let spectrogram = compute_spectrogram(&audio_for_stft, 2048, 512);
    log::info!(
        "Spectrogram: {} columns, freq_res={:.1} Hz, time_res={:.4}s",
        spectrogram.columns.len(),
        spectrogram.freq_resolution,
        spectrogram.time_resolution
    );

    state.files.update(|files| {
        if let Some(f) = files.get_mut(file_index) {
            if f.name == name_check {
                f.spectrogram = spectrogram;
            }
        }
    });

    Ok(())
}

async fn read_file_bytes(file: &File) -> Result<Vec<u8>, String> {
    let reader = FileReader::new().map_err(|e| format!("FileReader: {e:?}"))?;
    let reader_clone = reader.clone();

    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let resolve_clone = resolve.clone();
        let reject_clone = reject.clone();

        let onload = Closure::once(move |_: web_sys::Event| {
            resolve_clone.call0(&JsValue::NULL).unwrap();
        });
        let onerror = Closure::once(move |_: web_sys::Event| {
            reject_clone.call0(&JsValue::NULL).unwrap();
        });

        reader_clone.set_onloadend(Some(onload.as_ref().unchecked_ref()));
        reader_clone.set_onerror(Some(onerror.as_ref().unchecked_ref()));

        onload.forget();
        onerror.forget();
    });

    reader
        .read_as_array_buffer(file)
        .map_err(|e| format!("read_as_array_buffer: {e:?}"))?;

    JsFuture::from(promise)
        .await
        .map_err(|e| format!("FileReader await: {e:?}"))?;

    let result = reader.result().map_err(|e| format!("result: {e:?}"))?;
    let array_buffer = result
        .dyn_into::<js_sys::ArrayBuffer>()
        .map_err(|_| "Expected ArrayBuffer".to_string())?;
    let uint8_array = js_sys::Uint8Array::new(&array_buffer);
    Ok(uint8_array.to_vec())
}
