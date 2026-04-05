use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::internal::ui::shimmer::{prefix_glyph_span, shimmer_spans};
use crate::internal::ui::theme::theme_main::SapphireTheme;

use super::data::{DashboardSnapshot, WorkerView};
use super::state::{DashboardState, SidebarTab};
use super::widgets::{
    badge, bullet_line, key_value, muted_line, rule_line, section_header, truncate_line,
};

pub fn render(
    frame: &mut Frame<'_>,
    state: &DashboardState,
    snapshot: &DashboardSnapshot,
    rendered_markdown: &Text<'static>,
    theme: &SapphireTheme,
    shimmer_frame: usize,
    reduced_motion: bool,
) {
    let area = frame.area();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(2),
            Constraint::Min(12),
            Constraint::Length(3),
        ])
        .split(area);

    render_header(frame, layout[0], snapshot, theme, shimmer_frame, reduced_motion);
    render_section_bar(frame, layout[1], state, theme);
    render_body(frame, layout[2], state, snapshot, rendered_markdown, theme);
    render_footer(frame, layout[3], state, snapshot, theme);
}

fn render_header(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DashboardSnapshot,
    theme: &SapphireTheme,
    shimmer_frame: usize,
    reduced_motion: bool,
) {
    let mut brand = vec![
        prefix_glyph_span(shimmer_frame, reduced_motion, None),
        Span::raw(" "),
    ];
    brand.extend(shimmer_spans("Sapphire"));

    let mission_line = if snapshot.mission_status.eq_ignore_ascii_case("launching")
        && snapshot.worker_agent_count == 0
    {
        let mut line = vec![
            prefix_glyph_span(shimmer_frame, reduced_motion, None),
            Span::raw(" "),
        ];
        line.extend(shimmer_spans("Planning worker packets"));
        Line::from(line)
    } else {
        Line::from(Span::styled(
            truncate_line(&snapshot.mission_rewrite, area.width.saturating_sub(6) as usize),
            theme.surfaces.header.mission,
        ))
    };

    let meta_line = Line::from(vec![
        Span::styled("status ", theme.surfaces.panel.dimmed),
        Span::styled(snapshot.mission_status.clone(), theme.badge_style(&snapshot.mission_status)),
        Span::styled("  team ", theme.surfaces.panel.dimmed),
        Span::styled(
            snapshot.worker_agent_count.to_string(),
            theme.surfaces.header.meta_value,
        ),
        Span::styled("  elapsed ", theme.surfaces.panel.dimmed),
        Span::styled(snapshot.elapsed_label.clone(), theme.surfaces.header.timer),
    ]);

    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(theme.frame.subtle_rule);
    frame.render_widget(
        Paragraph::new(vec![Line::from(brand), mission_line, meta_line]).block(block),
        area,
    );
}

fn render_section_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &DashboardState,
    theme: &SapphireTheme,
) {
    let mut spans = Vec::new();
    for (index, section) in SidebarTab::ALL.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("  ", theme.surfaces.panel.dimmed));
        }
        let label = format!("[{}] {}", section.hotkey(), section.label());
        let style = if *section == state.active_section {
            theme.surfaces.panel.title
        } else {
            theme.surfaces.panel.dimmed
        };
        spans.push(Span::styled(label, style));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans))
            .block(Block::default().borders(Borders::BOTTOM).border_style(theme.frame.subtle_rule)),
        area,
    );
}

fn render_body(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &DashboardState,
    snapshot: &DashboardSnapshot,
    rendered_markdown: &Text<'static>,
    theme: &SapphireTheme,
) {
    let width = area.width.saturating_sub(2) as usize;
    let lines = build_body_lines(state, snapshot, rendered_markdown, theme, width);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((state.scroll, 0)),
        area,
    );
}

fn build_body_lines(
    state: &DashboardState,
    snapshot: &DashboardSnapshot,
    rendered_markdown: &Text<'static>,
    theme: &SapphireTheme,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    lines.push(section_header(theme, "Overview"));
    lines.push(rule_line(theme, width));
    lines.push(key_value(theme, "Mission", &snapshot.mission_rewrite));
    lines.push(key_value(
        theme,
        "Supervisor mode",
        &snapshot.watchdog.supervisor_mode,
    ));
    lines.push(key_value(
        theme,
        "Validation queue",
        &snapshot.health.validation_queue.to_string(),
    ));
    lines.push(key_value(
        theme,
        "Blocked",
        &snapshot.health.blocked.to_string(),
    ));
    lines.push(key_value(
        theme,
        "Contradictions",
        &snapshot.health.contradictions.to_string(),
    ));
    lines.push(muted_line(theme, format!("Watchdog {}", snapshot.health.watchdog_line)));
    lines.push(Line::default());

    lines.push(section_header(theme, "Team Snapshot"));
    lines.push(rule_line(theme, width));
    if snapshot.workers.is_empty() {
        lines.push(muted_line(theme, "No workers yet."));
    } else {
        for worker in &snapshot.workers {
            lines.extend(render_worker_summary(worker, theme));
        }
    }
    lines.push(Line::default());

    lines.push(section_header(theme, "Problems"));
    lines.push(rule_line(theme, width));
    lines.extend(render_problem_summary(snapshot, theme));
    lines.push(Line::default());

    lines.push(section_header(theme, "Watchdog Summary"));
    lines.push(rule_line(theme, width));
    lines.extend(render_watchdog_summary(snapshot, theme));
    lines.push(Line::default());

    lines.push(section_header(theme, "Supervisor Snapshot"));
    lines.push(rule_line(theme, width));
    lines.extend(render_supervisor_preview(snapshot, theme));
    lines.push(Line::default());

    let detail_title = format!("Detail · {}", state.active_section.label());
    lines.push(section_header(theme, &detail_title));
    lines.push(rule_line(theme, width));
    lines.extend(render_detail_section(
        state.active_section,
        snapshot,
        rendered_markdown,
        theme,
    ));

    lines
}

fn render_worker_summary(worker: &WorkerView, theme: &SapphireTheme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(badge(theme, &worker.name, &worker.state));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
                format!("{}  {}", worker.role, truncate_line(&worker.summary, 96)),
                theme.surfaces.panel.body,
            ),
        ]));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("focus {}", truncate_line(&worker.focus, 88)),
            theme.surfaces.panel.dimmed,
        ),
    ]));
    if let Some(validation) = &worker.validation {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("validation {}", validation),
                theme.surfaces.panel.dimmed,
            ),
        ]));
    }
    lines.push(Line::default());
    lines
}

fn render_problem_summary(snapshot: &DashboardSnapshot, theme: &SapphireTheme) -> Vec<Line<'static>> {
    let problems: Vec<_> = snapshot
        .workers
        .iter()
        .filter(|worker| {
            matches!(
                worker.state.as_str(),
                "stalled"
                    | "blocked"
                    | "contradictory"
                    | "failed"
                    | "needs_retry"
                    | "wrong_direction"
            ) || worker.validation.as_deref() == Some("awaiting validation")
        })
        .collect();

    if problems.is_empty() {
        return vec![muted_line(theme, "All clear. No workers currently need attention.")];
    }

    problems
        .into_iter()
        .flat_map(|worker| {
            vec![
                badge(theme, &worker.name, &worker.state),
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        truncate_line(&worker.summary, 108),
                        theme.surfaces.panel.body,
                    ),
                ]),
            ]
        })
        .collect()
}

fn render_watchdog_summary(snapshot: &DashboardSnapshot, theme: &SapphireTheme) -> Vec<Line<'static>> {
    let wd = &snapshot.watchdog;
    vec![
        bullet_line(theme, "•", "mode", wd.supervisor_mode.clone()),
        bullet_line(theme, "•", "directives", wd.directives_parsed.to_string()),
        bullet_line(theme, "•", "mail", wd.mail_routed.to_string()),
        bullet_line(
            theme,
            "•",
            "validation",
            wd.validation_challenges.to_string(),
        ),
        bullet_line(theme, "•", "stalls", wd.stall_interventions.to_string()),
        bullet_line(theme, "•", "conflicts", wd.lease_conflicts.to_string()),
        bullet_line(theme, "•", "reminders", wd.protocol_reminders.to_string()),
        bullet_line(
            theme,
            "•",
            "fallbacks",
            wd.supervisor_fallbacks.to_string(),
        ),
    ]
}

fn render_supervisor_preview(snapshot: &DashboardSnapshot, theme: &SapphireTheme) -> Vec<Line<'static>> {
    let Some(supervisor) = &snapshot.supervisor else {
        return vec![muted_line(theme, "Supervisor not attached yet.")];
    };

    let mut lines = vec![
        badge(theme, &supervisor.name, &supervisor.state),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                truncate_line(&supervisor.summary, 120),
                theme.surfaces.panel.body,
            ),
        ]),
    ];

    if supervisor.pending_decisions.is_empty() {
        lines.push(muted_line(theme, "No pending supervisor decisions."));
    } else {
        lines.push(muted_line(theme, "Pending decisions:"));
        for item in &supervisor.pending_decisions {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("• ", theme.surfaces.panel.accent),
                Span::styled(item.clone(), theme.surfaces.panel.body),
            ]));
        }
    }
    lines
}

fn render_detail_section(
    section: SidebarTab,
    snapshot: &DashboardSnapshot,
    rendered_markdown: &Text<'static>,
    theme: &SapphireTheme,
) -> Vec<Line<'static>> {
    match section {
        SidebarTab::Workers => render_workers_detail(snapshot, theme),
        SidebarTab::Problems => render_problems_detail(snapshot, theme),
        SidebarTab::Watchdog => render_watchdog_detail(snapshot, theme),
        SidebarTab::Events => render_events_detail(snapshot, theme),
        SidebarTab::Supervisor => render_supervisor_detail(snapshot, rendered_markdown, theme),
    }
}

fn render_workers_detail(snapshot: &DashboardSnapshot, theme: &SapphireTheme) -> Vec<Line<'static>> {
    if snapshot.workers.is_empty() {
        return vec![muted_line(theme, "No workers yet.")];
    }

    let mut lines = Vec::new();
    for worker in &snapshot.workers {
        lines.push(badge(theme, &worker.name, &worker.state));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{} · {}", worker.role, worker.summary),
                theme.surfaces.panel.body,
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("focus {}", worker.focus),
                theme.surfaces.panel.dimmed,
            ),
        ]));
        if let Some(validation) = &worker.validation {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("validation ", theme.surfaces.panel.dimmed),
                Span::styled(validation.clone(), theme.badge_style(validation)),
            ]));
        }
        lines.push(Line::default());
    }
    lines
}

fn render_problems_detail(snapshot: &DashboardSnapshot, theme: &SapphireTheme) -> Vec<Line<'static>> {
    let problems: Vec<_> = snapshot
        .workers
        .iter()
        .filter(|worker| {
            matches!(
                worker.state.as_str(),
                "stalled"
                    | "blocked"
                    | "contradictory"
                    | "failed"
                    | "needs_retry"
                    | "wrong_direction"
            ) || worker.validation.as_deref() == Some("awaiting validation")
        })
        .collect();

    if problems.is_empty() {
        return vec![muted_line(theme, "All clear. No workers currently need attention.")];
    }

    let mut lines = Vec::new();
    for worker in problems {
        lines.push(badge(theme, &worker.name, &worker.state));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(worker.summary.clone(), theme.surfaces.panel.body),
        ]));
        if let Some(validation) = &worker.validation {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("validation ", theme.surfaces.panel.dimmed),
                Span::styled(validation.clone(), theme.badge_style(validation)),
            ]));
        }
        lines.push(Line::default());
    }
    lines
}

fn render_watchdog_detail(snapshot: &DashboardSnapshot, theme: &SapphireTheme) -> Vec<Line<'static>> {
    let wd = &snapshot.watchdog;
    vec![
        key_value(theme, "Supervisor mode", &wd.supervisor_mode),
        key_value(theme, "Runtime events", &wd.runtime_events.to_string()),
        key_value(theme, "Directives parsed", &wd.directives_parsed.to_string()),
        key_value(theme, "Mail routed", &wd.mail_routed.to_string()),
        key_value(
            theme,
            "Validation challenges",
            &wd.validation_challenges.to_string(),
        ),
        key_value(
            theme,
            "Stall interventions",
            &wd.stall_interventions.to_string(),
        ),
        key_value(theme, "Lease conflicts", &wd.lease_conflicts.to_string()),
        key_value(
            theme,
            "Protocol reminders",
            &wd.protocol_reminders.to_string(),
        ),
        key_value(
            theme,
            "Supervisor health events",
            &wd.supervisor_health_events.to_string(),
        ),
        key_value(
            theme,
            "Supervisor fallbacks",
            &wd.supervisor_fallbacks.to_string(),
        ),
    ]
}

fn render_events_detail(_snapshot: &DashboardSnapshot, theme: &SapphireTheme) -> Vec<Line<'static>> {
    vec![muted_line(
        theme,
        "Live event logging is disabled in the dashboard to keep the control surface responsive.",
    )]
}

fn render_supervisor_detail(
    snapshot: &DashboardSnapshot,
    rendered_markdown: &Text<'static>,
    theme: &SapphireTheme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(supervisor) = &snapshot.supervisor {
        lines.push(badge(theme, &supervisor.name, &supervisor.state));
        lines.push(Line::default());
    }

    if let Some(final_summary) = &snapshot.final_summary {
        lines.push(section_header(theme, "Final Summary"));
        lines.push(Line::from(Span::styled(
            final_summary.clone(),
            theme.surfaces.panel.body,
        )));
        lines.push(Line::default());
    }

    if rendered_markdown.lines.is_empty() {
        lines.push(muted_line(theme, "Supervisor markdown is still empty."));
        return lines;
    }

    lines.extend(rendered_markdown.lines.clone());
    lines
}

fn render_footer(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &DashboardState,
    snapshot: &DashboardSnapshot,
    theme: &SapphireTheme,
) {
    let top = Line::from(vec![
        Span::styled("section ", theme.surfaces.panel.dimmed),
        Span::styled(
            state.active_section.label().to_owned(),
            theme.surfaces.panel.title,
        ),
        Span::styled("  elapsed ", theme.surfaces.panel.dimmed),
        Span::styled(snapshot.elapsed_label.clone(), theme.surfaces.header.timer),
        Span::styled("  workers ", theme.surfaces.panel.dimmed),
        Span::styled(
            snapshot.worker_agent_count.to_string(),
            theme.surfaces.header.meta_value,
        ),
        Span::styled("  state ", theme.surfaces.panel.dimmed),
        Span::styled(
            snapshot.mission_status.clone(),
            theme.badge_style(&snapshot.mission_status),
        ),
    ]);

    let bottom = if snapshot.done {
        "tab cycle sections  j/k scroll  pgup/pgdn jump  q quit"
    } else {
        "tab cycle sections  j/k scroll  pgup/pgdn jump  q waits until mission completes"
    };

    frame.render_widget(
        Paragraph::new(vec![top, Line::from(Span::styled(bottom, theme.footer_hint()))]).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(theme.frame.subtle_rule),
        ),
        area,
    );
}
