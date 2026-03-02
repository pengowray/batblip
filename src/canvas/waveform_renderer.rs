use web_sys::CanvasRenderingContext2d;

/// Common viewport calculation for waveform rendering.
struct WaveViewport {
    start_time: f64,
    px_per_sec: f64,
    samples_per_pixel: f64,
    mid_y: f64,
}

fn compute_viewport(
    samples: &[f32],
    sample_rate: u32,
    scroll_offset: f64,
    zoom: f64,
    time_resolution: f64,
    canvas_width: f64,
    canvas_height: f64,
) -> WaveViewport {
    let duration = samples.len() as f64 / sample_rate as f64;
    let mid_y = canvas_height / 2.0;
    let visible_time = (canvas_width / zoom) * time_resolution;
    let start_time = scroll_offset.max(0.0).min((duration - visible_time).max(0.0));
    let px_per_sec = canvas_width / visible_time;
    let samples_per_pixel = (visible_time * sample_rate as f64) / canvas_width;
    WaveViewport { start_time, px_per_sec, samples_per_pixel, mid_y }
}

/// Draw a single waveform layer with the given color.
fn draw_waveform_layer(
    ctx: &CanvasRenderingContext2d,
    samples: &[f32],
    sample_rate: u32,
    vp: &WaveViewport,
    canvas_width: f64,
    color: &str,
    gain_linear: f64,
) {
    ctx.set_stroke_style_str(color);
    ctx.set_line_width(1.0);

    if vp.samples_per_pixel <= 2.0 {
        ctx.begin_path();
        let mut first = true;
        for px in 0..(canvas_width as usize) {
            let t = vp.start_time + (px as f64 / vp.px_per_sec);
            let idx = (t * sample_rate as f64) as usize;
            if idx >= samples.len() {
                break;
            }
            let y = vp.mid_y - (samples[idx] as f64 * gain_linear * vp.mid_y * 0.9);
            if first {
                ctx.move_to(px as f64, y);
                first = false;
            } else {
                ctx.line_to(px as f64, y);
            }
        }
        ctx.stroke();
    } else {
        for px in 0..(canvas_width as usize) {
            let t0 = vp.start_time + (px as f64 / vp.px_per_sec);
            let t1 = vp.start_time + ((px as f64 + 1.0) / vp.px_per_sec);
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

            let y_min = vp.mid_y - (max_val as f64 * gain_linear * vp.mid_y * 0.9);
            let y_max = vp.mid_y - (min_val as f64 * gain_linear * vp.mid_y * 0.9);

            ctx.begin_path();
            ctx.move_to(px as f64, y_min);
            ctx.line_to(px as f64, y_max);
            ctx.stroke();
        }
    }
}

/// Draw selection highlight.
fn draw_selection(
    ctx: &CanvasRenderingContext2d,
    selection: Option<(f64, f64)>,
    vp: &WaveViewport,
    canvas_width: f64,
    canvas_height: f64,
) {
    if let Some((sel_start, sel_end)) = selection {
        let x0 = ((sel_start - vp.start_time) * vp.px_per_sec).max(0.0);
        let x1 = ((sel_end - vp.start_time) * vp.px_per_sec).min(canvas_width);
        if x1 > x0 {
            ctx.set_fill_style_str("rgba(50, 120, 200, 0.2)");
            ctx.fill_rect(x0, 0.0, x1 - x0, canvas_height);
        }
    }
}

/// Draw center line.
fn draw_center_line(ctx: &CanvasRenderingContext2d, mid_y: f64, canvas_width: f64) {
    ctx.set_stroke_style_str("#333");
    ctx.set_line_width(1.0);
    ctx.begin_path();
    ctx.move_to(0.0, mid_y);
    ctx.line_to(canvas_width, mid_y);
    ctx.stroke();
}

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
    gain_db: f64,
) {
    ctx.set_fill_style_str("#0a0a0a");
    ctx.fill_rect(0.0, 0.0, canvas_width, canvas_height);

    if samples.is_empty() {
        return;
    }

    let gain_linear = 10.0_f64.powf(gain_db / 20.0);
    let vp = compute_viewport(samples, sample_rate, scroll_offset, zoom, time_resolution, canvas_width, canvas_height);
    draw_selection(ctx, selection, &vp, canvas_width, canvas_height);
    draw_center_line(ctx, vp.mid_y, canvas_width);
    draw_waveform_layer(ctx, samples, sample_rate, &vp, canvas_width, "#6a6", gain_linear);
}

/// Draw dual waveform for HFR mode: original in dim color, bandpass-filtered overlay in bright cyan.
pub fn draw_waveform_hfr(
    ctx: &CanvasRenderingContext2d,
    samples: &[f32],
    filtered_samples: &[f32],
    sample_rate: u32,
    scroll_offset: f64,
    zoom: f64,
    time_resolution: f64,
    canvas_width: f64,
    canvas_height: f64,
    selection: Option<(f64, f64)>,
    gain_db: f64,
) {
    ctx.set_fill_style_str("#0a0a0a");
    ctx.fill_rect(0.0, 0.0, canvas_width, canvas_height);

    if samples.is_empty() {
        return;
    }

    let gain_linear = 10.0_f64.powf(gain_db / 20.0);
    let vp = compute_viewport(samples, sample_rate, scroll_offset, zoom, time_resolution, canvas_width, canvas_height);
    draw_selection(ctx, selection, &vp, canvas_width, canvas_height);
    draw_center_line(ctx, vp.mid_y, canvas_width);

    // Original waveform in dim color
    draw_waveform_layer(ctx, samples, sample_rate, &vp, canvas_width, "#444", gain_linear);

    // Filtered (HFR content) waveform overlay in bright cyan
    if !filtered_samples.is_empty() {
        draw_waveform_layer(ctx, filtered_samples, sample_rate, &vp, canvas_width, "#0cf", gain_linear);
    }
}

/// Draw a zero-crossing rate graph from pre-computed bins.
/// `bins` is a slice of (rate_hz, is_armed) with fixed `bin_duration` spacing.
pub fn draw_zc_rate(
    ctx: &CanvasRenderingContext2d,
    bins: &[(f64, bool)],
    bin_duration: f64,
    total_duration: f64,
    scroll_offset: f64,
    zoom: f64,
    time_resolution: f64,
    canvas_width: f64,
    canvas_height: f64,
    selection: Option<(f64, f64)>,
    max_freq_khz: f64,
) {
    ctx.set_fill_style_str("#0a0a0a");
    ctx.fill_rect(0.0, 0.0, canvas_width, canvas_height);

    if bins.is_empty() {
        return;
    }

    let visible_time = (canvas_width / zoom) * time_resolution;
    let start_time = scroll_offset.max(0.0).min((total_duration - visible_time).max(0.0));
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

    let max_freq_hz = max_freq_khz * 1000.0;

    // Horizontal grid lines
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

    // Only iterate visible bins
    let first_bin = ((start_time / bin_duration) as usize).saturating_sub(1);
    let end_time = start_time + visible_time;
    let last_bin = ((end_time / bin_duration) as usize + 2).min(bins.len());

    for bin_idx in first_bin..last_bin {
        let (rate_hz, armed) = bins[bin_idx];
        if rate_hz <= 0.0 {
            continue;
        }

        let bin_time = bin_idx as f64 * bin_duration;
        let x = (bin_time - start_time) * px_per_sec;
        let bar_w = (bin_duration * px_per_sec).max(1.0);

        let bar_h = (rate_hz / max_freq_hz * canvas_height).min(canvas_height);
        let y = canvas_height - bar_h;

        if armed {
            ctx.set_fill_style_str("rgba(100, 200, 100, 0.8)");
        } else {
            ctx.set_fill_style_str("rgba(60, 130, 60, 0.35)");
        }
        ctx.fill_rect(x, y, bar_w, bar_h);
    }
}
