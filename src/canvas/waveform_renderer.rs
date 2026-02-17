use web_sys::CanvasRenderingContext2d;
use crate::dsp::zc_divide::zc_rate_per_bin;

/// Draw waveform on a canvas context.
/// Uses min/max envelope at low zoom, individual samples at high zoom.
pub fn draw_waveform(
    ctx: &CanvasRenderingContext2d,
    samples: &[f32],
    sample_rate: u32,
    scroll_offset: f64,
    zoom: f64,
    time_resolution: f64,
    canvas_width: f64,
    canvas_height: f64,
    selection: Option<(f64, f64)>,
) {
    // Clear
    ctx.set_fill_style_str("#0a0a0a");
    ctx.fill_rect(0.0, 0.0, canvas_width, canvas_height);

    if samples.is_empty() {
        return;
    }

    let duration = samples.len() as f64 / sample_rate as f64;
    let mid_y = canvas_height / 2.0;

    // Pixels per second — match spectrogram's viewport calculation
    // In the spectrogram, visible_cols = canvas_width / zoom (in spectrogram columns)
    // Each column = time_resolution seconds
    // So visible_time = (canvas_width / zoom) * time_resolution
    let visible_time = (canvas_width / zoom) * time_resolution;
    let start_time = scroll_offset.max(0.0).min((duration - visible_time).max(0.0));
    let px_per_sec = canvas_width / visible_time;

    // Draw selection highlight
    if let Some((sel_start, sel_end)) = selection {
        let x0 = ((sel_start - start_time) * px_per_sec).max(0.0);
        let x1 = ((sel_end - start_time) * px_per_sec).min(canvas_width);
        if x1 > x0 {
            ctx.set_fill_style_str("rgba(50, 120, 200, 0.2)");
            ctx.fill_rect(x0, 0.0, x1 - x0, canvas_height);
        }
    }

    // Draw center line
    ctx.set_stroke_style_str("#333");
    ctx.set_line_width(1.0);
    ctx.begin_path();
    ctx.move_to(0.0, mid_y);
    ctx.line_to(canvas_width, mid_y);
    ctx.stroke();

    // Draw waveform
    ctx.set_stroke_style_str("#6a6");
    ctx.set_line_width(1.0);

    let samples_per_pixel = (visible_time * sample_rate as f64) / canvas_width;

    if samples_per_pixel <= 2.0 {
        // High zoom: draw individual samples as connected lines
        ctx.begin_path();
        let mut first = true;
        for px in 0..(canvas_width as usize) {
            let t = start_time + (px as f64 / px_per_sec);
            let idx = (t * sample_rate as f64) as usize;
            if idx >= samples.len() {
                break;
            }
            let y = mid_y - (samples[idx] as f64 * mid_y * 0.9);
            if first {
                ctx.move_to(px as f64, y);
                first = false;
            } else {
                ctx.line_to(px as f64, y);
            }
        }
        ctx.stroke();
    } else {
        // Low zoom: draw min/max envelope per pixel column
        for px in 0..(canvas_width as usize) {
            let t0 = start_time + (px as f64 / px_per_sec);
            let t1 = start_time + ((px as f64 + 1.0) / px_per_sec);
            let i0 = ((t0 * sample_rate as f64) as usize).min(samples.len());
            let i1 = ((t1 * sample_rate as f64) as usize).min(samples.len());

            if i0 >= i1 || i0 >= samples.len() {
                break;
            }

            let mut min_val = f32::MAX;
            let mut max_val = f32::MIN;
            for &s in &samples[i0..i1] {
                if s < min_val { min_val = s; }
                if s > max_val { max_val = s; }
            }

            let y_min = mid_y - (max_val as f64 * mid_y * 0.9);
            let y_max = mid_y - (min_val as f64 * mid_y * 0.9);

            ctx.begin_path();
            ctx.move_to(px as f64, y_min);
            ctx.line_to(px as f64, y_max);
            ctx.stroke();
        }
    }
}

/// Draw a zero-crossing rate graph instead of the waveform.
/// Shows ZC frequency (kHz) per time bin as vertical bars, with armed bins
/// highlighted. The Y axis spans 0 to `max_freq_khz`.
pub fn draw_zc_rate(
    ctx: &CanvasRenderingContext2d,
    samples: &[f32],
    sample_rate: u32,
    scroll_offset: f64,
    zoom: f64,
    time_resolution: f64,
    canvas_width: f64,
    canvas_height: f64,
    selection: Option<(f64, f64)>,
    max_freq_khz: f64,
) {
    // Clear
    ctx.set_fill_style_str("#0a0a0a");
    ctx.fill_rect(0.0, 0.0, canvas_width, canvas_height);

    if samples.is_empty() {
        return;
    }

    let duration = samples.len() as f64 / sample_rate as f64;
    let visible_time = (canvas_width / zoom) * time_resolution;
    let start_time = scroll_offset.max(0.0).min((duration - visible_time).max(0.0));
    let px_per_sec = canvas_width / visible_time;

    // Selection highlight
    if let Some((sel_start, sel_end)) = selection {
        let x0 = ((sel_start - start_time) * px_per_sec).max(0.0);
        let x1 = ((sel_end - start_time) * px_per_sec).min(canvas_width);
        if x1 > x0 {
            ctx.set_fill_style_str("rgba(50, 120, 200, 0.2)");
            ctx.fill_rect(x0, 0.0, x1 - x0, canvas_height);
        }
    }

    // Compute ZC rate bins — use 1ms bins for good resolution
    let bin_duration = 0.001;
    let bins = zc_rate_per_bin(samples, sample_rate, bin_duration);
    if bins.is_empty() {
        return;
    }

    let max_freq_hz = max_freq_khz * 1000.0;

    // Draw horizontal grid lines at key frequencies
    ctx.set_stroke_style_str("#222");
    ctx.set_line_width(1.0);
    let grid_freqs = [20.0, 40.0, 60.0, 80.0, 100.0, 120.0];
    ctx.set_fill_style_str("#555");
    ctx.set_font("10px monospace");
    for &freq_khz in &grid_freqs {
        if freq_khz >= max_freq_khz {
            break;
        }
        let y = canvas_height * (1.0 - freq_khz / max_freq_khz);
        ctx.begin_path();
        ctx.move_to(0.0, y);
        ctx.line_to(canvas_width, y);
        ctx.stroke();
        let _ = ctx.fill_text(&format!("{:.0}k", freq_khz), 2.0, y - 2.0);
    }

    // Draw ZC rate bars
    for (bin_idx, &(rate_hz, armed)) in bins.iter().enumerate() {
        let bin_time = bin_idx as f64 * bin_duration;
        let x = (bin_time - start_time) * px_per_sec;
        let bar_w = (bin_duration * px_per_sec).max(1.0);

        if x + bar_w < 0.0 || x > canvas_width {
            continue;
        }

        if rate_hz <= 0.0 {
            continue;
        }

        let bar_h = (rate_hz / max_freq_hz * canvas_height).min(canvas_height);
        let y = canvas_height - bar_h;

        if armed {
            ctx.set_fill_style_str("rgba(100, 200, 100, 0.8)");
        } else {
            ctx.set_fill_style_str("rgba(80, 80, 80, 0.4)");
        }
        ctx.fill_rect(x, y, bar_w, bar_h);
    }
}
