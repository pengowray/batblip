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

/// Render the file identity / hash section.
fn file_identity_section(f: &crate::state::LoadedFile) -> impl IntoView {
    let state = expect_context::<AppState>();
    let identity = f.identity.clone();
    let has_file_handle = f.file_handle.is_some();

    // Get sidecar identity for comparison (if annotations loaded)
    let file_idx = state.current_file_index.get_untracked();
    let sidecar_identity = file_idx.and_then(|idx| {
        state.annotation_store.with_untracked(|store| {
            store.sets.get(idx)
                .and_then(|s| s.as_ref())
                .map(|set| set.file_identity.clone())
        })
    });

    let mut items: Vec<leptos::tachys::view::any_view::AnyView> = Vec::new();

    if let Some(ref id) = identity {
        // Spot hash (Layer 2)
        if let Some(ref hash) = id.spot_hash_b3 {
            items.push(metadata_row(
                "Spot hash".into(),
                hash.clone(),
                None,
            ).into_any());
        } else {
            items.push(metadata_row(
                "Spot hash".into(),
                "computing...".into(),
                None,
            ).into_any());
        }

        // Content hash (Layer 3)
        if let Some(ref hash) = id.content_hash {
            items.push(metadata_row(
                "Content hash".into(),
                hash.clone(),
                None,
            ).into_any());
        }

        // Full BLAKE3 (Layer 4)
        if let Some(ref hash) = id.full_blake3 {
            let sidecar_match = sidecar_identity.as_ref()
                .and_then(|sid| sid.full_blake3.as_ref())
                .map(|sh| if sh == hash { " \u{2713}" } else { " \u{26A0}" }) // checkmark or warning
                .unwrap_or("");
            items.push(metadata_row(
                "Full BLAKE3".into(),
                format!("{}{}", hash, sidecar_match),
                None,
            ).into_any());
        } else if has_file_handle {
            let on_calc = move |_: web_sys::MouseEvent| {
                if let Some(idx) = state.current_file_index.get_untracked() {
                    crate::file_identity::start_full_hash_computation(state, idx, false);
                }
            };
            let computing = state.hash_computing.get();
            let label = if computing {
                "Computing..."
            } else if sidecar_identity.as_ref().and_then(|s| s.full_blake3.as_ref()).is_some() {
                "Check hash"
            } else {
                "Calculate hash"
            };
            items.push(view! {
                <div class="setting-row metadata-row">
                    <span class="setting-label">"Full BLAKE3"</span>
                    <button class="hash-calc-btn" on:click=on_calc disabled=computing>{label}</button>
                </div>
            }.into_any());
        }

        // Full SHA-256 (Layer 4-alt)
        if let Some(ref hash) = id.full_sha256 {
            let sidecar_match = sidecar_identity.as_ref()
                .and_then(|sid| sid.full_sha256.as_ref())
                .map(|sh| if sh == hash { " \u{2713}" } else { " \u{26A0}" })
                .unwrap_or("");
            items.push(metadata_row(
                "Full SHA-256".into(),
                format!("{}{}", hash, sidecar_match),
                None,
            ).into_any());
        } else if has_file_handle {
            let on_calc_sha = move |_: web_sys::MouseEvent| {
                if let Some(idx) = state.current_file_index.get_untracked() {
                    crate::file_identity::start_full_hash_computation(state, idx, true);
                }
            };
            let computing = state.hash_computing.get();
            let label = if computing {
                "Computing..."
            } else if sidecar_identity.as_ref().and_then(|s| s.full_sha256.as_ref()).is_some() {
                "Check SHA-256"
            } else {
                "Calculate SHA-256"
            };
            items.push(view! {
                <div class="setting-row metadata-row">
                    <span class="setting-label">"Full SHA-256"</span>
                    <button class="hash-calc-btn" on:click=on_calc_sha disabled=computing>{label}</button>
                </div>
            }.into_any());
        }
    }

    if items.is_empty() {
        view! { <span></span> }.into_any()
    } else {
        view! {
            <div class="setting-group">
                <div class="setting-group-title">"File Identity"</div>
                {items}
            </div>
        }.into_any()
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
                            // File Identity / Hash section
                            {file_identity_section(f)}
                        }.into_any()
                    }
                }
            }}
        </div>
    }
}
