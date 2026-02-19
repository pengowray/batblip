use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use js_sys;
use wasm_bindgen_futures::JsFuture;
use web_sys::{CanvasRenderingContext2d, DragEvent, File, FileReader, HtmlCanvasElement, HtmlInputElement, ImageData};
use crate::audio::loader::load_audio;
use crate::dsp::fft::{compute_preview, compute_spectrogram};
use crate::dsp::zero_crossing::zero_crossing_frequency;
use crate::audio::playback;
use crate::dsp::bit_analysis::{self, BitCaution};
use crate::dsp::wsnr;
use crate::state::{AppState, LoadedFile, PlaybackMode, SidebarTab, SpectrogramDisplay};
use crate::types::{PreviewImage, SpectrogramData};

/// Returns (section, display_key) for a GUANO field.
/// Known fields return "GUANO" as section; unknown pipe-separated keys
/// return the prefix (e.g. "BatGizmo App") as section and the last segment as display key.
fn categorize_guano_key(key: &str) -> (String, String) {
    let known = match key {
        "Loc|Lat" => Some("Latitude"),
        "Loc|Lon" => Some("Longitude"),
        "Loc|Elev" => Some("Elevation"),
        "Filter|HP" => Some("High-pass Filter"),
        "Filter|LP" => Some("Low-pass Filter"),
        "Species|Auto" => Some("Species (Auto)"),
        "Species|Manual" => Some("Species (Manual)"),
        "TE" => Some("Time Expansion"),
        "Samplerate" => Some("Sample Rate"),
        "Length" => Some("Length"),
        _ => None,
    };
    if let Some(display) = known {
        return ("GUANO".into(), display.into());
    }
    // Unknown key: split on last pipe to get section prefix and short name
    if let Some(pos) = key.rfind('|') {
        let prefix = &key[..pos];
        let short = &key[pos + 1..];
        (prefix.replace('|', " "), short.into())
    } else {
        ("GUANO".into(), key.into())
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
                SidebarTab::PreProcessing => view! { <FilterPanel /> }.into_any(),
                SidebarTab::Analysis => view! { <AnalysisPanel /> }.into_any(),
                SidebarTab::Metadata => view! { <MetadataPanel /> }.into_any(),
            }}
            <div class="sidebar-resize-handle" on:mousedown=on_resize_start></div>
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

    let file_input_ref = NodeRef::<leptos::html::Input>::new();

    let state_for_upload = state.clone();
    let on_upload_click = move |_: web_sys::MouseEvent| {
        if let Some(input) = file_input_ref.get() {
            let el: &HtmlInputElement = input.as_ref();
            el.click();
        }
    };

    let on_file_input_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: HtmlInputElement = target.unchecked_into();
        let Some(file_list) = input.files() else { return };

        for i in 0..file_list.length() {
            let Some(file) = file_list.get(i) else { continue };
            let state = state_for_upload.clone();
            state.loading_count.update(|c| *c += 1);
            spawn_local(async move {
                match read_and_load_file(file, state.clone()).await {
                    Ok(()) => {}
                    Err(e) => log::error!("Failed to load file: {e}"),
                }
                state.loading_count.update(|c| *c = c.saturating_sub(1));
            });
        }

        // Reset the input so the same file can be re-selected
        input.set_value("");
    };

    let demo_entries: RwSignal<Vec<DemoEntry>> = RwSignal::new(Vec::new());
    let demo_picker_open = RwSignal::new(false);
    let demo_loading = RwSignal::new(false);

    let on_demo_click = move |_: web_sys::MouseEvent| {
        if demo_picker_open.get_untracked() {
            demo_picker_open.set(false);
            return;
        }
        if !demo_entries.get_untracked().is_empty() {
            demo_picker_open.set(true);
            return;
        }
        // Fetch the index
        demo_loading.set(true);
        spawn_local(async move {
            match fetch_demo_index().await {
                Ok(entries) => {
                    demo_entries.set(entries);
                    demo_picker_open.set(true);
                }
                Err(e) => log::error!("Failed to fetch demo index: {e}"),
            }
            demo_loading.set(false);
        });
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
            <input
                node_ref=file_input_ref
                type="file"
                accept=".wav,.flac,.mp3,.ogg"
                multiple=true
                style="display:none"
                on:change=on_file_input_change
            />
            {move || {
                let file_vec = files.get();
                let lc = loading_count.get();
                if file_vec.is_empty() && lc == 0 {
                    view! {
                        <div class="drop-hint">
                            "Drop audio files here"
                            <button class="upload-btn" on:click=on_upload_click>"Browse files"</button>
                            <button class="upload-btn demo-btn" on:click=on_demo_click>
                                {move || if demo_loading.get() { "Loading..." } else { "Load demo" }}
                            </button>
                            {move || {
                                if demo_picker_open.get() {
                                    let entries = demo_entries.get();
                                    let items: Vec<_> = entries.iter().map(|entry| {
                                        let entry_clone = entry.clone();
                                        let display_name = entry.filename
                                            .trim_end_matches(".wav")
                                            .trim_end_matches(".flac")
                                            .to_string();
                                        view! {
                                            <button
                                                class="demo-item"
                                                on:click=move |_| {
                                                    let entry = entry_clone.clone();
                                                    state.loading_count.update(|c| *c += 1);
                                                    spawn_local(async move {
                                                        match load_single_demo(&entry, state).await {
                                                            Ok(()) => {}
                                                            Err(e) => log::error!("Failed to load demo sound: {e}"),
                                                        }
                                                        state.loading_count.update(|c| *c = c.saturating_sub(1));
                                                    });
                                                }
                                            >
                                                {display_name}
                                            </button>
                                        }
                                    }).collect();
                                    view! {
                                        <div class="demo-picker">{items}</div>
                                    }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                        </div>
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

    // Replay ZC audio when EQ settings change during playback
    let maybe_replay_zc = move || {
        if state.is_playing.get_untracked()
            && state.playback_mode.get_untracked() == PlaybackMode::ZeroCrossing
            && state.filter_enabled.get_untracked()
        {
            playback::replay(&state);
        }
    };

    let on_enable_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        state.filter_enabled.set(input.checked());
        maybe_replay_zc();
    };

    let set_band_mode = move |mode: u8| {
        state.filter_band_mode.set(mode);
        maybe_replay_zc();
    };

    let on_set_from_selection = move |_: web_sys::MouseEvent| {
        if let Some(sel) = state.selection.get_untracked() {
            if sel.freq_low > 0.0 && sel.freq_high > sel.freq_low {
                state.filter_freq_low.set(sel.freq_low);
                state.filter_freq_high.set(sel.freq_high);
                state.filter_set_from_selection.set(true);
                maybe_replay_zc();
            }
        }
    };

    let make_db_handler = |signal: RwSignal<f64>| {
        move |ev: web_sys::Event| {
            let target = ev.target().unwrap();
            let input: web_sys::HtmlInputElement = target.unchecked_into();
            if let Ok(val) = input.value().parse::<f64>() {
                signal.set(val);
                maybe_replay_zc();
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
    let show_harmonics = move || band_mode() >= 4;
    let show_above = move || band_mode() >= 3;

    let quality = move || state.filter_quality.get();
    let set_quality = move |q: crate::state::FilterQuality| {
        state.filter_quality.set(q);
        maybe_replay_zc();
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
                    <span class="setting-label">"Enable pre-processing"</span>
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
                        <div class="setting-group-title">"Pre-processing EQ"</div>

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

                }.into_any()
            }}

            // Mode-specific filter chain (always visible, not gated by EQ enable)
            {move || {
                let mode = state.playback_mode.get();
                match mode {
                    crate::state::PlaybackMode::Heterodyne => {
                        view! {
                            <div class="setting-group">
                                <div class="setting-group-title">"Mode filters"</div>
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
                            </div>
                        }.into_any()
                    }
                    crate::state::PlaybackMode::ZeroCrossing => {
                        let filter_on = state.filter_enabled.get();
                        view! {
                            <div class="setting-group">
                                <div class="setting-group-title">"Mode filters"</div>
                                <div class="filter-mode-info">
                                    {if filter_on {
                                        "ZC: using pre-processing EQ"
                                    } else {
                                        "ZC: bandpass 15\u{2013}150 kHz"
                                    }}
                                </div>
                            </div>
                        }.into_any()
                    }
                    _ => view! { <span></span> }.into_any(),
                }
            }}
        </div>
    }
}

#[component]
fn AnalysisPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    let analysis = Memo::new(move |_| {
        let files = state.files.get();
        let idx = state.current_file_index.get();
        idx.and_then(|i| files.get(i).cloned()).map(|file| {
            let meta = &file.audio.metadata;
            bit_analysis::analyze_bits(
                &file.audio.samples,
                meta.bits_per_sample,
                meta.is_float,
                file.audio.duration_secs,
            )
        })
    });

    let wsnr_result = Memo::new(move |_| {
        let files = state.files.get();
        let idx = state.current_file_index.get();
        idx.and_then(|i| files.get(i).cloned()).map(|file| {
            wsnr::analyze_wsnr(&file.audio.samples, file.audio.sample_rate)
        })
    });

    let xc_quality = Memo::new(move |_| {
        let files = state.files.get();
        let idx = state.current_file_index.get();
        idx.and_then(|i| files.get(i).cloned())
            .and_then(|file| file.xc_metadata)
            .and_then(|meta| {
                meta.iter()
                    .find(|(k, _)| k == "Quality")
                    .map(|(_, v)| v.clone())
            })
    });

    let report_text = Memo::new(move |_| {
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let file = idx.and_then(|i| files.get(i).cloned());

        let mut report = "=== Audio Analysis ===\n".to_string();

        if let Some(ref f) = file {
            let meta = &f.audio.metadata;
            let sr = f.audio.sample_rate;
            let sr_text = if sr % 1000 == 0 {
                format!("{} kHz", sr / 1000)
            } else {
                format!("{:.1} kHz", sr as f64 / 1000.0)
            };
            let ch_text = match f.audio.channels {
                1 => "Mono".to_string(),
                2 => "Stereo".to_string(),
                n => format!("{} ch", n),
            };
            let bit_text = if meta.is_float {
                format!("{}-bit float", meta.bits_per_sample)
            } else {
                format!("{}-bit", meta.bits_per_sample)
            };
            let total_samples = f.audio.samples.len();
            let dur_text = format!("{:.3} s", f.audio.duration_secs);
            report.push_str(&format!(
                "\nFile\n  Sample rate: {}\n  Channels: {}\n  Bit depth: {}\n  Duration: {}\n  Samples: {}\n",
                sr_text, ch_text, bit_text, dur_text, total_samples
            ));

            // Signal stats
            let smp = &f.audio.samples;
            let len = smp.len();
            if len > 0 {
                let mut smin = f32::INFINITY;
                let mut smax = f32::NEG_INFINITY;
                let mut sum = 0.0f64;
                let mut sum_sq = 0.0f64;
                for &s in smp.iter() {
                    if s < smin { smin = s; }
                    if s > smax { smax = s; }
                    sum += s as f64;
                    sum_sq += (s as f64) * (s as f64);
                }
                let dc_bias = sum / len as f64;
                let rms = (sum_sq / len as f64).sqrt();
                let min_db = if smin.abs() > 0.0 { format!("{:.1} dB", 20.0 * (smin.abs() as f64).log10()) } else { "-\u{221e} dB".into() };
                let max_db = if smax.abs() > 0.0 { format!("{:.1} dB", 20.0 * (smax.abs() as f64).log10()) } else { "-\u{221e} dB".into() };
                let rms_db = if rms > 0.0 { format!("{:.1} dB", 20.0 * rms.log10()) } else { "-\u{221e} dB".into() };
                let dc_db = if dc_bias.abs() > 0.0 { format!("{:.1} dB", 20.0 * dc_bias.abs().log10()) } else { "-\u{221e} dB".into() };
                let precision_str = analysis.get()
                    .map(|a| format!("~{:.1} bits", a.effective_bits_f64))
                    .unwrap_or_else(|| "—".into());
                report.push_str(&format!(
                    "\nSignal\n  Min: {:.4} ({})\n  Max: {:.4} ({})\n  RMS: {}\n  DC bias: {}\n  Precision: {}\n",
                    smin, min_db, smax, max_db, rms_db, dc_db, precision_str
                ));
            }
        }

        // wSNR
        if let Some(ref w) = wsnr_result.get() {
            let grade = w.grade.label();
            report.push_str(&format!(
                "\nRecording Quality (wSNR): {}\n  SNR: {:.1} dB(ISO/ITU)\n  Signal: {:.1} dB (ISO 226)\n  Noise: {:.1} dB (ITU-R 468)\n",
                grade, w.snr_db, w.signal_db, w.noise_db
            ));
            if let Some(xc) = xc_quality.get() {
                report.push_str(&format!("  XC quality (metadata): {}\n", xc.trim()));
            }
            for msg in &w.warnings {
                report.push_str(&format!("  \u{26a0} {}\n", msg));
            }
        }

        // Bit analysis
        if let Some(ref a) = analysis.get() {
            report.push_str(&format!("\nBit Usage\n  {}\n", a.summary));
            for w in &a.warnings {
                report.push_str(&format!("  \u{26a0} {}\n", w));
            }
            let caution_list: Vec<String> = a.bit_cautions.iter().enumerate()
                .filter(|(_, cs)| !cs.is_empty())
                .map(|(i, cs)| {
                    let label = bit_analysis::bit_label(i, a.bits_per_sample, a.is_float);
                    let names: Vec<&str> = cs.iter().map(|c| match c {
                        BitCaution::SignBitSkewed => "SignBitSkewed",
                        BitCaution::Always1 => "Always1",
                        BitCaution::OnlyInFade => "OnlyInFade",
                        BitCaution::VeryLowUsage => "VeryLowUsage",
                    }).collect();
                    format!("{} ({})", label, names.join(", "))
                })
                .collect();
            if !caution_list.is_empty() {
                report.push_str(&format!("  Cautions: {}\n", caution_list.join("; ")));
            }
        }

        report
    });

    view! {
        <div class="sidebar-panel">
            // Copy report button
            {move || {
                let has_file = {
                    let files = state.files.get();
                    let idx = state.current_file_index.get();
                    idx.and_then(|i| files.get(i)).is_some()
                };
                if has_file {
                    let text = report_text.get();
                    let on_copy = move |_: web_sys::MouseEvent| {
                        copy_to_clipboard(&text);
                    };
                    view! {
                        <div class="copy-report-row">
                            <button class="copy-report-btn" on:click=on_copy title="Copy full analysis report to clipboard">"Copy report"</button>
                        </div>
                    }.into_any()
                } else {
                    view! { <span></span> }.into_any()
                }
            }}
            // File info + signal stats
            {move || {
                let files = state.files.get();
                let idx = state.current_file_index.get();
                let file = idx.and_then(|i| files.get(i).cloned());
                match file.as_ref() {
                    None => view! {
                        <div class="sidebar-panel-empty">"No file selected"</div>
                    }.into_any(),
                    Some(f) => {
                        let meta = &f.audio.metadata;
                        let sr = f.audio.sample_rate;
                        let sr_text = if sr % 1000 == 0 {
                            format!("{} kHz", sr / 1000)
                        } else {
                            format!("{:.1} kHz", sr as f64 / 1000.0)
                        };
                        let ch_text = match f.audio.channels {
                            1 => "Mono".to_string(),
                            2 => "Stereo".to_string(),
                            n => format!("{} ch", n),
                        };
                        let bit_text = if meta.is_float {
                            format!("{}-bit float", meta.bits_per_sample)
                        } else {
                            format!("{}-bit", meta.bits_per_sample)
                        };
                        let total_samples = f.audio.samples.len();
                        let dur_text = format!("{:.3} s", f.audio.duration_secs);
                        let samples_text = format!("{}", total_samples);

                        // Signal stats
                        let samples = &f.audio.samples;
                        let len = samples.len();
                        let (sig_min, sig_max, dc_bias, rms) = if len > 0 {
                            let mut smin = f32::INFINITY;
                            let mut smax = f32::NEG_INFINITY;
                            let mut sum = 0.0f64;
                            let mut sum_sq = 0.0f64;
                            for &s in samples.iter() {
                                if s < smin { smin = s; }
                                if s > smax { smax = s; }
                                sum += s as f64;
                                sum_sq += (s as f64) * (s as f64);
                            }
                            (smin, smax, sum / len as f64, (sum_sq / len as f64).sqrt())
                        } else {
                            (0.0f32, 0.0f32, 0.0f64, 0.0f64)
                        };
                        let min_db = if sig_min.abs() > 0.0 { format!("{:.1} dB", 20.0 * (sig_min.abs() as f64).log10()) } else { "-\u{221E} dB".into() };
                        let max_db = if sig_max.abs() > 0.0 { format!("{:.1} dB", 20.0 * (sig_max.abs() as f64).log10()) } else { "-\u{221E} dB".into() };
                        let rms_db = if rms > 0.0 { format!("{:.1} dB", 20.0 * rms.log10()) } else { "-\u{221E} dB".into() };
                        let dc_db = if dc_bias.abs() > 0.0 { format!("{:.1} dB", 20.0 * dc_bias.abs().log10()) } else { "-\u{221E} dB".into() };
                        let dc_raw_tooltip = format!("{:.6} (raw)", dc_bias);
                        // DC relative to RMS: gives perceptual sense of DC severity
                        let dc_rms_ratio = if rms > 0.0 { dc_bias.abs() / rms } else { 0.0 };
                        // Fractional bit depth estimate from bit analysis
                        let (precision_text, precision_tooltip) = analysis.get()
                            .map(|a| {
                                let text = format!("~{:.1}", a.effective_bits_f64);
                                let tip = format!("Estimated effective bit depth (entropy-based); nominal: {}-bit", a.bits_per_sample);
                                (text, tip)
                            })
                            .unwrap_or_else(|| ("—".into(), String::new()));
                        // Warning: notable DC if |dc| > 1% of full scale OR dc/rms > 5%, gated on N
                        let dc_notable = len > 10_000 && (dc_bias.abs() > 0.01 || dc_rms_ratio > 0.05);
                        let dc_warning = if dc_notable {
                            Some(format!("DC offset: {} \u{2014} {:.0}% of RMS level", dc_db, dc_rms_ratio * 100.0))
                        } else {
                            None
                        };

                        view! {
                            <div class="setting-group">
                                <div class="setting-group-title">"File"</div>
                                <div class="analysis-stats">
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{sr_text}</span>
                                        <span class="analysis-stat-label">"Sample rate"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{ch_text}</span>
                                        <span class="analysis-stat-label">"Channels"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{bit_text}</span>
                                        <span class="analysis-stat-label">"Bit depth"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{dur_text}</span>
                                        <span class="analysis-stat-label">"Duration"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{samples_text}</span>
                                        <span class="analysis-stat-label">"Samples"</span>
                                    </div>
                                </div>
                            </div>
                            <div class="setting-group">
                                <div class="setting-group-title">"Signal"</div>
                                <div class="analysis-stats">
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{format!("{:.4}", sig_min)}</span>
                                        <span class="analysis-stat-label" title=min_db>"Min"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{format!("{:.4}", sig_max)}</span>
                                        <span class="analysis-stat-label" title=max_db>"Max"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{rms_db}</span>
                                        <span class="analysis-stat-label">"RMS"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{dc_db}</span>
                                        <span class="analysis-stat-label" title=dc_raw_tooltip>"DC bias"</span>
                                    </div>
                                    <div class="analysis-stat">
                                        <span class="analysis-stat-value">{precision_text}</span>
                                        <span class="analysis-stat-label" title=precision_tooltip>"Precision"</span>
                                    </div>
                                </div>
                                {dc_warning.map(|w| view! { <div class="analysis-warning">{w}</div> })}
                            </div>
                        }.into_any()
                    }
                }
            }}
            // wSNR section
            {move || {
                match wsnr_result.get().as_ref() {
                    None => view! { <span></span> }.into_any(),
                    Some(w) => {
                        let grade_class = match w.grade {
                            wsnr::WsnrGrade::A => "wsnr-grade wsnr-grade-a",
                            wsnr::WsnrGrade::B => "wsnr-grade wsnr-grade-b",
                            wsnr::WsnrGrade::C => "wsnr-grade wsnr-grade-c",
                            wsnr::WsnrGrade::D => "wsnr-grade wsnr-grade-d",
                            wsnr::WsnrGrade::E => "wsnr-grade wsnr-grade-e",
                        };
                        let grade_label = w.grade.label().to_string();
                        let snr_text = format!("{:.1} dB(ISO/ITU)", w.snr_db);
                        let signal_text = format!("Signal: {:.1} dB (ISO 226)", w.signal_db);
                        let noise_text = format!("Noise: {:.1} dB (ITU-R 468)", w.noise_db);

                        let xc_comparison = xc_quality.get().map(|xc_q| {
                            let xc_letter = xc_q.trim().to_uppercase();
                            let xc_badge_class = match xc_letter.as_str() {
                                "A" => "wsnr-grade-sm wsnr-grade-a",
                                "B" => "wsnr-grade-sm wsnr-grade-b",
                                "C" => "wsnr-grade-sm wsnr-grade-c",
                                "D" => "wsnr-grade-sm wsnr-grade-d",
                                _ => "wsnr-grade-sm wsnr-grade-e",
                            };
                            //let computed = grade_label.clone();
                            let note = "";
                            /*
                            let note = if xc_letter == computed {
                                "(matches)".to_string()
                            } else {
                                format!("(estimated: {})", computed)
                            };
                            */
                            (xc_letter, xc_badge_class.to_string(), note)
                        });

                        let warnings: Vec<_> = w.warnings.iter().map(|msg| {
                            let msg = msg.clone();
                            view! { <div class="wsnr-warning">{msg}</div> }
                        }).collect();

                        view! {
                            <div class="setting-group">
                                <div class="setting-group-title">"Recording Quality"</div>
                                <div class="wsnr-result">
                                    <div class="wsnr-detail">Estimated wSNR:</div>
                                    <div class="wsnr-header">
                                        <span class=grade_class>{grade_label}</span>
                                        <span class="wsnr-snr">{snr_text}</span>
                                    </div>
                                    <div class="wsnr-detail">{signal_text}</div>
                                    <div class="wsnr-detail">{noise_text}</div>
                                    {if !warnings.is_empty() {
                                        view! { <div class="wsnr-warnings">{warnings}</div> }.into_any()
                                    } else {
                                        view! { <span></span> }.into_any()
                                    }}
                                    {xc_comparison.map(|(letter, badge_class, note)| view! {
                                        <div class="wsnr-comparison">
                                            "Metadata: "
                                            <span class=badge_class>{letter}</span>
                                            " (XC grade)" {note}
                                        </div>
                                    })}
                                </div>
                            </div>
                        }.into_any()
                    }
                }
            }}
            // Bit analysis section
            {move || {
                match analysis.get().as_ref() {
                    None => view! { <span></span> }.into_any(),
                    Some(a) => {
                        let bits = a.bits_per_sample as usize;
                        let cols = 4usize;
                        let rows = (bits + cols - 1) / cols;
                        let total = a.total_samples;
                        let summary = a.summary.clone();
                        let warnings = a.warnings.clone();

                        let grid_cells: Vec<_> = (0..rows).flat_map(|row| {
                            (0..cols).map(move |col| {
                                let idx = row * cols + col;
                                (row, col, idx)
                            })
                        }).filter(|&(_, _, idx)| idx < bits)
                        .map(|(_, _, idx)| {
                            let stat = &a.bit_stats[idx];
                            let cautions = &a.bit_cautions[idx];
                            let label = bit_analysis::bit_label(idx, a.bits_per_sample, a.is_float);
                            let count = stat.count;
                            let used = count > 0;
                            let expected = bit_analysis::is_expected_used(idx, a.bits_per_sample, a.is_float, a.effective_bits);

                            let cell_class = if used {
                                if cautions.iter().any(|c| matches!(c, BitCaution::SignBitSkewed | BitCaution::OnlyInFade | BitCaution::VeryLowUsage)) {
                                    "bit-cell used caution"
                                } else if cautions.iter().any(|c| matches!(c, BitCaution::Always1)) {
                                    "bit-cell used always1"
                                } else {
                                    "bit-cell used"
                                }
                            } else if expected {
                                "bit-cell unused-expected"
                            } else {
                                "bit-cell unused"
                            };

                            let value_text = if count == 0 {
                                "\u{2013}".to_string() // em-dash
                            } else if total > 0 {
                                let pct = count as f64 / total as f64 * 100.0;
                                if pct >= 1.0 {
                                    format!("{:.0}%", pct)
                                } else if count > 99 {
                                    "99+".into()
                                } else {
                                    format!("{}", count)
                                }
                            } else {
                                "\u{2013}".to_string()
                            };

                            let mut tooltip = format!("Bit {}: {} / {} samples", label, count, total);
                            if let Some(fi) = stat.first_index {
                                tooltip.push_str(&format!("\nFirst: sample {}", fi));
                            }
                            if let Some(li) = stat.last_index {
                                tooltip.push_str(&format!("\nLast: sample {}", li));
                            }
                            for c in cautions {
                                match c {
                                    BitCaution::SignBitSkewed => tooltip.push_str("\n\u{26A0} Asymmetric — sample distribution is far from 50/50 +/\u{2212}"),
                                    BitCaution::OnlyInFade => tooltip.push_str("\n\u{26A0} Only used in fade regions"),
                                    BitCaution::Always1 => tooltip.push_str("\n\u{26A0} Always 1 (100%)"),
                                    BitCaution::VeryLowUsage => tooltip.push_str("\n\u{26A0} Very low usage"),
                                }
                            }

                            view! {
                                <div class=cell_class title=tooltip>
                                    <span class="bit-label">{label}</span>
                                    <span class="bit-value">{value_text}</span>
                                </div>
                            }
                        }).collect();

                        // Float region labels
                        let region_labels = if a.is_float && a.bits_per_sample == 32 {
                            view! {
                                <div class="bit-region-labels">
                                    <span class="bit-region sign">"S"</span>
                                    <span class="bit-region exponent">"Exponent"</span>
                                    <span class="bit-region mantissa">"Mantissa"</span>
                                </div>
                            }.into_any()
                        } else {
                            view! { <span></span> }.into_any()
                        };

                        let warning_items: Vec<_> = warnings.iter().map(|w| {
                            let w = w.clone();
                            view! { <div class="bit-warning">{w}</div> }
                        }).collect();

                        // Positive/negative/zero split grids
                        let pos_total = a.positive_total;
                        let neg_total = a.negative_total;
                        let zero_total = a.zero_total;
                        let pos_counts = a.positive_counts.clone();
                        let neg_counts = a.negative_counts.clone();
                        let bits_per_sample = a.bits_per_sample;
                        let is_float = a.is_float;
                        let effective_bits = a.effective_bits;

                        let make_sign_grid = |sign_counts: &[usize], sign_total: usize, polarity: &str| -> Vec<_> {
                            (0..bits).map(|idx| {
                                let count = sign_counts[idx];
                                let label = bit_analysis::bit_label(idx, bits_per_sample, is_float);
                                let is_sign_bit = idx == 0;
                                // Sign bit is always 0% or 100% by definition in pos/neg splits — grey it out
                                if is_sign_bit {
                                    let value_text = if sign_total > 0 && count == sign_total {
                                        "100%".to_string()
                                    } else {
                                        "0%".to_string()
                                    };
                                    let sign_tooltip = if polarity == "positive" {
                                        "Sign bit: always 0 for positive samples".to_string()
                                    } else {
                                        "Sign bit: always 1 for negative samples".to_string()
                                    };
                                    return view! {
                                        <div class="bit-cell unused" title=sign_tooltip>
                                            <span class="bit-label">{label}</span>
                                            <span class="bit-value">{value_text}</span>
                                        </div>
                                    };
                                }
                                let used = count > 0;
                                let expected = bit_analysis::is_expected_used(idx, bits_per_sample, is_float, effective_bits);
                                let cell_class = if used {
                                    "bit-cell used"
                                } else if expected {
                                    "bit-cell unused-expected"
                                } else {
                                    "bit-cell unused"
                                };
                                let value_text = if count == 0 {
                                    "\u{2013}".to_string()
                                } else if sign_total > 0 {
                                    let pct = count as f64 / sign_total as f64 * 100.0;
                                    if pct >= 1.0 { format!("{:.0}%", pct) }
                                    else if count > 99 { "99+".into() }
                                    else { format!("{}", count) }
                                } else {
                                    "\u{2013}".to_string()
                                };
                                let tooltip = if sign_total > 0 {
                                    let pct = count as f64 / sign_total as f64 * 100.0;
                                    format!("Bit {}: {} / {} {} samples ({:.1}%)", label, count, sign_total, polarity, pct)
                                } else {
                                    format!("Bit {}: no {} samples", label, polarity)
                                };
                                view! {
                                    <div class=cell_class title=tooltip>
                                        <span class="bit-label">{label}</span>
                                        <span class="bit-value">{value_text}</span>
                                    </div>
                                }
                            }).collect()
                        };

                        let pos_grid = make_sign_grid(&pos_counts, pos_total, "positive");
                        let neg_grid = make_sign_grid(&neg_counts, neg_total, "negative");

                        let pos_pct = if total > 0 { format!("{:.0}%", pos_total as f64 / total as f64 * 100.0) } else { "0%".into() };
                        let neg_pct = if total > 0 { format!("{:.0}%", neg_total as f64 / total as f64 * 100.0) } else { "0%".into() };
                        let zero_pct = if total > 0 { format!("{:.0}%", zero_total as f64 / total as f64 * 100.0) } else { "0%".into() };
                        let pos_tooltip = format!("{} samples", pos_total);
                        let neg_tooltip = format!("{} samples", neg_total);
                        let zero_tooltip = format!("{} samples", zero_total);

                        view! {
                            <div class="setting-group">
                                <div class="setting-group-title">"Bit Usage"</div>
                                {region_labels}
                                <div class="bit-grid" style=format!("grid-template-columns: repeat({}, 1fr);", cols)>
                                    {grid_cells}
                                </div>
                            </div>
                            <div class="setting-group">
                                <div class="bit-summary">{summary}</div>
                                {warning_items}
                            </div>
                            <details class="bit-details">
                                <summary class="bit-details-summary">"Bit Details"</summary>
                                <div class="setting-group">
                                    <div class="bit-sign-split">
                                        <div class="bit-sign-col">
                                            <div class="bit-sign-header" title=pos_tooltip>{format!("+ {}", pos_pct)}</div>
                                            <div class="bit-grid bit-grid-mini" style=format!("grid-template-columns: repeat({}, 1fr);", cols)>
                                                {pos_grid}
                                            </div>
                                        </div>
                                        <div class="bit-sign-col">
                                            <div class="bit-sign-header" title=neg_tooltip>{format!("\u{2212} {}", neg_pct)}</div>
                                            <div class="bit-grid bit-grid-mini" style=format!("grid-template-columns: repeat({}, 1fr);", cols)>
                                                {neg_grid}
                                            </div>
                                        </div>
                                    </div>
                                    <div class="bit-zero-row" title=zero_tooltip>
                                        {format!("0 (silence): {}", zero_pct)}
                                    </div>
                                </div>
                            </details>
                        }.into_any()
                    }
                }
            }}
        </div>
    }
}

fn copy_to_clipboard(text: &str) {
    if let Some(window) = web_sys::window() {
        let clipboard = window.navigator().clipboard();
        let _ = clipboard.write_text(text);
    }
}

fn metadata_row(label: String, value: String, label_title: Option<String>) -> impl IntoView {
    let value_for_copy = value.clone();
    let value_for_title = value.clone();
    let on_copy = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        copy_to_clipboard(&value_for_copy);
    };
    view! {
        <div class="setting-row metadata-row">
            <span class="setting-label" title=label_title.unwrap_or_default()>{label}</span>
            <span class="setting-value metadata-value" title=value_for_title>{value}</span>
            <button class="copy-btn" on:click=on_copy title="Copy">{"\u{2398}"}</button>
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
                        let xc_fields: Vec<_> = f.xc_metadata.clone().unwrap_or_default();
                        let has_xc = !xc_fields.is_empty();
                        let guano_fields: Vec<_> = meta.guano.as_ref()
                            .map(|g| g.fields.clone())
                            .unwrap_or_default();
                        let has_guano = !guano_fields.is_empty();

                        view! {
                            <div class="setting-group">
                                <div class="setting-group-title">"File"</div>
                                {metadata_row("Name".into(), f.name.clone(), None)}
                                {metadata_row("Format".into(), meta.format.to_string(), None)}
                                {metadata_row("Duration".into(), format!("{:.3} s", f.audio.duration_secs), None)}
                                {metadata_row("Sample rate".into(), format!("{} kHz", f.audio.sample_rate / 1000), None)}
                                {metadata_row("Channels".into(), f.audio.channels.to_string(), None)}
                                {metadata_row("Bit depth".into(), format!("{}-bit", meta.bits_per_sample), None)}
                                {metadata_row("File size".into(), size_str, None)}
                            </div>
                            {if has_xc {
                                let items: Vec<_> = xc_fields.into_iter().map(|(label, value)| {
                                    metadata_row(label, value, None).into_any()
                                }).collect();
                                view! {
                                    <div class="setting-group">
                                        <div class="setting-group-title">"Xeno-canto"</div>
                                        {items}
                                    </div>
                                }.into_any()
                            } else {
                                view! { <span></span> }.into_any()
                            }}
                            {if has_guano {
                                let mut items: Vec<leptos::tachys::view::any_view::AnyView> = Vec::new();
                                let mut current_section: Option<String> = None;
                                for (k, v) in guano_fields {
                                    let (section, display_key) = categorize_guano_key(&k);
                                    if current_section.as_ref() != Some(&section) {
                                        let heading = section.clone();
                                        let show_badge = heading != "GUANO";
                                        items.push(view! {
                                            <div class="setting-group-title">
                                                {heading}
                                                {if show_badge {
                                                    view! { <span class="metadata-source-badge">"GUANO"</span> }.into_any()
                                                } else {
                                                    view! { <span></span> }.into_any()
                                                }}
                                            </div>
                                        }.into_any());
                                        current_section = Some(section);
                                    }
                                    items.push(metadata_row(display_key, v, Some(k)).into_any());
                                }
                                view! {
                                    <div class="setting-group">
                                        {items}
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
    load_named_bytes(name, &bytes, None, state).await
}

async fn load_named_bytes(name: String, bytes: &[u8], xc_metadata: Option<Vec<(String, String)>>, state: AppState) -> Result<(), String> {
    let audio = load_audio(bytes)?;
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
                xc_metadata,
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

const DEMO_SOUNDS_BASE: &str =
    "https://raw.githubusercontent.com/pengowray/batchi-demo-sounds/main";

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("No window")?;
    let resp_value = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("fetch error: {e:?}"))?;
    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| "Response cast failed".to_string())?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let buf = JsFuture::from(resp.array_buffer().map_err(|e| format!("{e:?}"))?)
        .await
        .map_err(|e| format!("array_buffer: {e:?}"))?;
    let uint8 = js_sys::Uint8Array::new(&buf);
    Ok(uint8.to_vec())
}

async fn fetch_text(url: &str) -> Result<String, String> {
    let window = web_sys::window().ok_or("No window")?;
    let resp_value = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("fetch error: {e:?}"))?;
    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| "Response cast failed".to_string())?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text = JsFuture::from(resp.text().map_err(|e| format!("{e:?}"))?)
        .await
        .map_err(|e| format!("text: {e:?}"))?;
    text.as_string().ok_or("Not a string".to_string())
}

fn parse_xc_metadata(json: &serde_json::Value) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let s = |key: &str| json[key].as_str().unwrap_or("").to_string();

    let en = s("en");
    if !en.is_empty() {
        fields.push(("Species".into(), en));
    }
    let gen = s("gen");
    let sp = s("sp");
    if !gen.is_empty() && !sp.is_empty() {
        fields.push(("Scientific name".into(), format!("{} {}", gen, sp)));
    }
    for (key, label) in [
        ("rec", "Recordist"),
        ("lic", "License"),
        ("attribution", "Attribution"),
        ("cnt", "Country"),
        ("loc", "Location"),
    ] {
        let v = s(key);
        if !v.is_empty() {
            fields.push((label.into(), v));
        }
    }
    let lat = s("lat");
    let lon = s("lon");
    if !lat.is_empty() && !lon.is_empty() {
        fields.push(("Coordinates".into(), format!("{}, {}", lat, lon)));
    }
    for (key, label) in [
        ("date", "Date"),
        ("type", "Sound type"),
        ("q", "Quality"),
        ("url", "URL"),
    ] {
        let v = s(key);
        if !v.is_empty() {
            fields.push((label.into(), v));
        }
    }
    fields
}

#[derive(Clone, Debug)]
struct DemoEntry {
    filename: String,
    metadata_file: Option<String>,
}

async fn fetch_demo_index() -> Result<Vec<DemoEntry>, String> {
    let index_url = format!("{}/index.json", DEMO_SOUNDS_BASE);
    let index_text = fetch_text(&index_url).await?;
    let index: serde_json::Value =
        serde_json::from_str(&index_text).map_err(|e| format!("index parse: {e}"))?;

    let sounds = index["sounds"]
        .as_array()
        .ok_or("No sounds array in index")?;

    let entries = sounds
        .iter()
        .filter_map(|sound| {
            let filename = sound["filename"].as_str()?.to_string();
            let metadata_file = sound["metadata"].as_str().map(|s| s.to_string());
            Some(DemoEntry { filename, metadata_file })
        })
        .collect();

    Ok(entries)
}

async fn load_single_demo(entry: &DemoEntry, state: AppState) -> Result<(), String> {
    // Fetch XC metadata sidecar if available
    let xc_metadata = if let Some(meta_file) = &entry.metadata_file {
        let encoded = js_sys::encode_uri_component(meta_file);
        let meta_url = format!(
            "{}/sounds/{}",
            DEMO_SOUNDS_BASE,
            encoded.as_string().unwrap_or_default()
        );
        match fetch_text(&meta_url).await {
            Ok(text) => {
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(json) => Some(parse_xc_metadata(&json)),
                    Err(e) => {
                        log::warn!("Failed to parse XC metadata for {}: {}", entry.filename, e);
                        None
                    }
                }
            }
            Err(e) => {
                log::warn!("Failed to fetch XC metadata for {}: {}", entry.filename, e);
                None
            }
        }
    } else {
        None
    };

    let encoded = js_sys::encode_uri_component(&entry.filename);
    let audio_url = format!(
        "{}/sounds/{}",
        DEMO_SOUNDS_BASE,
        encoded.as_string().unwrap_or_default()
    );
    log::info!("Fetching demo: {}", entry.filename);
    let bytes = fetch_bytes(&audio_url).await?;
    load_named_bytes(entry.filename.clone(), &bytes, xc_metadata, state).await
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
