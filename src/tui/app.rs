//! TUI application shell and dashboard.
//! Functions are prepared incrementally as the TUI surface is wired up.
#![allow(dead_code)]

use std::io::{self, Stdout};
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::text::Text;
use ratatui::Terminal;
use tokio::task::JoinHandle;

use crate::model::LaunchSummary;
use crate::store::Store;
use crate::tmux::Tmux;

use super::data::{AttachTarget, DashboardDataSource};
use super::markdown::MarkdownRenderer;
use super::render;
use super::state::{DashboardState, SidebarTab};
use crate::internal::ui::theme::fallback::markdown::theme::MarkdownTheme;
use crate::internal::ui::theme::theme_main::SapphireTheme;

pub struct TuiSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

struct RenderCache {
    markdown_source: String,
    markdown_width: u16,
    rendered_markdown: Text<'static>,
}

impl RenderCache {
    fn new() -> Self {
        Self {
            markdown_source: String::new(),
            markdown_width: 0,
            rendered_markdown: Text::default(),
        }
    }

    fn markdown<'a>(
        &'a mut self,
        snapshot: &super::data::DashboardSnapshot,
        area_width: u16,
        markdown_renderer: &MarkdownRenderer,
    ) -> &'a Text<'static> {
        let inner_width = area_width.saturating_sub(4);
        let width = inner_width.max(8);
        if self.markdown_width != width || self.markdown_source != snapshot.supervisor_markdown {
            self.rendered_markdown =
                markdown_renderer.render(&snapshot.supervisor_markdown, width);
            self.markdown_source = snapshot.supervisor_markdown.clone();
            self.markdown_width = width;
        }
        &self.rendered_markdown
    }
}

const DATA_REFRESH_INTERVAL: Duration = Duration::from_millis(1000);
const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const TMUX_SESSION_POLL_INTERVAL: Duration = Duration::from_millis(750);

impl TuiSession {
    pub fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw mode")?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("failed to initialize terminal backend")?;
        Ok(Self { terminal })
    }
}

impl Drop for TuiSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

pub async fn run_launch_dashboard(
    db_path: PathBuf,
    control_status_path: PathBuf,
    attach_target: AttachTarget,
    task: JoinHandle<Result<LaunchSummary>>,
) -> Result<LaunchSummary> {
    let mut tui = TuiSession::enter()?;
    let theme = SapphireTheme::default();
    let markdown_renderer = MarkdownRenderer::new(MarkdownTheme::from_app_theme(&theme));
    let data_source = DashboardDataSource::open(&db_path, control_status_path)?;
    let mut state = DashboardState::default();
    let mut shimmer_frame = 0usize;
    let mut task = Some(task);
    let mut finished_summary = None;
    let mut snapshot = data_source.snapshot(&attach_target)?;
    let mut render_cache = RenderCache::new();
    let mut last_refresh = Instant::now();
    let mut needs_redraw = true;

    loop {
        if last_refresh.elapsed() >= DATA_REFRESH_INTERVAL {
            snapshot = data_source.snapshot(&attach_target)?;
            last_refresh = Instant::now();
            needs_redraw = true;
        }
        if needs_redraw {
            let rendered_markdown = render_cache
                .markdown(&snapshot, tui.terminal.size()?.width, &markdown_renderer)
                .clone();
            tui.terminal.draw(|frame| {
                render::render(
                    frame,
                    &state,
                    &snapshot,
                    &rendered_markdown,
                    &theme,
                    shimmer_frame,
                    false,
                )
            })?;
            needs_redraw = false;
        }

        if let Some(handle) = task.as_ref() {
            if handle.is_finished() {
                let summary = task
                    .take()
                    .expect("join handle still present")
                    .await
                    .context("launch task failed")??;
                finished_summary = Some(summary);
                needs_redraw = true;
            }
        }

        if event::poll(EVENT_POLL_INTERVAL).context("failed to poll terminal events")? {
            if let Event::Key(key) = event::read().context("failed to read terminal event")? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Tab => {
                            state.next_section();
                            needs_redraw = true;
                        }
                        KeyCode::BackTab => {
                            state.previous_section();
                            needs_redraw = true;
                        }
                        KeyCode::Char('1') => {
                            state.select_section(SidebarTab::Workers);
                            needs_redraw = true;
                        }
                        KeyCode::Char('p') | KeyCode::Char('P') => {
                            state.select_section(SidebarTab::Problems);
                            needs_redraw = true;
                        }
                        KeyCode::Char('2') => {
                            state.select_section(SidebarTab::Watchdog);
                            needs_redraw = true;
                        }
                        KeyCode::Char('3') => {
                            state.select_section(SidebarTab::Events);
                            needs_redraw = true;
                        }
                        KeyCode::Char('4') => {
                            state.select_section(SidebarTab::Supervisor);
                            needs_redraw = true;
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            state.scroll_down();
                            needs_redraw = true;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            state.scroll_up();
                            needs_redraw = true;
                        }
                        KeyCode::PageDown => {
                            for _ in 0..8 {
                                state.scroll_down();
                            }
                            needs_redraw = true;
                        }
                        KeyCode::PageUp => {
                            for _ in 0..8 {
                                state.scroll_up();
                            }
                            needs_redraw = true;
                        }
                        KeyCode::Char('q') if snapshot.done && finished_summary.is_some() => break,
                        _ => {}
                    }
                }
            }
        }

        shimmer_frame = shimmer_frame.wrapping_add(1);
        if finished_summary.is_some() && snapshot.done {
            // Keep the terminal open until the user confirms with q.
            continue;
        }
    }

    finished_summary.context("dashboard exited before launch summary was available")
}

pub async fn run_startup_dashboard_until_tmux(
    db_path: PathBuf,
    control_status_path: PathBuf,
    attach_target: AttachTarget,
    session_name: &str,
    task: &JoinHandle<Result<LaunchSummary>>,
) -> Result<bool> {
    let mut tui = TuiSession::enter()?;
    let theme = SapphireTheme::default();
    let markdown_renderer = MarkdownRenderer::new(MarkdownTheme::from_app_theme(&theme));
    let data_source = DashboardDataSource::open(&db_path, control_status_path)?;
    let mut state = DashboardState::default();
    let mut shimmer_frame = 0usize;
    let mut snapshot = data_source.snapshot(&attach_target)?;
    let mut render_cache = RenderCache::new();
    let mut last_refresh = Instant::now();
    let mut last_tmux_check = Instant::now() - TMUX_SESSION_POLL_INTERVAL;
    let mut needs_redraw = true;

    loop {
        if last_refresh.elapsed() >= DATA_REFRESH_INTERVAL {
            snapshot = data_source.snapshot(&attach_target)?;
            last_refresh = Instant::now();
            needs_redraw = true;
        }
        if needs_redraw {
            let rendered_markdown = render_cache
                .markdown(&snapshot, tui.terminal.size()?.width, &markdown_renderer)
                .clone();
            tui.terminal.draw(|frame| {
                render::render(
                    frame,
                    &state,
                    &snapshot,
                    &rendered_markdown,
                    &theme,
                    shimmer_frame,
                    false,
                )
            })?;
            needs_redraw = false;
        }

        if last_tmux_check.elapsed() >= TMUX_SESSION_POLL_INTERVAL {
            last_tmux_check = Instant::now();
            if Tmux::new(None).has_session(session_name) {
                return Ok(true);
            }
        }
        if task.is_finished() {
            return Ok(false);
        }

        if event::poll(EVENT_POLL_INTERVAL).context("failed to poll terminal events")? {
            if let Event::Key(key) = event::read().context("failed to read terminal event")? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Tab => {
                            state.next_section();
                            needs_redraw = true;
                        }
                        KeyCode::BackTab => {
                            state.previous_section();
                            needs_redraw = true;
                        }
                        KeyCode::Char('1') => {
                            state.select_section(SidebarTab::Workers);
                            needs_redraw = true;
                        }
                        KeyCode::Char('p') | KeyCode::Char('P') => {
                            state.select_section(SidebarTab::Problems);
                            needs_redraw = true;
                        }
                        KeyCode::Char('2') => {
                            state.select_section(SidebarTab::Watchdog);
                            needs_redraw = true;
                        }
                        KeyCode::Char('3') => {
                            state.select_section(SidebarTab::Events);
                            needs_redraw = true;
                        }
                        KeyCode::Char('4') => {
                            state.select_section(SidebarTab::Supervisor);
                            needs_redraw = true;
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            state.scroll_down();
                            needs_redraw = true;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            state.scroll_up();
                            needs_redraw = true;
                        }
                        KeyCode::PageDown => {
                            for _ in 0..8 {
                                state.scroll_down();
                            }
                            needs_redraw = true;
                        }
                        KeyCode::PageUp => {
                            for _ in 0..8 {
                                state.scroll_up();
                            }
                            needs_redraw = true;
                        }
                        _ => {}
                    }
                }
            }
        }

        shimmer_frame = shimmer_frame.wrapping_add(1);
    }
}

pub fn interactive_supported() -> bool {
    std::io::IsTerminal::is_terminal(&io::stdout())
}

pub fn run_enabled_for_launch(dry_run: bool) -> bool {
    interactive_supported() && !dry_run
}

pub fn attach_for_repo(repo_path: PathBuf, started_after: DateTime<Utc>) -> AttachTarget {
    AttachTarget::LatestForRepo {
        repo_path,
        started_after,
    }
}

pub fn attach_for_mission(mission_id: uuid::Uuid) -> AttachTarget {
    AttachTarget::Mission(mission_id)
}

pub fn open_store(db_path: &std::path::Path) -> Result<Store> {
    Store::open(db_path)
}
