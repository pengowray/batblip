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
            Self::Analysis => "Analysis",
            Self::Harmonics => "Harmonics (beta)",
            Self::Metadata => "Info",
        }
    }

    pub const ALL: &'static [SidebarTab] = &[
        Self::Files,
        Self::Spectrogram,
        Self::Selection,
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
    Custom,          // user-adjusted range
}

impl FrequencyFocus {
    /// Display-range in Hz (low, high). None = show all.
    pub fn freq_range_hz(self) -> Option<(f64, f64)> {
        match self {
            Self::None | Self::Custom => None,
            Self::HumanHearing   => Some((20.0, 20_000.0)),
            Self::HumanSpeech    => Some((300.0, 3_400.0)),
            Self::Bat1           => Some((20_000.0, 35_000.0)),
            Self::Bat2           => Some((35_000.0, 50_000.0)),
            Self::Infra          => Some((10.0, 20.0)),
            Self::FullUltrasound => Some((18_000.0, f64::MAX)),
            Self::FullSpectrum   => Some((10.0, f64::MAX))
        }
    }

    /// Padded view window for this focus.
    /// Returns (view_low, view_high) in Hz — ~1/3 larger than the FF range, clamped to [0, nyquist].
    /// Returns None for FrequencyFocus::None (show everything).
    pub fn view_range_hz(self, nyquist: f64) -> Option<(f64, f64)> {
        let (ff_lo, ff_hi) = self.freq_range_hz()?;
        let ff_hi = ff_hi.min(nyquist);
        let bandwidth = ff_hi - ff_lo;
        let padding = bandwidth / 6.0; // 1/6 each side = 1/3 total extra
        let view_lo = (ff_lo - padding).max(0.0);
        let view_hi = (ff_hi + padding).min(nyquist);
        Some((view_lo, view_hi))
    }

    /// Auto listen-mode implied by this focus (used when ListenAdjustment::Auto).
    pub fn auto_listen_mode(self) -> PlaybackMode {
        match self {
            Self::None | Self::Custom | Self::HumanHearing | Self::HumanSpeech => PlaybackMode::Normal,
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
            Self::Custom         => "Custom",
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

/// Bandpass filter mode: Auto (from FF), Off, or On (manual).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BandpassMode {
    #[default]
    Auto,
    Off,
    On,
}

/// Whether the bandpass frequency range follows the Focus or is set independently.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BandpassRange {
    #[default]
    FollowFocus,
    Custom,
}

/// Which spectrogram overlay handle is being dragged / hovered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpectrogramHandle {
    FfUpper,       // FF upper boundary
    FfLower,       // FF lower boundary
    FfMiddle,      // FF midpoint (transpose whole range)
    HetCenter,     // HET center freq
    HetBandUpper,  // HET upper band edge
    HetBandLower,  // HET lower band edge
}

/// How TE / PS factors are auto-computed from the FF range.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum AutoFactorMode {
    #[default]
    Target3k,    // factor = FF_center / 3000
    MinAudible,  // factor = FF_high / 20000
    Fixed10x,    // factor = 10
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

/// Microphone capture state machine.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MicState {
    #[default]
    Off,       // Mic inactive
    Armed,     // Mic open, monitoring through speakers
    Recording, // Mic open, monitoring + accumulating samples
}

/// Which floating layer panel is currently open (only one at a time).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayerPanel {
    OverviewLayers,
    FrequencyFocus,
    ListenMode,
    Tool,
    FreqRange,
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
    pub min_display_freq: RwSignal<Option<f64>>,
    pub max_display_freq: RwSignal<Option<f64>>,
    pub mouse_freq: RwSignal<Option<f64>>,
    pub mouse_canvas_x: RwSignal<f64>,
    pub mouse_in_label_area: RwSignal<bool>,
    pub label_hover_opacity: RwSignal<f64>,
    pub follow_cursor: RwSignal<bool>,
    pub pre_play_scroll: RwSignal<f64>,
    // Filter EQ (driven by bandpass_mode effect)
    pub filter_enabled: RwSignal<bool>,
    pub filter_band_mode: RwSignal<u8>,
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

    // Listen adjustment
    pub listen_adjustment: RwSignal<ListenAdjustment>,

    // Bandpass
    pub bandpass_mode: RwSignal<BandpassMode>,
    pub bandpass_range: RwSignal<BandpassRange>,

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

    // Spectrogram drag handles (FF + HET)
    pub spec_drag_handle: RwSignal<Option<SpectrogramHandle>>,
    pub spec_hover_handle: RwSignal<Option<SpectrogramHandle>>,

    // FF frequency range (0.0 = no FF active)
    pub ff_freq_lo: RwSignal<f64>,
    pub ff_freq_hi: RwSignal<f64>,

    // Per-parameter auto flags (true = computed from FF)
    pub het_freq_auto: RwSignal<bool>,
    pub het_cutoff_auto: RwSignal<bool>,
    pub te_factor_auto: RwSignal<bool>,
    pub ps_factor_auto: RwSignal<bool>,
    pub auto_factor_mode: RwSignal<AutoFactorMode>,

    // Microphone
    pub mic_state: RwSignal<MicState>,
    pub mic_sample_rate: RwSignal<u32>,
    pub mic_samples_recorded: RwSignal<usize>,

    // Transient status message (e.g. permission errors)
    pub status_message: RwSignal<Option<String>>,

    // Mobile detection
    pub is_mobile: RwSignal<bool>,
}

fn detect_mobile() -> bool {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return false,
    };
    if let Ok(ua) = window.navigator().user_agent() {
        let ua_lower = ua.to_lowercase();
        if ua_lower.contains("android") || ua_lower.contains("iphone") || ua_lower.contains("ipad") || ua_lower.contains("mobile") {
            return true;
        }
    }
    if let Ok(w) = window.inner_width() {
        if let Some(w) = w.as_f64() {
            return w < 768.0;
        }
    }
    false
}

impl AppState {
    pub fn new() -> Self {
        let s = Self {
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
            min_display_freq: RwSignal::new(None),
            max_display_freq: RwSignal::new(None),
            mouse_freq: RwSignal::new(None),
            mouse_canvas_x: RwSignal::new(0.0),
            mouse_in_label_area: RwSignal::new(false),
            label_hover_opacity: RwSignal::new(0.0),
            follow_cursor: RwSignal::new(true),
            pre_play_scroll: RwSignal::new(0.0),
            filter_enabled: RwSignal::new(false),
            filter_band_mode: RwSignal::new(3),
            filter_freq_low: RwSignal::new(20_000.0),
            filter_freq_high: RwSignal::new(60_000.0),
            filter_db_below: RwSignal::new(-20.0),
            filter_db_selected: RwSignal::new(0.0),
            filter_db_harmonics: RwSignal::new(0.0),
            filter_db_above: RwSignal::new(-20.0),
            filter_hovering_band: RwSignal::new(None),
            filter_quality: RwSignal::new(FilterQuality::HQ),
            het_cutoff: RwSignal::new(15_000.0),
            sidebar_collapsed: RwSignal::new(false),
            sidebar_width: RwSignal::new(220.0),
            sidebar_dropdown_open: RwSignal::new(false),
            gain_db: RwSignal::new(0.0),
            auto_gain: RwSignal::new(true),

            // New
            canvas_tool: RwSignal::new(CanvasTool::Hand),
            frequency_focus: RwSignal::new(FrequencyFocus::None),
            listen_adjustment: RwSignal::new(ListenAdjustment::Auto),
            bandpass_mode: RwSignal::new(BandpassMode::Auto),
            bandpass_range: RwSignal::new(BandpassRange::FollowFocus),
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
            spec_drag_handle: RwSignal::new(None),
            spec_hover_handle: RwSignal::new(None),
            ff_freq_lo: RwSignal::new(0.0),
            ff_freq_hi: RwSignal::new(0.0),
            het_freq_auto: RwSignal::new(true),
            het_cutoff_auto: RwSignal::new(true),
            te_factor_auto: RwSignal::new(true),
            ps_factor_auto: RwSignal::new(true),
            auto_factor_mode: RwSignal::new(AutoFactorMode::Target3k),
            mic_state: RwSignal::new(MicState::Off),
            mic_sample_rate: RwSignal::new(0),
            mic_samples_recorded: RwSignal::new(0),
            status_message: RwSignal::new(None),
            is_mobile: RwSignal::new(detect_mobile()),
        };

        // On mobile, start with sidebar collapsed
        if s.is_mobile.get_untracked() {
            s.sidebar_collapsed.set(true);
        }

        s
    }

    pub fn current_file(&self) -> Option<LoadedFile> {
        let files = self.files.get();
        let idx = self.current_file_index.get()?;
        files.get(idx).cloned()
    }
}
