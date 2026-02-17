use crate::canvas::colors::{freq_marker_color, freq_marker_label, magnitude_to_greyscale, movement_rgb};
use crate::state::Selection;
use crate::types::SpectrogramData;
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

/// Blit the pre-rendered spectrogram to a visible canvas, handling scroll, zoom, and freq crop.
/// `freq_crop` is 0.0–1.0: fraction of the vertical image to show (from bottom / low freq).
/// 1.0 = show all, 0.5 = show bottom half only.
pub fn blit_viewport(
    ctx: &CanvasRenderingContext2d,
    pre_rendered: &PreRendered,
    canvas: &HtmlCanvasElement,
    scroll_col: f64,
    zoom: f64,
    freq_crop: f64,
) {
    let cw = canvas.width() as f64;
    let ch = canvas.height() as f64;

    // Clear canvas
    ctx.set_fill_style_str("#000");
    ctx.fill_rect(0.0, 0.0, cw, ch);

    if pre_rendered.width == 0 || pre_rendered.height == 0 {
        return;
    }

    let fc = freq_crop.max(0.01);

    // How many source columns are visible at current zoom
    let visible_cols = (cw / zoom).min(pre_rendered.width as f64);
    let src_start = scroll_col.max(0.0).min((pre_rendered.width as f64 - visible_cols).max(0.0));

    // Vertical crop: row 0 = highest freq, last row = 0 Hz
    let full_h = pre_rendered.height as f64;
    let (src_y, src_h, dst_y, dst_h) = if fc <= 1.0 {
        // Display range <= data range: crop to bottom `fc` fraction of source
        let sy = full_h * (1.0 - fc);
        (sy, full_h * fc, 0.0, ch)
    } else {
        // Display range > data range (above Nyquist): draw all data into
        // the bottom portion of the canvas, leaving the top black
        let frac = 1.0 / fc; // fraction of canvas the data occupies
        (0.0, full_h, ch * (1.0 - frac), ch * frac)
    };

    // Create ImageData from pixel buffer and draw it
    let clamped = Clamped(&pre_rendered.pixels[..]);
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

            // Draw the visible portion scaled to fill the canvas
            let _ = ctx.draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                &tmp,
                src_start,
                src_y,
                visible_cols,
                src_h,
                0.0,
                dst_y,
                cw,
                dst_h,
            );
        }
        Err(e) => {
            log::error!("Failed to create ImageData: {e:?}");
        }
    }
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
}

/// Frequency marker hover/interaction state passed to drawing functions.
pub struct FreqMarkerState {
    pub mouse_freq: Option<f64>,
    pub mouse_in_label_area: bool,
    pub label_hover_opacity: f64,
    pub has_selection: bool,
    pub file_max_freq: f64,
}

/// Draw horizontal frequency marker lines with subtle, interactive UI.
/// Labels are white; colored range bars indicate the resistor-band color.
pub fn draw_freq_markers(
    ctx: &CanvasRenderingContext2d,
    max_freq: f64,
    canvas_height: f64,
    canvas_width: f64,
    shift_mode: FreqShiftMode,
    ms: &FreqMarkerState,
) {
    let cutoff = 15_000.0;
    let color_bar_w = 6.0;
    let color_bar_x = 0.0; // flush left
    let label_x = color_bar_w + 3.0; // text starts after color bar
    let tick_len = 22.0; // short tick under label (~half old label_area_w)
    let right_tick_len = 15.0;

    // Collect all division freqs to check for nyquist overlap
    let mut divisions: Vec<f64> = Vec::new();
    let mut freq = 10_000.0;
    while freq < max_freq {
        divisions.push(freq);
        freq += 10_000.0;
    }

    // Check if top of display is nyquist
    let is_nyquist_top = (max_freq - ms.file_max_freq).abs() < 1.0;
    // Find topmost division for nyquist overlap check
    let topmost_div = divisions.last().copied().unwrap_or(0.0);
    let topmost_div_y_frac = if max_freq > 0.0 { topmost_div / max_freq } else { 0.0 };
    let hide_topmost_for_nyquist = is_nyquist_top && topmost_div_y_frac > 0.95;

    for &freq in &divisions {
        let y = canvas_height * (1.0 - freq / max_freq);

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
        if ms.has_selection || mouse_in_range {
            let bar_alpha = if ms.has_selection { 0.6 } else { 0.8 };
            let bar_y_top = canvas_height * (1.0 - bar_top_freq / max_freq);
            let bar_y_bot = canvas_height * (1.0 - freq / max_freq);
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
                let diff = (freq - hf).abs();
                if diff <= cutoff {
                    let diff_khz = (diff / 1000.0).round() as u32;
                    if ms.label_hover_opacity > 0.01 {
                        format!("{base_label} kHz \u{2192} {diff_khz} kHz")
                    } else {
                        format!("{base_label} \u{2192} {diff_khz}")
                    }
                } else if ms.label_hover_opacity > 0.01 {
                    format!("{base_label} kHz")
                } else {
                    base_label.clone()
                }
            }
            FreqShiftMode::Divide(factor) if factor > 1.0 => {
                let shifted = freq / factor;
                let shifted_khz = shifted / 1000.0;
                let suffix = if ms.label_hover_opacity > 0.01 { " kHz" } else { "" };
                if shifted_khz >= 1.0 {
                    format!("{base_label}{suffix} \u{2192} {:.0} kHz", shifted_khz)
                } else {
                    format!("{base_label}{suffix} \u{2192} {:.0} Hz", shifted)
                }
            }
            _ => {
                if ms.label_hover_opacity > 0.01 {
                    format!("{base_label} kHz")
                } else {
                    base_label.clone()
                }
            }
        };

        // kHz fade: use opacity^2 for faster fade
        let khz_fade = ms.label_hover_opacity * ms.label_hover_opacity;
        if matches!(shift_mode, FreqShiftMode::None) && khz_fade > 0.01 && khz_fade < 0.99 {
            // Draw number at full alpha
            ctx.set_fill_style_str(&format!("rgba(255,255,255,{:.2})", label_alpha));
            let _ = ctx.fill_text(&base_label, label_x, y - 2.0);
            // Draw " kHz" suffix at faded alpha
            let metrics = ctx.measure_text(&base_label).unwrap();
            let num_w = metrics.width();
            let khz_alpha = label_alpha * khz_fade;
            ctx.set_fill_style_str(&format!("rgba(255,255,255,{:.2})", khz_alpha));
            let _ = ctx.fill_text(" kHz", label_x + num_w, y - 2.0);
        } else {
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

        // --- Full-width line (fades in when hovering label area, colored) ---
        if ms.label_hover_opacity > 0.001 {
            let full_alpha = ms.label_hover_opacity * 0.7 * base_alpha;
            ctx.set_stroke_style_str(&format!("rgba({},{},{},{:.3})", color[0], color[1], color[2], full_alpha));
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

    // --- Cursor frequency indicator ---
    if let Some(mf) = ms.mouse_freq {
        if !ms.mouse_in_label_area && mf > 0.0 && mf < max_freq {
            let y = canvas_height * (1.0 - mf / max_freq);

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

/// Draw the heterodyne frequency overlay: center line, audible band, and dimmed regions.
pub fn draw_het_overlay(
    ctx: &CanvasRenderingContext2d,
    het_freq: f64,
    max_freq: f64,
    canvas_height: f64,
    canvas_width: f64,
) {
    let cutoff = 15_000.0;
    let band_low = (het_freq - cutoff).max(0.0);
    let band_high = (het_freq + cutoff).min(max_freq);

    let y_center = canvas_height * (1.0 - het_freq / max_freq);
    let y_band_top = canvas_height * (1.0 - band_high / max_freq);
    let y_band_bottom = canvas_height * (1.0 - band_low / max_freq);

    // Dim regions outside the audible band
    ctx.set_fill_style_str("rgba(0, 0, 0, 0.5)");
    if y_band_top > 0.0 {
        ctx.fill_rect(0.0, 0.0, canvas_width, y_band_top);
    }
    if y_band_bottom < canvas_height {
        ctx.fill_rect(0.0, y_band_bottom, canvas_width, canvas_height - y_band_bottom);
    }

    // Audible band highlight
    ctx.set_fill_style_str("rgba(0, 200, 255, 0.07)");
    ctx.fill_rect(0.0, y_band_top, canvas_width, y_band_bottom - y_band_top);

    // Band edge lines (subtle)
    ctx.set_stroke_style_str("rgba(0, 200, 255, 0.3)");
    ctx.set_line_width(1.0);
    for &y in &[y_band_top, y_band_bottom] {
        ctx.begin_path();
        ctx.move_to(0.0, y);
        ctx.line_to(canvas_width, y);
        ctx.stroke();
    }

    // Center line at het_freq (dashed)
    ctx.set_stroke_style_str("rgba(0, 230, 255, 0.8)");
    ctx.set_line_width(1.5);
    let _ = ctx.set_line_dash(&js_sys::Array::of2(
        &wasm_bindgen::JsValue::from_f64(6.0),
        &wasm_bindgen::JsValue::from_f64(4.0),
    ));
    ctx.begin_path();
    ctx.move_to(0.0, y_center);
    ctx.line_to(canvas_width, y_center);
    ctx.stroke();
    let _ = ctx.set_line_dash(&js_sys::Array::new()); // reset dash

    // Label at center line (positioned on left, offset from freq division labels)
    ctx.set_fill_style_str("rgba(0, 230, 255, 0.9)");
    ctx.set_font("bold 12px sans-serif");
    let label = format!("HET {:.0} kHz", het_freq / 1000.0);
    let _ = ctx.fill_text(&label, 55.0, y_center - 5.0);
}

/// Draw selection rectangle overlay on spectrogram.
pub fn draw_selection(
    ctx: &CanvasRenderingContext2d,
    selection: &Selection,
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
    let y0 = (canvas_height * (1.0 - selection.freq_high / max_freq)).max(0.0);
    let y1 = (canvas_height * (1.0 - selection.freq_low / max_freq)).min(canvas_height);

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
        let y0 = (canvas_height * (1.0 - hi_high.min(max_freq) / max_freq)).max(0.0);
        let y1 = (canvas_height * (1.0 - hi_low / max_freq)).min(canvas_height);
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
        let y0 = (canvas_height * (1.0 - lo_high / max_freq)).max(0.0);
        let y1 = (canvas_height * (1.0 - lo_low.max(0.0) / max_freq)).min(canvas_height);
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

/// Convert pixel coordinates on the spectrogram canvas to (time, frequency).
pub fn pixel_to_time_freq(
    px_x: f64,
    px_y: f64,
    max_freq: f64,
    scroll_offset: f64,
    time_resolution: f64,
    zoom: f64,
    canvas_width: f64,
    canvas_height: f64,
) -> (f64, f64) {
    let visible_time = (canvas_width / zoom) * time_resolution;
    let time = scroll_offset + (px_x / canvas_width) * visible_time;
    let freq = max_freq * (1.0 - px_y / canvas_height);
    (time, freq)
}
