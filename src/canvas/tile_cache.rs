//! Progressive tile cache for spectrogram rendering.
//!
//! Google Maps-style LOD system: each LOD level has its own tile index space.
//! All tiles have the same number of columns (`TILE_COLS = 256`), but different
//! LODs cover different time ranges:
//!
//! - LOD 0: hop=2048, covers ~524K samples/tile (wide, blurry)
//! - LOD 1: hop=512, covers ~131K samples/tile (normal)
//! - LOD 2: hop=128, covers ~33K samples/tile (zoomed in)
//! - LOD 3: hop=32, covers ~8K samples/tile (deep zoom)
//!
//! Each level is 4× finer than the previous. The renderer picks the ideal LOD
//! for the current zoom and falls back to lower LODs when tiles aren't cached.
//!
//! The cache uses an LRU eviction policy capped at `MAX_BYTES` total pixel storage.

use std::cell::RefCell;
use std::collections::HashMap;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::canvas::spectrogram_renderer::{self, PreRendered, FlowAlgo};
use crate::state::{AppState, LoadedFile};

/// Number of spectrogram columns per tile (constant across all LODs).
pub const TILE_COLS: usize = 256;

/// ~120 MB cap for tile pixel data.
const MAX_BYTES: usize = 120 * 1024 * 1024;

// ── LOD configuration ────────────────────────────────────────────────────────

pub struct LodConfig {
    pub fft_size: usize,
    pub hop_size: usize,
}

pub const NUM_LODS: usize = 4;

pub const LOD_CONFIGS: [LodConfig; NUM_LODS] = [
    LodConfig { fft_size: 512,  hop_size: 2048 }, // LOD 0 — wide, blurry preview
    LodConfig { fft_size: 2048, hop_size: 512 },  // LOD 1 — normal resolution
    LodConfig { fft_size: 2048, hop_size: 128 },  // LOD 2 — zoomed in
    LodConfig { fft_size: 2048, hop_size: 32 },   // LOD 3 — deep zoom
];

/// Select the ideal LOD level for the current zoom.
/// `zoom` is pixels per LOD1 column.
pub fn select_lod(zoom: f64) -> u8 {
    if zoom >= 8.0 { 3 }
    else if zoom >= 2.0 { 2 }
    else if zoom >= 0.5 { 1 }
    else { 0 }
}

/// Ratio of LOD1 columns to LOD_L columns (how many LOD_L cols per LOD1 col).
/// LOD0: 0.25, LOD1: 1.0, LOD2: 4.0, LOD3: 16.0
pub fn lod_ratio(lod: u8) -> f64 {
    LOD_CONFIGS[1].hop_size as f64 / LOD_CONFIGS[lod as usize].hop_size as f64
}

/// Tile count at a given LOD for a file with `total_samples` audio samples.
pub fn tile_count_for_samples(total_samples: usize, lod: u8) -> usize {
    let config = &LOD_CONFIGS[lod as usize];
    if total_samples < config.fft_size { return 0; }
    let total_cols = (total_samples - config.fft_size) / config.hop_size + 1;
    (total_cols + TILE_COLS - 1) / TILE_COLS
}

/// Map a tile index from one LOD to the corresponding tile at a lower (coarser) LOD.
/// Returns (fallback_tile_idx, sub_col_start, sub_col_end) — the sub-region within
/// the fallback tile that covers the same time range.
pub fn fallback_tile_info(target_lod: u8, target_tile: usize, fallback_lod: u8) -> (usize, f64, f64) {
    let target_hop = LOD_CONFIGS[target_lod as usize].hop_size;
    let fb_hop = LOD_CONFIGS[fallback_lod as usize].hop_size;

    // Sample range of the target tile
    let sample_start = target_tile * TILE_COLS * target_hop;
    let sample_end = sample_start + TILE_COLS * target_hop;

    // Convert to fallback tile/column space
    let fb_col_start = sample_start as f64 / fb_hop as f64;
    let fb_col_end = sample_end as f64 / fb_hop as f64;

    let fb_tile = (fb_col_start / TILE_COLS as f64).floor() as usize;
    let fb_src_start = fb_col_start - (fb_tile * TILE_COLS) as f64;
    let fb_src_end = fb_col_end - (fb_tile * TILE_COLS) as f64;

    (fb_tile, fb_src_start, fb_src_end)
}

// ── Cache data structures ────────────────────────────────────────────────────

/// Cache key: (file_idx, lod, tile_idx)
type CacheKey = (usize, u8, usize);

pub struct Tile {
    pub tile_idx: usize,
    pub file_idx: usize,
    pub lod: u8,
    pub rendered: PreRendered,
}

struct TileCache {
    tiles: HashMap<CacheKey, Tile>,
    /// LRU order: front = oldest, back = most recently used
    lru: Vec<CacheKey>,
    total_bytes: usize,
}

impl TileCache {
    fn new() -> Self {
        Self { tiles: HashMap::new(), lru: Vec::new(), total_bytes: 0 }
    }

    fn insert(&mut self, file_idx: usize, lod: u8, tile_idx: usize, rendered: PreRendered) {
        let key = (file_idx, lod, tile_idx);
        let bytes = rendered.byte_len();
        // Remove old entry if replacing
        if let Some(old) = self.tiles.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(old.rendered.byte_len());
            self.lru.retain(|k| k != &key);
        }
        // Evict until under cap
        while self.total_bytes + bytes > MAX_BYTES && !self.lru.is_empty() {
            let oldest = self.lru.remove(0);
            if let Some(evicted) = self.tiles.remove(&oldest) {
                self.total_bytes = self.total_bytes.saturating_sub(evicted.rendered.byte_len());
            }
        }
        self.total_bytes += bytes;
        self.tiles.insert(key, Tile { tile_idx, file_idx, lod, rendered });
        self.lru.push(key);
    }

    fn get(&self, file_idx: usize, lod: u8, tile_idx: usize) -> Option<&Tile> {
        self.tiles.get(&(file_idx, lod, tile_idx))
    }

    fn touch(&mut self, key: CacheKey) {
        self.lru.retain(|k| k != &key);
        self.lru.push(key);
    }

    fn evict_far_from(&mut self, file_idx: usize, lod: u8, center_tile: usize, keep_radius: usize) {
        let keys_to_evict: Vec<CacheKey> = self.tiles.keys().copied()
            .filter(|&(fi, l, ti)| {
                fi == file_idx && l == lod && ti.abs_diff(center_tile) > keep_radius
            })
            .collect();
        for key in keys_to_evict {
            if let Some(evicted) = self.tiles.remove(&key) {
                self.total_bytes = self.total_bytes.saturating_sub(evicted.rendered.byte_len());
                self.lru.retain(|k| k != &key);
            }
        }
    }

    fn clear_for_file(&mut self, file_idx: usize) {
        let keys: Vec<_> = self.tiles.keys().copied().filter(|k| k.0 == file_idx).collect();
        for key in keys {
            if let Some(evicted) = self.tiles.remove(&key) {
                self.total_bytes = self.total_bytes.saturating_sub(evicted.rendered.byte_len());
                self.lru.retain(|k| k != &key);
            }
        }
    }
}

// ── Flow cache key (LOD1-only for now) ───────────────────────────────────────
type FlowKey = (usize, usize); // (file_idx, tile_idx)

struct FlowTileCache {
    tiles: HashMap<FlowKey, Tile>,
    lru: Vec<FlowKey>,
    total_bytes: usize,
}

impl FlowTileCache {
    fn new() -> Self {
        Self { tiles: HashMap::new(), lru: Vec::new(), total_bytes: 0 }
    }

    fn insert(&mut self, file_idx: usize, tile_idx: usize, rendered: PreRendered) {
        let key = (file_idx, tile_idx);
        let bytes = rendered.byte_len();
        if let Some(old) = self.tiles.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(old.rendered.byte_len());
            self.lru.retain(|k| k != &key);
        }
        while self.total_bytes + bytes > MAX_BYTES && !self.lru.is_empty() {
            let oldest = self.lru.remove(0);
            if let Some(evicted) = self.tiles.remove(&oldest) {
                self.total_bytes = self.total_bytes.saturating_sub(evicted.rendered.byte_len());
            }
        }
        self.total_bytes += bytes;
        self.tiles.insert(key, Tile { tile_idx, file_idx, lod: 1, rendered });
        self.lru.push(key);
    }

    fn get(&self, file_idx: usize, tile_idx: usize) -> Option<&Tile> {
        self.tiles.get(&(file_idx, tile_idx))
    }

    fn touch(&mut self, key: FlowKey) {
        self.lru.retain(|k| k != &key);
        self.lru.push(key);
    }

    fn clear_for_file(&mut self, file_idx: usize) {
        let keys: Vec<_> = self.tiles.keys().copied().filter(|k| k.0 == file_idx).collect();
        for key in keys {
            if let Some(evicted) = self.tiles.remove(&key) {
                self.total_bytes = self.total_bytes.saturating_sub(evicted.rendered.byte_len());
                self.lru.retain(|k| k != &key);
            }
        }
    }
}

// Reuse FlowTileCache type for chroma too (same key shape)
type ChromaKey = (usize, usize);

struct ChromaTileCache {
    tiles: HashMap<ChromaKey, Tile>,
    lru: Vec<ChromaKey>,
    total_bytes: usize,
}

impl ChromaTileCache {
    fn new() -> Self {
        Self { tiles: HashMap::new(), lru: Vec::new(), total_bytes: 0 }
    }

    fn insert(&mut self, file_idx: usize, tile_idx: usize, rendered: PreRendered) {
        let key = (file_idx, tile_idx);
        let bytes = rendered.byte_len();
        if let Some(old) = self.tiles.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(old.rendered.byte_len());
            self.lru.retain(|k| k != &key);
        }
        while self.total_bytes + bytes > MAX_BYTES && !self.lru.is_empty() {
            let oldest = self.lru.remove(0);
            if let Some(evicted) = self.tiles.remove(&oldest) {
                self.total_bytes = self.total_bytes.saturating_sub(evicted.rendered.byte_len());
            }
        }
        self.total_bytes += bytes;
        self.tiles.insert(key, Tile { tile_idx, file_idx, lod: 1, rendered });
        self.lru.push(key);
    }

    fn get(&self, file_idx: usize, tile_idx: usize) -> Option<&Tile> {
        self.tiles.get(&(file_idx, tile_idx))
    }

    fn touch(&mut self, key: ChromaKey) {
        self.lru.retain(|k| k != &key);
        self.lru.push(key);
    }
}

thread_local! {
    /// Unified magnitude tile cache — all LOD levels in one cache.
    static CACHE: RefCell<TileCache> = RefCell::new(TileCache::new());
    /// Set of (file_idx, lod, tile_idx) currently being generated.
    static IN_FLIGHT: RefCell<std::collections::HashSet<CacheKey>> =
        RefCell::new(std::collections::HashSet::new());

    /// Flow-mode tile cache (LOD1-only for now).
    static FLOW_CACHE: RefCell<FlowTileCache> = RefCell::new(FlowTileCache::new());
    static FLOW_IN_FLIGHT: RefCell<std::collections::HashSet<FlowKey>> =
        RefCell::new(std::collections::HashSet::new());

    /// Chromagram tile cache (LOD1-only).
    static CHROMA_CACHE: RefCell<ChromaTileCache> = RefCell::new(ChromaTileCache::new());
    static CHROMA_IN_FLIGHT: RefCell<std::collections::HashSet<ChromaKey>> =
        RefCell::new(std::collections::HashSet::new());

    /// Cached per-file global chromagram normalisation maxima (max_class, max_note).
    static CHROMA_GLOBAL_MAX: RefCell<HashMap<usize, (f32, f32)>> =
        RefCell::new(HashMap::new());
}

// ── Public API: magnitude tile cache ─────────────────────────────────────────

pub fn get_tile(file_idx: usize, lod: u8, tile_idx: usize) -> Option<()> {
    CACHE.with(|c| c.borrow().get(file_idx, lod, tile_idx).map(|_| ()))
}

pub fn borrow_tile<R>(file_idx: usize, lod: u8, tile_idx: usize, f: impl FnOnce(&Tile) -> R) -> Option<R> {
    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        let key = (file_idx, lod, tile_idx);
        if cache.tiles.contains_key(&key) {
            cache.touch(key);
            drop(cache);
            CACHE.with(|c| {
                c.borrow().tiles.get(&key).map(|t| f(t))
            })
        } else {
            None
        }
    })
}

pub fn clear_file(file_idx: usize) {
    CACHE.with(|c| c.borrow_mut().clear_for_file(file_idx));
    IN_FLIGHT.with(|s| s.borrow_mut().retain(|k| k.0 != file_idx));
}

pub fn evict_far(file_idx: usize, lod: u8, center_tile: usize, keep_radius: usize) {
    CACHE.with(|c| c.borrow_mut().evict_far_from(file_idx, lod, center_tile, keep_radius));
}

/// Returns the number of complete LOD1 tiles for a file currently in the cache.
pub fn tiles_ready(file_idx: usize, n_tiles: usize) -> usize {
    CACHE.with(|c| {
        let cache = c.borrow();
        (0..n_tiles).filter(|&i| cache.tiles.contains_key(&(file_idx, 1, i))).count()
    })
}

// ── Generic LOD tile scheduling ──────────────────────────────────────────────

/// Schedule a tile at any LOD level. Computes STFT from audio samples.
pub fn schedule_tile_lod(state: AppState, file_idx: usize, lod: u8, tile_idx: usize) {
    use crate::dsp::fft::compute_spectrogram_partial;

    let key: CacheKey = (file_idx, lod, tile_idx);
    if CACHE.with(|c| c.borrow().tiles.contains_key(&key)) { return; }
    if IN_FLIGHT.with(|s| s.borrow().contains(&key)) { return; }
    IN_FLIGHT.with(|s| s.borrow_mut().insert(key));

    let config_fft = LOD_CONFIGS[lod as usize].fft_size;
    let config_hop = LOD_CONFIGS[lod as usize].hop_size;

    spawn_local(async move {
        yield_to_browser().await;

        // Extra yields for expensive LODs (LOD2, LOD3) and non-current files
        if lod >= 2 {
            yield_to_browser().await;
        }
        let is_current = state.current_file_index.get_untracked() == Some(file_idx);
        if !is_current {
            for _ in 0..3 {
                yield_to_browser().await;
            }
        }

        let audio = state.files.with_untracked(|files| {
            files.get(file_idx).map(|f| f.audio.clone())
        });
        let Some(audio) = audio else {
            IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        };

        // Compute STFT columns for this tile
        let col_start = tile_idx * TILE_COLS;
        let cols = compute_spectrogram_partial(
            &audio, config_fft, config_hop, col_start, TILE_COLS,
        );
        IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));

        if cols.is_empty() { return; }

        let rendered = spectrogram_renderer::pre_render_columns(&cols);
        CACHE.with(|c| c.borrow_mut().insert(file_idx, lod, tile_idx, rendered));
        state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
    });
}

// ── LOD 1-specific scheduling (from in-memory columns / spectral store) ─────

/// Schedule generation of a LOD1 tile from in-memory spectrogram columns.
/// Used during initial file loading when LoadedFile.spectrogram.columns is available.
pub fn schedule_tile(state: AppState, file: LoadedFile, file_idx: usize, tile_idx: usize) {
    let key: CacheKey = (file_idx, 1, tile_idx);
    if CACHE.with(|c| c.borrow().tiles.contains_key(&key)) { return; }
    if IN_FLIGHT.with(|s| s.borrow().contains(&key)) { return; }
    IN_FLIGHT.with(|s| s.borrow_mut().insert(key));

    spawn_local(async move {
        yield_to_browser().await;

        let is_current = state.current_file_index.get_untracked() == Some(file_idx);
        if !is_current {
            for _ in 0..3 {
                yield_to_browser().await;
            }
        }

        let still_loaded = state.files.with_untracked(|files| {
            files.get(file_idx).map(|f| f.name == file.name).unwrap_or(false)
        });
        if !still_loaded {
            IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        }

        let col_start = tile_idx * TILE_COLS;
        let col_end = (col_start + TILE_COLS).min(file.spectrogram.columns.len());
        if col_start >= col_end {
            IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        }

        let rendered = spectrogram_renderer::pre_render_columns(
            &file.spectrogram.columns[col_start..col_end],
        );

        CACHE.with(|c| c.borrow_mut().insert(file_idx, 1, tile_idx, rendered));
        IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
        state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
    });
}

/// Schedule generation of all LOD1 tiles for a file (called after file load).
pub fn schedule_all_tiles(state: AppState, file: LoadedFile, file_idx: usize) {
    let total_cols = if file.spectrogram.total_columns > 0 {
        file.spectrogram.total_columns
    } else {
        file.spectrogram.columns.len()
    };
    if total_cols == 0 { return; }
    let n_tiles = (total_cols + TILE_COLS - 1) / TILE_COLS;

    for tile_idx in 0..n_tiles {
        schedule_tile(state.clone(), file.clone(), file_idx, tile_idx);
    }
}

/// Render a LOD1 tile synchronously from the spectral column store.
pub fn render_tile_from_store_sync(file_idx: usize, tile_idx: usize) -> bool {
    use crate::canvas::spectral_store;

    let key: CacheKey = (file_idx, 1, tile_idx);
    if CACHE.with(|c| c.borrow().tiles.contains_key(&key)) { return true; }

    let col_start = tile_idx * TILE_COLS;
    let col_end = col_start + TILE_COLS;

    let rendered = spectral_store::with_columns(file_idx, col_start, col_end, |cols, _max_mag| {
        spectrogram_renderer::pre_render_columns(cols)
    });

    if let Some(rendered) = rendered {
        CACHE.with(|c| c.borrow_mut().insert(file_idx, 1, tile_idx, rendered));
        true
    } else {
        false
    }
}

/// Render a partial (live) LOD1 tile from the spectral store.
pub fn render_live_tile_sync(file_idx: usize, tile_idx: usize, col_start: usize, available_cols: usize) -> bool {
    use crate::canvas::spectral_store;

    let col_end = col_start + available_cols;
    let rendered = spectral_store::with_columns(file_idx, col_start, col_end, |cols, _max_mag| {
        let partial = spectrogram_renderer::pre_render_columns(cols);

        if partial.width == 0 || partial.height == 0 {
            return partial;
        }

        if available_cols >= TILE_COLS {
            return partial;
        }

        let full_width = TILE_COLS as u32;
        let height = partial.height;

        if !partial.db_data.is_empty() {
            let mut full_db = vec![f32::NEG_INFINITY; (full_width * height) as usize];
            for y in 0..height {
                let src_start = (y * partial.width) as usize;
                let src_end = src_start + partial.width as usize;
                let dst_start = (y * full_width) as usize;
                let dst_end = dst_start + partial.width as usize;
                if src_end <= partial.db_data.len() {
                    full_db[dst_start..dst_end]
                        .copy_from_slice(&partial.db_data[src_start..src_end]);
                }
            }
            PreRendered {
                width: full_width,
                height,
                pixels: Vec::new(),
                db_data: full_db,
                flow_shifts: Vec::new(),
            }
        } else {
            let mut full_pixels = vec![0u8; (full_width * height * 4) as usize];
            for y in 0..height {
                let src_start = (y * partial.width * 4) as usize;
                let src_end = src_start + (partial.width * 4) as usize;
                let dst_start = (y * full_width * 4) as usize;
                let dst_end = dst_start + (partial.width * 4) as usize;
                if src_end <= partial.pixels.len() {
                    full_pixels[dst_start..dst_end]
                        .copy_from_slice(&partial.pixels[src_start..src_end]);
                }
            }
            PreRendered {
                width: full_width,
                height,
                pixels: full_pixels,
                db_data: Vec::new(),
                flow_shifts: Vec::new(),
            }
        }
    });

    if let Some(rendered) = rendered {
        CACHE.with(|c| c.borrow_mut().insert(file_idx, 1, tile_idx, rendered));
        true
    } else {
        false
    }
}

/// Schedule LOD1 tile generation from the spectral column store.
pub fn schedule_tile_from_store(state: AppState, file_idx: usize, tile_idx: usize) {
    use crate::canvas::spectral_store;

    let key: CacheKey = (file_idx, 1, tile_idx);
    if CACHE.with(|c| c.borrow().tiles.contains_key(&key)) { return; }
    if IN_FLIGHT.with(|s| s.borrow().contains(&key)) { return; }
    IN_FLIGHT.with(|s| s.borrow_mut().insert(key));

    spawn_local(async move {
        yield_to_browser().await;

        let is_current = state.current_file_index.get_untracked() == Some(file_idx);
        if !is_current {
            for _ in 0..3 {
                yield_to_browser().await;
            }
        }

        let still_loaded = state.files.with_untracked(|files| {
            file_idx < files.len()
        });
        if !still_loaded {
            IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        }

        let col_start = tile_idx * TILE_COLS;
        let col_end = col_start + TILE_COLS;

        let rendered = spectral_store::with_columns(file_idx, col_start, col_end, |cols, _max_mag| {
            spectrogram_renderer::pre_render_columns(cols)
        });

        IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));

        if let Some(rendered) = rendered {
            CACHE.with(|c| c.borrow_mut().insert(file_idx, 1, tile_idx, rendered));
            state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
        }
    });
}

/// Schedule visible LOD1 tiles from the spectral store.
pub fn schedule_visible_tiles_from_store(state: AppState, file_idx: usize, total_cols: usize) {
    if total_cols == 0 { return; }
    let n_tiles = (total_cols + TILE_COLS - 1) / TILE_COLS;

    let time_res = state.files.with_untracked(|files| {
        files.get(file_idx).map(|f| f.spectrogram.time_resolution).unwrap_or(0.01)
    });
    let scroll = state.scroll_offset.get_untracked();
    let zoom = state.zoom_level.get_untracked();
    let canvas_w = state.spectrogram_canvas_width.get_untracked();
    let visible_time = if zoom > 0.0 { canvas_w / zoom * time_res } else { 1.0 };
    let center_col = ((scroll + visible_time / 2.0) / time_res) as usize;
    let center_tile = center_col / TILE_COLS;

    let max_schedule = 20.min(n_tiles);
    let mut scheduled = 0;
    let mut dist = 0usize;
    while scheduled < max_schedule {
        let tiles: Vec<usize> = if dist == 0 {
            vec![center_tile]
        } else {
            let mut v = Vec::new();
            if let Some(l) = center_tile.checked_sub(dist) {
                if l < n_tiles { v.push(l); }
            }
            if center_tile + dist < n_tiles {
                v.push(center_tile + dist);
            }
            v
        };
        if tiles.is_empty() { break; }
        for t in tiles {
            schedule_tile_from_store(state.clone(), file_idx, t);
            scheduled += 1;
        }
        dist += 1;
    }
}

/// Schedule LOD1 on-demand tile computation from audio samples.
pub fn schedule_tile_on_demand(
    state: AppState,
    file_idx: usize,
    tile_idx: usize,
) {
    use crate::canvas::spectral_store;
    use crate::dsp::fft::compute_spectrogram_partial;

    let key: CacheKey = (file_idx, 1, tile_idx);
    if CACHE.with(|c| c.borrow().tiles.contains_key(&key)) { return; }
    if IN_FLIGHT.with(|s| s.borrow().contains(&key)) { return; }
    IN_FLIGHT.with(|s| s.borrow_mut().insert(key));

    spawn_local(async move {
        yield_to_browser().await;

        let is_current = state.current_file_index.get_untracked() == Some(file_idx);
        if !is_current {
            for _ in 0..3 {
                yield_to_browser().await;
            }
        }

        let audio = state.files.with_untracked(|files| {
            files.get(file_idx).map(|f| f.audio.clone())
        });
        let Some(audio) = audio else {
            IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        };

        let col_start = tile_idx * TILE_COLS;

        let cols = compute_spectrogram_partial(&audio, 2048, 512, col_start, TILE_COLS);
        if cols.is_empty() {
            IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        }

        spectral_store::insert_columns(file_idx, col_start, &cols);

        let rendered = spectrogram_renderer::pre_render_columns(&cols);

        CACHE.with(|c| c.borrow_mut().insert(file_idx, 1, tile_idx, rendered));
        IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
        state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
    });
}

// ── Flow tile cache (LOD1-only) ──────────────────────────────────────────────

pub fn get_flow_tile(file_idx: usize, tile_idx: usize) -> Option<()> {
    FLOW_CACHE.with(|c| c.borrow().get(file_idx, tile_idx).map(|_| ()))
}

pub fn borrow_flow_tile<R>(file_idx: usize, tile_idx: usize, f: impl FnOnce(&Tile) -> R) -> Option<R> {
    FLOW_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        let key = (file_idx, tile_idx);
        if cache.tiles.contains_key(&key) {
            cache.touch(key);
            drop(cache);
            FLOW_CACHE.with(|c| {
                c.borrow().tiles.get(&key).map(|t| f(t))
            })
        } else {
            None
        }
    })
}

pub fn clear_flow_cache() {
    FLOW_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        cache.tiles.clear();
        cache.lru.clear();
        cache.total_bytes = 0;
    });
    FLOW_IN_FLIGHT.with(|s| s.borrow_mut().clear());
}

pub fn clear_flow_file(file_idx: usize) {
    FLOW_CACHE.with(|c| c.borrow_mut().clear_for_file(file_idx));
    FLOW_IN_FLIGHT.with(|s| s.borrow_mut().retain(|k| k.0 != file_idx));
}

/// Schedule a flow tile for background generation (LOD1).
pub fn schedule_flow_tile(
    state: AppState,
    file_idx: usize,
    tile_idx: usize,
    algo: FlowAlgo,
) {
    use crate::canvas::spectral_store;

    let key = (file_idx, tile_idx);
    if FLOW_CACHE.with(|c| c.borrow().tiles.contains_key(&key)) { return; }
    if FLOW_IN_FLIGHT.with(|s| s.borrow().contains(&key)) { return; }
    FLOW_IN_FLIGHT.with(|s| s.borrow_mut().insert(key));

    spawn_local(async move {
        yield_to_browser().await;

        let is_current = state.current_file_index.get_untracked() == Some(file_idx);
        if !is_current {
            for _ in 0..3 { yield_to_browser().await; }
        }

        let still_loaded = state.files.with_untracked(|files| file_idx < files.len());
        if !still_loaded {
            FLOW_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        }

        let rendered = if algo == FlowAlgo::PhaseCoherence {
            use crate::dsp::harmonics;

            let tile_data = state.files.with_untracked(|files| {
                let file = files.get(file_idx)?;
                let sr = file.audio.sample_rate;
                let freq_res = file.spectrogram.freq_resolution;
                let time_res = file.spectrogram.time_resolution;
                let fft_size = (sr as f64 / freq_res).round() as usize;
                let hop_size = (time_res * sr as f64).round() as usize;

                let col_start = tile_idx * TILE_COLS;
                let sample_start = col_start * hop_size;
                let sample_end = (sample_start + (TILE_COLS + 1) * hop_size + fft_size).min(file.audio.samples.len());
                if sample_start >= file.audio.samples.len() || sample_start >= sample_end {
                    return None;
                }

                let samples = file.audio.samples[sample_start..sample_end].to_vec();
                Some((samples, fft_size, hop_size))
            });

            let Some((samples, fft_size, hop_size)) = tile_data else {
                FLOW_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
                return;
            };

            yield_to_browser().await;

            harmonics::compute_tile_phase_data(
                &samples, TILE_COLS, fft_size, hop_size,
            )
        } else {
            let col_start = tile_idx * TILE_COLS;

            let result = spectral_store::with_columns(file_idx, col_start, col_start + TILE_COLS, |cols, _max_mag| {
                let prev_col = if tile_idx > 0 {
                    let prev_end = col_start;
                    let prev_start = prev_end.saturating_sub(1);
                    spectral_store::with_columns(file_idx, prev_start, prev_end, |prev_cols, _| {
                        prev_cols.last().map(|c| c.magnitudes.clone())
                    }).flatten()
                } else {
                    None
                };
                spectrogram_renderer::pre_render_flow_columns(
                    cols, prev_col.as_deref(), algo,
                )
            });

            if let Some(r) = result {
                r
            } else {
                let fallback = state.files.with_untracked(|files| {
                    files.get(file_idx).and_then(|f| {
                        if f.spectrogram.columns.is_empty() { return None; }
                        let end = (col_start + TILE_COLS).min(f.spectrogram.columns.len());
                        if col_start >= end { return None; }
                        let cols = &f.spectrogram.columns[col_start..end];
                        let prev_col = if col_start > 0 {
                            Some(f.spectrogram.columns[col_start - 1].magnitudes.as_slice())
                        } else {
                            None
                        };
                        Some(spectrogram_renderer::pre_render_flow_columns(
                            cols, prev_col, algo,
                        ))
                    })
                });
                match fallback {
                    Some(r) => r,
                    None => {
                        FLOW_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
                        return;
                    }
                }
            }
        };

        FLOW_CACHE.with(|c| c.borrow_mut().insert(file_idx, tile_idx, rendered));
        FLOW_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
        state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
    });
}

// ── Chromagram tile cache (LOD1-only) ────────────────────────────────────────

pub fn get_chroma_tile(file_idx: usize, tile_idx: usize) -> Option<()> {
    CHROMA_CACHE.with(|c| c.borrow().get(file_idx, tile_idx).map(|_| ()))
}

pub fn borrow_chroma_tile<R>(file_idx: usize, tile_idx: usize, f: impl FnOnce(&Tile) -> R) -> Option<R> {
    CHROMA_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        let key = (file_idx, tile_idx);
        if cache.tiles.contains_key(&key) {
            cache.touch(key);
            drop(cache);
            CHROMA_CACHE.with(|c| {
                c.borrow().tiles.get(&key).map(|t| f(t))
            })
        } else {
            None
        }
    })
}

pub fn clear_chroma_cache() {
    CHROMA_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        cache.tiles.clear();
        cache.lru.clear();
        cache.total_bytes = 0;
    });
    CHROMA_IN_FLIGHT.with(|s| s.borrow_mut().clear());
    CHROMA_GLOBAL_MAX.with(|m| m.borrow_mut().clear());
}

/// Schedule a chromagram tile for background generation (LOD1).
pub fn schedule_chroma_tile(
    state: AppState,
    file_idx: usize,
    tile_idx: usize,
) {
    use crate::canvas::spectral_store;
    use crate::dsp::chromagram;

    let key = (file_idx, tile_idx);
    if CHROMA_CACHE.with(|c| c.borrow().tiles.contains_key(&key)) { return; }
    if CHROMA_IN_FLIGHT.with(|s| s.borrow().contains(&key)) { return; }
    CHROMA_IN_FLIGHT.with(|s| s.borrow_mut().insert(key));

    spawn_local(async move {
        yield_to_browser().await;

        let is_current = state.current_file_index.get_untracked() == Some(file_idx);
        if !is_current {
            for _ in 0..3 { yield_to_browser().await; }
        }

        let still_loaded = state.files.with_untracked(|files| file_idx < files.len());
        if !still_loaded {
            CHROMA_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        }

        let col_start = tile_idx * TILE_COLS;

        let freq_res = state.files.with_untracked(|files| {
            files.get(file_idx).map(|f| f.spectrogram.freq_resolution)
        }).unwrap_or(1.0);

        let global_max = CHROMA_GLOBAL_MAX.with(|m| m.borrow().get(&file_idx).copied());
        let (max_class, max_note) = if let Some(gm) = global_max {
            gm
        } else {
            let from_store = spectral_store::compute_chroma_global_max(file_idx, freq_res);
            let gm = from_store.unwrap_or_else(|| {
                state.files.with_untracked(|files| {
                    files.get(file_idx)
                        .filter(|f| !f.spectrogram.columns.is_empty())
                        .map(|f| chromagram::compute_chroma_max(&f.spectrogram.columns, freq_res))
                        .unwrap_or((0.0, 0.0))
                })
            });
            if gm.0 > 0.0 {
                CHROMA_GLOBAL_MAX.with(|m| m.borrow_mut().insert(file_idx, gm));
            }
            gm
        };

        let result = spectral_store::with_columns(file_idx, col_start, col_start + TILE_COLS, |cols, _max_mag| {
            chromagram::pre_render_chromagram_columns(cols, freq_res, max_class, max_note)
        });

        let rendered = if let Some(r) = result {
            r
        } else {
            let fallback = state.files.with_untracked(|files| {
                files.get(file_idx).and_then(|f| {
                    if f.spectrogram.columns.is_empty() { return None; }
                    let end = (col_start + TILE_COLS).min(f.spectrogram.columns.len());
                    if col_start >= end { return None; }
                    Some(chromagram::pre_render_chromagram_columns(
                        &f.spectrogram.columns[col_start..end],
                        freq_res,
                        max_class,
                        max_note,
                    ))
                })
            });
            match fallback {
                Some(r) => r,
                None => {
                    CHROMA_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
                    return;
                }
            }
        };

        CHROMA_CACHE.with(|c| c.borrow_mut().insert(file_idx, tile_idx, rendered));
        CHROMA_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
        state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
    });
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Yield once to the browser event loop via a zero-duration setTimeout.
async fn yield_to_browser() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let win = web_sys::window().unwrap();
        let cb = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::NULL);
        });
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
            cb.unchecked_ref(), 0,
        );
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
