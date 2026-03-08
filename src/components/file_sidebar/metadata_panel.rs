use leptos::prelude::*;
use crate::state::AppState;

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

fn metadata_row(label: String, value: String, label_title: Option<String>) -> impl IntoView {
    let value_for_copy = value.clone();
    let value_for_title = value.clone();
    let on_copy = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        super::copy_to_clipboard(&value_for_copy);
    };
    view! {
        <div class="setting-row metadata-row">
            <span class="setting-label" title=label_title.unwrap_or_default()>{label}</span>
            <span class="setting-value metadata-value" title=value_for_title>{value}</span>
            <button class="copy-btn" on:click=on_copy title="Copy">{"\u{2398}"}</button>
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
pub(crate) fn MetadataPanel() -> impl IntoView {
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
                                {metadata_row("Duration".into(), crate::format_time::format_duration(f.audio.duration_secs, 3), None)}
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
