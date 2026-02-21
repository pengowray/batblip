use leptos::prelude::*;
use crate::types::{AudioData, PreviewImage, SpectrogramData};

#[derive(Clone, Debug)]
pub struct LoadedFile {
    pub name: String,
    pub audio: AudioData,
    pub spectrogram: SpectrogramData,
    pub preview: Option<PreviewImage>,
    pub xc_metadata: Option<Vec<(String, String)>>,
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
    ZeroCrossing,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum SpectrogramDisplay {
    MovementCentroid,
    MovementGradient,
    #[default]
    MovementFlow,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum SidebarTab {
    #[default]
    Files,
    Spectrogram,
    Selection,
    PreProcessing,
    Analysis,
    Harmonics,
    Metadata,
}

impl SidebarTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Spectrogram => "Display",
            Self::Selection => "Selection",
            Self::PreProcessing => "EQ",
            Self::Analysis => "Analysis",
            Self::Harmonics => "Harmonics (beta)",
            Self::Metadata => "Info",
        }
    }

    pub const ALL: &'static [SidebarTab] = &[
        Self::Files,
        Self::Spectrogram,
        Self::Selection,
        Self::PreProcessing,
        Self::Analysis,
        Self::Harmonics,
        Self::Metadata,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum FilterQuality {
    #[default]
    Fast,
    HQ,
}

// ── New enums ────────────────────────────────────────────────────────────────

/// Which frequency range to focus on (affects display + auto-listen mode).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum FrequencyFocus {
    #[default]
    None,
    HumanHearing,    // 20 Hz – 20 kHz
    HumanSpeech,     // 300 Hz – 3.4 kHz
    Bat1,            // 20k – 35 kHz
    Bat2,            // 35k – 50 kHz
    Infra,           // 10 – 20 Hz
    FullUltrasound,  // 18 kHz – Nyquist
    FullSpectrum,    // 10 Hz – Nyquist
}

impl FrequencyFocus {
    /// Display-range in Hz (low, high). None = show all.
    pub fn freq_range_hz(self) -> Option<(f64, f64)> {
        match self {
            Self::None | 
            Self::HumanHearing   => Some((20.0, 20_000.0)),
            Self::HumanSpeech    => Some((300.0, 3_400.0)),
            Self::Bat1           => Some((20_000.0, 35_000.0)),
            Self::Bat2           => Some((35_000.0, 50_000.0)),
            Self::Infra          => Some((10.0, 20.0)),
            Self::FullUltrasound => Some((18_000.0, f64::MAX)),
            Self::FullSpectrum   => Some((10.0, f64::MAX))
        }
    }

    /// Auto listen-mode implied by this focus (used when ListenAdjustment::Auto).
    pub fn auto_listen_mode(self) -> PlaybackMode {
        match self {
            Self::None | Self::HumanHearing | Self::HumanSpeech => PlaybackMode::Normal,
            Self::Bat1 | Self::Bat2 => PlaybackMode::Heterodyne,
            Self::Infra => PlaybackMode::TimeExpansion,
            Self::FullUltrasound | Self::FullSpectrum => PlaybackMode::PitchShift,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None           => "None",
            Self::HumanHearing   => "Human hearing",
            Self::HumanSpeech    => "Human speech",
            Self::Bat1           => "Bat 1 (20–35k)",
            Self::Bat2           => "Bat 2 (35–50k)",
            Self::Infra          => "Infra (10–20)",
            Self::FullUltrasound => "Full ultrasound (18k+)",
            Self::FullSpectrum   => "Full spectrum (10hz+)",
        }
    }

    pub const ALL: &'static [FrequencyFocus] = &[
        Self::None, Self::HumanHearing, Self::HumanSpeech,
        Self::Bat1, Self::Bat2, Self::Infra,
        Self::FullUltrasound, Self::FullSpectrum,
    ];
}

/// Whether the listen mode is driven automatically by FrequencyFocus or set manually.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ListenAdjustment {
    #[default]
    Auto,
    Manual,
}

/// Bandpass filter strength applied during listen.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BandpassStrength {
    #[default]
    Auto,
    Off,
    Some,
    Strong,
    Custom,
}

/// Active interaction tool for the main spectrogram canvas.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum CanvasTool {
    #[default]
    Hand,      // drag to pan
    Selection, // drag to select
}

/// What the overview strip shows.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum OverviewView {
    #[default]
    Spectrogram,
    Waveform,
}

/// Which frequency range the overview displays.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum OverviewFreqMode {
    #[default]
    All,
    Human,      // 20 Hz – 20 kHz
    MatchMain,  // tracks max_display_freq
}

/// Which floating layer panel is currently open (only one at a time).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayerPanel {
    OverviewLayers,
    FrequencyFocus,
    ListenMode,
    Tool,
}

/// A navigation history entry (for overview back/forward buttons).
#[derive(Clone, Copy, Debug)]
pub struct NavEntry {
    pub scroll_offset: f64,
    pub zoom_level: f64,
}

/// A time-position bookmark created during or after playback.
#[derive(Clone, Copy, Debug)]
pub struct Bookmark {
    pub time: f64,
}

// ── AppState ─────────────────────────────────────────────────────────────────

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
    pub zc_factor: RwSignal<f64>,
    pub het_interacting: RwSignal<bool>,
    pub is_dragging: RwSignal<bool>,
    pub spectrogram_display: RwSignal<SpectrogramDisplay>,
    pub mv_enabled: RwSignal<bool>,
    pub sidebar_tab: RwSignal<SidebarTab>,
    pub mv_intensity_gate: RwSignal<f32>,
    pub mv_movement_gate: RwSignal<f32>,
    pub mv_opacity: RwSignal<f32>,
    pub max_display_freq: RwSignal<Option<f64>>,
    pub mouse_freq: RwSignal<Option<f64>>,
    pub mouse_canvas_x: RwSignal<f64>,
    pub label_hover_opacity: RwSignal<f64>,
    pub follow_cursor: RwSignal<bool>,
    pub pre_play_scroll: RwSignal<f64>,
    // Filter EQ
    pub filter_enabled: RwSignal<bool>,
    pub filter_band_mode: RwSignal<u8>,
    pub filter_set_from_selection: RwSignal<bool>,
    pub filter_freq_low: RwSignal<f64>,
    pub filter_freq_high: RwSignal<f64>,
    pub filter_db_below: RwSignal<f64>,
    pub filter_db_selected: RwSignal<f64>,
    pub filter_db_harmonics: RwSignal<f64>,
    pub filter_db_above: RwSignal<f64>,
    pub filter_hovering_band: RwSignal<Option<u8>>,
    pub filter_quality: RwSignal<FilterQuality>,
    pub het_cutoff: RwSignal<f64>,
    pub sidebar_collapsed: RwSignal<bool>,
    pub sidebar_width: RwSignal<f64>,
    pub sidebar_dropdown_open: RwSignal<bool>,
    // Gain
    pub gain_db: RwSignal<f64>,
    pub auto_gain: RwSignal<bool>,

    // ── New signals ──────────────────────────────────────────────────────────

    // Tool
    pub canvas_tool: RwSignal<CanvasTool>,

    // Frequency Focus
    pub frequency_focus: RwSignal<FrequencyFocus>,
    pub ff_filter_strength: RwSignal<BandpassStrength>,

    // Listen adjustment
    pub listen_adjustment: RwSignal<ListenAdjustment>,
    pub bandpass_strength: RwSignal<BandpassStrength>,
    pub bandpass_freq_low: RwSignal<f64>,
    pub bandpass_freq_high: RwSignal<f64>,

    // Overview
    pub overview_view: RwSignal<OverviewView>,
    pub overview_freq_mode: RwSignal<OverviewFreqMode>,

    // Navigation history (for back/forward buttons in overview)
    pub nav_history: RwSignal<Vec<NavEntry>>,
    pub nav_index: RwSignal<usize>,

    // Bookmarks
    pub bookmarks: RwSignal<Vec<Bookmark>>,
    pub show_bookmark_popup: RwSignal<bool>,

    // Play-from-here time (updated by Spectrogram on scroll/zoom change)
    pub play_from_here_time: RwSignal<f64>,

    // Tile system: incrementing this triggers a spectrogram redraw
    pub tile_ready_signal: RwSignal<u32>,

    // Which floating layer panel is currently open
    pub layer_panel_open: RwSignal<Option<LayerPanel>>,

    // Actual pixel width of the main spectrogram canvas (written by Spectrogram, read by Overview)
    pub spectrogram_canvas_width: RwSignal<f64>,

    // Main panel view mode (Spectrogram or Waveform)
    pub main_view: RwSignal<OverviewView>,
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
            zc_factor: RwSignal::new(8.0),
            het_interacting: RwSignal::new(false),
            is_dragging: RwSignal::new(false),
            spectrogram_display: RwSignal::new(SpectrogramDisplay::MovementFlow),
            mv_enabled: RwSignal::new(false),
            sidebar_tab: RwSignal::new(SidebarTab::Files),
            mv_intensity_gate: RwSignal::new(0.5),
            mv_movement_gate: RwSignal::new(0.75),
            mv_opacity: RwSignal::new(0.75),
            max_display_freq: RwSignal::new(None),
            mouse_freq: RwSignal::new(None),
            mouse_canvas_x: RwSignal::new(0.0),
            label_hover_opacity: RwSignal::new(0.0),
            follow_cursor: RwSignal::new(true),
            pre_play_scroll: RwSignal::new(0.0),
            filter_enabled: RwSignal::new(false),
            filter_band_mode: RwSignal::new(3),
            filter_set_from_selection: RwSignal::new(false),
            filter_freq_low: RwSignal::new(20_000.0),
            filter_freq_high: RwSignal::new(60_000.0),
            filter_db_below: RwSignal::new(-60.0),
            filter_db_selected: RwSignal::new(0.0),
            filter_db_harmonics: RwSignal::new(0.0),
            filter_db_above: RwSignal::new(0.0),
            filter_hovering_band: RwSignal::new(None),
            filter_quality: RwSignal::new(FilterQuality::Fast),
            het_cutoff: RwSignal::new(15_000.0),
            sidebar_collapsed: RwSignal::new(false),
            sidebar_width: RwSignal::new(220.0),
            sidebar_dropdown_open: RwSignal::new(false),
            gain_db: RwSignal::new(0.0),
            auto_gain: RwSignal::new(true),

            // New
            canvas_tool: RwSignal::new(CanvasTool::Hand),
            frequency_focus: RwSignal::new(FrequencyFocus::None),
            ff_filter_strength: RwSignal::new(BandpassStrength::Off),
            listen_adjustment: RwSignal::new(ListenAdjustment::Auto),
            bandpass_strength: RwSignal::new(BandpassStrength::Auto),
            bandpass_freq_low: RwSignal::new(20_000.0),
            bandpass_freq_high: RwSignal::new(60_000.0),
            overview_view: RwSignal::new(OverviewView::Spectrogram),
            overview_freq_mode: RwSignal::new(OverviewFreqMode::All),
            nav_history: RwSignal::new(Vec::new()),
            nav_index: RwSignal::new(0),
            bookmarks: RwSignal::new(Vec::new()),
            show_bookmark_popup: RwSignal::new(false),
            play_from_here_time: RwSignal::new(0.0),
            tile_ready_signal: RwSignal::new(0),
            layer_panel_open: RwSignal::new(None),
            spectrogram_canvas_width: RwSignal::new(1000.0),
            main_view: RwSignal::new(OverviewView::Spectrogram),
        }
    }

    pub fn current_file(&self) -> Option<LoadedFile> {
        let files = self.files.get();
        let idx = self.current_file_index.get()?;
        files.get(idx).cloned()
    }
}
