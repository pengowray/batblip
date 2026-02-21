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
use crate::canvas::spectrogram_renderer::{self, PreRendered};
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
}

pub fn evict_far(file_idx: usize, center_tile: usize, keep_radius: usize) {
    CACHE.with(|c| c.borrow_mut().evict_far_from(file_idx, center_tile, keep_radius));
}

/// Schedule background generation of a tile if not already cached or in-flight.
pub fn schedule_tile(state: AppState, file: LoadedFile, file_idx: usize, tile_idx: usize) {
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

        // Check if still relevant (user might have switched files)
        let still_needed = state.current_file_index.get_untracked() == Some(file_idx);
        if !still_needed {
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

        // Compute global max for normalisation (so tiles have consistent brightness)
        let max_mag = spectrogram_renderer::global_max_magnitude(&file.spectrogram);
        let rendered = spectrogram_renderer::pre_render_columns(
            &file.spectrogram.columns[col_start..col_end],
            max_mag,
        );

        // Store in cache
        CACHE.with(|c| c.borrow_mut().insert(file_idx, tile_idx, rendered));
        IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));

        // Signal that a new tile is ready → triggers spectrogram redraw
        state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
    });
}

/// Schedule generation of all tiles for a file (called after file load).
/// Yields between tiles so the browser stays responsive.
pub fn schedule_all_tiles(state: AppState, file: LoadedFile, file_idx: usize) {
    let total_cols = file.spectrogram.columns.len();
    if total_cols == 0 { return; }
    let n_tiles = (total_cols + TILE_COLS - 1) / TILE_COLS;

    for tile_idx in 0..n_tiles {
        schedule_tile(state.clone(), file.clone(), file_idx, tile_idx);
    }
}

/// Returns the number of complete tiles for a file currently in the cache.
pub fn tiles_ready(file_idx: usize, n_tiles: usize) -> usize {
    CACHE.with(|c| {
        let cache = c.borrow();
        (0..n_tiles).filter(|&i| cache.tiles.contains_key(&(file_idx, i))).count()
    })
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
