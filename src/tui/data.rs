//! Data loading and view mapping for the TUI layer.
//! Clean structured data only — no JSON blobs, no noisy raw output.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::model::{MissionSnapshot, SessionRole, SessionState, WorkerSnapshot};
use crate::store::Store;

use super::time::format_elapsed;

#[derive(Debug, Clone)]
pub enum AttachTarget {
    Mission(Uuid),
    LatestForRepo {
        repo_path: PathBuf,
        started_after: DateTime<Utc>,
    },
}

#[derive(Debug, Clone)]
pub struct WorkerView {
    pub id: Uuid,
    pub name: String,
    pub role: String,
    pub state: String,
    pub summary: String,
    pub focus: String,
    pub validation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SupervisorView {
    pub name: String,
    pub state: String,
    pub summary: String,
    pub pending_decisions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WatchdogView {
    pub runtime_events: usize,
    pub directives_parsed: usize,
    pub mail_routed: usize,
    pub validation_challenges: usize,
    pub stall_interventions: usize,
    pub lease_conflicts: usize,
    pub protocol_reminders: usize,
    pub supervisor_health_events: usize,
    pub supervisor_fallbacks: usize,
    pub supervisor_mode: String,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub ts: String,
    pub source: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct HealthView {
    pub validation_queue: usize,
    pub blocked: usize,
    pub contradictions: usize,
    pub watchdog_line: String,
}

#[derive(Debug, Clone)]
pub struct DashboardSnapshot {
    pub mission_id: Option<Uuid>,
    pub mission_rewrite: String,
    pub mission_status: String,
    pub worker_agent_count: usize,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub elapsed_label: String,
    pub supervisor_markdown: String,
    pub workers: Vec<WorkerView>,
    pub health: HealthView,
    pub watchdog: WatchdogView,
    pub live_log: Vec<LogEntry>,
    pub final_summary: Option<String>,
    pub attached: bool,
    pub done: bool,
    pub supervisor: Option<SupervisorView>,
}

pub struct DashboardDataSource {
    store: Store,
    control_status_path: PathBuf,
}

impl DashboardDataSource {
    pub fn open(db_path: &Path, control_status_path: PathBuf) -> Result<Self> {
        Ok(Self {
            store: Store::open(db_path)?,
            control_status_path,
        })
    }

    pub fn snapshot(&self, target: &AttachTarget) -> Result<DashboardSnapshot> {
        let mission = self.resolve_mission(target)?;
        let Some(mission) = mission else {
            return Ok(DashboardSnapshot {
                mission_id: None,
                mission_rewrite: "Starting the mission control surface...".to_owned(),
                mission_status: "launching".to_owned(),
                worker_agent_count: 0,
                started_at: None,
                updated_at: None,
                elapsed_label: "\u{23f1}  0s".to_owned(),
                supervisor_markdown:
                    "# Control\n\nWaiting for the supervisor plan and first worker sessions..."
                        .to_owned(),
                workers: Vec::new(),
                health: HealthView {
                    validation_queue: 0,
                    blocked: 0,
                    contradictions: 0,
                    watchdog_line: "Waiting for the newest mission in this repo.".to_owned(),
                },
                watchdog: WatchdogView::default(),
                live_log: Vec::new(),
                final_summary: None,
                attached: false,
                done: false,
                supervisor: None,
            });
        };

        let workers = self.store.load_workers(mission.id)?;
        let supervisor_summary = self.store.latest_supervisor_summary(mission.id)?;
        let control_status = fs::read_to_string(&self.control_status_path).ok();

        let worker_views: Vec<WorkerView> = workers
            .iter()
            .filter(|worker| worker.session.role == SessionRole::Worker)
            .map(worker_view)
            .collect();

        let health = HealthView {
            validation_queue: worker_views
                .iter()
                .filter(|w| matches!(w.validation.as_deref(), Some("awaiting validation")))
                .count(),
            blocked: worker_views
                .iter()
                .filter(|w| matches!(w.state.as_str(), "blocked" | "stalled"))
                .count(),
            contradictions: worker_views
                .iter()
                .filter(|w| w.state == "contradictory")
                .count(),
            watchdog_line: control_status
                .as_ref()
                .and_then(|s| {
                    s.lines()
                        .find(|line| line.starts_with("Workers: "))
                        .map(ToOwned::to_owned)
                })
                .unwrap_or_else(|| "No watchdog snapshot yet.".to_owned()),
        };

        // Parse watchdog stats from control status file
        let watchdog = parse_watchdog_stats(control_status.as_deref());

        let live_log: Vec<LogEntry> = Vec::new();

        // Extract supervisor view
        let supervisor_view = workers
            .iter()
            .find(|w| w.session.role == SessionRole::Supervisor)
            .map(|s| SupervisorView {
                name: s.session.name.clone(),
                state: s.session.status.as_str().to_owned(),
                summary: s.session.last_summary.clone().unwrap_or_default(),
                pending_decisions: Vec::new(),
            });

        let markdown = compose_supervisor_markdown(
            &mission,
            supervisor_summary.as_deref(),
            mission.final_summary.as_deref(),
            &worker_views,
            &health,
        );

        let done = matches!(mission.status.as_str(), "completed" | "failed")
            || (!worker_views.is_empty()
                && worker_views
                    .iter()
                    .all(|worker| matches!(worker.state.as_str(), "validated" | "failed" | "exited")));

        Ok(DashboardSnapshot {
            mission_id: Some(mission.id),
            mission_rewrite: mission.mission_rewrite.clone(),
            mission_status: mission.status.clone(),
            worker_agent_count: worker_views.len(),
            started_at: Some(mission.created_at),
            updated_at: Some(mission.updated_at),
            elapsed_label: format_elapsed(
                mission.created_at,
                if done {
                    Some(mission.updated_at)
                } else {
                    None
                },
            ),
            supervisor_markdown: markdown,
            workers: worker_views,
            health,
            watchdog,
            live_log,
            final_summary: mission.final_summary.clone(),
            attached: true,
            done,
            supervisor: supervisor_view,
        })
    }

    fn resolve_mission(&self, target: &AttachTarget) -> Result<Option<MissionSnapshot>> {
        match target {
            AttachTarget::Mission(mission_id) => self.store.load_mission_snapshot(*mission_id),
            AttachTarget::LatestForRepo {
                repo_path,
                started_after,
            } => {
                let Some(session) =
                    self.store.latest_session_for_repo(repo_path, *started_after)?
                else {
                    return Ok(None);
                };
                self.store.load_mission_snapshot(session.id)
            }
        }
    }
}

fn worker_view(worker: &WorkerSnapshot) -> WorkerView {
    WorkerView {
        id: worker.session.id,
        name: worker.session.name.clone(),
        role: worker
            .packet
            .as_ref()
            .map(|packet| packet.role.clone())
            .unwrap_or_else(|| worker.session.role_string().to_owned()),
        state: worker.session.status.as_str().to_owned(),
        summary: worker
            .session
            .last_summary
            .clone()
            .unwrap_or_else(|| "Working...".to_owned()),
        focus: worker
            .packet
            .as_ref()
            .map(|p| truncate(&p.explicit_task, 96))
            .unwrap_or_else(|| "Waiting for a scoped task.".to_owned()),
        validation: validation_hint(worker.session.status),
    }
}

fn validation_hint(state: SessionState) -> Option<String> {
    match state {
        SessionState::DoneClaimed | SessionState::NeedsValidation => {
            Some("awaiting validation".to_owned())
        }
        SessionState::Validated => Some("validated".to_owned()),
        SessionState::Failed => Some("failed".to_owned()),
        _ => None,
    }
}

/// Clean up event body text — strip JSON noise, prefixes, and raw directive output.
fn clean_event_body(kind: &str, body: &str) -> String {
    let trimmed = body.trim();

    if trimmed.is_empty()
        || trimmed.starts_with('{')
        || trimmed.starts_with("SAPPHIRE_")
        || trimmed.starts_with("ReadFile ")
        || trimmed.starts_with("Bash(")
        || trimmed.starts_with("Glob(")
        || trimmed.starts_with("Search(")
    {
        return String::new();
    }

    for prefix in &["output: ", "event: ", "directive: ", "state: ", "status: "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim().to_owned();
        }
    }

    if kind == "supervisor_notice" && let Some(rest) = trimmed.strip_prefix("Worker ") {
        return rest.to_owned();
    }

    trimmed.to_owned()
}

fn is_operator_event_kind(kind: &str) -> bool {
    matches!(
        kind,
        "supervisor_notice"
            | "supervisor_action"
            | "supervisor_health"
            | "watchdog_timeout"
            | "mail_timeout"
            | "mail_probe"
            | "mail_routed"
            | "critical_failure"
            | "mass_failure_detected"
            | "session_restarted"
            | "final_summary"
    ) || kind.starts_with("state:")
}

fn operator_source_label(kind: &str) -> &'static str {
    match kind {
        "supervisor_notice" => "supervisor",
        "supervisor_action" => "action",
        "supervisor_health" => "health",
        "watchdog_timeout" => "watchdog",
        "mail_timeout" | "mail_probe" | "mail_routed" => "mail",
        "critical_failure" | "mass_failure_detected" => "critical",
        "session_restarted" => "restart",
        "final_summary" => "final",
        _ if kind.starts_with("state:") => "state",
        _ => "event",
    }
}

fn compose_supervisor_markdown(
    mission: &MissionSnapshot,
    supervisor_summary: Option<&str>,
    final_summary: Option<&str>,
    workers: &[WorkerView],
    health: &HealthView,
) -> String {
    let mut out = String::new();

    out.push_str("# Supervisor Brief\n\n");

    if let Some(summary) = final_summary {
        let trimmed = summary.trim();
        if !trimmed.is_empty() {
            out.push_str("## Final Summary\n\n");
            out.push_str(trimmed);
            out.push_str("\n\n---\n\n");
        }
    } else if let Some(summary) = supervisor_summary {
        let trimmed = summary.trim();
        if !trimmed.is_empty() {
            out.push_str(trimmed);
            out.push_str("\n\n---\n\n");
        }
    } else {
        out.push_str("_Waiting for the next useful supervisor update._\n\n");
    }

    out.push_str("## Mission\n\n");
    out.push_str(&mission.mission_rewrite);
    out.push_str("\n\n---\n\n");

    out.push_str("## Dispatch\n\n");
    if workers.is_empty() {
        out.push_str("- Workers will appear after the supervisor emits the launch plan.\n\n");
    } else {
        for worker in workers {
            out.push_str(&format!(
                "- **{}** · {} · {}\n",
                worker.name, worker.role, worker.state
            ));
            out.push_str(&format!("  - Summary: {}\n", truncate(&worker.summary, 88)));
            out.push_str(&format!("  - Focus: {}\n", truncate(&worker.focus, 88)));
        }
        out.push('\n');
    }

    // Health summary
    out.push_str("## Health\n\n");
    out.push_str(&format!(
        "- Validation queue: {}\n- Blocked: {}\n- Contradictions: {}\n",
        health.validation_queue, health.blocked, health.contradictions
    ));
    out.push_str(&format!("- {}\n\n", health.watchdog_line));

    // Validation queue (detailed)
    let awaiting: Vec<&WorkerView> = workers
        .iter()
        .filter(|w| matches!(w.validation.as_deref(), Some("awaiting validation")))
        .collect();
    if !awaiting.is_empty() {
        out.push_str("### Awaiting Validation\n\n");
        for w in &awaiting {
            out.push_str(&format!("- **{}**: {}\n", w.name, w.summary));
        }
        out.push('\n');
    }

    // Blockers
    let blocked: Vec<&WorkerView> = workers
        .iter()
        .filter(|w| matches!(w.state.as_str(), "blocked" | "stalled"))
        .collect();
    if !blocked.is_empty() {
        out.push_str("### Blockers\n\n");
        for w in &blocked {
            out.push_str(&format!("- **{}**: {}\n", w.name, w.summary));
        }
        out.push('\n');
    }

    // Contradictions
    let contradictions: Vec<&WorkerView> = workers
        .iter()
        .filter(|w| w.state == "contradictory")
        .collect();
    if !contradictions.is_empty() {
        out.push_str("### Contradictions\n\n");
        for w in &contradictions {
            out.push_str(&format!("- **{}**: {}\n", w.name, w.summary));
        }
        out.push('\n');
    }

    out
}

fn truncate(value: &str, max: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max {
        compact
    } else {
        compact.chars().take(max.saturating_sub(1)).collect::<String>() + "\u{2026}"
    }
}

trait SessionRoleDisplay {
    fn role_string(&self) -> &'static str;
}

impl SessionRoleDisplay for crate::model::SessionRecord {
    fn role_string(&self) -> &'static str {
        match self.role {
            SessionRole::Supervisor => "Supervisor",
            SessionRole::Worker => "Worker",
        }
    }
}

impl Default for WatchdogView {
    fn default() -> Self {
        Self {
            runtime_events: 0,
            directives_parsed: 0,
            mail_routed: 0,
            validation_challenges: 0,
            stall_interventions: 0,
            lease_conflicts: 0,
            protocol_reminders: 0,
            supervisor_health_events: 0,
            supervisor_fallbacks: 0,
            supervisor_mode: "Unknown".to_owned(),
        }
    }
}

/// Parse watchdog stats from the control status file.
/// Expected line format:
/// `Workers: N, directives=X, mail=Y, validation=Z, stalls=A, conflicts=B, reminders=C, supervisor_health=D, fallbacks=E`
/// `Supervisor: ... [Mode] ...`
fn parse_watchdog_stats(content: Option<&str>) -> WatchdogView {
    let mut view = WatchdogView::default();
    let Some(text) = content else {
        return view;
    };

    for line in text.lines() {
        if line.starts_with("Workers: ") {
            // Parse key=value pairs from the Workers line
            for part in line.split(',') {
                let part = part.trim();
                if let Some(v) = extract_value(part, "directives") {
                    view.directives_parsed = v;
                } else if let Some(v) = extract_value(part, "mail") {
                    view.mail_routed = v;
                } else if let Some(v) = extract_value(part, "validation") {
                    view.validation_challenges = v;
                } else if let Some(v) = extract_value(part, "stalls") {
                    view.stall_interventions = v;
                } else if let Some(v) = extract_value(part, "conflicts") {
                    view.lease_conflicts = v;
                } else if let Some(v) = extract_value(part, "reminders") {
                    view.protocol_reminders = v;
                } else if let Some(v) = extract_value(part, "supervisor_health") {
                    view.supervisor_health_events = v;
                } else if let Some(v) = extract_value(part, "fallbacks") {
                    view.supervisor_fallbacks = v;
                }
            }
            // Extract worker count from "Workers: N"
            if let Some(n_str) = line.strip_prefix("Workers: ") {
                if let Some(first) = n_str.split(',').next() {
                    if let Ok(n) = first.trim().parse::<usize>() {
                        view.runtime_events = n; // use as worker count proxy
                    }
                }
            }
        } else if line.starts_with("Supervisor: ") {
            view.supervisor_mode = extract_supervisor_mode(line);
        }
    }

    view
}

fn extract_value(part: &str, key: &str) -> Option<usize> {
    let trimmed = part.trim();
    if let Some(val_str) = trimmed.strip_prefix(&format!("{}=", key)) {
        val_str.trim().parse::<usize>().ok()
    } else {
        None
    }
}

fn extract_supervisor_mode(line: &str) -> String {
    // Look for [Mode] in "Supervisor: name [Mode] summary"
    if let Some(start) = line.find('[') {
        if let Some(end) = line[start..].find(']') {
            return line[start + 1..start + end].to_owned();
        }
    }
    // Fallback: look for common mode keywords
    let lower = line.to_lowercase();
    if lower.contains("degraded") {
        return "Degraded".to_owned();
    } else if lower.contains("recovering") {
        return "Recovering".to_owned();
    }
    "Healthy".to_owned()
}
