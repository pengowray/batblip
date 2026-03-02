use web_sys::CanvasRenderingContext2d;

// ── Time scale ────────────────────────────────────────────────────────────

/// Nice 1-2-5 progression of tick intervals in seconds, from 0.1 ms to 10 min.
const TICK_INTERVALS: &[f64] = &[
    0.0001, 0.0002, 0.0005,         // sub-ms
    0.001, 0.002, 0.005,             // 1–5 ms
    0.01, 0.02, 0.05,               // 10–50 ms
    0.1, 0.2, 0.5,                  // 100–500 ms
    1.0, 2.0, 5.0,                  // 1–5 s
    10.0, 30.0, 60.0,               // 10 s – 1 min
    120.0, 300.0, 600.0,            // 2–10 min
];

/// Format a time value as a compact label whose precision matches the tick interval.
fn format_time_label(seconds: f64, interval: f64) -> String {
    if interval < 0.001 {
        // Sub-millisecond: "X.Xms"
        format!("{:.1}ms", seconds * 1000.0)
    } else if interval < 1.0 {
        let ms = seconds * 1000.0;
        if interval >= 0.01 {
            format!("{:.0}ms", ms)
        } else {
            format!("{:.1}ms", ms)
        }
    } else if interval < 60.0 {
        if interval >= 1.0 && (seconds - seconds.round()).abs() < 0.001 {
            format!("{:.0}s", seconds)
        } else {
            format!("{:.1}s", seconds)
        }
    } else {
        let mins = (seconds / 60.0).floor() as u32;
        let secs = (seconds % 60.0).round() as u32;
        if secs == 0 {
            format!("{}m", mins)
        } else {
            format!("{}m{:02}s", mins, secs)
        }
    }
}

/// Draw time tick marks and labels along the bottom of a canvas.
pub fn draw_time_markers(
    ctx: &CanvasRenderingContext2d,
    scroll_offset: f64,
    visible_time: f64,
    canvas_width: f64,
    canvas_height: f64,
    duration: f64,
) {
    if visible_time <= 0.0 || canvas_width <= 0.0 {
        return;
    }

    let px_per_sec = canvas_width / visible_time;

    // Pick the smallest nice interval that keeps labels ≥100 px apart
    let min_interval = 100.0 / px_per_sec;
    let interval = TICK_INTERVALS
        .iter()
        .copied()
        .find(|&i| i >= min_interval)
        .unwrap_or(*TICK_INTERVALS.last().unwrap());

    let end_time = (scroll_offset + visible_time).min(duration);

    // ── Minor ticks (no labels) ──
    let minor_interval = interval / 5.0;
    let minor_px = minor_interval * px_per_sec;
    if minor_px >= 4.0 {
        let first_minor = (scroll_offset / minor_interval).ceil() * minor_interval;
        ctx.set_stroke_style_str("rgba(255,255,255,0.15)");
        ctx.set_line_width(1.0);
        let mut t = first_minor;
        while t <= end_time + minor_interval * 0.5 {
            // Skip major-tick positions
            if ((t / interval).round() * interval - t).abs() < minor_interval * 0.01 {
                t += minor_interval;
                continue;
            }
            let x = (t - scroll_offset) * px_per_sec;
            if x >= 0.0 && x <= canvas_width {
                ctx.begin_path();
                ctx.move_to(x, canvas_height - 6.0);
                ctx.line_to(x, canvas_height);
                ctx.stroke();
            }
            t += minor_interval;
        }
    }

    // ── Major ticks + labels ──
    let tick_h = 12.0;
    ctx.set_font("10px sans-serif");
    ctx.set_text_baseline("bottom");

    let first_tick = (scroll_offset / interval).ceil() * interval;
    let mut t = first_tick;
    while t <= end_time + interval * 0.01 {
        let x = (t - scroll_offset) * px_per_sec;
        if x >= 0.0 && x <= canvas_width {
            // Bottom tick
            ctx.set_stroke_style_str("rgba(255,255,255,0.35)");
            ctx.set_line_width(1.0);
            ctx.begin_path();
            ctx.move_to(x, canvas_height - tick_h);
            ctx.line_to(x, canvas_height);
            ctx.stroke();

            // Subtle top tick
            ctx.set_stroke_style_str("rgba(255,255,255,0.10)");
            ctx.begin_path();
            ctx.move_to(x, 0.0);
            ctx.line_to(x, 4.0);
            ctx.stroke();

            // Label (to the right of the tick)
            let label = format_time_label(t, interval);
            if let Ok(metrics) = ctx.measure_text(&label) {
                let tw = metrics.width();
                let lx = x + 3.0;
                if lx + tw < canvas_width - 2.0 {
                    // Dark background for readability
                    ctx.set_fill_style_str("rgba(0,0,0,0.6)");
                    ctx.fill_rect(lx - 1.0, canvas_height - tick_h - 12.0, tw + 2.0, 12.0);
                    // White text
                    ctx.set_fill_style_str("rgba(255,255,255,0.7)");
                    let _ = ctx.fill_text(&label, lx, canvas_height - tick_h - 1.0);
                }
            }
        }
        t += interval;
    }

    ctx.set_text_baseline("alphabetic"); // reset
}
