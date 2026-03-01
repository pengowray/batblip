//! Progressive tile cache for spectrogram rendering.
//!
//! The full spectrogram is divided into fixed-width column chunks ("tiles").
//! Tiles are generated asynchronously in the background using `spawn_local` +
//! `set_timeout(0)` to yield between tiles, keeping the UI responsive.
//!
//! The cache uses an LRU eviction policy capped at `MAX_BYTES` total RGBA pixel storage.

use std::cell::RefCell;
use std::collections::HashMap;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::canvas::spectrogram_renderer::{self, PreRendered, FlowAlgo};
use crate::state::{AppState, LoadedFile};

/// Number of spectrogram columns per tile.
pub const TILE_COLS: usize = 256;

/// ~120 MB cap for tile RGBA data (4 bytes/px × width × height × n_tiles).
const MAX_BYTES: usize = 120 * 1024 * 1024;

// ── Cache data structures ─────────────────────────────────────────────────────

pub struct Tile {
    pub tile_idx: usize,
    /// File index this tile belongs to (so stale tiles can be evicted).
    pub file_idx: usize,
    pub rendered: PreRendered,
}

struct TileCache {
    /// Keyed by (file_idx, tile_idx)
    tiles: HashMap<(usize, usize), Tile>,
    /// LRU order: front = most recently used
    lru: Vec<(usize, usize)>,
    total_bytes: usize,
}

impl TileCache {
    fn new() -> Self {
        Self { tiles: HashMap::new(), lru: Vec::new(), total_bytes: 0 }
    }

    fn insert(&mut self, file_idx: usize, tile_idx: usize, rendered: PreRendered) {
        let key = (file_idx, tile_idx);
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
        self.tiles.insert(key, Tile { tile_idx, file_idx, rendered });
        self.lru.push(key);
    }

    fn get(&self, file_idx: usize, tile_idx: usize) -> Option<&Tile> {
        self.tiles.get(&(file_idx, tile_idx))
    }

    fn touch(&mut self, key: (usize, usize)) {
        self.lru.retain(|k| k != &key);
        self.lru.push(key);
    }

    fn evict_far_from(&mut self, file_idx: usize, center_tile: usize, keep_radius: usize) {
        let keys_to_evict: Vec<(usize, usize)> = self.tiles.keys().copied()
            .filter(|&(fi, ti)| {
                fi != file_idx || ti.abs_diff(center_tile) > keep_radius
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

thread_local! {
    static CACHE: RefCell<TileCache> = RefCell::new(TileCache::new());
    /// Set of (file_idx, tile_idx) currently being generated (to avoid duplicate work).
    static IN_FLIGHT: RefCell<std::collections::HashSet<(usize, usize)>> =
        RefCell::new(std::collections::HashSet::new());
    /// Separate cache for LOD 0 (quick-preview) tiles.
    /// These are blurry but fast (~5ms vs ~50ms for LOD 1).
    static LOD0_CACHE: RefCell<HashMap<(usize, usize), Tile>> =
        RefCell::new(HashMap::new());
    static LOD0_IN_FLIGHT: RefCell<std::collections::HashSet<(usize, usize)>> =
        RefCell::new(std::collections::HashSet::new());

    /// Separate cache for flow-mode tiles (pre-colored RGBA with 2D colormap applied).
    static FLOW_CACHE: RefCell<TileCache> = RefCell::new(TileCache::new());
    static FLOW_IN_FLIGHT: RefCell<std::collections::HashSet<(usize, usize)>> =
        RefCell::new(std::collections::HashSet::new());

    /// Separate cache for chromagram tiles.
    static CHROMA_CACHE: RefCell<TileCache> = RefCell::new(TileCache::new());
    static CHROMA_IN_FLIGHT: RefCell<std::collections::HashSet<(usize, usize)>> =
        RefCell::new(std::collections::HashSet::new());

    /// Cached per-file global chromagram normalisation maxima (max_class, max_note).
    /// Computed once per file so all chroma tiles use the same brightness scale.
    static CHROMA_GLOBAL_MAX: RefCell<HashMap<usize, (f32, f32)>> =
        RefCell::new(HashMap::new());

    /// Separate cache for LOD 2 (high-resolution) tiles.
    /// These provide 4× time resolution when zoomed in (hop=128 vs LOD1 hop=512).
    static LOD2_CACHE: RefCell<TileCache> = RefCell::new(TileCache::new());
    static LOD2_IN_FLIGHT: RefCell<std::collections::HashSet<(usize, usize)>> =
        RefCell::new(std::collections::HashSet::new());
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn get_tile(file_idx: usize, tile_idx: usize) -> Option<()> {
    CACHE.with(|c| c.borrow().get(file_idx, tile_idx).map(|_| ()))
}

pub fn borrow_tile<R>(file_idx: usize, tile_idx: usize, f: impl FnOnce(&Tile) -> R) -> Option<R> {
    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        let key = (file_idx, tile_idx);
        if cache.tiles.contains_key(&key) {
            cache.touch(key);
            // Re-borrow immutably for the callback
            drop(cache);
            let c2 = CACHE.with(|c| {
                c.borrow().tiles.get(&key).map(|t| {
                    f(t)
                })
            });
            c2
        } else {
            None
        }
    })
}

pub fn clear_file(file_idx: usize) {
    CACHE.with(|c| c.borrow_mut().clear_for_file(file_idx));
    IN_FLIGHT.with(|s| s.borrow_mut().retain(|k| k.0 != file_idx));
    LOD0_CACHE.with(|c| c.borrow_mut().retain(|k, _| k.0 != file_idx));
    LOD0_IN_FLIGHT.with(|s| s.borrow_mut().retain(|k| k.0 != file_idx));
    LOD2_CACHE.with(|c| c.borrow_mut().clear_for_file(file_idx));
    LOD2_IN_FLIGHT.with(|s| s.borrow_mut().retain(|k| k.0 != file_idx));
}

pub fn evict_far(file_idx: usize, center_tile: usize, keep_radius: usize) {
    CACHE.with(|c| c.borrow_mut().evict_far_from(file_idx, center_tile, keep_radius));
}

/// Schedule background generation of a tile if not already cached or in-flight.
pub fn schedule_tile(state: AppState, file: LoadedFile, file_idx: usize, tile_idx: usize) {
    let max_mag = spectrogram_renderer::global_max_magnitude(&file.spectrogram);
    schedule_tile_with_max(state, file, file_idx, tile_idx, max_mag);
}

/// Like `schedule_tile` but accepts a pre-computed max magnitude to avoid
/// redundantly scanning all columns for each tile.
pub fn schedule_tile_with_max(
    state: AppState,
    file: LoadedFile,
    file_idx: usize,
    tile_idx: usize,
    max_mag: f32,
) {
    let key = (file_idx, tile_idx);
    // Skip if already cached
    if CACHE.with(|c| c.borrow().tiles.contains_key(&key)) {
        return;
    }
    // Skip if already being generated
    if IN_FLIGHT.with(|s| s.borrow().contains(&key)) {
        return;
    }
    IN_FLIGHT.with(|s| s.borrow_mut().insert(key));

    spawn_local(async move {
        // Yield to let the browser process events before heavy FFT work
        yield_to_browser().await;

        // Deprioritize non-current files: yield extra times so current-file
        // tiles run first from the microtask queue.
        let is_current = state.current_file_index.get_untracked() == Some(file_idx);
        if !is_current {
            for _ in 0..3 {
                yield_to_browser().await;
            }
        }

        // Check if still relevant (file might have been removed)
        let still_loaded = state.files.with_untracked(|files| {
            files.get(file_idx).map(|f| f.name == file.name).unwrap_or(false)
        });
        if !still_loaded {
            IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        }

        // Compute the tile
        let col_start = tile_idx * TILE_COLS;
        let col_end = (col_start + TILE_COLS).min(file.spectrogram.columns.len());
        if col_start >= col_end {
            IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        }

        let rendered = spectrogram_renderer::pre_render_columns(
            &file.spectrogram.columns[col_start..col_end],
            max_mag,
        );

        // Store in cache and evict corresponding LOD 0 (no longer needed)
        CACHE.with(|c| c.borrow_mut().insert(file_idx, tile_idx, rendered));
        evict_lod0_for_tile(file_idx, tile_idx);
        IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));

        // Signal that a new tile is ready → triggers spectrogram redraw
        state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
    });
}

/// Schedule generation of all tiles for a file (called after file load).
/// Yields between tiles so the browser stays responsive.
pub fn schedule_all_tiles(state: AppState, file: LoadedFile, file_idx: usize) {
    let total_cols = if file.spectrogram.total_columns > 0 {
        file.spectrogram.total_columns
    } else {
        file.spectrogram.columns.len()
    };
    if total_cols == 0 { return; }
    let n_tiles = (total_cols + TILE_COLS - 1) / TILE_COLS;

    // Compute global max once and share across all tiles
    let max_mag = spectrogram_renderer::global_max_magnitude(&file.spectrogram);

    for tile_idx in 0..n_tiles {
        schedule_tile_with_max(state.clone(), file.clone(), file_idx, tile_idx, max_mag);
    }
}

/// Render a tile synchronously from the spectral column store.
/// Used during the loading loop to render tiles immediately before eviction
/// can discard their columns.  Returns true if the tile was successfully rendered.
pub fn render_tile_from_store_sync(file_idx: usize, tile_idx: usize) -> bool {
    use crate::canvas::spectral_store;

    let key = (file_idx, tile_idx);
    if CACHE.with(|c| c.borrow().tiles.contains_key(&key)) { return true; }

    let col_start = tile_idx * TILE_COLS;
    let col_end = col_start + TILE_COLS; // with_columns clamps to store len

    let rendered = spectral_store::with_columns(file_idx, col_start, col_end, |cols, max_mag| {
        spectrogram_renderer::pre_render_columns(cols, max_mag)
    });

    if let Some(rendered) = rendered {
        CACHE.with(|c| c.borrow_mut().insert(file_idx, tile_idx, rendered));
        evict_lod0_for_tile(file_idx, tile_idx);
        true
    } else {
        false
    }
}

/// Render a partial (live) tile from the spectral store.
///
/// Renders `available_cols` columns starting at `col_start`, padding the rest
/// of the tile width with black. Overwrites any existing tile at this position
/// unconditionally (the live tile changes every processing cycle).
/// Used during live recording for the rightmost tile.
pub fn render_live_tile_sync(file_idx: usize, tile_idx: usize, col_start: usize, available_cols: usize) -> bool {
    use crate::canvas::spectral_store;

    let col_end = col_start + available_cols;
    let rendered = spectral_store::with_columns(file_idx, col_start, col_end, |cols, max_mag| {
        let partial = spectrogram_renderer::pre_render_columns(cols, max_mag);

        if partial.width == 0 || partial.height == 0 {
            return partial;
        }

        // If already full tile width, no padding needed
        if available_cols >= TILE_COLS {
            return partial;
        }

        // Pad to full TILE_COLS width with silence (NEG_INFINITY dB / black)
        let full_width = TILE_COLS as u32;
        let height = partial.height;

        if !partial.db_data.is_empty() {
            // dB tile: pad with NEG_INFINITY (renders as black)
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
            // RGBA tile (legacy path)
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
        CACHE.with(|c| c.borrow_mut().insert(file_idx, tile_idx, rendered));
        true
    } else {
        false
    }
}

/// Schedule tile generation from the spectral column store (used during
/// progressive loading when full SpectrogramData isn't assembled yet).
pub fn schedule_tile_from_store(state: AppState, file_idx: usize, tile_idx: usize) {
    use crate::canvas::spectral_store;

    let key = (file_idx, tile_idx);
    if CACHE.with(|c| c.borrow().tiles.contains_key(&key)) { return; }
    if IN_FLIGHT.with(|s| s.borrow().contains(&key)) { return; }
    IN_FLIGHT.with(|s| s.borrow_mut().insert(key));

    spawn_local(async move {
        yield_to_browser().await;

        // Deprioritize non-current files: yield extra times so current-file
        // tiles run first from the microtask queue.
        let is_current = state.current_file_index.get_untracked() == Some(file_idx);
        if !is_current {
            for _ in 0..3 {
                yield_to_browser().await;
            }
        }

        // Check if the file is still loaded (not removed by user)
        let still_loaded = state.files.with_untracked(|files| {
            file_idx < files.len()
        });
        if !still_loaded {
            IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        }

        let col_start = tile_idx * TILE_COLS;
        let col_end = col_start + TILE_COLS; // with_columns clamps to store len

        let rendered = spectral_store::with_columns(file_idx, col_start, col_end, |cols, max_mag| {
            spectrogram_renderer::pre_render_columns(cols, max_mag)
        });

        IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));

        if let Some(rendered) = rendered {
            CACHE.with(|c| c.borrow_mut().insert(file_idx, tile_idx, rendered));
            evict_lod0_for_tile(file_idx, tile_idx);
            state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
        }
    });
}

/// Schedule visible tiles from the spectral store (for large files after loading).
/// Computes which tiles are near the current viewport and schedules them.
pub fn schedule_visible_tiles_from_store(state: AppState, file_idx: usize, total_cols: usize) {
    if total_cols == 0 { return; }
    let n_tiles = (total_cols + TILE_COLS - 1) / TILE_COLS;

    // Determine viewport center tile
    let time_res = state.files.with_untracked(|files| {
        files.get(file_idx).map(|f| f.spectrogram.time_resolution).unwrap_or(0.01)
    });
    let scroll = state.scroll_offset.get_untracked();
    let zoom = state.zoom_level.get_untracked();
    let canvas_w = state.spectrogram_canvas_width.get_untracked();
    let visible_time = if zoom > 0.0 { canvas_w / zoom * time_res } else { 1.0 };
    let center_col = ((scroll + visible_time / 2.0) / time_res) as usize;
    let center_tile = center_col / TILE_COLS;

    // Schedule tiles in expanding-ring order from center, limited to a reasonable count
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

/// Schedule on-demand tile computation from audio samples.
/// Used when the spectral store has evicted the needed columns.
pub fn schedule_tile_on_demand(
    state: AppState,
    file_idx: usize,
    tile_idx: usize,
) {
    use crate::canvas::spectral_store;
    use crate::dsp::fft::compute_spectrogram_partial;

    let key = (file_idx, tile_idx);
    if CACHE.with(|c| c.borrow().tiles.contains_key(&key)) { return; }
    if IN_FLIGHT.with(|s| s.borrow().contains(&key)) { return; }
    IN_FLIGHT.with(|s| s.borrow_mut().insert(key));

    spawn_local(async move {
        yield_to_browser().await;

        // Deprioritize non-current files
        let is_current = state.current_file_index.get_untracked() == Some(file_idx);
        if !is_current {
            for _ in 0..3 {
                yield_to_browser().await;
            }
        }

        // Get audio data for STFT recomputation
        let audio = state.files.with_untracked(|files| {
            files.get(file_idx).map(|f| f.audio.clone())
        });
        let Some(audio) = audio else {
            IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        };

        let col_start = tile_idx * TILE_COLS;

        // Recompute STFT columns from audio samples
        let cols = compute_spectrogram_partial(&audio, 2048, 512, col_start, TILE_COLS);
        if cols.is_empty() {
            IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        }

        // Insert into spectral store so future requests can use them
        spectral_store::insert_columns(file_idx, col_start, &cols);
        let max_mag = spectral_store::get_max_magnitude(file_idx);

        // Render the tile
        let rendered = spectrogram_renderer::pre_render_columns(&cols, max_mag);

        CACHE.with(|c| c.borrow_mut().insert(file_idx, tile_idx, rendered));
        evict_lod0_for_tile(file_idx, tile_idx);
        IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
        state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
    });
}

// ── LOD 0 (quick-preview) tiles ──────────────────────────────────────────────

/// LOD 0 uses FFT=512, hop=2048 for a fast blurry preview covering the same
/// time range as a full-quality (LOD 1) tile.
const LOD0_FFT: usize = 512;
const LOD0_HOP: usize = 2048;

/// Number of LOD 0 columns per tile (same time range as TILE_COLS at LOD 1).
/// One LOD 1 tile covers TILE_COLS * 512 = 131072 samples.
/// LOD 0 columns: 131072 / LOD0_HOP = 64.
const LOD0_COLS_PER_TILE: usize = (TILE_COLS * 512) / LOD0_HOP;

// ── LOD 2 (high-resolution) tile constants ──────────────────────────────────

/// LOD 2 uses the same FFT size as LOD 1 (preserving frequency resolution)
/// but a smaller hop for 4× finer time resolution when zoomed in.
const LOD2_FFT: usize = 2048;
const LOD2_HOP: usize = 128;

/// Number of LOD 2 columns per tile (same time range as TILE_COLS at LOD 1).
/// One LOD 1 tile covers TILE_COLS * 512 = 131072 samples.
/// LOD 2 columns: 131072 / LOD2_HOP = 1024.
pub const LOD2_COLS_PER_TILE: usize = (TILE_COLS * 512) / LOD2_HOP;

/// Zoom threshold: schedule/render LOD 2 tiles when zoom >= this value.
pub const LOD2_ZOOM_THRESHOLD: f64 = 2.0;

/// Schedule a LOD 0 (quick-preview) tile if not already cached.
/// LOD 0 tiles are fast to compute (~5ms) and provide a blurry preview
/// while full-quality tiles are being generated.
pub fn schedule_lod0_tile(state: AppState, file_idx: usize, tile_idx: usize) {
    use crate::canvas::spectral_store;
    use crate::dsp::fft::compute_spectrogram_partial;

    let key = (file_idx, tile_idx);
    // Don't compute LOD 0 if LOD 1 is already cached
    if CACHE.with(|c| c.borrow().tiles.contains_key(&key)) { return; }
    if LOD0_CACHE.with(|c| c.borrow().contains_key(&key)) { return; }
    if LOD0_IN_FLIGHT.with(|s| s.borrow().contains(&key)) { return; }
    LOD0_IN_FLIGHT.with(|s| s.borrow_mut().insert(key));

    spawn_local(async move {
        yield_to_browser().await;

        let audio = state.files.with_untracked(|files| {
            files.get(file_idx).map(|f| f.audio.clone())
        });
        let Some(audio) = audio else {
            LOD0_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        };

        // Convert tile index to LOD 0 column space.
        // LOD 1 col_start corresponds to sample offset: tile_idx * TILE_COLS * 512.
        // LOD 0 col_start at the same sample offset: sample_offset / LOD0_HOP.
        let sample_offset = tile_idx * TILE_COLS * 512;
        let lod0_col_start = sample_offset / LOD0_HOP;

        let cols = compute_spectrogram_partial(
            &audio, LOD0_FFT, LOD0_HOP, lod0_col_start, LOD0_COLS_PER_TILE,
        );
        LOD0_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));

        if cols.is_empty() { return; }

        // Use global max from spectral store for consistent normalization with LOD 1.
        // Fall back to per-tile max if the store has no data yet.
        let store_max = spectral_store::get_max_magnitude(file_idx);
        let tile_max = cols.iter()
            .flat_map(|c| c.magnitudes.iter())
            .copied()
            .fold(0.0f32, f32::max);
        let max_mag = if store_max > 0.0 { store_max } else { tile_max };

        let rendered = spectrogram_renderer::pre_render_columns(&cols, max_mag);
        LOD0_CACHE.with(|c| {
            c.borrow_mut().insert(key, Tile {
                tile_idx,
                file_idx,
                rendered,
            });
        });
        state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
    });
}

/// Borrow a LOD 0 tile for rendering.
pub fn borrow_lod0_tile<R>(file_idx: usize, tile_idx: usize, f: impl FnOnce(&Tile) -> R) -> Option<R> {
    LOD0_CACHE.with(|c| {
        c.borrow().get(&(file_idx, tile_idx)).map(f)
    })
}

/// Evict LOD 0 tiles when LOD 1 tiles are ready (they're no longer needed).
fn evict_lod0_for_tile(file_idx: usize, tile_idx: usize) {
    LOD0_CACHE.with(|c| {
        c.borrow_mut().remove(&(file_idx, tile_idx));
    });
}

// ── LOD 2 (high-resolution) tiles ────────────────────────────────────────────

/// Schedule a LOD 2 (high-resolution) tile if not already cached.
/// LOD 2 tiles use FFT=2048 hop=128 for 4× time resolution, computed when zoomed in.
pub fn schedule_lod2_tile(state: AppState, file_idx: usize, tile_idx: usize) {
    use crate::canvas::spectral_store;
    use crate::dsp::fft::compute_spectrogram_partial;

    let key = (file_idx, tile_idx);
    // Don't compute LOD 2 if already cached
    if LOD2_CACHE.with(|c| c.borrow().tiles.contains_key(&key)) { return; }
    if LOD2_IN_FLIGHT.with(|s| s.borrow().contains(&key)) { return; }
    LOD2_IN_FLIGHT.with(|s| s.borrow_mut().insert(key));

    spawn_local(async move {
        yield_to_browser().await;

        // LOD 2 is expensive — yield extra to keep UI responsive
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
            LOD2_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        };

        // Convert tile index to LOD 2 column space.
        // LOD 1 col_start corresponds to sample offset: tile_idx * TILE_COLS * 512.
        // LOD 2 col_start at the same sample offset: sample_offset / LOD2_HOP.
        let sample_offset = tile_idx * TILE_COLS * 512;
        let lod2_col_start = sample_offset / LOD2_HOP;

        let cols = compute_spectrogram_partial(
            &audio, LOD2_FFT, LOD2_HOP, lod2_col_start, LOD2_COLS_PER_TILE,
        );
        LOD2_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));

        if cols.is_empty() { return; }

        // Use global max from spectral store for consistent normalization with LOD 1.
        // Fall back to per-tile max if the store has no data yet.
        let store_max = spectral_store::get_max_magnitude(file_idx);
        let tile_max = cols.iter()
            .flat_map(|c| c.magnitudes.iter())
            .copied()
            .fold(0.0f32, f32::max);
        let max_mag = if store_max > 0.0 { store_max } else { tile_max };

        let rendered = spectrogram_renderer::pre_render_columns(&cols, max_mag);
        LOD2_CACHE.with(|c| c.borrow_mut().insert(file_idx, tile_idx, rendered));
        state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
    });
}

/// Borrow a LOD 2 tile for rendering.
pub fn borrow_lod2_tile<R>(file_idx: usize, tile_idx: usize, f: impl FnOnce(&Tile) -> R) -> Option<R> {
    LOD2_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        let key = (file_idx, tile_idx);
        if cache.tiles.contains_key(&key) {
            cache.touch(key);
            drop(cache);
            LOD2_CACHE.with(|c| {
                c.borrow().tiles.get(&key).map(|t| f(t))
            })
        } else {
            None
        }
    })
}

/// Check if a LOD 2 tile exists in cache.
pub fn get_lod2_tile(file_idx: usize, tile_idx: usize) -> Option<()> {
    LOD2_CACHE.with(|c| c.borrow().get(file_idx, tile_idx).map(|_| ()))
}

/// Returns the number of complete tiles for a file currently in the cache.
pub fn tiles_ready(file_idx: usize, n_tiles: usize) -> usize {
    CACHE.with(|c| {
        let cache = c.borrow();
        (0..n_tiles).filter(|&i| cache.tiles.contains_key(&(file_idx, i))).count()
    })
}

// ── Flow tile cache ──────────────────────────────────────────────────────────

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

/// Clear all flow tiles (called when algorithm or settings change).
pub fn clear_flow_cache() {
    FLOW_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        cache.tiles.clear();
        cache.lru.clear();
        cache.total_bytes = 0;
    });
    FLOW_IN_FLIGHT.with(|s| s.borrow_mut().clear());
}

/// Clear flow tiles for a specific file.
pub fn clear_flow_file(file_idx: usize) {
    FLOW_CACHE.with(|c| c.borrow_mut().clear_for_file(file_idx));
    FLOW_IN_FLIGHT.with(|s| s.borrow_mut().retain(|k| k.0 != file_idx));
}

/// Schedule a flow tile for background generation.
///
/// Reads STFT columns from the spectral store (or from `file.spectrogram.columns`),
/// fetches the last column of the previous tile for boundary shift computation,
/// computes per-pixel shifts, and applies the 2D colormap.
///
/// The resulting tile stores pre-colored RGBA — no colormap step during blit.
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
            // Phase coherence: compute from raw audio samples
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

                let store_max = spectral_store::get_max_magnitude(file_idx);
                let max_mag = if store_max > 0.0 {
                    store_max
                } else {
                    file.spectrogram.columns.iter()
                        .flat_map(|c| c.magnitudes.iter())
                        .copied()
                        .fold(0.0f32, f32::max)
                        .max(1e-10)
                };

                Some((samples, fft_size, hop_size, max_mag))
            });

            let Some((samples, fft_size, hop_size, max_mag)) = tile_data else {
                FLOW_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
                return;
            };

            yield_to_browser().await;

            harmonics::compute_tile_phase_data(
                &samples, TILE_COLS, fft_size, hop_size, max_mag,
            )
        } else {
            // Shift-based flow: compute from spectrogram columns
            let col_start = tile_idx * TILE_COLS;

            let result = spectral_store::with_columns(file_idx, col_start, col_start + TILE_COLS, |cols, max_mag| {
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
                    cols, prev_col.as_deref(), max_mag, algo,
                )
            });

            if let Some(r) = result {
                r
            } else {
                let fallback = state.files.with_untracked(|files| {
                    files.get(file_idx).and_then(|f| {
                        if f.spectrogram.columns.is_empty() { return None; }
                        let max_mag = spectrogram_renderer::global_max_magnitude(&f.spectrogram);
                        let end = (col_start + TILE_COLS).min(f.spectrogram.columns.len());
                        if col_start >= end { return None; }
                        let cols = &f.spectrogram.columns[col_start..end];
                        let prev_col = if col_start > 0 {
                            Some(f.spectrogram.columns[col_start - 1].magnitudes.as_slice())
                        } else {
                            None
                        };
                        Some(spectrogram_renderer::pre_render_flow_columns(
                            cols, prev_col, max_mag, algo,
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

// ── Chromagram tile cache ────────────────────────────────────────────────────

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

/// Schedule a chromagram tile for background generation.
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

        // Get freq_resolution from the file
        let freq_res = state.files.with_untracked(|files| {
            files.get(file_idx).map(|f| f.spectrogram.freq_resolution)
        }).unwrap_or(1.0);

        // Compute/retrieve global chroma max for consistent normalisation across tiles.
        // Try cached value first, then compute from spectral store or in-memory columns.
        let global_max = CHROMA_GLOBAL_MAX.with(|m| m.borrow().get(&file_idx).copied());
        let (max_class, max_note) = if let Some(gm) = global_max {
            gm
        } else {
            // Try spectral store (iterates by reference, no cloning)
            let from_store = spectral_store::compute_chroma_global_max(file_idx, freq_res);
            let gm = from_store.unwrap_or_else(|| {
                // Fall back to in-memory columns
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

        // Try spectral store first
        let result = spectral_store::with_columns(file_idx, col_start, col_start + TILE_COLS, |cols, _max_mag| {
            chromagram::pre_render_chromagram_columns(cols, freq_res, max_class, max_note)
        });

        let rendered = if let Some(r) = result {
            r
        } else {
            // Fall back to in-memory columns
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


// ── Internal helpers ──────────────────────────────────────────────────────────

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
