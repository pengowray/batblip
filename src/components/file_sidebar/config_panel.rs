use leptos::prelude::*;
use wasm_bindgen::JsCast;
use crate::state::{AppState, ChromaColormap, ColormapPreference};

fn parse_colormap_pref(s: &str) -> ColormapPreference {
    match s {
        "inferno" => ColormapPreference::Inferno,
        "magma" => ColormapPreference::Magma,
        "plasma" => ColormapPreference::Plasma,
        "cividis" => ColormapPreference::Cividis,
        "turbo" => ColormapPreference::Turbo,
        "greyscale" => ColormapPreference::Greyscale,
        _ => ColormapPreference::Viridis,
    }
}

#[component]
pub(super) fn ConfigPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    let on_follow_cursor = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        state.follow_cursor.set(input.checked());
    };

    let on_always_show_view_range = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        state.always_show_view_range.set(input.checked());
    };

    let on_colormap_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let select: web_sys::HtmlSelectElement = target.unchecked_into();
        state.colormap_preference.set(parse_colormap_pref(&select.value()));
        state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
    };

    let on_hfr_colormap_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let select: web_sys::HtmlSelectElement = target.unchecked_into();
        state.hfr_colormap_preference.set(parse_colormap_pref(&select.value()));
        state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
    };

    view! {
        <div class="sidebar-panel">
            <div class="setting-group">
                <div class="setting-group-title">"Playback"</div>
                <div class="setting-row">
                    <span class="setting-label">"Follow cursor"</span>
                    <input
                        type="checkbox"
                        class="setting-checkbox"
                        prop:checked=move || state.follow_cursor.get()
                        on:change=on_follow_cursor
                    />
                </div>
            </div>

            <div class="setting-group">
                <div class="setting-group-title">"Display"</div>
                <div class="setting-row">
                    <span class="setting-label">"Color scheme"</span>
                    <select
                        class="setting-select"
                        on:change=on_colormap_change
                    >
                        <option value="viridis" selected=move || state.colormap_preference.get() == ColormapPreference::Viridis>"Viridis"</option>
                        <option value="inferno" selected=move || state.colormap_preference.get() == ColormapPreference::Inferno>"Inferno"</option>
                        <option value="magma" selected=move || state.colormap_preference.get() == ColormapPreference::Magma>"Magma"</option>
                        <option value="plasma" selected=move || state.colormap_preference.get() == ColormapPreference::Plasma>"Plasma"</option>
                        <option value="cividis" selected=move || state.colormap_preference.get() == ColormapPreference::Cividis>"Cividis"</option>
                        <option value="turbo" selected=move || state.colormap_preference.get() == ColormapPreference::Turbo>"Turbo"</option>
                        <option value="greyscale" selected=move || state.colormap_preference.get() == ColormapPreference::Greyscale>"Greyscale"</option>
                    </select>
                </div>
                <div class="setting-row">
                    <span class="setting-label">"HFR color scheme"</span>
                    <select
                        class="setting-select"
                        on:change=on_hfr_colormap_change
                    >
                        <option value="viridis" selected=move || state.hfr_colormap_preference.get() == ColormapPreference::Viridis>"Viridis"</option>
                        <option value="inferno" selected=move || state.hfr_colormap_preference.get() == ColormapPreference::Inferno>"Inferno"</option>
                        <option value="magma" selected=move || state.hfr_colormap_preference.get() == ColormapPreference::Magma>"Magma"</option>
                        <option value="plasma" selected=move || state.hfr_colormap_preference.get() == ColormapPreference::Plasma>"Plasma"</option>
                        <option value="cividis" selected=move || state.hfr_colormap_preference.get() == ColormapPreference::Cividis>"Cividis"</option>
                        <option value="turbo" selected=move || state.hfr_colormap_preference.get() == ColormapPreference::Turbo>"Turbo"</option>
                        <option value="greyscale" selected=move || state.hfr_colormap_preference.get() == ColormapPreference::Greyscale>"Greyscale"</option>
                    </select>
                </div>
                <div class="setting-row">
                    <span class="setting-label">"Chromagram colors"</span>
                    <select
                        class="setting-select"
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let select: web_sys::HtmlSelectElement = target.unchecked_into();
                            let mode = match select.value().as_str() {
                                "warm" => ChromaColormap::Warm,
                                "solid" => ChromaColormap::Solid,
                                "octave" => ChromaColormap::Octave,
                                "flow" => ChromaColormap::Flow,
                                _ => ChromaColormap::PitchClass,
                            };
                            state.chroma_colormap.set(mode);
                        }
                    >
                        <option value="pitch_class" selected=move || state.chroma_colormap.get() == ChromaColormap::PitchClass>"Pitch Class"</option>
                        <option value="solid" selected=move || state.chroma_colormap.get() == ChromaColormap::Solid>"Solid"</option>
                        <option value="warm" selected=move || state.chroma_colormap.get() == ChromaColormap::Warm>"Warm"</option>
                        <option value="octave" selected=move || state.chroma_colormap.get() == ChromaColormap::Octave>"Octave"</option>
                        <option value="flow" selected=move || state.chroma_colormap.get() == ChromaColormap::Flow>"Flow"</option>
                    </select>
                </div>
                <div class="setting-row">
                    <span class="setting-label">"Always show view range"</span>
                    <input
                        type="checkbox"
                        class="setting-checkbox"
                        prop:checked=move || state.always_show_view_range.get()
                        on:change=on_always_show_view_range
                    />
                </div>
            </div>
        </div>
    }
}
