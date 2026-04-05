use ratatui::text::{Line, Span};

use crate::internal::ui::theme::theme_main::SapphireTheme;

pub fn badge(theme: &SapphireTheme, label: &str, status: &str) -> Line<'static> {
    let (dot, dot_style) = theme.state_dot(status);
    Line::from(vec![
        Span::styled(dot.to_owned(), dot_style),
        Span::raw(" "),
        Span::styled(label.to_owned(), theme.badge_style(status)),
    ])
}

pub fn key_value(theme: &SapphireTheme, label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), theme.key_value_label()),
        Span::styled(value.to_owned(), theme.key_value_value()),
    ])
}

pub fn truncate_line(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_owned()
    } else {
        value.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

pub fn section_header(theme: &SapphireTheme, title: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("## ", theme.surfaces.panel.rule),
        Span::styled(title.to_owned(), theme.surfaces.panel.title),
    ])
}

pub fn muted_line(theme: &SapphireTheme, text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(text.into(), theme.surfaces.panel.dimmed))
}

pub fn bullet_line(
    theme: &SapphireTheme,
    bullet: &str,
    label: impl Into<String>,
    value: impl Into<String>,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(bullet.to_owned(), theme.surfaces.panel.accent),
        Span::raw(" "),
        Span::styled(label.into(), theme.surfaces.panel.title),
        Span::raw(" "),
        Span::styled(value.into(), theme.surfaces.panel.body),
    ])
}

pub fn rule_line(theme: &SapphireTheme, width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width.max(8)),
        theme.surfaces.panel.rule,
    ))
}
