//! Collapsible export section: WAV / MP4 export with format radio buttons,
//! video settings, progress bar, and .batm import/export.

use leptos::prelude::*;

use crate::audio::export;
use crate::audio::video_export;
use crate::audio::webcodecs_bindings as wc;
use crate::state::{AppState, ExportFormat, VideoCodec, VideoResolution};

/// Collapsible export section component.
/// Expects `AppState` in context and the batm handler closures as props.
#[component]
pub fn ExportSection(
    on_export_batm: Callback<()>,
    on_save_sidecar: Callback<()>,
    on_import_batm: Callback<()>,
    has_annotations: Signal<Option<bool>>,
    has_file_path: Signal<Option<bool>>,
) -> impl IntoView {
    let state = expect_context::<AppState>();

    let webcodecs_available = wc::has_video_encoder() && wc::has_mp4_muxer();

    // Export button text (reactive)
    let export_button_text = move || {
        let format = state.export_format.get();
        let ext = match format {
            ExportFormat::Wav => ".wav",
            ExportFormat::Mp4 => ".mp4",
        };
        match export::get_export_info(&state) {
            Some(info) => {
                let mode_suffix = info.mode_label
                    .map(|m| format!(" ({m})"))
                    .unwrap_or_default();
                format!("Export {} {} to {ext}{mode_suffix}", info.count, info.source_label)
            }
            None => format!("Export to {ext}"),
        }
    };

    let export_disabled = move || {
        export::get_export_info(&state).is_none()
            || state.video_export_progress.get().is_some()
    };

    let on_export_click = move |_: web_sys::MouseEvent| {
        match state.export_format.get_untracked() {
            ExportFormat::Wav => {
                export::export_selected(&state);
            }
            ExportFormat::Mp4 => {
                video_export::start_export(&state);
            }
        }
    };

    let on_format_change = move |format: ExportFormat| {
        state.export_format.set(format);
    };

    view! {
        <div class="export-section">
            <div
                class="export-section-header"
                on:click=move |_| state.export_section_open.update(|v| *v = !*v)
            >
                <span class=move || if state.export_section_open.get() {
                    "export-toggle-arrow open"
                } else {
                    "export-toggle-arrow"
                }>
                    {"\u{25B6}"}
                </span>
                " Export"
            </div>

            <div class=move || if state.export_section_open.get() {
                "export-section-body open"
            } else {
                "export-section-body"
            }>
                // Format radio buttons
                <div class="setting-row export-format-row">
                    <span class="export-format-label">"Format:"</span>
                    <label class="export-radio">
                        <input
                            type="radio"
                            name="export-format"
                            checked=move || state.export_format.get() == ExportFormat::Wav
                            on:change=move |_| on_format_change(ExportFormat::Wav)
                        />
                        " WAV"
                    </label>
                    <label class=move || if webcodecs_available {
                        "export-radio"
                    } else {
                        "export-radio disabled"
                    }>
                        <input
                            type="radio"
                            name="export-format"
                            checked=move || state.export_format.get() == ExportFormat::Mp4
                            on:change=move |_| on_format_change(ExportFormat::Mp4)
                            disabled=move || !webcodecs_available
                        />
                        " MP4"
                        {if !webcodecs_available {
                            Some(view! {
                                <span class="export-tooltip" title="WebCodecs not available in this browser">{" (?)"}</span>
                            })
                        } else {
                            None
                        }}
                    </label>
                </div>

                // MP4-specific options (shown when MP4 selected)
                {move || {
                    if state.export_format.get() == ExportFormat::Mp4 && webcodecs_available {
                        Some(view! {
                            <div class="export-mp4-options">
                                <div class="setting-row" style="gap: 4px; align-items: center;">
                                    <span class="export-option-label">"Resolution:"</span>
                                    <select
                                        class="sidebar-select"
                                        on:change=move |ev| {
                                            let val = event_target_value(&ev);
                                            let res = match val.as_str() {
                                                "720" => VideoResolution::Hd720,
                                                "1080" => VideoResolution::Hd1080,
                                                "canvas" => VideoResolution::MatchCanvas,
                                                _ => VideoResolution::Hd720,
                                            };
                                            state.video_resolution.set(res);
                                        }
                                    >
                                        <option value="720" selected=move || state.video_resolution.get() == VideoResolution::Hd720>
                                            "720p"
                                        </option>
                                        <option value="1080" selected=move || state.video_resolution.get() == VideoResolution::Hd1080>
                                            "1080p"
                                        </option>
                                        <option value="canvas" selected=move || state.video_resolution.get() == VideoResolution::MatchCanvas>
                                            "Match canvas"
                                        </option>
                                    </select>
                                </div>
                                <div class="setting-row" style="gap: 4px; align-items: center;">
                                    <span class="export-option-label">"Codec:"</span>
                                    <select
                                        class="sidebar-select"
                                        on:change=move |ev| {
                                            let val = event_target_value(&ev);
                                            let codec = match val.as_str() {
                                                "av1" => VideoCodec::Av1,
                                                _ => VideoCodec::H264,
                                            };
                                            state.video_codec.set(codec);
                                        }
                                    >
                                        <option value="h264" selected=move || state.video_codec.get() == VideoCodec::H264>
                                            "H.264"
                                        </option>
                                        <option value="av1" selected=move || state.video_codec.get() == VideoCodec::Av1>
                                            "AV1"
                                        </option>
                                    </select>
                                </div>
                            </div>
                        })
                    } else {
                        None
                    }
                }}

                // Main export button
                <div class="setting-row" style="gap: 4px; align-items: center;">
                    <button
                        class="sidebar-btn"
                        style="flex: 1;"
                        on:click=on_export_click
                        disabled=export_disabled
                    >
                        {export_button_text}
                    </button>
                </div>

                // Progress bar (video export only)
                {move || {
                    state.video_export_progress.get().map(|progress| {
                        let status = state.video_export_status.get()
                            .unwrap_or_else(|| "Exporting...".to_string());
                        view! {
                            <div class="export-progress">
                                <div class="export-progress-bar">
                                    <div
                                        class="export-progress-fill"
                                        style=move || format!("width: {}%", (progress * 100.0) as u32)
                                    ></div>
                                </div>
                                <div class="export-progress-text">{status}</div>
                            </div>
                        }
                    })
                }}

                // .batm section
                <div class="setting-row" style="gap: 4px; margin-top: 4px;">
                    {if state.is_tauri {
                        view! {
                            <button
                                class="sidebar-btn"
                                style="flex: 1;"
                                on:click={
                                    let cb = on_save_sidecar.clone();
                                    move |_: web_sys::MouseEvent| cb.run(())
                                }
                                disabled=move || has_annotations.get().is_none() || has_file_path.get().is_none()
                                title="Save .batm sidecar next to the audio file"
                            >
                                "Save .batm"
                            </button>
                            <button
                                class="sidebar-btn"
                                style="flex: 1;"
                                on:click={
                                    let cb = on_export_batm.clone();
                                    move |_: web_sys::MouseEvent| cb.run(())
                                }
                                disabled=move || has_annotations.get().is_none()
                                title="Export .batm to a chosen location"
                            >
                                "Save as\u{2026}"
                            </button>
                        }.into_any()
                    } else {
                        view! {
                            <button
                                class="sidebar-btn"
                                style="flex: 1;"
                                on:click={
                                    let cb = on_export_batm.clone();
                                    move |_: web_sys::MouseEvent| cb.run(())
                                }
                                disabled=move || has_annotations.get().is_none()
                            >
                                "Export .batm"
                            </button>
                        }.into_any()
                    }}
                    <button
                        class="sidebar-btn"
                        style="flex: 1;"
                        on:click={
                            let cb = on_import_batm.clone();
                            move |_: web_sys::MouseEvent| cb.run(())
                        }
                    >
                        "Import .batm"
                    </button>
                </div>
            </div>
        </div>
    }
}
