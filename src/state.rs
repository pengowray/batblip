use leptos::prelude::*;
use crate::types::{AudioData, SpectrogramData};

#[derive(Clone, Debug)]
pub struct LoadedFile {
    pub name: String,
    pub audio: AudioData,
    pub spectrogram: SpectrogramData,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Selection {
    pub time_start: f64,
    pub time_end: f64,
    pub freq_low: f64,
    pub freq_high: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlaybackMode {
    Normal,
    Heterodyne,
    TimeExpansion,
    PitchShift,
}

#[derive(Clone, Copy)]
pub struct AppState {
    pub files: RwSignal<Vec<LoadedFile>>,
    pub current_file_index: RwSignal<Option<usize>>,
    pub selection: RwSignal<Option<Selection>>,
    pub playback_mode: RwSignal<PlaybackMode>,
    pub het_frequency: RwSignal<f64>,
    pub te_factor: RwSignal<f64>,
    pub zoom_level: RwSignal<f64>,
    pub scroll_offset: RwSignal<f64>,
    pub is_playing: RwSignal<bool>,
    pub playhead_time: RwSignal<f64>,
    pub loading_count: RwSignal<usize>,
    pub join_files: RwSignal<bool>,
    pub auto_advance: RwSignal<bool>,
    pub ps_factor: RwSignal<f64>,
    pub het_interacting: RwSignal<bool>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            files: RwSignal::new(Vec::new()),
            current_file_index: RwSignal::new(None),
            selection: RwSignal::new(None),
            playback_mode: RwSignal::new(PlaybackMode::Normal),
            het_frequency: RwSignal::new(45_000.0),
            te_factor: RwSignal::new(10.0),
            zoom_level: RwSignal::new(1.0),
            scroll_offset: RwSignal::new(0.0),
            is_playing: RwSignal::new(false),
            playhead_time: RwSignal::new(0.0),
            loading_count: RwSignal::new(0),
            join_files: RwSignal::new(false),
            auto_advance: RwSignal::new(true),
            ps_factor: RwSignal::new(10.0),
            het_interacting: RwSignal::new(false),
        }
    }

    pub fn current_file(&self) -> Option<LoadedFile> {
        let files = self.files.get();
        let idx = self.current_file_index.get()?;
        files.get(idx).cloned()
    }
}
