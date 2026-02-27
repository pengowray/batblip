use crate::canvas::colors::{
    freq_marker_color, freq_marker_label, magnitude_to_greyscale, movement_rgb,
    greyscale_to_viridis, greyscale_to_inferno,
    greyscale_to_magma, greyscale_to_plasma, greyscale_to_cividis, greyscale_to_turbo,
};
use crate::state::{SpectrogramHandle, Selection};
use crate::types::{PreviewImage, SpectrogramData};
use wasm_bindgen::JsCast;
use wasm_bindgen::Clamped;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};

/// Pre-rendered spectrogram image data (RGBA pixels).
pub struct PreRendered {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Pre-render the entire spectrogram to an RGBA pixel buffer.
/// Width = number of columns, Height = number of frequency bins.
/// Frequency axis: row 0 = highest frequency (top), last row = 0 Hz (bottom).
pub fn pre_render(data: &SpectrogramData) -> PreRendered {
    if data.columns.is_empty() {
        return PreRendered {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        };
    }

    let width = data.columns.len() as u32;
    let height = data.columns[0].magnitudes.len() as u32;

    // Find global max magnitude for normalization
    let max_mag = data
        .columns
        .iter()
        .flat_map(|c| c.magnitudes.iter())
        .copied()
        .fold(0.0f32, f32::max);

    let mut pixels = vec![0u8; (width * height * 4) as usize];

    for (col_idx, col) in data.columns.iter().enumerate() {
        for (bin_idx, &mag) in col.magnitudes.iter().enumerate() {
            let grey = magnitude_to_greyscale(mag, max_mag);
            // Flip vertically: bin 0 = lowest freq → bottom row
            let y = height as usize - 1 - bin_idx;
            let pixel_idx = (y * width as usize + col_idx) * 4;
            pixels[pixel_idx] = grey;     // R
            pixels[pixel_idx + 1] = grey; // G
            pixels[pixel_idx + 2] = grey; // B
            pixels[pixel_idx + 3] = 255;  // A
        }
    }

    PreRendered {
        width,
        height,
        pixels,
    }
}

/// Pre-render a slice of columns (a tile) with a given global max magnitude for normalization.
/// The global max is passed in so all tiles use the same normalisation scale.
pub fn pre_render_columns(
    columns: &[crate::types::SpectrogramColumn],
    max_mag: f32,
) -> PreRendered {
    if columns.is_empty() || max_mag <= 0.0 {
        return PreRendered { width: 0, height: 0, pixels: Vec::new() };
    }
    let width = columns.len() as u32;
    let height = columns[0].magnitudes.len() as u32;
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    for (col_idx, col) in columns.iter().enumerate() {
        for (bin_idx, &mag) in col.magnitudes.iter().enumerate() {
            let grey = magnitude_to_greyscale(mag, max_mag);
            let y = height as usize - 1 - bin_idx;
            let pixel_idx = (y * width as usize + col_idx) * 4;
            pixels[pixel_idx] = grey;
            pixels[pixel_idx + 1] = grey;
            pixels[pixel_idx + 2] = grey;
            pixels[pixel_idx + 3] = 255;
        }
    }
    PreRendered { width, height, pixels }
}

/// Compute the global max magnitude across a full spectrogram (for tile normalisation).
pub fn global_max_magnitude(data: &SpectrogramData) -> f32 {
    data.columns.iter().flat_map(|c| c.magnitudes.iter()).copied().fold(0.0f32, f32::max)
}

/// Algorithm selector for movement detection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MovementAlgo {
    Centroid,
    Gradient,
    Flow,
}

/// Cached intermediate data: greyscale intensities + shift values per pixel.
/// The expensive shift computation only needs to run when file or algorithm changes.
/// Color mapping (gates, opacity) can then be applied cheaply via `composite_movement`.
pub struct MovementData {
    pub width: u32,
    pub height: u32,
    /// Greyscale intensity per pixel (row-major, flipped: row 0 = highest freq).
    pub greys: Vec<u8>,
    /// Frequency shift value per pixel (same layout as greys).
    pub shifts: Vec<f32>,
}

/// Compute movement data (expensive): greyscale + shift values for every pixel.
/// Only needs to re-run when the file or algorithm changes.
pub fn compute_movement_data(data: &SpectrogramData, algo: MovementAlgo) -> MovementData {
    if data.columns.is_empty() {
        return MovementData {
            width: 0,
            height: 0,
            greys: Vec::new(),
            shifts: Vec::new(),
        };
    }

    let width = data.columns.len() as u32;
    let height = data.columns[0].magnitudes.len() as u32;
    let h = height as usize;
    let total = (width as usize) * (height as usize);

    let max_mag = data
        .columns
        .iter()
        .flat_map(|c| c.magnitudes.iter())
        .copied()
        .fold(0.0f32, f32::max);

    let mut greys = vec![0u8; total];
    let mut shifts = vec![0.0f32; total];

    for (col_idx, col) in data.columns.iter().enumerate() {
        let prev = if col_idx > 0 {
            Some(&data.columns[col_idx - 1].magnitudes)
        } else {
            None
        };

        for (bin_idx, &mag) in col.magnitudes.iter().enumerate() {
            let grey = magnitude_to_greyscale(mag, max_mag);
            let shift = match prev {
                None => 0.0,
                Some(prev_mags) => match algo {
                    MovementAlgo::Centroid => compute_centroid_shift(prev_mags, &col.magnitudes, bin_idx, h),
                    MovementAlgo::Gradient => compute_gradient_shift(prev_mags, &col.magnitudes, bin_idx, h),
                    MovementAlgo::Flow => compute_flow_shift(prev_mags, &col.magnitudes, bin_idx, h),
                },
            };

            let y = height as usize - 1 - bin_idx;
            let idx = y * width as usize + col_idx;
            greys[idx] = grey;
            shifts[idx] = shift;
        }
    }

    MovementData { width, height, greys, shifts }
}

/// Composite movement data into RGBA pixels (cheap).
/// Re-runs when intensity_gate, movement_gate, or opacity changes.
pub fn composite_movement(
    md: &MovementData,
    intensity_gate: f32,
    movement_gate: f32,
    opacity: f32,
) -> PreRendered {
    let total = (md.width as usize) * (md.height as usize);
    let mut pixels = vec![0u8; total * 4];

    for i in 0..total {
        let [r, g, b] = movement_rgb(md.greys[i], md.shifts[i], intensity_gate, movement_gate, opacity);
        let pi = i * 4;
        pixels[pi] = r;
        pixels[pi + 1] = g;
        pixels[pi + 2] = b;
        pixels[pi + 3] = 255;
    }

    PreRendered {
        width: md.width,
        height: md.height,
        pixels,
    }
}

/// Spectral centroid shift: compute local weighted centroid in a ±radius window
/// around `bin` for both prev and current column, return the difference.
fn compute_centroid_shift(prev: &[f32], curr: &[f32], bin: usize, h: usize) -> f32 {
    let radius: usize = 3;
    let lo = bin.saturating_sub(radius);
    let hi = (bin + radius + 1).min(h);

    let centroid = |mags: &[f32]| -> f32 {
        let mut sum_w = 0.0f32;
        let mut sum_wf = 0.0f32;
        for i in lo..hi {
            let w = mags[i] * mags[i]; // weight by energy
            sum_w += w;
            sum_wf += w * i as f32;
        }
        if sum_w > 0.0 {
            sum_wf / sum_w
        } else {
            bin as f32
        }
    };

    let c_prev = centroid(prev);
    let c_curr = centroid(curr);
    // Normalize by radius so result is roughly in [-1, 1]
    (c_curr - c_prev) / radius as f32
}

/// Vertical gradient of temporal difference.
fn compute_gradient_shift(prev: &[f32], curr: &[f32], bin: usize, h: usize) -> f32 {
    // diff at neighboring bins
    let diff_above = if bin + 1 < h {
        curr[bin + 1] - prev[bin + 1]
    } else {
        0.0
    };
    let diff_below = if bin > 0 {
        curr[bin - 1] - prev[bin - 1]
    } else {
        0.0
    };
    let max_energy = curr[bin].max(prev[bin]).max(1e-10);
    // Positive gradient means energy is appearing above & disappearing below → upward shift
    (diff_above - diff_below) / (2.0 * max_energy)
}

/// 1D vertical optical flow via cross-correlation in a small window.
/// Returns fractional bin displacement (positive = upward shift).
fn compute_flow_shift(prev: &[f32], curr: &[f32], bin: usize, h: usize) -> f32 {
    let radius: usize = 3;
    let max_disp: isize = 2;

    let lo = bin.saturating_sub(radius);
    let hi = (bin + radius + 1).min(h);

    // Check there's enough energy to bother
    let energy: f32 = (lo..hi).map(|i| curr[i]).sum();
    if energy < 1e-8 {
        return 0.0;
    }

    let mut best_corr = f32::NEG_INFINITY;
    let mut best_d: isize = 0;

    for d in -max_disp..=max_disp {
        let mut corr = 0.0f32;
        for i in lo..hi {
            let j = (i as isize + d) as usize;
            if j < h {
                corr += curr[i] * prev[j];
            }
        }
        if corr > best_corr {
            best_corr = corr;
            best_d = d;
        }
    }

    // Sub-pixel refinement using parabolic interpolation
    if best_d.abs() < max_disp {
        let c0 = {
            let d = best_d - 1;
            (lo..hi)
                .map(|i| {
                    let j = (i as isize + d) as usize;
                    if j < h { curr[i] * prev[j] } else { 0.0 }
                })
                .sum::<f32>()
        };
        let c2 = {
            let d = best_d + 1;
            (lo..hi)
                .map(|i| {
                    let j = (i as isize + d) as usize;
                    if j < h { curr[i] * prev[j] } else { 0.0 }
                })
                .sum::<f32>()
        };
        let denom = 2.0 * (2.0 * best_corr - c0 - c2);
        if denom.abs() > 1e-10 {
            let sub = (c0 - c2) / denom;
            return (best_d as f32 + sub) / max_disp as f32;
        }
    }

    best_d as f32 / max_disp as f32
}

/// Convert a frequency to a canvas Y coordinate.
/// min_freq is shown at the bottom (y = canvas_height), max_freq at the top (y = 0).
#[inline]
pub fn freq_to_y(freq: f64, min_freq: f64, max_freq: f64, canvas_height: f64) -> f64 {
    canvas_height * (1.0 - (freq - min_freq) / (max_freq - min_freq))
}

/// Convert a canvas Y coordinate back to a frequency.
#[inline]
pub fn y_to_freq(y: f64, min_freq: f64, max_freq: f64, canvas_height: f64) -> f64 {
    min_freq + (max_freq - min_freq) * (1.0 - y / canvas_height)
}

/// A base colormap LUT choice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Colormap {
    Viridis,
    Inferno,
    Magma,
    Plasma,
    Cividis,
    Turbo,
    Greyscale,
}

impl Colormap {
    /// Apply this colormap's LUT to a greyscale value.
    #[inline]
    pub fn apply(self, grey: u8) -> [u8; 3] {
        match self {
            Colormap::Viridis => greyscale_to_viridis(grey),
            Colormap::Inferno => greyscale_to_inferno(grey),
            Colormap::Magma => greyscale_to_magma(grey),
            Colormap::Plasma => greyscale_to_plasma(grey),
            Colormap::Cividis => greyscale_to_cividis(grey),
            Colormap::Turbo => greyscale_to_turbo(grey),
            Colormap::Greyscale => [grey, grey, grey],
        }
    }
}

/// Which colormap to apply when blitting the spectrogram.
#[derive(Clone, Copy)]
pub enum ColormapMode {
    /// Uniform colormap across the entire spectrogram.
    Uniform(Colormap),
    /// Colormap inside HFR focus band, greyscale outside.
    /// Fractions are relative to the full image (0 Hz = 0.0, file_max_freq = 1.0).
    HfrFocus { colormap: Colormap, ff_lo_frac: f64, ff_hi_frac: f64 },
}

/// Blit the pre-rendered spectrogram to a visible canvas, handling scroll, zoom, and freq crop.
/// `freq_crop_lo` / `freq_crop_hi` are fractions (0..1) of the full image height:
/// lo = min_display_freq / file_max_freq, hi = max_display_freq / file_max_freq.
pub fn blit_viewport(
    ctx: &CanvasRenderingContext2d,
    pre_rendered: &PreRendered,
    canvas: &HtmlCanvasElement,
    scroll_col: f64,
    zoom: f64,
    freq_crop_lo: f64,
    freq_crop_hi: f64,
    colormap: ColormapMode,
) {
    let cw = canvas.width() as f64;
    let ch = canvas.height() as f64;

    // Clear canvas
    ctx.set_fill_style_str("#000");
    ctx.fill_rect(0.0, 0.0, cw, ch);

    if pre_rendered.width == 0 || pre_rendered.height == 0 {
        return;
    }

    let fc_lo = freq_crop_lo.max(0.0);
    let fc_hi = freq_crop_hi.max(0.01);

    // How many source columns are visible at current zoom
    let natural_visible_cols = cw / zoom;
    let visible_cols = natural_visible_cols.min(pre_rendered.width as f64);
    let src_start = scroll_col.max(0.0).min((pre_rendered.width as f64 - visible_cols).max(0.0));

    // If file has fewer columns than the view span, draw at correct proportional
    // width instead of stretching.  This keeps the spectrogram aligned with the
    // time-to-pixel mapping used by the playhead, waveform, and overlays.
    let dst_w = if (pre_rendered.width as f64) < natural_visible_cols {
        cw * (pre_rendered.width as f64 / natural_visible_cols)
    } else {
        cw
    };

    // Vertical crop: row 0 = highest freq, last row = 0 Hz
    // Extract the band from fc_lo to fc_hi of the full image
    let full_h = pre_rendered.height as f64;
    let (src_y, src_h, dst_y, dst_h) = if fc_hi <= 1.0 {
        let sy = full_h * (1.0 - fc_hi);
        let sh = full_h * (fc_hi - fc_lo).max(0.001);
        (sy, sh, 0.0, ch)
    } else {
        // Display range extends above Nyquist
        let fc_range = (fc_hi - fc_lo).max(0.001);
        let data_frac = (1.0 - fc_lo) / fc_range;
        let sh = full_h * (1.0 - fc_lo);
        (0.0, sh, ch * (1.0 - data_frac), ch * data_frac)
    };

    // Apply colormap (remap greyscale pixels to RGB)
    let mapped_pixels;
    let pixel_data: &[u8] = match colormap {
        ColormapMode::Uniform(cm) => {
            if cm == Colormap::Greyscale {
                &pre_rendered.pixels
            } else {
                mapped_pixels = {
                    let mut buf = pre_rendered.pixels.clone();
                    for chunk in buf.chunks_exact_mut(4) {
                        let [r, g, b] = cm.apply(chunk[0]);
                        chunk[0] = r;
                        chunk[1] = g;
                        chunk[2] = b;
                    }
                    buf
                };
                &mapped_pixels
            }
        }
        ColormapMode::HfrFocus { colormap: cm, ff_lo_frac, ff_hi_frac } => {
            mapped_pixels = {
                let mut buf = pre_rendered.pixels.clone();
                let h = pre_rendered.height as f64;
                let w = pre_rendered.width as usize;
                // Row 0 = highest freq; last row = 0 Hz
                let focus_top = (h * (1.0 - ff_hi_frac)).round() as usize;
                let focus_bot = (h * (1.0 - ff_lo_frac)).round() as usize;
                for row in 0..pre_rendered.height as usize {
                    if row >= focus_top && row < focus_bot {
                        let base = row * w * 4;
                        for col in 0..w {
                            let i = base + col * 4;
                            let [r, g, b] = cm.apply(buf[i]);
                            buf[i] = r;
                            buf[i + 1] = g;
                            buf[i + 2] = b;
                        }
                    }
                    // Outside focus: keep greyscale
                }
                buf
            };
            &mapped_pixels
        }
    };

    // Create ImageData from pixel buffer and draw it
    let clamped = Clamped(pixel_data);
    let image_data = ImageData::new_with_u8_clamped_array_and_sh(
        clamped,
        pre_rendered.width,
        pre_rendered.height,
    );

    match image_data {
        Ok(img) => {
            // Create a temporary canvas to hold the image data, then draw from it
            let doc = web_sys::window().unwrap().document().unwrap();
            let tmp = doc
                .create_element("canvas")
                .unwrap()
                .dyn_into::<HtmlCanvasElement>()
                .unwrap();
            tmp.set_width(pre_rendered.width);
            tmp.set_height(pre_rendered.height);
            let tmp_ctx = tmp
                .get_context("2d")
                .unwrap()
                .unwrap()
                .dyn_into::<CanvasRenderingContext2d>()
                .unwrap();
            let _ = tmp_ctx.put_image_data(&img, 0.0, 0.0);

            // Draw the visible portion, proportionally sized to match overlay coordinate space
            let _ = ctx.draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                &tmp,
                src_start,
                src_y,
                visible_cols,
                src_h,
                0.0,
                dst_y,
                dst_w,
                dst_h,
            );
        }
        Err(e) => {
            log::error!("Failed to create ImageData: {e:?}");
        }
    }
}

/// Blit a `PreviewImage` as a viewport background, correctly mapping its time
/// axis to the current scroll position and visible time range.  Used as a
/// fallback while the full-resolution spectrogram tiles are being computed.
pub fn blit_preview_as_background(
    ctx: &CanvasRenderingContext2d,
    preview: &PreviewImage,
    canvas: &HtmlCanvasElement,
    scroll_offset: f64,    // left edge of viewport in seconds
    visible_time: f64,     // seconds of audio visible in viewport
    total_duration: f64,   // total file duration in seconds
    freq_crop_lo: f64,     // 0..1 fraction of Nyquist
    freq_crop_hi: f64,     // 0..1 fraction of Nyquist
    colormap: ColormapMode,
) {
    let cw = canvas.width() as f64;
    let ch = canvas.height() as f64;

    ctx.set_fill_style_str("#000");
    ctx.fill_rect(0.0, 0.0, cw, ch);

    if preview.width == 0 || preview.height == 0 || total_duration <= 0.0 {
        return;
    }

    // Map viewport time range to preview pixel columns.
    // The preview spans the entire file: column 0 = time 0, column W = total_duration.
    let pw = preview.width as f64;
    let src_x = (scroll_offset / total_duration * pw).clamp(0.0, pw);
    let src_w = (visible_time / total_duration * pw).clamp(1.0, pw - src_x);

    // For short files where the entire file fits in the viewport, scale the
    // destination width proportionally (same logic as blit_viewport).
    let dst_w = if visible_time > total_duration {
        cw * (total_duration / visible_time)
    } else {
        cw
    };

    // Vertical crop: row 0 = highest freq, last row = 0 Hz
    let fc_lo = freq_crop_lo.max(0.0);
    let fc_hi = freq_crop_hi.max(0.01);
    let full_h = preview.height as f64;
    let (src_y, src_h, dst_y, dst_h) = if fc_hi <= 1.0 {
        let sy = full_h * (1.0 - fc_hi);
        let sh = full_h * (fc_hi - fc_lo).max(0.001);
        (sy, sh, 0.0, ch)
    } else {
        let fc_range = (fc_hi - fc_lo).max(0.001);
        let data_frac = (1.0 - fc_lo) / fc_range;
        let sh = full_h * (1.0 - fc_lo);
        (0.0, sh, ch * (1.0 - data_frac), ch * data_frac)
    };

    // Apply colormap to preview pixels (preview is stored as greyscale)
    let mut pixels = preview.pixels.as_ref().clone();
    match colormap {
        ColormapMode::Uniform(cm) => apply_colormap_to_tile(&mut pixels, cm),
        ColormapMode::HfrFocus { colormap: cm, ff_lo_frac, ff_hi_frac } => {
            apply_hfr_colormap_to_tile(
                &mut pixels, preview.width, preview.height,
                cm, ff_lo_frac, ff_hi_frac,
            );
        }
    }

    let clamped = Clamped(&pixels[..]);
    let Ok(img) = ImageData::new_with_u8_clamped_array_and_sh(
        clamped, preview.width, preview.height,
    ) else { return };

    let doc = web_sys::window().unwrap().document().unwrap();
    let tmp = doc.create_element("canvas").unwrap()
        .dyn_into::<HtmlCanvasElement>().unwrap();
    tmp.set_width(preview.width);
    tmp.set_height(preview.height);
    let Some(tmp_ctx) = tmp.get_context("2d").ok().flatten()
        .and_then(|c| c.dyn_into::<CanvasRenderingContext2d>().ok())
    else { return };
    let _ = tmp_ctx.put_image_data(&img, 0.0, 0.0);

    let _ = ctx.draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
        &tmp,
        src_x, src_y, src_w, src_h,
        0.0, dst_y, dst_w, dst_h,
    );
}

// ── Tile-based rendering ─────────────────────────────────────────────────────

use crate::canvas::tile_cache::{self, TILE_COLS};
use std::cell::RefCell;

thread_local! {
    /// Reusable off-screen canvas for blitting tile ImageData.
    /// Avoids creating a new canvas element every frame for each tile.
    static TMP_CANVAS: RefCell<Option<(HtmlCanvasElement, CanvasRenderingContext2d)>> =
        const { RefCell::new(None) };
}

/// Get or create a reusable off-screen canvas of at least the given dimensions.
fn get_tmp_canvas(w: u32, h: u32) -> Option<(HtmlCanvasElement, CanvasRenderingContext2d)> {
    TMP_CANVAS.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some((ref c, ref ctx)) = *slot {
            if c.width() >= w && c.height() >= h {
                return Some((c.clone(), ctx.clone()));
            }
        }
        // Create new
        let doc = web_sys::window()?.document()?;
        let c = doc.create_element("canvas").ok()?
            .dyn_into::<HtmlCanvasElement>().ok()?;
        c.set_width(w);
        c.set_height(h);
        let ctx = c.get_context("2d").ok()??.dyn_into::<CanvasRenderingContext2d>().ok()?;
        *slot = Some((c.clone(), ctx.clone()));
        Some((c, ctx))
    })
}

/// Apply a colormap LUT to greyscale RGBA pixels in-place.
fn apply_colormap_to_tile(pixels: &mut [u8], colormap: Colormap) {
    if colormap == Colormap::Greyscale {
        return;
    }
    for chunk in pixels.chunks_exact_mut(4) {
        let [r, g, b] = colormap.apply(chunk[0]);
        chunk[0] = r;
        chunk[1] = g;
        chunk[2] = b;
    }
}

/// Apply HFR-focus colormap: color inside focus band, greyscale outside.
fn apply_hfr_colormap_to_tile(
    pixels: &mut [u8], width: u32, height: u32,
    colormap: Colormap, ff_lo_frac: f64, ff_hi_frac: f64,
) {
    let h = height as f64;
    let w = width as usize;
    let focus_top = (h * (1.0 - ff_hi_frac)).round() as usize;
    let focus_bot = (h * (1.0 - ff_lo_frac)).round() as usize;
    for row in 0..height as usize {
        if row >= focus_top && row < focus_bot {
            let base = row * w * 4;
            for col in 0..w {
                let i = base + col * 4;
                let [r, g, b] = colormap.apply(pixels[i]);
                pixels[i] = r;
                pixels[i + 1] = g;
                pixels[i + 2] = b;
            }
        }
    }
}

/// Composite spectrogram tiles from the tile cache onto the canvas.
/// Falls back to a preview image for tiles not yet cached.
/// Returns true if at least one tile was drawn, false if nothing was available.
pub fn blit_tiles_viewport(
    ctx: &CanvasRenderingContext2d,
    canvas: &HtmlCanvasElement,
    file_idx: usize,
    total_cols: usize,
    scroll_col: f64,
    zoom: f64,
    freq_crop_lo: f64,
    freq_crop_hi: f64,
    colormap: ColormapMode,
    // Preview fallback for missing tiles
    preview: Option<&PreviewImage>,
    scroll_offset: f64,    // seconds (for preview mapping)
    visible_time: f64,     // seconds (for preview mapping)
    total_duration: f64,   // seconds (for preview mapping)
) -> bool {
    let cw = canvas.width() as f64;
    let ch = canvas.height() as f64;

    // Draw colormapped preview as base layer so tile gaps show preview, not black.
    // Tiles (opaque) are drawn on top and fully cover the preview where they exist.
    if let Some(pv) = preview {
        blit_preview_as_background(
            ctx, pv, canvas,
            scroll_offset, visible_time, total_duration,
            freq_crop_lo, freq_crop_hi, colormap,
        );
    } else {
        ctx.set_fill_style_str("#000");
        ctx.fill_rect(0.0, 0.0, cw, ch);
    }

    if total_cols == 0 || zoom <= 0.0 {
        return preview.is_some();
    }

    // Visible column range
    let visible_cols = cw / zoom;
    let src_start = scroll_col.max(0.0).min((total_cols as f64 - 1.0).max(0.0));
    let src_end = (src_start + visible_cols).min(total_cols as f64);

    // Tile index range
    let first_tile = (src_start / TILE_COLS as f64).floor() as usize;
    let last_tile = ((src_end - 1.0).max(0.0) / TILE_COLS as f64).floor() as usize;
    let n_tiles = (total_cols + TILE_COLS - 1) / TILE_COLS;

    // Vertical crop
    let fc_lo = freq_crop_lo.max(0.0);
    let fc_hi = freq_crop_hi.max(0.01);

    let mut any_drawn = false;

    for tile_idx in first_tile..=last_tile.min(n_tiles.saturating_sub(1)) {
        let tile_col_start = tile_idx * TILE_COLS;

        let drawn = tile_cache::borrow_tile(file_idx, tile_idx, |tile| {
            let tw = tile.rendered.width as f64;
            let th = tile.rendered.height as f64;
            if tw == 0.0 || th == 0.0 { return; }

            // Clone pixels for colormap application
            let mut pixels = tile.rendered.pixels.clone();
            match colormap {
                ColormapMode::Uniform(cm) => apply_colormap_to_tile(&mut pixels, cm),
                ColormapMode::HfrFocus { colormap: cm, ff_lo_frac, ff_hi_frac } => {
                    apply_hfr_colormap_to_tile(
                        &mut pixels, tile.rendered.width, tile.rendered.height,
                        cm, ff_lo_frac, ff_hi_frac,
                    );
                }
            }

            // Create ImageData and draw to off-screen canvas
            let clamped = Clamped(&pixels[..]);
            let Ok(img) = ImageData::new_with_u8_clamped_array_and_sh(
                clamped, tile.rendered.width, tile.rendered.height,
            ) else { return };

            let Some((tmp, tmp_ctx)) = get_tmp_canvas(tile.rendered.width, tile.rendered.height) else { return };
            // Resize if needed (get_tmp_canvas guarantees >= size)
            if tmp.width() != tile.rendered.width || tmp.height() != tile.rendered.height {
                tmp.set_width(tile.rendered.width);
                tmp.set_height(tile.rendered.height);
            }
            let _ = tmp_ctx.put_image_data(&img, 0.0, 0.0);

            // Source rect within this tile
            let tile_src_x = (src_start - tile_col_start as f64).max(0.0);
            let tile_src_end = (src_end - tile_col_start as f64).min(tw);
            let tile_src_w = (tile_src_end - tile_src_x).max(0.0);
            if tile_src_w <= 0.0 { return; }

            // Vertical crop
            let (src_y, src_h, dst_y, dst_h) = if fc_hi <= 1.0 {
                let sy = th * (1.0 - fc_hi);
                let sh = th * (fc_hi - fc_lo).max(0.001);
                (sy, sh, 0.0, ch)
            } else {
                let fc_range = (fc_hi - fc_lo).max(0.001);
                let data_frac = (1.0 - fc_lo) / fc_range;
                let sh = th * (1.0 - fc_lo);
                (0.0, sh, ch * (1.0 - data_frac), ch * data_frac)
            };

            // Destination x: where this tile's visible portion starts on canvas.
            // Floor left edge & ceil right edge to eliminate sub-pixel gaps between tiles.
            let dst_x_raw = ((tile_col_start as f64 + tile_src_x) - src_start) * zoom;
            let dst_x_end_raw = ((tile_col_start as f64 + tile_src_x + tile_src_w) - src_start) * zoom;
            let dst_x = dst_x_raw.floor();
            let dst_w = (dst_x_end_raw.ceil() - dst_x).max(1.0);

            let _ = ctx.draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                &tmp,
                tile_src_x, src_y, tile_src_w, src_h,
                dst_x, dst_y, dst_w, dst_h,
            );
        });

        if drawn.is_some() {
            any_drawn = true;
        } else {
            // LOD 1 tile missing — try LOD 0 (blurry but fast) as fallback
            let lod0_drawn = tile_cache::borrow_lod0_tile(file_idx, tile_idx, |tile| {
                let tw = tile.rendered.width as f64;
                let th = tile.rendered.height as f64;
                if tw == 0.0 || th == 0.0 { return; }

                let mut pixels = tile.rendered.pixels.clone();
                match colormap {
                    ColormapMode::Uniform(cm) => apply_colormap_to_tile(&mut pixels, cm),
                    ColormapMode::HfrFocus { colormap: cm, ff_lo_frac, ff_hi_frac } => {
                        apply_hfr_colormap_to_tile(
                            &mut pixels, tile.rendered.width, tile.rendered.height,
                            cm, ff_lo_frac, ff_hi_frac,
                        );
                    }
                }

                let clamped = Clamped(&pixels[..]);
                let Ok(img) = ImageData::new_with_u8_clamped_array_and_sh(
                    clamped, tile.rendered.width, tile.rendered.height,
                ) else { return };

                let Some((tmp, tmp_ctx)) = get_tmp_canvas(tile.rendered.width, tile.rendered.height) else { return };
                if tmp.width() != tile.rendered.width || tmp.height() != tile.rendered.height {
                    tmp.set_width(tile.rendered.width);
                    tmp.set_height(tile.rendered.height);
                }
                let _ = tmp_ctx.put_image_data(&img, 0.0, 0.0);

                // LOD 0 tile covers the same time range but has fewer columns.
                // Map source coordinates proportionally: LOD 0's tw columns span
                // the same range as TILE_COLS at LOD 1.
                let scale = tw / TILE_COLS as f64;
                let tile_src_x = ((src_start - tile_col_start as f64).max(0.0)) * scale;
                let tile_src_end = ((src_end - tile_col_start as f64).min(TILE_COLS as f64)) * scale;
                let tile_src_w = (tile_src_end - tile_src_x).max(0.0);
                if tile_src_w <= 0.0 { return; }

                // Vertical crop
                let (src_y, src_h, dst_y, dst_h) = if fc_hi <= 1.0 {
                    let sy = th * (1.0 - fc_hi);
                    let sh = th * (fc_hi - fc_lo).max(0.001);
                    (sy, sh, 0.0, ch)
                } else {
                    let fc_range = (fc_hi - fc_lo).max(0.001);
                    let data_frac = (1.0 - fc_lo) / fc_range;
                    let sh = th * (1.0 - fc_lo);
                    (0.0, sh, ch * (1.0 - data_frac), ch * data_frac)
                };

                // Destination: same position as the LOD 1 tile would occupy.
                // Floor/ceil to match LOD 1 tile snapping and avoid sub-pixel gaps.
                let col_offset = (src_start - tile_col_start as f64).max(0.0);
                let dst_x_raw = ((tile_col_start as f64 + col_offset) - src_start) * zoom;
                let visible_cols = ((src_end - tile_col_start as f64).min(TILE_COLS as f64) - col_offset).max(0.0);
                let dst_x_end_raw = dst_x_raw + visible_cols * zoom;
                let dst_x = dst_x_raw.floor();
                let dst_w = (dst_x_end_raw.ceil() - dst_x).max(1.0);

                // Enable image smoothing for stretched LOD 0
                ctx.set_image_smoothing_enabled(true);
                let _ = ctx.draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                    &tmp,
                    tile_src_x, src_y, tile_src_w, src_h,
                    dst_x, dst_y, dst_w, dst_h,
                );
            });
            if lod0_drawn.is_some() {
                any_drawn = true;
            }
        }
    }

    // Preview was drawn as base layer, so even if no tiles exist, something is visible
    any_drawn || preview.is_some()
}

/// Describes how frequency markers should show shifted output frequencies.
#[derive(Clone, Copy)]
pub enum FreqShiftMode {
    /// No shift annotation.
    None,
    /// Heterodyne: show |freq - het_freq| for markers within ±15 kHz of het_freq.
    Heterodyne(f64),
    /// Time expansion or pitch shift: all freqs divide by factor.
    Divide(f64),
    /// Shift up: all freqs multiply by factor (infrasound → audible).
    Multiply(f64),
}

/// Frequency marker hover/interaction state passed to drawing functions.
pub struct FreqMarkerState {
    pub mouse_freq: Option<f64>,
    pub mouse_in_label_area: bool,
    pub label_hover_opacity: f64,
    pub has_selection: bool,
    pub file_max_freq: f64,
    /// Axis drag range for lighting up color bars
    pub axis_drag_lo: Option<f64>,
    pub axis_drag_hi: Option<f64>,
    /// FF handle drag is active (light up FF range bars)
    pub ff_drag_active: bool,
    pub ff_lo: f64,
    pub ff_hi: f64,
    /// FF handles are hovered or being dragged (hide cursor indicator)
    pub ff_handles_active: bool,
}

/// Draw horizontal frequency marker lines with subtle, interactive UI.
/// Labels are white; colored range bars indicate the resistor-band color.
pub fn draw_freq_markers(
    ctx: &CanvasRenderingContext2d,
    min_freq: f64,
    max_freq: f64,
    canvas_height: f64,
    canvas_width: f64,
    shift_mode: FreqShiftMode,
    ms: &FreqMarkerState,
    het_cutoff: f64,
) {
    let cutoff = het_cutoff;
    let color_bar_w = 6.0;
    let color_bar_x = 0.0; // flush left
    let label_x = color_bar_w + 3.0; // text starts after color bar
    let tick_len = 22.0; // short tick under label (~half old label_area_w)
    let right_tick_len = 15.0;

    // Collect all division freqs within visible range
    let mut divisions: Vec<f64> = Vec::new();
    let first_div = ((min_freq / 10_000.0).ceil() * 10_000.0).max(10_000.0);
    let mut freq = first_div;
    while freq < max_freq {
        divisions.push(freq);
        freq += 10_000.0;
    }

    // Check if top of display is nyquist
    let is_nyquist_top = (max_freq - ms.file_max_freq).abs() < 1.0;
    // Find topmost division for nyquist overlap check
    let topmost_div = divisions.last().copied().unwrap_or(0.0);
    let topmost_div_y_frac = if max_freq > min_freq { (topmost_div - min_freq) / (max_freq - min_freq) } else { 0.0 };
    let hide_topmost_for_nyquist = is_nyquist_top && topmost_div_y_frac > 0.95;

    for &freq in &divisions {
        let y = freq_to_y(freq, min_freq, max_freq, canvas_height);

        // Skip topmost division if it would overlap nyquist marker
        if hide_topmost_for_nyquist && freq == topmost_div && !ms.mouse_in_label_area {
            continue;
        }

        let color = freq_marker_color(freq);

        // Determine alpha based on HET audible band
        let base_alpha = match shift_mode {
            FreqShiftMode::Heterodyne(hf) => {
                if (freq - hf).abs() <= cutoff { 0.8 } else { 0.3 }
            }
            _ => 0.7,
        };

        // --- Color range bar (left edge, covering the decade above: freq to freq+10k) ---
        // e.g. 40kHz marker (yellow) covers 40–50kHz
        let bar_top_freq = (freq + 10_000.0).min(max_freq);
        let mouse_in_range = ms.mouse_freq.map_or(false, |mf| mf >= freq && mf < bar_top_freq);
        let axis_drag_in_range = match (ms.axis_drag_lo, ms.axis_drag_hi) {
            (Some(lo), Some(hi)) => bar_top_freq > lo && freq < hi,
            _ => false,
        };
        let ff_drag_in_range = ms.ff_drag_active && ms.ff_hi > ms.ff_lo && bar_top_freq > ms.ff_lo && freq < ms.ff_hi;
        if ms.has_selection || mouse_in_range || axis_drag_in_range || ff_drag_in_range {
            let bar_alpha = if axis_drag_in_range || ff_drag_in_range { 0.8 } else if ms.has_selection { 0.6 } else { 0.8 };
            let bar_y_top = freq_to_y(bar_top_freq, min_freq, max_freq, canvas_height);
            let bar_y_bot = freq_to_y(freq, min_freq, max_freq, canvas_height);
            ctx.set_fill_style_str(&format!("rgba({},{},{},{:.2})", color[0], color[1], color[2], bar_alpha));
            ctx.fill_rect(color_bar_x, bar_y_top, color_bar_w, bar_y_bot - bar_y_top);
        }

        // --- White text label (drawn ABOVE the division line) ---
        ctx.set_font("11px sans-serif");
        ctx.set_text_baseline("bottom"); // text sits above the line
        let base_label = freq_marker_label(freq);
        let label_alpha = base_alpha;

        // Build label with optional kHz suffix and shift info
        let label = match shift_mode {
            FreqShiftMode::Heterodyne(hf) => {
                if ms.label_hover_opacity > 0.01 {
                    let diff = (freq - hf).abs();
                    if diff <= cutoff {
                        let diff_khz = (diff / 1000.0).round() as u32;
                        format!("{base_label} kHz \u{2192} {diff_khz} kHz")
                    } else {
                        format!("{base_label} kHz")
                    }
                } else {
                    base_label.clone()
                }
            }
            FreqShiftMode::Divide(factor) if factor > 1.0 => {
                if ms.label_hover_opacity > 0.01 {
                    let shifted = freq / factor;
                    let shifted_khz = shifted / 1000.0;
                    if shifted_khz >= 1.0 {
                        format!("{base_label} kHz \u{2192} {:.0} kHz", shifted_khz)
                    } else {
                        format!("{base_label} kHz \u{2192} {:.0} Hz", shifted)
                    }
                } else {
                    base_label.clone()
                }
            }
            FreqShiftMode::Multiply(factor) if factor > 1.0 => {
                if ms.label_hover_opacity > 0.01 {
                    let shifted = freq * factor;
                    let shifted_khz = shifted / 1000.0;
                    if shifted_khz >= 1.0 {
                        format!("{base_label} kHz \u{2192} {:.0} kHz", shifted_khz)
                    } else {
                        format!("{base_label} kHz \u{2192} {:.0} Hz", shifted)
                    }
                } else {
                    base_label.clone()
                }
            }
            _ => {
                // For FreqShiftMode::None, never include " kHz" here;
                // it's drawn separately below with a smooth fade.
                base_label.clone()
            }
        };

        // kHz fade: use opacity^2 for faster visual fade
        let khz_fade = ms.label_hover_opacity * ms.label_hover_opacity;
        if matches!(shift_mode, FreqShiftMode::None) && ms.label_hover_opacity > 0.001 {
            // Split rendering: number at full alpha, " kHz" suffix fading
            // Dark background behind label
            let full_label_for_measure = if khz_fade > 0.01 {
                format!("{} kHz", base_label)
            } else {
                base_label.clone()
            };
            let bg_metrics = ctx.measure_text(&full_label_for_measure).unwrap();
            let bg_w = bg_metrics.width() + 4.0;
            let bg_h = 14.0;
            ctx.set_fill_style_str("rgba(0,0,0,0.6)");
            ctx.fill_rect(label_x - 2.0, y - 2.0 - bg_h, bg_w, bg_h);

            ctx.set_fill_style_str(&format!("rgba(255,255,255,{:.2})", label_alpha));
            let _ = ctx.fill_text(&base_label, label_x, y - 2.0);
            let khz_alpha = label_alpha * khz_fade;
            if khz_alpha > 0.002 {
                let metrics = ctx.measure_text(&base_label).unwrap();
                let num_w = metrics.width();
                ctx.set_fill_style_str(&format!("rgba(255,255,255,{:.2})", khz_alpha));
                let _ = ctx.fill_text(" kHz", label_x + num_w, y - 2.0);
            }
        } else {
            // Dark background behind label
            let bg_metrics = ctx.measure_text(&label).unwrap();
            let bg_w = bg_metrics.width() + 4.0;
            let bg_h = 14.0;
            ctx.set_fill_style_str("rgba(0,0,0,0.6)");
            ctx.fill_rect(label_x - 2.0, y - 2.0 - bg_h, bg_w, bg_h);

            ctx.set_fill_style_str(&format!("rgba(255,255,255,{:.2})", label_alpha));
            let _ = ctx.fill_text(&label, label_x, y - 2.0);
        }

        // --- Short left tick line (lightly colored, under the label) ---
        // Blend: mostly white with a hint of the marker color
        let tr = 200 + (color[0] as u16 * 55 / 255) as u8;
        let tg = 200 + (color[1] as u16 * 55 / 255) as u8;
        let tb = 200 + (color[2] as u16 * 55 / 255) as u8;
        ctx.set_stroke_style_str(&format!("rgba({},{},{},{:.2})", tr, tg, tb, base_alpha * 0.5));
        ctx.set_line_width(1.0);
        ctx.begin_path();
        ctx.move_to(0.0, y);
        ctx.line_to(tick_len, y);
        ctx.stroke();

        // --- Short right tick line (same tint) ---
        ctx.begin_path();
        ctx.move_to(canvas_width - right_tick_len, y);
        ctx.line_to(canvas_width, y);
        ctx.stroke();

        // --- Full-width line (fades in when hovering label area, white) ---
        if ms.label_hover_opacity > 0.001 {
            let full_alpha = ms.label_hover_opacity * 0.7 * base_alpha;
            ctx.set_stroke_style_str(&format!("rgba(255,255,255,{:.3})", full_alpha));
            ctx.set_line_width(1.0);
            ctx.begin_path();
            ctx.move_to(tick_len, y);
            ctx.line_to(canvas_width - right_tick_len, y);
            ctx.stroke();
        }
    }

    // --- Nyquist / MAX marker ---
    if is_nyquist_top && !ms.mouse_in_label_area {
        let ny_y = 2.0; // just below top edge
        let ny_khz = ms.file_max_freq / 1000.0;
        let ny_label = if ny_khz == ny_khz.round() {
            format!("{:.0}k MAX", ny_khz)
        } else {
            format!("{:.1}k MAX", ny_khz)
        };
        ctx.set_fill_style_str("rgba(255,255,255,0.45)");
        ctx.set_font("10px sans-serif");
        ctx.set_text_baseline("top");
        let _ = ctx.fill_text(&ny_label, label_x, ny_y);
        ctx.set_stroke_style_str("rgba(255,255,255,0.3)");
        ctx.set_line_width(1.0);
        ctx.begin_path();
        ctx.move_to(0.0, 0.5);
        ctx.line_to(tick_len, 0.5);
        ctx.stroke();
        // Right tick
        ctx.begin_path();
        ctx.move_to(canvas_width - right_tick_len, 0.5);
        ctx.line_to(canvas_width, 0.5);
        ctx.stroke();
    }

    // --- Cursor frequency indicator (hidden when FF handles are active) ---
    if let Some(mf) = ms.mouse_freq {
        if !ms.mouse_in_label_area && !ms.ff_handles_active && mf > min_freq && mf < max_freq {
            let y = freq_to_y(mf, min_freq, max_freq, canvas_height);

            // Label (above the dashed line, starting around midpoint)
            let freq_label = if mf >= 1000.0 {
                format!("{:.1} kHz", mf / 1000.0)
            } else {
                format!("{:.0} Hz", mf)
            };
            let cursor_line_len = canvas_width * 0.5;
            let label_start_x = cursor_line_len * 0.5;
            ctx.set_font("10px sans-serif");
            ctx.set_text_baseline("bottom");
            ctx.set_fill_style_str("rgba(0,210,240,0.8)");
            let _ = ctx.fill_text(&freq_label, label_start_x, y - 2.0);

            // Dashed line (cyan)
            ctx.set_stroke_style_str("rgba(0,210,240,0.45)");
            ctx.set_line_width(1.0);
            let _ = ctx.set_line_dash(&js_sys::Array::of2(
                &wasm_bindgen::JsValue::from_f64(4.0),
                &wasm_bindgen::JsValue::from_f64(4.0),
            ));
            ctx.begin_path();
            ctx.move_to(0.0, y);
            ctx.line_to(cursor_line_len, y);
            ctx.stroke();
            let _ = ctx.set_line_dash(&js_sys::Array::new());

            // Right-side frequency label
            ctx.set_text_baseline("middle");
            ctx.set_fill_style_str("rgba(0,210,240,0.7)");
            let metrics = ctx.measure_text(&freq_label).unwrap();
            let text_w = metrics.width();
            let _ = ctx.fill_text(&freq_label, canvas_width - text_w - right_tick_len - 4.0, y);
        }
    }

    ctx.set_text_baseline("alphabetic"); // reset
}

/// Draw the Frequency Focus overlay: dim outside the FF range, amber edge lines with drag handles.
pub fn draw_ff_overlay(
    ctx: &CanvasRenderingContext2d,
    ff_lo: f64,
    ff_hi: f64,
    min_freq: f64,
    max_freq: f64,
    canvas_height: f64,
    canvas_width: f64,
    hover_handle: Option<SpectrogramHandle>,
    drag_handle: Option<SpectrogramHandle>,
) {
    if ff_hi <= ff_lo { return; }

    let y_top = freq_to_y(ff_hi.min(max_freq), min_freq, max_freq, canvas_height);
    let y_bottom = freq_to_y(ff_lo.max(min_freq), min_freq, max_freq, canvas_height);

    // Dim outside the FF range
    ctx.set_fill_style_str("rgba(0, 0, 0, 0.45)");
    if y_top > 0.0 {
        ctx.fill_rect(0.0, 0.0, canvas_width, y_top);
    }
    if y_bottom < canvas_height {
        ctx.fill_rect(0.0, y_bottom, canvas_width, canvas_height - y_bottom);
    }

    let is_active = |handle: SpectrogramHandle| -> bool {
        drag_handle == Some(handle) || hover_handle == Some(handle)
    };

    // Amber edge lines + triangular drag handles
    for &(y, handle) in &[(y_top, SpectrogramHandle::FfUpper), (y_bottom, SpectrogramHandle::FfLower)] {
        let active = is_active(handle);
        let alpha = if active { 0.9 } else { 0.4 };
        let width = if active { 2.0 } else { 1.0 };
        ctx.set_stroke_style_str(&format!("rgba(255, 180, 60, {:.2})", alpha));
        ctx.set_line_width(width);
        ctx.begin_path();
        ctx.move_to(0.0, y);
        ctx.line_to(canvas_width, y);
        ctx.stroke();

        // Triangle handle at right edge
        let handle_size = if active { 10.0 } else { 6.0 };
        let handle_alpha = if active { 0.9 } else { 0.4 };
        ctx.set_fill_style_str(&format!("rgba(255, 180, 60, {:.2})", handle_alpha));
        ctx.begin_path();
        ctx.move_to(canvas_width, y - handle_size);
        ctx.line_to(canvas_width - handle_size, y);
        ctx.line_to(canvas_width, y + handle_size);
        ctx.close_path();
        let _ = ctx.fill();
    }

    // Middle handle (triangle at midpoint on right edge)
    let mid_y = (y_top + y_bottom) / 2.0;
    let mid_active = is_active(SpectrogramHandle::FfMiddle);
    let mid_alpha = if mid_active { 0.9 } else { 0.3 };
    let mid_size = if mid_active { 8.0 } else { 5.0 };
    ctx.set_fill_style_str(&format!("rgba(255, 180, 60, {:.2})", mid_alpha));
    ctx.begin_path();
    ctx.move_to(canvas_width, mid_y - mid_size);
    ctx.line_to(canvas_width - mid_size, mid_y);
    ctx.line_to(canvas_width, mid_y + mid_size);
    ctx.close_path();
    let _ = ctx.fill();

    // FF range labels (only when handles are active): top and bottom frequencies
    if hover_handle.is_some() || drag_handle.is_some() {
        ctx.set_fill_style_str("rgba(255, 180, 60, 0.8)");
        ctx.set_font("11px sans-serif");
        let label_x = canvas_width * 0.35;

        // Top frequency label: just above the upper FF line
        let top_label = format!("{:.1} kHz", ff_hi / 1000.0);
        ctx.set_text_baseline("bottom");
        let _ = ctx.fill_text(&top_label, label_x, y_top - 4.0);

        // Bottom frequency label: just below the lower FF line
        let bottom_label = format!("{:.1} kHz", ff_lo / 1000.0);
        ctx.set_text_baseline("top");
        let _ = ctx.fill_text(&bottom_label, label_x, y_bottom + 4.0);

        ctx.set_text_baseline("alphabetic");
    }
}

/// Draw the heterodyne frequency overlay: cyan center + band edge lines (no dimming — FF handles that).
pub fn draw_het_overlay(
    ctx: &CanvasRenderingContext2d,
    het_freq: f64,
    het_cutoff: f64,
    min_freq: f64,
    max_freq: f64,
    canvas_height: f64,
    canvas_width: f64,
    hover_handle: Option<SpectrogramHandle>,
    drag_handle: Option<SpectrogramHandle>,
    interactive: bool,
) {
    let cutoff = het_cutoff;
    let band_low = (het_freq - cutoff).max(min_freq);
    let band_high = (het_freq + cutoff).min(max_freq);

    let y_center = freq_to_y(het_freq, min_freq, max_freq, canvas_height);
    let y_band_top = freq_to_y(band_high, min_freq, max_freq, canvas_height);
    let y_band_bottom = freq_to_y(band_low, min_freq, max_freq, canvas_height);

    // Opacity multiplier: lower when non-interactive (auto mode without hover)
    let op = if interactive { 1.0 } else { 0.5 };

    let is_active = |handle: SpectrogramHandle| -> bool {
        drag_handle == Some(handle) || hover_handle == Some(handle)
    };

    // Band edge lines
    for &(y, handle) in &[(y_band_top, SpectrogramHandle::HetBandUpper), (y_band_bottom, SpectrogramHandle::HetBandLower)] {
        let active = interactive && is_active(handle);
        let alpha = (if active { 0.7 } else { 0.3 }) * op;
        let width = if active { 2.0 } else { 1.0 };
        ctx.set_stroke_style_str(&format!("rgba(0, 200, 255, {:.2})", alpha));
        ctx.set_line_width(width);
        ctx.begin_path();
        ctx.move_to(0.0, y);
        ctx.line_to(canvas_width, y);
        ctx.stroke();

        // Draw handle triangle at right edge (only when interactive)
        if interactive {
            let handle_size = if active { 10.0 } else { 6.0 };
            let handle_alpha = if active { 0.9 } else { 0.4 };
            ctx.set_fill_style_str(&format!("rgba(0, 200, 255, {:.2})", handle_alpha));
            ctx.begin_path();
            ctx.move_to(canvas_width, y - handle_size);
            ctx.line_to(canvas_width - handle_size, y);
            ctx.line_to(canvas_width, y + handle_size);
            ctx.close_path();
            let _ = ctx.fill();
        }
    }

    // Center line at het_freq
    let center_active = interactive && is_active(SpectrogramHandle::HetCenter);
    let center_dragging = interactive && drag_handle == Some(SpectrogramHandle::HetCenter);
    if center_dragging {
        ctx.set_stroke_style_str("rgba(0, 230, 255, 1.0)");
        ctx.set_line_width(2.0);
    } else if center_active {
        ctx.set_stroke_style_str("rgba(0, 230, 255, 1.0)");
        ctx.set_line_width(2.0);
        let _ = ctx.set_line_dash(&js_sys::Array::of2(
            &wasm_bindgen::JsValue::from_f64(6.0),
            &wasm_bindgen::JsValue::from_f64(4.0),
        ));
    } else {
        ctx.set_stroke_style_str(&format!("rgba(0, 230, 255, {:.1})", 0.8 * op));
        ctx.set_line_width(1.5);
        let _ = ctx.set_line_dash(&js_sys::Array::of2(
            &wasm_bindgen::JsValue::from_f64(6.0),
            &wasm_bindgen::JsValue::from_f64(4.0),
        ));
    }
    ctx.begin_path();
    ctx.move_to(0.0, y_center);
    ctx.line_to(canvas_width, y_center);
    ctx.stroke();
    let _ = ctx.set_line_dash(&js_sys::Array::new());

    // Center handle triangle (only when interactive)
    if interactive {
        let handle_size = if center_active { 10.0 } else { 6.0 };
        let handle_alpha = if center_active { 0.9 } else { 0.5 };
        ctx.set_fill_style_str(&format!("rgba(0, 230, 255, {:.2})", handle_alpha));
        ctx.begin_path();
        ctx.move_to(canvas_width, y_center - handle_size);
        ctx.line_to(canvas_width - handle_size, y_center);
        ctx.line_to(canvas_width, y_center + handle_size);
        ctx.close_path();
        let _ = ctx.fill();
    }

    // Label at center line
    ctx.set_fill_style_str(&format!("rgba(0, 230, 255, {:.1})", 0.9 * op));
    ctx.set_font("bold 12px sans-serif");
    let label = format!("HET {:.1} kHz", het_freq / 1000.0);
    let _ = ctx.fill_text(&label, 55.0, y_center - 5.0);

    // LP cutoff label near band edges (show when any HET handle is active)
    if interactive && (hover_handle.is_some() || drag_handle.is_some()) {
        ctx.set_fill_style_str("rgba(0, 200, 255, 0.7)");
        ctx.set_font("11px sans-serif");
        let lp_label = format!("LP ±{:.1} kHz", het_cutoff / 1000.0);
        let _ = ctx.fill_text(&lp_label, 55.0, y_band_bottom + 14.0);
    }
}

/// Draw selection rectangle overlay on spectrogram.
pub fn draw_selection(
    ctx: &CanvasRenderingContext2d,
    selection: &Selection,
    min_freq: f64,
    max_freq: f64,
    scroll_offset: f64,
    time_resolution: f64,
    zoom: f64,
    canvas_width: f64,
    canvas_height: f64,
) {
    let visible_time = (canvas_width / zoom) * time_resolution;
    let start_time = scroll_offset;
    let px_per_sec = canvas_width / visible_time;

    let x0 = ((selection.time_start - start_time) * px_per_sec).max(0.0);
    let x1 = ((selection.time_end - start_time) * px_per_sec).min(canvas_width);
    let y0 = freq_to_y(selection.freq_high, min_freq, max_freq, canvas_height).max(0.0);
    let y1 = freq_to_y(selection.freq_low, min_freq, max_freq, canvas_height).min(canvas_height);

    if x1 <= x0 || y1 <= y0 {
        return;
    }

    // Fill
    ctx.set_fill_style_str("rgba(50, 120, 200, 0.15)");
    ctx.fill_rect(x0, y0, x1 - x0, y1 - y0);

    // Border
    ctx.set_stroke_style_str("rgba(80, 160, 255, 0.7)");
    ctx.set_line_width(1.0);
    ctx.stroke_rect(x0, y0, x1 - x0, y1 - y0);
}

/// Draw shadow selection boxes one octave higher and lower to highlight harmonics.
/// Only drawn when the selection spans less than 1 octave.
pub fn draw_harmonic_shadows(
    ctx: &CanvasRenderingContext2d,
    selection: &Selection,
    min_freq: f64,
    max_freq: f64,
    scroll_offset: f64,
    time_resolution: f64,
    zoom: f64,
    canvas_width: f64,
    canvas_height: f64,
) {
    // Only show shadows if selection is less than 1 octave
    if selection.freq_low <= 0.0 || selection.freq_high / selection.freq_low >= 2.0 {
        return;
    }

    let visible_time = (canvas_width / zoom) * time_resolution;
    let start_time = scroll_offset;
    let px_per_sec = canvas_width / visible_time;

    let x0 = ((selection.time_start - start_time) * px_per_sec).max(0.0);
    let x1 = ((selection.time_end - start_time) * px_per_sec).min(canvas_width);
    if x1 <= x0 {
        return;
    }
    let w = x1 - x0;

    // Set up dashed border style
    let _ = ctx.set_line_dash(&js_sys::Array::of2(
        &wasm_bindgen::JsValue::from_f64(4.0),
        &wasm_bindgen::JsValue::from_f64(4.0),
    ));

    // Octave higher
    let hi_low = selection.freq_low * 2.0;
    let hi_high = selection.freq_high * 2.0;
    if hi_low < max_freq {
        let y0 = freq_to_y(hi_high.min(max_freq), min_freq, max_freq, canvas_height).max(0.0);
        let y1 = freq_to_y(hi_low, min_freq, max_freq, canvas_height).min(canvas_height);
        if y1 > y0 {
            ctx.set_fill_style_str("rgba(50, 120, 200, 0.06)");
            ctx.fill_rect(x0, y0, w, y1 - y0);
            ctx.set_stroke_style_str("rgba(80, 160, 255, 0.3)");
            ctx.set_line_width(1.0);
            ctx.stroke_rect(x0, y0, w, y1 - y0);
        }
    }

    // Octave lower
    let lo_low = selection.freq_low / 2.0;
    let lo_high = selection.freq_high / 2.0;
    {
        let y0 = freq_to_y(lo_high, min_freq, max_freq, canvas_height).max(0.0);
        let y1 = freq_to_y(lo_low.max(min_freq), min_freq, max_freq, canvas_height).min(canvas_height);
        if y1 > y0 {
            ctx.set_fill_style_str("rgba(50, 120, 200, 0.06)");
            ctx.fill_rect(x0, y0, w, y1 - y0);
            ctx.set_stroke_style_str("rgba(80, 160, 255, 0.3)");
            ctx.set_line_width(1.0);
            ctx.stroke_rect(x0, y0, w, y1 - y0);
        }
    }

    // Reset dash
    let _ = ctx.set_line_dash(&js_sys::Array::new());
}

/// Draw filter EQ band overlay on the spectrogram.
///
/// Highlights the frequency region of the currently hovered band slider.
/// band: 0=below, 1=selected, 2=harmonics, 3=above
pub fn draw_filter_overlay(
    ctx: &CanvasRenderingContext2d,
    hovered_band: u8,
    freq_low: f64,
    freq_high: f64,
    band_mode: u8,
    min_freq: f64,
    max_freq: f64,
    canvas_width: f64,
    canvas_height: f64,
) {
    let harmonics_active = band_mode >= 4 && freq_low > 0.0 && freq_high / freq_low < 2.0;
    let harmonics_upper = freq_high * 2.0;

    // Determine the frequency range for the hovered band
    let (band_lo, band_hi, color) = match hovered_band {
        0 => (0.0, freq_low, "rgba(255, 80, 80, 0.15)"),       // below — red tint
        1 => (freq_low, freq_high, "rgba(80, 255, 120, 0.15)"), // selected — green
        2 if harmonics_active => (freq_high, harmonics_upper, "rgba(80, 120, 255, 0.15)"), // harmonics — blue
        3 => {
            let lo = if harmonics_active { harmonics_upper } else { freq_high };
            (lo, max_freq, "rgba(255, 180, 60, 0.15)")          // above — orange
        }
        _ => return,
    };

    let y_top = freq_to_y(band_hi.min(max_freq), min_freq, max_freq, canvas_height).max(0.0);
    let y_bot = freq_to_y(band_lo.max(min_freq), min_freq, max_freq, canvas_height).min(canvas_height);

    if y_bot <= y_top {
        return;
    }

    // Fill the band region
    ctx.set_fill_style_str(color);
    ctx.fill_rect(0.0, y_top, canvas_width, y_bot - y_top);

    // Edge lines
    let edge_color = match hovered_band {
        0 => "rgba(255, 80, 80, 0.5)",
        1 => "rgba(80, 255, 120, 0.5)",
        2 => "rgba(80, 120, 255, 0.5)",
        3 => "rgba(255, 180, 60, 0.5)",
        _ => return,
    };
    ctx.set_stroke_style_str(edge_color);
    ctx.set_line_width(1.0);
    for &y in &[y_top, y_bot] {
        ctx.begin_path();
        ctx.move_to(0.0, y);
        ctx.line_to(canvas_width, y);
        ctx.stroke();
    }
}

/// Convert pixel coordinates on the spectrogram canvas to (time, frequency).
pub fn pixel_to_time_freq(
    px_x: f64,
    px_y: f64,
    min_freq: f64,
    max_freq: f64,
    scroll_offset: f64,
    time_resolution: f64,
    zoom: f64,
    canvas_width: f64,
    canvas_height: f64,
) -> (f64, f64) {
    let visible_time = (canvas_width / zoom) * time_resolution;
    let time = scroll_offset + (px_x / canvas_width) * visible_time;
    let freq = y_to_freq(px_y, min_freq, max_freq, canvas_height);
    (time, freq)
}
