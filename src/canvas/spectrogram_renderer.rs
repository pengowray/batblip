use crate::canvas::colors::{freq_marker_color, freq_marker_label, magnitude_to_greyscale};
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

/// Blit the pre-rendered spectrogram to a visible canvas, handling scroll and zoom.
pub fn blit_viewport(
    ctx: &CanvasRenderingContext2d,
    pre_rendered: &PreRendered,
    canvas: &HtmlCanvasElement,
    scroll_col: f64,
    zoom: f64,
) {
    let cw = canvas.width() as f64;
    let ch = canvas.height() as f64;

    // Clear canvas
    ctx.set_fill_style_str("#000");
    ctx.fill_rect(0.0, 0.0, cw, ch);

    if pre_rendered.width == 0 || pre_rendered.height == 0 {
        return;
    }

    // How many source columns are visible at current zoom
    let visible_cols = (cw / zoom).min(pre_rendered.width as f64);
    let src_start = scroll_col.max(0.0).min((pre_rendered.width as f64 - visible_cols).max(0.0));

    // Create ImageData from pixel buffer and draw it
    // We'll draw the full pre-rendered image scaled to the canvas
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
                0.0,
                visible_cols,
                pre_rendered.height as f64,
                0.0,
                0.0,
                cw,
                ch,
            );
        }
        Err(e) => {
            log::error!("Failed to create ImageData: {e:?}");
        }
    }
}

/// Draw horizontal frequency marker lines with resistor color band colors.
/// When `het_freq` is Some, markers within the audible band show the shifted frequency
/// and markers outside the band are dimmed.
pub fn draw_freq_markers(
    ctx: &CanvasRenderingContext2d,
    max_freq: f64,
    canvas_height: f64,
    canvas_width: f64,
    het_freq: Option<f64>,
) {
    let cutoff = 15_000.0;
    let mut freq = 10_000.0;
    while freq < max_freq {
        let y = canvas_height * (1.0 - freq / max_freq);
        let color = freq_marker_color(freq);

        // Determine alpha based on whether marker is in audible band
        let (line_alpha, label_alpha) = if let Some(hf) = het_freq {
            if (freq - hf).abs() <= cutoff {
                (0.6, 0.9)
            } else {
                (0.2, 0.3)
            }
        } else {
            (0.6, 0.8)
        };

        ctx.set_stroke_style_str(&format!("rgba({},{},{},{line_alpha})", color[0], color[1], color[2]));
        ctx.set_line_width(1.0);
        ctx.begin_path();
        ctx.move_to(0.0, y);
        ctx.line_to(canvas_width, y);
        ctx.stroke();

        // Label
        ctx.set_fill_style_str(&format!("rgba({},{},{},{label_alpha})", color[0], color[1], color[2]));
        ctx.set_font("11px sans-serif");
        let base_label = freq_marker_label(freq);
        let label = if let Some(hf) = het_freq {
            let diff = (freq - hf).abs();
            if diff <= cutoff {
                let diff_khz = (diff / 1000.0).round() as u32;
                format!("{base_label} \u{2192} {diff_khz} kHz")
            } else {
                base_label
            }
        } else {
            base_label
        };
        let _ = ctx.fill_text(&label, 4.0, y - 3.0);

        freq += 10_000.0;
    }
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

    // Label at center line
    ctx.set_fill_style_str("rgba(0, 230, 255, 0.9)");
    ctx.set_font("bold 12px sans-serif");
    let label = format!("HET {:.0} kHz", het_freq / 1000.0);
    let _ = ctx.fill_text(&label, canvas_width - 120.0, y_center - 5.0);
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
