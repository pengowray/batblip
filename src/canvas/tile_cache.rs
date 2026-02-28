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
use crate::canvas::spectrogram_renderer::{self, PreRendered, MovementAlgo};
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
        let bytes = rendered.pixels.len();
        // Remove old entry if replacing
        if let Some(old) = self.tiles.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(old.rendered.pixels.len());
            self.lru.retain(|k| k != &key);
        }
        // Evict until under cap
        while self.total_bytes + bytes > MAX_BYTES && !self.lru.is_empty() {
            let oldest = self.lru.remove(0);
            if let Some(evicted) = self.tiles.remove(&oldest) {
                self.total_bytes = self.total_bytes.saturating_sub(evicted.rendered.pixels.len());
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
                self.total_bytes = self.total_bytes.saturating_sub(evicted.rendered.pixels.len());
                self.lru.retain(|k| k != &key);
            }
        }
    }

    fn clear_for_file(&mut self, file_idx: usize) {
        let keys: Vec<_> = self.tiles.keys().copied().filter(|k| k.0 == file_idx).collect();
        for key in keys {
            if let Some(evicted) = self.tiles.remove(&key) {
                self.total_bytes = self.total_bytes.saturating_sub(evicted.rendered.pixels.len());
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

    /// Separate cache for movement-mode tiles (pre-colored RGBA with 2D colormap applied).
    static MV_CACHE: RefCell<TileCache> = RefCell::new(TileCache::new());
    static MV_IN_FLIGHT: RefCell<std::collections::HashSet<(usize, usize)>> =
        RefCell::new(std::collections::HashSet::new());

    /// Separate cache for chromagram tiles.
    static CHROMA_CACHE: RefCell<TileCache> = RefCell::new(TileCache::new());
    static CHROMA_IN_FLIGHT: RefCell<std::collections::HashSet<(usize, usize)>> =
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

/// Returns the number of complete tiles for a file currently in the cache.
pub fn tiles_ready(file_idx: usize, n_tiles: usize) -> usize {
    CACHE.with(|c| {
        let cache = c.borrow();
        (0..n_tiles).filter(|&i| cache.tiles.contains_key(&(file_idx, i))).count()
    })
}

// ── Movement tile cache ──────────────────────────────────────────────────────

pub fn get_mv_tile(file_idx: usize, tile_idx: usize) -> Option<()> {
    MV_CACHE.with(|c| c.borrow().get(file_idx, tile_idx).map(|_| ()))
}

pub fn borrow_mv_tile<R>(file_idx: usize, tile_idx: usize, f: impl FnOnce(&Tile) -> R) -> Option<R> {
    MV_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        let key = (file_idx, tile_idx);
        if cache.tiles.contains_key(&key) {
            cache.touch(key);
            drop(cache);
            MV_CACHE.with(|c| {
                c.borrow().tiles.get(&key).map(|t| f(t))
            })
        } else {
            None
        }
    })
}

/// Clear all movement tiles (called when algorithm or settings change).
pub fn clear_mv_cache() {
    MV_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        cache.tiles.clear();
        cache.lru.clear();
        cache.total_bytes = 0;
    });
    MV_IN_FLIGHT.with(|s| s.borrow_mut().clear());
}

/// Clear movement tiles for a specific file.
pub fn clear_mv_file(file_idx: usize) {
    MV_CACHE.with(|c| c.borrow_mut().clear_for_file(file_idx));
    MV_IN_FLIGHT.with(|s| s.borrow_mut().retain(|k| k.0 != file_idx));
}

/// Schedule a movement tile for background generation.
///
/// Reads STFT columns from the spectral store (or from `file.spectrogram.columns`),
/// fetches the last column of the previous tile for boundary shift computation,
/// computes per-pixel shifts, and applies the 2D colormap.
///
/// The resulting tile stores pre-colored RGBA — no colormap step during blit.
pub fn schedule_movement_tile(
    state: AppState,
    file_idx: usize,
    tile_idx: usize,
    algo: MovementAlgo,
) {
    use crate::canvas::spectral_store;

    let key = (file_idx, tile_idx);
    if MV_CACHE.with(|c| c.borrow().tiles.contains_key(&key)) { return; }
    if MV_IN_FLIGHT.with(|s| s.borrow().contains(&key)) { return; }
    MV_IN_FLIGHT.with(|s| s.borrow_mut().insert(key));

    spawn_local(async move {
        yield_to_browser().await;

        let is_current = state.current_file_index.get_untracked() == Some(file_idx);
        if !is_current {
            for _ in 0..3 { yield_to_browser().await; }
        }

        let still_loaded = state.files.with_untracked(|files| file_idx < files.len());
        if !still_loaded {
            MV_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        }

        // Read movement settings
        let ig = state.mv_intensity_gate.get_untracked();
        let mg = state.mv_movement_gate.get_untracked();
        let op = state.mv_opacity.get_untracked();

        let col_start = tile_idx * TILE_COLS;

        // Try to get columns from the spectral store first, fall back to in-memory columns
        let result = spectral_store::with_columns(file_idx, col_start, col_start + TILE_COLS, |cols, max_mag| {
            // Get the last column of the previous tile for boundary shift computation
            let prev_col = if tile_idx > 0 {
                let prev_end = col_start; // = previous tile's last column + 1
                let prev_start = prev_end.saturating_sub(1);
                spectral_store::with_columns(file_idx, prev_start, prev_end, |prev_cols, _| {
                    prev_cols.last().map(|c| c.magnitudes.clone())
                }).flatten()
            } else {
                None
            };
            spectrogram_renderer::pre_render_movement_columns(
                cols, prev_col.as_deref(), max_mag, algo, ig, mg, op,
            )
        });

        let rendered = if let Some(r) = result {
            r
        } else {
            // Fall back to in-memory columns
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
                    Some(spectrogram_renderer::pre_render_movement_columns(
                        cols, prev_col, max_mag, algo, ig, mg, op,
                    ))
                })
            });
            match fallback {
                Some(r) => r,
                None => {
                    MV_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
                    return;
                }
            }
        };

        MV_CACHE.with(|c| c.borrow_mut().insert(file_idx, tile_idx, rendered));
        MV_IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
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

        // Try spectral store first
        let result = spectral_store::with_columns(file_idx, col_start, col_start + TILE_COLS, |cols, _max_mag| {
            chromagram::pre_render_chromagram_columns(cols, freq_res)
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
