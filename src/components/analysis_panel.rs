use leptos::prelude::*;
use crate::state::AppState;
use crate::dsp::zero_crossing::zero_crossing_frequency;

#[component]
pub fn AnalysisPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    let analysis = move || {
        let selection = state.selection.get()?;
        let dragging = state.is_dragging.get();
        let files = state.files.get();
        let idx = state.current_file_index.get()?;
        let file = files.get(idx)?;

        let sr = file.audio.sample_rate;
        let start = (selection.time_start * sr as f64) as usize;
        let end = (selection.time_end * sr as f64) as usize;
        let start = start.min(file.audio.samples.len());
        let end = end.min(file.audio.samples.len());

        if end <= start {
            return None;
        }

        let duration = selection.time_end - selection.time_start;
        let frames = end - start;

        // Skip expensive ZC calculation while dragging
        let (crossing_count, estimated_freq) = if dragging {
            (None, None)
        } else {
            let slice = &file.audio.samples[start..end];
            let zc = zero_crossing_frequency(slice, sr);
            (Some(zc.crossing_count), Some(zc.estimated_frequency_hz))
        };

        Some(AnalysisData {
            duration,
            frames,
            crossing_count,
            estimated_freq,
            freq_low: selection.freq_low,
            freq_high: selection.freq_high,
        })
    };

    view! {
        <div class="analysis-panel">
            {move || {
                match analysis() {
                    Some(a) => {
                        view! {
                            <span>{format!("{:.3}s", a.duration)}</span>
                            <span>{format!("{} frames", a.frames)}</span>
                            <span>{match a.crossing_count { Some(c) => format!("ZC: {c}"), None => "ZC: ...".into() }}</span>
                            <span>{match a.estimated_freq { Some(f) => format!("~{:.1} kHz", f / 1000.0), None => "~... kHz".into() }}</span>
                            <span>{format!("{:.0}-{:.0} kHz", a.freq_low / 1000.0, a.freq_high / 1000.0)}</span>
                        }.into_any()
                    }
                    None => {
                        view! {
                            <span style="color: #555">"No selection — drag on spectrogram to select"</span>
                        }.into_any()
                    }
                }
            }}
        </div>
    }
}

struct AnalysisData {
    duration: f64,
    frames: usize,
    crossing_count: Option<usize>,
    estimated_freq: Option<f64>,
    freq_low: f64,
    freq_high: f64,
}
