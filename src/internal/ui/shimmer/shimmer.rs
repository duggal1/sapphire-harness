use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Span;

use crate::color::blend;
use crate::terminal_palette::default_bg;
use crate::terminal_palette::default_fg;

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

fn elapsed_since_start() -> Duration {
    let start = PROCESS_START.get_or_init(Instant::now);
    start.elapsed()
}

pub fn shimmer_spans(text: &str) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    // Use time-based sweep synchronized to process start.
    let padding = 10usize;
    let period = chars.len() + padding * 2;
    let sweep_seconds = 2.0f32;
    let pos_f =
        (elapsed_since_start().as_secs_f32() % sweep_seconds) / sweep_seconds * (period as f32);
    let pos = pos_f as usize;
    let has_true_color = supports_color::on_cached(supports_color::Stream::Stdout)
        .map(|level| level.has_16m)
        .unwrap_or(false);
    let band_half_width = 5.0;

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(chars.len());
    let base_color = default_fg().unwrap_or((128, 128, 128));
    let highlight_color = default_bg().unwrap_or((255, 255, 255));
    for (i, ch) in chars.iter().enumerate() {
        let i_pos = i as isize + padding as isize;
        let pos = pos as isize;
        let dist = (i_pos - pos).abs() as f32;

        let t = if dist <= band_half_width {
            let x = std::f32::consts::PI * (dist / band_half_width);
            0.5 * (1.0 + x.cos())
        } else {
            0.0
        };
        let style = if has_true_color {
            let highlight = t.clamp(0.0, 1.0);
            let (r, g, b) = blend(highlight_color, base_color, highlight * 0.9);
            // Allow custom RGB colors, as the implementation is thoughtfully
            // adjusting the level of the default foreground color.
            #[allow(clippy::disallowed_methods)]
            {
                Style::default()
                    .fg(Color::Rgb(r, g, b))
                    .add_modifier(Modifier::BOLD)
            }
        } else {
            color_for_level(t)
        };
        spans.push(Span::styled(ch.to_string(), style));
    }
    spans
}

fn color_for_level(intensity: f32) -> Style {
    // Tune fallback styling so the shimmer band reads even without RGB support.
    if intensity < 0.2 {
        Style::default().add_modifier(Modifier::DIM)
    } else if intensity < 0.6 {
        Style::default()
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    }
}

// ─── Prefix glyph (terminal loader spinner) ─────────────────────────────────

const DOT_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const REDUCED_MOTION_DOT: char = '●';
const REDUCED_MOTION_CYCLE_MS: u128 = 2000;

pub fn prefix_glyph_span(
    frame: usize,
    reduced_motion: bool,
    time: Option<Duration>,
) -> Span<'static> {
    let now = time.unwrap_or_else(elapsed_since_start);
    let has_true_color = supports_color::on_cached(supports_color::Stream::Stdout)
        .map(|level| level.has_16m)
        .unwrap_or(false);

    if reduced_motion {
        let is_dim = ((now.as_millis() / (REDUCED_MOTION_CYCLE_MS / 2)) % 2) == 1;
        let base_color = default_fg().unwrap_or((210, 210, 220));
        let highlight_color = default_bg().unwrap_or((180, 140, 255));

        let style = if has_true_color {
            let rgb = if is_dim { base_color } else { highlight_color };
            let mut style = Style::default().fg(Color::Rgb(rgb.0, rgb.1, rgb.2));
            if !is_dim {
                style = style.add_modifier(Modifier::BOLD);
            }
            style
        } else if is_dim {
            Style::default().add_modifier(Modifier::DIM)
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        };

        return Span::styled(REDUCED_MOTION_DOT.to_string(), style);
    }

    let glyph = DOT_FRAMES[frame % DOT_FRAMES.len()];
    let pulse = ((now.as_secs_f32() * 2.2).sin() + 1.0) * 0.5;
    let base_color = default_fg().unwrap_or((210, 210, 220));
    let highlight_color = default_bg().unwrap_or((180, 140, 255));

    let style = if has_true_color {
        let (r, g, b) = blend(base_color, highlight_color, pulse);
        #[allow(clippy::disallowed_methods)]
        {
            Style::default()
                .fg(Color::Rgb(r, g, b))
                .add_modifier(Modifier::BOLD)
        }
    } else if pulse > 0.6 {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    Span::styled(glyph.to_string(), style)
}
