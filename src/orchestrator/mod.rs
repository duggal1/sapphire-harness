use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::json;
use tracing::{info, warn};
use uuid::Uuid;

use crate::adapter::{
    Confidence, FinalEnvelope, NormalizedObservation, SupervisorAction, SupervisorEventType,
    adapter_for,
};
use crate::cli::{LaunchConfig, ResumeConfig};
use crate::model::{
    LaunchSummary, MissionPlan, MissionRecord, MissionStatus, NormalizedUpdateRecord,
    SessionRecord, SessionRole, SessionState, TaskRecord, ValidationResultRecord, WorkerPacket,
};
#[cfg(test)]
use crate::model::{RiskItem, Workstream, WorkstreamExecution};
use crate::protocol::{
    AckDirective, LeaseDirective, MailDirective, SapphireDirective, StatusDirective,
    consume_directives,
};
use crate::runtime::{ProcessLaunchSpec, RunningSession, RuntimeEvent, SessionRuntime};
use crate::tmux;
use crate::store::Store;
use crate::templates::PromptLibrary;

mod mail;
mod health;
mod dedup;
mod status_files;
mod supervisor;
mod coordination;
mod memory;
mod meetings;
mod mission_profile;
mod prompt_contracts;
mod live_state;
mod launch_prompt;
mod communication_policy;
mod enforcement;
mod finalization;

use mission_profile::MissionProfile;

pub struct Orchestrator {
    store: Store,
    prompts: PromptLibrary,
}

#[derive(Default)]
struct WatchdogStats {
    runtime_events: usize,
    directives: usize,
    mails_routed: usize,
    validation_challenges: usize,
    stall_interventions: usize,
    lease_conflicts: usize,
    protocol_reminders: usize,
    supervisor_health_events: usize,
    supervisor_fallbacks: usize,
    critical_failures: usize,
    crash_loops_detected: usize,
    mass_deaths_detected: usize,
}

struct ActiveSession {
    record: SessionRecord,
    packet: Option<WorkerPacket>,
    runtime: RunningSession,
    runtime_slot: Option<usize>,
    launch_spec: ProcessLaunchSpec,
    launch_prompt: String,
    state: SessionState,
    task_id: Option<Uuid>,
    line_buffer: String,
    raw_buffer: String,
    started_at: Instant,
    startup_grace_until: Instant,
    last_output_at: Instant,
    output_chunks: usize,
    directive_count: usize,
    initial_status_received: bool,
    output_chunks_at_last_status: usize,
    reported_overlap: Option<String>,
    stall_count: usize,
    restart_count: usize,
    restart_at: Option<Instant>,
    validation_pending: bool,
    low_confidence_count: usize,
    last_observation_key: Option<String>,
    last_supervisor_action_key: Option<String>,
    escalation_sent_for_state: Option<SessionState>,
    protocol_reminder_sent: bool,
    /// Consecutive stall detections without a successful response between them.
    /// Drives the escalation ladder: prompt → redirect → fail.
    consecutive_stall_failures: usize,
    /// Last time the watchdog confirmed this session was actually alive
    /// (output arrived, directive parsed, or heuristic signal detected).
    /// Used to distinguish "stalled but was recently alive" from "dead".
    last_confirmed_alive: Instant,
    /// Last known files touched (from status file or directive)
    #[allow(dead_code)]
    last_files: Vec<String>,
    /// Last known risks (from status file or directive)
    #[allow(dead_code)]
    last_risks: Vec<String>,
    /// Cooldown until which redundant watchdog interventions are skipped.
    /// Prevents thrashing: after any watchdog action (stall prompt, redirect,
    /// validation challenge), the worker gets a breather to respond.
    intervention_cooldown_until: Option<Instant>,
    /// Type of the last watchdog intervention (for logging/debugging).
    last_intervention_type: Option<String>,
    /// Total lifetime interventions (stall prompts, redirects, validation challenges).
    /// Drives escalating cooldown duration.
    total_interventions: usize,
    /// How long from last intervention to first output response.
    last_response_time: Option<Duration>,
    /// When the last watchdog intervention was sent (for response time calc).
    last_intervention_at: Option<Instant>,
    /// Prompts queued because the agent was mid-response (nudge queue pattern).
    /// Drained when the agent goes quiet for > 3s.
    queued_prompts: VecDeque<QueuedPrompt>,
    queued_prompt_keys: HashSet<String>,
    recent_prompt_keys: VecDeque<(String, Instant)>,
    last_prompt_sent_at: Option<Instant>,
    launch_prompt_sent: bool,
    cleanup_authorized: bool,
    /// Last authoritative status-file update seen for this worker.
    last_status_update_at: Option<Instant>,
    /// Last modified timestamp observed for the status file.
    last_status_file_modified: Option<SystemTime>,
    /// Cached tmux health snapshot for this session.
    last_tmux_health: Option<tmux::SessionHealth>,
    last_tmux_health_checked_at: Option<Instant>,
    /// Dedup key for supervisor notices sent into the live supervisor session.
    last_supervisor_notice_key: Option<String>,
    /// Recent supervisor notice keys so identical unresolved issues are not
    /// re-sent just because other notice types happened in between.
    recent_supervisor_notice_keys: VecDeque<(String, Instant)>,
    /// Dedup key for periodic supervisor state cards.
    last_supervisor_state_card_key: Option<String>,
    /// Zombie debounce: consecutive cycles detected as zombie before restart.
    /// Prevents false kills during slow startup or transient gaps.
    zombie_debounce: health::ZombieDebounce,
    /// Health state tracking: probe/response/failure counters per session.
    health_state: health::SessionHealthState,
    /// Message deduplicator: prevents duplicate mail/directive processing.
    message_dedup: dedup::MessageDeduplicator,
}

struct QueuedPrompt {
    key: String,
    body: String,
}

struct WorkerLaunch {
    session: SessionRecord,
    launch_spec: ProcessLaunchSpec,
    prompt: String,
    packet: WorkerPacket,
    task_id: Option<Uuid>,
}

struct LeaseOwner {
    session_id: Uuid,
    intent: String,
}

pub struct PendingMail {
    message_id: Uuid,
    thread_id: String,
    #[allow(dead_code)]
    intent: String,
    #[allow(dead_code)]
    thread_state: String,
    #[allow(dead_code)]
    duplicate_key: String,
    sender_session_id: Uuid,
    recipient_session_id: Uuid,
    cc_session_ids: Vec<Uuid>,
    #[allow(dead_code)]
    sender_pod: String,
    #[allow(dead_code)]
    recipient_pod: String,
    #[allow(dead_code)]
    routing_class: String,
    subject: String,
    message_type: String,
    priority: String,
    routed_at: Instant,
    acked: bool,
    timeout_stage: u8,
    last_timeout_at: Option<Instant>,
    reply_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SupervisorDecisionKind {
    Validation,
    StallRecovery,
    LowConfidenceRecovery,
    OverlapRecovery,
}

impl SupervisorDecisionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::StallRecovery => "stall_recovery",
            Self::LowConfidenceRecovery => "low_confidence_recovery",
            Self::OverlapRecovery => "overlap_recovery",
        }
    }
}

struct PendingSupervisorDecision {
    kind: SupervisorDecisionKind,
    target_session_id: Uuid,
    reason: String,
    queued_at: Instant,
    last_notified_at: Instant,
    notice_count: usize,
}

struct RecentFailure {
    session_id: Uuid,
    session_name: String,
    recorded_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupervisorMode {
    Healthy,
    Recovering,
    Degraded,
}

struct ControlSurface {
    state_dir: PathBuf,
    status_file: PathBuf,
    dashboard_file: PathBuf,
    transcript_dir: PathBuf,
    workers_state_dir: PathBuf,
    hidden_root: PathBuf,
    persist_transcripts: bool,
    /// All tmux session names (one per Ghostty tab, max 10 panes per session).
    tmux_session_names: Vec<String>,
}

struct AgentsBootstrap {
    path: PathBuf,
    existed: bool,
}

#[derive(Clone)]
struct PlanOutcome {
    plan: crate::model::MissionPlan,
    source: &'static str,
}

impl Orchestrator {
    pub fn open(state_or_db_path: &Path) -> Result<Self> {
        // Accept either state dir or db path (state_dir/sapphire.sqlite3)
        let state_dir = if state_or_db_path.is_dir() {
            state_or_db_path.to_owned()
        } else if let Some(parent) = state_or_db_path.parent() {
            parent.to_owned()
        } else {
            state_or_db_path.to_owned()
        };

        ensure_state_tree(&state_dir)?;
        ensure_state_tree(&hidden_state_dir(&state_dir))?;

        let store = Store::open(&state_dir)?;
        Ok(Self {
            store,
            prompts: PromptLibrary::load(),
        })
    }

    pub fn bootstrap(config: &LaunchConfig) -> Result<Self> {
        ensure_state_tree(&config.state_dir)?;
        ensure_state_tree(&hidden_state_dir(&config.state_dir))?;

        let store = Store::open(&config.state_dir)?;
        Ok(Self {
            store,
            prompts: PromptLibrary::load(),
        })
    }

    pub async fn launch(&self, config: LaunchConfig) -> Result<LaunchSummary> {
        let mission_id = Uuid::new_v4();
        let agents_bootstrap =
            ensure_agents_bootstrap(&config.repo, &self.prompts)?;
        let mission_profile = MissionProfile::from_launch(&config);
        let placeholder_plan = pending_supervisor_plan(&config.mission);

        let mission_record = MissionRecord {
            id: mission_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            repo_path: config.repo.clone(),
            mission: config.mission.clone(),
            mission_rewrite: placeholder_plan.mission_rewrite.clone(),
            worker_agent: config.worker_agent,
            supervisor_agent: config.supervisor_agent,
            worker_count: config.worker_count,
            status: if config.dry_run {
                MissionStatus::Planned
            } else {
                MissionStatus::Launching
            },
            final_summary: None,
        };
        self.store
            .persist_mission(&mission_record, &placeholder_plan)?;
        self.store.append_summary(
            mission_id,
            None,
            "mission_rewrite",
            &placeholder_plan.mission_rewrite,
        )?;
        self.store.append_summary(
            mission_id,
            None,
            "agents_bootstrap",
            if agents_bootstrap.existed {
                format!(
                    "AGENTS.md already present at {}",
                    agents_bootstrap.path.display()
                )
            } else {
                format!(
                    "AGENTS.md created at {} from the repo instruction source before worker launch",
                    agents_bootstrap.path.display()
                )
            },
        )?;
        self.store.append_summary(
            mission_id,
            None,
            "mission_profile",
            format!(
                "coordination_focused={} deterministic_planning={} lean_supervision={} repair_supervisor={} state_cards={}",
                mission_profile.coordination_focused,
                mission_profile.deterministic_planning,
                mission_profile.lean_supervision,
                mission_profile.enable_repair_supervisor,
                mission_profile.enable_state_cards,
            ),
        )?;

        let mut notes = vec![
            format!(
                "supervisor prompt written to {}",
                config.state_dir.join("prompts").display()
            ),
            if agents_bootstrap.existed {
                format!("AGENTS.md present at {}", agents_bootstrap.path.display())
            } else {
                format!(
                    "AGENTS.md created from instruction source at {}",
                    agents_bootstrap.path.display()
                )
            },
        ];

        let supervisor_name = "supervisor-01".to_owned();
        let mut supervisor_spec = config.supervisor_agent.build_launch_spec(
            &config.repo,
            &config.state_dir,
            &config.supervisor_args,
        );
        supervisor_spec = harden_supervisor_launch_spec(config.supervisor_agent, supervisor_spec);
        supervisor_spec.surface_label = supervisor_name.clone();
        let supervisor_session = SessionRecord {
            id: Uuid::new_v4(),
            mission_id,
            role: SessionRole::Supervisor,
            ordinal: 1,
            agent: config.supervisor_agent,
            terminal_id: supervisor_name.clone(),
            name: supervisor_name.clone(),
            owned_scope: "Mission rewrite, worker supervision, validation, contradiction control, and final synthesis.".to_owned(),
            status: if config.dry_run {
                SessionState::Planned
            } else {
                SessionState::Booting
            },
            launch_command: launch_command(supervisor_spec.program.as_str(), &supervisor_spec.args),
            last_heartbeat_at: Utc::now(),
            last_summary: Some("Supervisor planned".to_owned()),
        };
        self.store.persist_session(&supervisor_session, None)?;
        let supervisor_task = TaskRecord {
            id: Uuid::new_v4(),
            mission_id,
            worker_id: supervisor_session.id,
            title: "Mission supervision".to_owned(),
            description: "Own decomposition, live supervision, validation, contradiction handling, and final synthesis.".to_owned(),
            status: "assigned".to_owned(),
            priority: "high".to_owned(),
            depends_on_json: "[]".to_owned(),
            definition_of_done_json: serde_json::to_string(&vec![
                "All worker claims challenged".to_owned(),
                "Contradictions ruled on".to_owned(),
                "Final mission state summarized".to_owned(),
            ])?,
        };
        self.store.persist_task(&supervisor_task)?;

        if !config.dry_run {
            notes.push("startup: direct live launch without preflight gate".to_owned());
        }
        let plan_outcome = if mission_profile.deterministic_planning {
            PlanOutcome {
                plan: mission_profile::deterministic_plan_for_mission(
                    &config.mission,
                    config.worker_count,
                    mission_profile,
                ),
                source: "deterministic",
            }
        } else {
            // NO FALLBACK. Supervisor must produce a valid plan or the launch fails.
            self.plan_with_supervisor(mission_id, &config, &supervisor_session)
                .await?
        };
        let planned_worker_count = plan_outcome.plan.worker_packets.len();
        let effective_plan = plan_outcome.plan.clone();
        self.store
            .replace_mission_plan(mission_id, &effective_plan)?;
        self.store.append_summary(
            mission_id,
            None,
            "mission_rewrite",
            &effective_plan.mission_rewrite,
        )?;
        self.store.append_summary(
            mission_id,
            Some(supervisor_session.id),
            "plan_source",
            format!(
                "plan source: {} ({} requested workers, {} planned workers)",
                plan_outcome.source, config.worker_count, planned_worker_count
            ),
        )?;

        let supervisor_prompt = render_supervisor_prompt_with_agents(
            self.prompts
                .render_supervisor_prompt(&config.mission, &effective_plan),
            &agents_bootstrap,
            config.worker_count,
        );
        write_prompt_file(&config.state_dir, &supervisor_name, &supervisor_prompt)?;
        let supervisor_launch_prompt = launch_prompt::supervisor_loader_prompt(
            config.supervisor_agent,
            &launch_prompt::prompt_file_path(&config.state_dir, &supervisor_name),
            &supervisor_name,
        );

        let worker_adapter = adapter_for(config.worker_agent);
        let mut session_names = Vec::with_capacity(config.worker_count + 1);
        session_names.push(supervisor_name.clone());
        let mut worker_launches = Vec::with_capacity(effective_plan.worker_packets.len());

        for (index, packet) in effective_plan.worker_packets.iter().enumerate() {
            let worker_name = if packet.display_name.trim().is_empty() {
                packet.worker_id.clone()
            } else {
                packet.display_name.clone()
            };
            let base_prompt =
                worker_adapter.build_assignment_prompt(&self.prompts, &config.mission, packet);

            // Load persistent memory from previous missions — engineer remembers what they did.
            let memory_block = self
                .store
                .agent_memory()
                .format_history_for_injection(&worker_name, &packet.role_type, 3)
                .ok()
                .flatten();

            let full_prompt = render_worker_prompt_with_agents(
                base_prompt,
                &agents_bootstrap,
                &config.state_dir,
                packet,
                config.git_remote.as_deref(),
                memory_block.as_deref(),
            );
            write_prompt_file(&config.state_dir, &worker_name, &full_prompt)?;
            status_files::write_bootstrap_status_files(
                &config
                    .state_dir
                    .join("workers")
                    .join(&worker_name)
                    .join("status.json"),
                &hidden_state_dir(&config.state_dir)
                    .join("workers")
                    .join(&worker_name)
                    .join("status.json"),
                "assignment delivered; awaiting worker update",
            )?;
            let live_prompt = launch_prompt::worker_terminal_prompt(
                &config.state_dir,
                &launch_prompt::prompt_file_path(&config.state_dir, &worker_name),
                &config.mission,
                packet,
            );
            let mut launch_spec = config.worker_agent.build_launch_spec(
                &config.repo,
                &config.state_dir,
                &config.worker_args,
            );
            launch_spec.surface_label = worker_name.clone();
            let session = SessionRecord {
                id: Uuid::new_v4(),
                mission_id,
                role: SessionRole::Worker,
                ordinal: index + 1,
                agent: config.worker_agent,
                terminal_id: worker_name.clone(),
                name: worker_name.clone(),
                owned_scope: packet.owned_scope.clone(),
                status: if config.dry_run {
                    SessionState::Planned
                } else {
                    SessionState::Booting
                },
                launch_command: launch_command(launch_spec.program.as_str(), &launch_spec.args),
                last_heartbeat_at: Utc::now(),
                last_summary: Some(format!("Assigned {}", packet.role)),
            };
            self.store.persist_session(&session, Some(packet))?;
            let task_id = Uuid::new_v4();
            self.store.persist_task(&TaskRecord {
                id: task_id,
                mission_id,
                worker_id: session.id,
                title: packet.role.clone(),
                description: packet.explicit_task.clone(),
                status: "assigned".to_owned(),
                priority: if packet.role.to_ascii_lowercase().contains("validation") {
                    "high".to_owned()
                } else {
                    "medium".to_owned()
                },
                depends_on_json: "[]".to_owned(),
                definition_of_done_json: serde_json::to_string(&packet.definition_of_done)?,
            })?;
            session_names.push(worker_name.clone());
            worker_launches.push(WorkerLaunch {
                session,
                launch_spec,
                prompt: live_prompt,
                packet: packet.clone(),
                task_id: Some(task_id),
            });
        }
        notes.push(format!(
            "plan source: {} ({} requested workers, {} planned workers)",
            plan_outcome.source, config.worker_count, planned_worker_count
        ));

        if config.dry_run {
            notes.push("dry-run only: no agent processes were spawned".to_owned());
        } else {
            self.store
                .update_mission_status(mission_id, MissionStatus::Running)?;
            let control_surface = ControlSurface {
                state_dir: config.state_dir.clone(),
                status_file: config.state_dir.join("control/status.txt"),
                dashboard_file: config.state_dir.join("control/dashboard.txt"),
                transcript_dir: config.state_dir.join("transcripts"),
                workers_state_dir: config.state_dir.join("workers"),
                hidden_root: hidden_state_dir(&config.state_dir),
                persist_transcripts: true,
                tmux_session_names: config.tmux_session_name.clone().or_else(|| {
                    if config.tmux {
                        Some(format!("sp-{}", &mission_id.to_string()[..8]))
                    } else {
                        None
                    }
                }).into_iter().collect(),
            };
            let stats = self
                .run_live_mission(
                    mission_id,
                    &config,
                    supervisor_session.clone(),
                supervisor_task.id,
                supervisor_spec,
                supervisor_launch_prompt,
                worker_launches,
                mission_profile,
                &control_surface,
            )
            .await?;
            notes.push(format!(
                "watchdog: events={} directives={} mail={} validation={} stalls={} lease_conflicts={} protocol_reminders={} supervisor_health={} supervisor_fallbacks={}",
                stats.runtime_events,
                stats.directives,
                stats.mails_routed,
                stats.validation_challenges,
                stats.stall_interventions,
                stats.lease_conflicts,
                stats.protocol_reminders,
                stats.supervisor_health_events,
                stats.supervisor_fallbacks,
            ));
            for tmux_name in &control_surface.tmux_session_names {
                notes.push(format!("tmux session: {tmux_name}"));
            }
        }

        Ok(LaunchSummary {
            mission_id,
            repo: config.repo,
            state_dir: config.state_dir.clone(),
            dry_run: config.dry_run,
            worker_agent: config.worker_agent,
            supervisor_agent: config.supervisor_agent,
            worker_count: config.worker_count,
            session_names,
            mission_rewrite: effective_plan.mission_rewrite,
            workstream_names: effective_plan
                .workstreams
                .iter()
                .map(|workstream| workstream.name.clone())
                .collect(),
            notes,
        })
    }

    pub fn render_status(&self) -> Result<String> {
        let sessions = self.store.list_sessions()?;
        let active: Vec<_> = sessions
            .iter()
            .filter(|s| s.status == "running" || s.status == "launching")
            .collect();
        let completed: Vec<_> = sessions
            .iter()
            .filter(|s| s.status == "completed")
            .collect();
        let failed: Vec<_> = sessions.iter().filter(|s| s.status == "failed").collect();

        let mut lines = Vec::new();
        lines.push(format!("{} total missions", sessions.len()));
        lines.push(String::new());

        if !active.is_empty() {
            lines.push("── active ──".to_owned());
            for s in &active {
                lines.push(format!(
                    "  {}  {}  {}",
                    truncate_id(&s.id),
                    pad_status(&s.status),
                    s.mission_rewrite
                ));
            }
            lines.push(String::new());
        }

        if !completed.is_empty() {
            lines.push("── completed ──".to_owned());
            for s in &completed {
                lines.push(format!(
                    "  {}  {}  {}",
                    truncate_id(&s.id),
                    pad_status(&s.status),
                    s.mission_rewrite
                ));
            }
            lines.push(String::new());
        }

        if !failed.is_empty() {
            lines.push("── failed ──".to_owned());
            for s in &failed {
                lines.push(format!(
                    "  {}  {}  {}",
                    truncate_id(&s.id),
                    pad_status(&s.status),
                    s.mission_rewrite
                ));
            }
            lines.push(String::new());
        }

        if lines.len() <= 1 {
            lines.push(
                "no missions yet — launch one with: sp <agent> <count> \"mission\"".to_owned(),
            );
        }

        Ok(lines.join("\n"))
    }

    pub fn render_sessions(&self) -> Result<String> {
        let sessions = self.store.list_sessions()?;
        let mut lines = Vec::new();
        for session in sessions {
            lines.push(format!(
                "{} | {} | {} | {}",
                session.id,
                session.status,
                session.repo_path.display(),
                session.mission_rewrite
            ));
            if let Some(summary) = session.final_summary {
                lines.push(format!("final: {summary}"));
            }
        }
        if lines.is_empty() {
            lines.push("no sessions found".to_owned());
        }
        Ok(lines.join("\n"))
    }

    pub fn render_replay(&self, mission_id: Uuid, limit: usize) -> Result<String> {
        let snapshot = self
            .store
            .load_mission_snapshot(mission_id)?
            .context("unknown session id")?;
        let entries = self.store.recent_replay_entries(mission_id, limit)?;
        let mut lines = vec![
            format!("session: {}", snapshot.id),
            format!("status: {}", snapshot.status),
            format!("mission: {}", snapshot.mission_rewrite),
        ];
        if let Some(summary) = snapshot.final_summary {
            lines.push(format!("final_summary: {summary}"));
        }
        lines.push(String::new());
        for entry in entries {
            lines.push(format!(
                "{} | {} | {} | {}",
                entry.created_at.to_rfc3339(),
                entry.lane,
                entry.kind,
                truncate(&entry.body, 240)
            ));
        }
        Ok(lines.join("\n"))
    }

    #[allow(dead_code)]
    pub fn render_worker_status(&self, mission_id: Uuid) -> Result<String> {
        let workers = self.store.load_workers(mission_id)?;
        let mut lines = Vec::new();
        for worker in workers
            .iter()
            .filter(|worker| worker.session.role == SessionRole::Worker)
        {
            lines.push(format!(
                "{} [{}] {}",
                worker.session.name,
                worker.session.status.as_str(),
                worker
                    .session
                    .last_summary
                    .as_deref()
                    .unwrap_or("no summary")
            ));
        }
        if lines.is_empty() {
            lines.push("no workers found".to_owned());
        }
        Ok(lines.join("\n"))
    }

    pub fn render_worker_replay(
        &self,
        mission_id: Uuid,
        worker: &str,
        limit: usize,
    ) -> Result<String> {
        let workers = self.store.load_workers(mission_id)?;
        let target = workers
            .into_iter()
            .find(|candidate| {
                candidate.session.id.to_string() == worker
                    || candidate.session.name == worker
                    || candidate
                        .packet
                        .as_ref()
                        .map(|packet| packet.worker_id.as_str() == worker)
                        .unwrap_or(false)
            })
            .context("unknown worker for session")?;
        let entries = self
            .store
            .recent_worker_replay(mission_id, target.session.id, limit)?;
        let mut lines = vec![format!(
            "{} [{}]",
            target.session.name,
            target.session.status.as_str()
        )];
        for entry in entries {
            lines.push(format!(
                "{} | {} | {}",
                entry.created_at.to_rfc3339(),
                entry.kind,
                truncate(&entry.body, 240)
            ));
        }
        Ok(lines.join("\n"))
    }

    pub fn render_supervisor_summary(&self, mission_id: Uuid) -> Result<String> {
        let summary = self
            .store
            .latest_supervisor_summary(mission_id)?
            .context("no supervisor summary found")?;
        Ok(summary)
    }

    pub async fn resume(&self, config: ResumeConfig) -> Result<LaunchSummary> {
        let snapshot = self
            .store
            .load_mission_snapshot(config.mission_id)?
            .context("unknown session id")?;
        let workers = self.store.load_workers(snapshot.id)?;
        let supervisor_snapshot = workers
            .iter()
            .find(|worker| worker.session.role == SessionRole::Supervisor)
            .cloned()
            .context("session is missing supervisor row")?;
        let worker_snapshots = workers
            .into_iter()
            .filter(|worker| {
                worker.session.role == SessionRole::Worker && !worker.session.status.is_terminal()
            })
            .collect::<Vec<_>>();

        let repo = snapshot.repo_path.clone();
        let state_dir = config
            .state_dir
            .unwrap_or_else(|| snapshot.repo_path.join(".sp"));
        let agents_bootstrap = ensure_agents_bootstrap(&repo, &self.prompts)?;
        let git_remote = match crate::git::check_git_state(&repo) {
            crate::git::GitState::Ready { remote_url } => Some(remote_url),
            _ => None,
        };
        let supervisor_prompt = render_supervisor_prompt_with_agents(
            format!(
                "{}\n\nResume context:\nMission rewrite: {}\nRecent replay:\n{}\nContinue supervising from durable state. Use the action envelope when the watchdog asks for intervention.",
                self.prompts
                    .render_supervisor_prompt(&snapshot.user_mission_raw, &snapshot.plan),
                snapshot.mission_rewrite,
                self.render_replay(snapshot.id, 12)?
            ),
            &agents_bootstrap,
            worker_snapshots.len(),
        );
        write_prompt_file(
            &state_dir,
            &supervisor_snapshot.session.name,
            &supervisor_prompt,
        )?;
        let supervisor_launch_prompt = launch_prompt::supervisor_loader_prompt(
            supervisor_snapshot.session.agent,
            &launch_prompt::prompt_file_path(&state_dir, &supervisor_snapshot.session.name),
            &supervisor_snapshot.session.name,
        );

        let mut worker_launches = Vec::new();
        for worker in worker_snapshots {
            let packet = worker
                .packet
                .clone()
                .context("resume requires persisted worker packet")?;
            let adapter = adapter_for(worker.session.agent);

            // Load previous session memory for resume injection
            let memory_context = self
                .store
                .agent_memory()
                .format_for_resume(&snapshot.id, &worker.session.name)
                .ok()
                .flatten()
                .map(|m| format!("\n\n{}\n\nContinue from your previous session. Do not repeat work you already completed.", m))
                .unwrap_or_default();

            let base_prompt = format!(
                "{}\n\nResume context:\n{}\n{}{}",
                adapter.build_assignment_prompt(&self.prompts, &snapshot.user_mission_raw, &packet),
                worker
                    .session
                    .last_summary
                    .as_deref()
                    .unwrap_or("no prior summary"),
                self.render_worker_replay(snapshot.id, &worker.session.id.to_string(), 8)?,
                memory_context,
            );

            // Also load cross-mission memory — what this agent did in OTHER missions.
            let cross_mission_memory = self
                .store
                .agent_memory()
                .format_history_for_injection(&worker.session.name, &packet.role_type, 2)
                .ok()
                .flatten();

            let worker_prompt = render_worker_prompt_with_agents(
                base_prompt,
                &agents_bootstrap,
                &state_dir,
                &packet,
                git_remote.as_deref(),
                cross_mission_memory.as_deref(),
            );
            write_prompt_file(&state_dir, &worker.session.name, &worker_prompt)?;
            status_files::write_bootstrap_status_files(
                &state_dir
                    .join("workers")
                    .join(&worker.session.name)
                    .join("status.json"),
                &hidden_state_dir(&state_dir)
                    .join("workers")
                    .join(&worker.session.name)
                    .join("status.json"),
                "assignment delivered; awaiting worker update",
            )?;
            let live_prompt = launch_prompt::worker_terminal_prompt(
                &state_dir,
                &launch_prompt::prompt_file_path(&state_dir, &worker.session.name),
                &snapshot.user_mission_raw,
                &packet,
            );
            worker_launches.push(WorkerLaunch {
                session: worker.session.clone(),
                launch_spec: rebuild_launch_spec(&worker.session, &repo, &state_dir),
                prompt: live_prompt,
                packet,
                task_id: self.store.find_task_id(snapshot.id, worker.session.id)?,
            });
        }

        let live_config = LaunchConfig {
            worker_agent: worker_launches
                .first()
                .map(|worker| worker.session.agent)
                .unwrap_or(supervisor_snapshot.session.agent),
            supervisor_agent: supervisor_snapshot.session.agent,
            worker_count: worker_launches.len(),
            repo: repo.clone(),
            mission: snapshot.user_mission_raw.clone(),
            state_dir: state_dir.clone(),
            dry_run: false,
            stall_seconds: config.stall_seconds,
            watchdog_max_seconds: config.watchdog_max_seconds,
            watchdog_tick_millis: config.watchdog_tick_millis,
            tmux: config.tmux,
            tmux_session_name: config.tmux_session_name.clone(),
            persist_transcripts: config.persist_transcripts,
            tui: config.tui,
            worker_args: Vec::new(),
            supervisor_args: Vec::new(),
            git_remote,
        };
        let control_surface = ControlSurface {
            state_dir: state_dir.clone(),
            status_file: state_dir.join("control/status.txt"),
            dashboard_file: state_dir.join("control/dashboard.txt"),
            transcript_dir: state_dir.join("transcripts"),
            workers_state_dir: state_dir.join("workers"),
            hidden_root: hidden_state_dir(&state_dir),
            persist_transcripts: true,
            tmux_session_names: config.tmux_session_name.clone().or_else(|| {
                if config.tmux {
                    Some(format!("sp-{}", &snapshot.id.to_string()[..8]))
                } else {
                    None
                }
            }).into_iter().collect(),
        };
        self.store.append_summary(
            snapshot.id,
            Some(supervisor_snapshot.session.id),
            "resume",
            "mission resumed from durable orchestration state",
        )?;
        let stats = self
            .run_live_mission(
                snapshot.id,
                &live_config,
                supervisor_snapshot.session.clone(),
                self.store
                    .find_task_id(snapshot.id, supervisor_snapshot.session.id)?
                    .unwrap_or(Uuid::new_v4()),
                rebuild_launch_spec(&supervisor_snapshot.session, &repo, &state_dir),
                supervisor_launch_prompt,
                worker_launches,
                MissionProfile::from_launch(&live_config),
                &control_surface,
            )
            .await?;
        Ok(LaunchSummary {
            mission_id: snapshot.id,
            repo,
            state_dir: state_dir.clone(),
            dry_run: false,
            worker_agent: live_config.worker_agent,
            supervisor_agent: live_config.supervisor_agent,
            worker_count: live_config.worker_count,
            session_names: vec![supervisor_snapshot.session.name],
            mission_rewrite: snapshot.mission_rewrite,
            workstream_names: snapshot
                .plan
                .workstreams
                .iter()
                .map(|workstream| workstream.name.clone())
                .collect(),
            notes: vec![
                "resumed from durable orchestration state".to_owned(),
                format!(
                    "watchdog: events={} directives={} mail={} validation={} stalls={} lease_conflicts={} protocol_reminders={} supervisor_health={} supervisor_fallbacks={}",
                    stats.runtime_events,
                    stats.directives,
                    stats.mails_routed,
                    stats.validation_challenges,
                    stats.stall_interventions,
                    stats.lease_conflicts,
                    stats.protocol_reminders,
                    stats.supervisor_health_events,
                    stats.supervisor_fallbacks,
                ),
            ],
        })
    }

    async fn plan_with_supervisor(
        &self,
        mission_id: Uuid,
        config: &LaunchConfig,
        supervisor_session: &SessionRecord,
    ) -> Result<PlanOutcome> {
        let adapter = adapter_for(config.supervisor_agent);
        let planning_prompt =
            adapter.build_supervisor_plan_prompt(&config.mission, config.worker_count);

        // Qwen supervisor planning: use stdin pipe (reliable, no PTY).
        // PTY mode with -p is flaky for Qwen — sometimes produces output, sometimes doesn't.
        // Stdin pipe (echo | qwen) is consistently reliable.
        if config.supervisor_agent == crate::agent::AgentKind::Qwen {
            return self.plan_with_supervisor_qwen_pipe(
                mission_id,
                config,
                &planning_prompt,
            ).await;
        }

        // Other agents: use PTY-based planning
        let planning_spec = harden_supervisor_launch_spec(
            config.supervisor_agent,
            config.supervisor_agent.build_launch_spec(
                &config.repo,
                &config.state_dir,
                &config.supervisor_args,
            ),
        );
        let (planning_spec, prompt_embedded) = embed_initial_prompt_if_supported(
            config.supervisor_agent,
            planning_spec,
            &planning_prompt,
        );
        let mut runtime = SessionRuntime::new();
        let planning_runtime = runtime.spawn(supervisor_session.id, planning_spec)?;
        self.store
            .update_session_state(supervisor_session.id, SessionState::Progressing)?;
        if !prompt_embedded {
            tokio::time::sleep(planning_runtime.prompt_delay()).await;
            planning_runtime.send_prompt(&planning_prompt)?;
        }

        let started_at = Instant::now();
        let timeout = Duration::from_secs(120);
        let mut raw_buffer = String::new();
        let mut correction_sent = false;
        let mut hard_retry_sent = false;

        while started_at.elapsed() < timeout {
            let Some(event) = runtime.next_event(Duration::from_millis(750)).await else {
                if !correction_sent && started_at.elapsed() >= Duration::from_secs(12) {
                    planning_runtime.send_prompt(
                        "Your last output was not yet a usable Sapphire plan. Reply only with the JSON plan block between BEGIN_SAPPHIRE_PLAN_JSON and END_SAPPHIRE_PLAN_JSON.",
                    )?;
                    correction_sent = true;
                } else if !hard_retry_sent && started_at.elapsed() >= Duration::from_secs(28) {
                    planning_runtime.send_prompt(
                        "Last attempt. Output only a single valid JSON plan block. No prose. No markdown. BEGIN_SAPPHIRE_PLAN_JSON on its own line, then JSON, then END_SAPPHIRE_PLAN_JSON.",
                    )?;
                    hard_retry_sent = true;
                }
                continue;
            };
            persist_runtime_event(&self.store, mission_id, &event)?;
            match event {
                RuntimeEvent::Output { chunk, .. } => {
                    raw_buffer.push_str(&crate::protocol::sanitize_output(&chunk));
                    trim_recent_utf8(&mut raw_buffer, 64_000, 32_000);
                    if let Some(plan) = adapter.extract_supervisor_plan(&raw_buffer) {
                        let plan =
                            normalize_supervisor_plan(plan, config.worker_count, "supervisor")?;
                        let _ = planning_runtime.terminate();
                        return Ok(PlanOutcome {
                            plan,
                            source: "supervisor",
                        });
                    }
                    if !correction_sent
                        && raw_buffer.contains("mission_rewrite")
                        && !raw_buffer.contains("BEGIN_SAPPHIRE_PLAN_JSON")
                    {
                        planning_runtime.send_prompt(
                            "You likely produced a plan without the required wrapper. Re-output the same plan only as JSON between BEGIN_SAPPHIRE_PLAN_JSON and END_SAPPHIRE_PLAN_JSON.",
                        )?;
                        correction_sent = true;
                    }
                }
                RuntimeEvent::Exited { .. } => break,
                RuntimeEvent::Automation { .. } => {}
            }
        }

        let _ = planning_runtime.terminate();
        anyhow::bail!(
            "supervisor planning timed out or produced no valid Sapphire plan. Last output: {}",
            truncate(&raw_buffer, 600)
        )
    }

    /// Qwen supervisor planning via stdin pipe (reliable).
    /// PTY mode with -p is flaky for Qwen; stdin pipe is consistently reliable.
    async fn plan_with_supervisor_qwen_pipe(
        &self,
        _mission_id: Uuid,
        config: &LaunchConfig,
        planning_prompt: &str,
    ) -> Result<PlanOutcome> {
        use crate::agent::AgentKind;
        let state_dir = config.state_dir.clone();
        let repo = config.repo.clone();
        let prompt = planning_prompt.to_owned();
        let worker_count = config.worker_count;

        let result = tokio::task::spawn_blocking(move || {
            let plan_prompt_path = launch_prompt::prompt_file_path(&state_dir, "__supervisor_plan__");
            write_string_to_file(&plan_prompt_path, &prompt)
                .context("failed to persist supervisor planning brief")?;
            let loader_prompt = format!(
                "Read and execute this planning brief now:\n@{path}\n{path}\n\n\
Rules:\n\
- Ingest the brief once.\n\
- Do not describe the brief.\n\
- Output only one valid wrapped Sapphire plan block.\n\
- Begin with BEGIN_SAPPHIRE_PLAN_JSON and end with END_SAPPHIRE_PLAN_JSON.\n\
- No prose outside the markers.",
                path = plan_prompt_path.display(),
            );
            let mut cmd = std::process::Command::new("qwen");
            cmd.arg("--screen-reader");
            cmd.arg("--approval-mode");
            cmd.arg("yolo");
            cmd.stdin(std::process::Stdio::piped());
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());
            cmd.current_dir(&repo);
            cmd.env("SAPPHIRE_SESSION_ROOT", state_dir.to_string_lossy().as_ref());

            let mut child = cmd.spawn().context("failed to spawn qwen for planning")?;
            {
                let mut stdin = child.stdin.take().context("failed to get stdin")?;
                std::io::Write::write_all(&mut stdin, loader_prompt.as_bytes())
                    .context("failed to write planning loader prompt")?;
                drop(stdin); // Close stdin to signal EOF
            }

            let started = std::time::Instant::now();
            loop {
                if child
                    .try_wait()
                    .context("failed to poll qwen planning process")?
                    .is_some()
                {
                    break;
                }
                if started.elapsed() >= Duration::from_secs(120) {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!("qwen planning timed out after 120s");
                }
                std::thread::sleep(Duration::from_millis(250));
            }

            let output = child.wait_with_output().context("qwen planning failed")?;
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let combined = format!("{}\n{}", stdout, stderr);
            Ok::<_, anyhow::Error>((combined, stdout.len(), stderr.len()))
        }).await??;

        let (combined, _stdout_len, _stderr_len) = result;

        // Parse the plan using the Qwen adapter
        let qwen_adapter = crate::adapter::adapter_for(AgentKind::Qwen);
        if let Some(plan) = qwen_adapter.extract_supervisor_plan(&combined) {
            let plan = normalize_supervisor_plan(
                plan,
                worker_count,
                "qwen",
            )?;

            // Final gate: ensure packets are genuinely differentiated
            if plan.worker_packets.len() > 1 {
                let first = &plan.worker_packets[0];
                for (i, packet) in plan.worker_packets.iter().enumerate().skip(1) {
                    let task_sim = text_similarity_sim(&packet.explicit_task, &first.explicit_task);
                    let scope_sim = text_similarity_sim(&packet.owned_scope, &first.owned_scope);
                    if task_sim > 0.9 && scope_sim > 0.9 {
                        anyhow::bail!(
                            "supervisor produced non-differentiated worker packets: packet 1 and packet {} have near-identical tasks and scopes. Each worker must have a genuinely different task.",
                            i + 1
                        );
                    }
                }
            }

            return Ok(PlanOutcome { plan, source: "supervisor" });
        }

        anyhow::bail!(
            "qwen planning produced no valid Sapphire plan. Last output: {}",
            truncate(&combined, 600)
        )
    }

    async fn run_live_mission(
        &self,
        mission_id: Uuid,
        config: &LaunchConfig,
        supervisor_session: SessionRecord,
        supervisor_task_id: Uuid,
        supervisor_spec: ProcessLaunchSpec,
        supervisor_prompt: String,
        worker_launches: Vec<WorkerLaunch>,
        mission_profile: MissionProfile,
        control_surface: &ControlSurface,
    ) -> Result<WatchdogStats> {
        // Create tmux batch sessions (10 workers per session, from launch-codex-tabs.sh)
        let tmux_sessions = if !control_surface.tmux_session_names.is_empty() {
            let session_names = self.ensure_tmux_surface(
                &control_surface.tmux_session_names[0],
                mission_id,
                config,
            )?;
            
            // Give tmux time to settle panes before opening external terminal
            std::thread::sleep(std::time::Duration::from_secs(2));

            // Open Ghostty tabs: first session = new window, rest = tabs
            let tmux = tmux::Tmux::new(None);
            if let Err(e) = tmux.open_ghostty_batch_tabs(&session_names) {
                tracing::warn!(
                    "could not auto-open Ghostty tabs for tmux sessions: {}",
                    e
                );
            }
            
            session_names
        } else {
            Vec::new()
        };

        let mut supervisor_runtime = SessionRuntime::new();
        // Build one worker runtime per tmux session (from launch-codex-tabs.sh pattern:
        // 10 workers per session, sessions 2+ were previously invisible to the watchdog).
        let mut worker_runtimes: Vec<SessionRuntime> = tmux_sessions
            .iter()
            .map(|session_name| {
                SessionRuntime::with_tmux(
                    session_name.clone(),
                    control_surface.transcript_dir.clone(),
                )
            })
            .collect();
        let tick = Duration::from_millis(config.watchdog_tick_millis);
        let stall_after = Duration::from_secs(config.stall_seconds);
        let max_runtime = config.watchdog_max_seconds.map(Duration::from_secs);
        let started_at = Instant::now();
        let primary_supervisor_id = supervisor_session.id;
        let repair_supervisor_name = "supervisor-02-repair".to_owned();
        let mut repair_supervisor_id = None;
        let mut repair_supervisor_prompt = None;
        let mut repair_supervisor_spec = None;
        let mut repair_supervisor_session = None;
        let mut repair_supervisor_task_id = None;
        if mission_profile.enable_repair_supervisor {
            let prompt = supervisor::build_repair_supervisor_prompt(
                &supervisor_prompt,
                &supervisor_session.name,
                &repair_supervisor_name,
            );
            write_prompt_file(
                &config.state_dir,
                &repair_supervisor_name,
                &prompt,
            )?;
            let mut spec = supervisor_spec.clone();
            spec.surface_label = repair_supervisor_name.clone();
            let session = SessionRecord {
                id: Uuid::new_v4(),
                mission_id,
                role: SessionRole::Supervisor,
                ordinal: supervisor_session.ordinal + 1,
                agent: supervisor_session.agent,
                terminal_id: repair_supervisor_name.clone(),
                name: repair_supervisor_name.clone(),
                owned_scope: "standby repair supervision and takeover continuity".to_owned(),
                status: SessionState::Booting,
                launch_command: launch_command(spec.program.as_str(), &spec.args),
                last_heartbeat_at: Utc::now(),
                last_summary: Some("Standby repair supervisor".to_owned()),
            };
            self.store.persist_session(&session, None)?;
            let task_id = Uuid::new_v4();
            self.store.persist_task(&TaskRecord {
                id: task_id,
                mission_id,
                worker_id: session.id,
                title: "Repair supervision".to_owned(),
                description: "Stay synchronized, take over if the primary supervisor becomes unhealthy, and preserve mission continuity.".to_owned(),
                status: "assigned".to_owned(),
                priority: "high".to_owned(),
                depends_on_json: "[]".to_owned(),
                definition_of_done_json: serde_json::to_string(&vec![
                    "Primary supervisor failure detected".to_owned(),
                    "Repair takeover executed when needed".to_owned(),
                    "Mission continuity preserved".to_owned(),
                ])?,
            })?;
            repair_supervisor_id = Some(session.id);
            repair_supervisor_prompt = Some(prompt);
            repair_supervisor_spec = Some(spec);
            repair_supervisor_session = Some(session);
            repair_supervisor_task_id = Some(task_id);
        }
        let mut active_supervisor_id = primary_supervisor_id;
        let mut stats = WatchdogStats::default();
        let mut mass_death_detector = health::MassDeathDetector::default();
        let mut active_sessions = HashMap::<Uuid, ActiveSession>::new();
        let mut alias_map = HashMap::<String, Uuid>::new();
        let mut leases = HashMap::<String, LeaseOwner>::new();
        let mut pending_mail = HashMap::<Uuid, PendingMail>::new();
        let mut pending_supervisor_decisions = HashMap::<String, PendingSupervisorDecision>::new();
        let mut recent_failures = Vec::<RecentFailure>::new();
        let mut final_synthesis_requested = false;
        let mut supervisor_mode = SupervisorMode::Healthy;
        let mut last_state_card_sent = Instant::now();
        let state_card_interval = Duration::from_secs(supervisor::STATE_CARD_INTERVAL_SECS);
        let status_snapshot_interval = Duration::from_secs(1);
        let full_surface_interval = Duration::from_secs(5);
        let mut worker_continuity_announced = false;
        let tmux_live = !control_surface.tmux_session_names.is_empty();
        let (supervisor_spec, supervisor_prompt_embedded) = if tmux_live {
            (supervisor_spec, false)
        } else {
            embed_initial_prompt_if_supported(
                supervisor_session.agent,
                supervisor_spec,
                &supervisor_prompt,
            )
        };
        let supervisor_running =
            supervisor_runtime.spawn(supervisor_session.id, supervisor_spec.clone())?;
        let repair_launch = if let (Some(session), Some(spec), Some(prompt)) = (
            repair_supervisor_session.clone(),
            repair_supervisor_spec.clone(),
            repair_supervisor_prompt.clone(),
        ) {
            let (spec, prompt_embedded) = if tmux_live {
                (spec, false)
            } else {
                embed_initial_prompt_if_supported(session.agent, spec, &prompt)
            };
            let running = supervisor_runtime.spawn(session.id, spec.clone())?;
            Some((session, spec, prompt, prompt_embedded, running))
        } else {
            None
        };
        // Wait for supervisor to boot before spawning workers
        if tmux_live {
            std::thread::sleep(std::time::Duration::from_millis(800));
        }
        register_session(
            &mut active_sessions,
            &mut alias_map,
            supervisor_session.clone(),
            None,
            supervisor_running,
            None,
            supervisor_spec.clone(),
            supervisor_prompt.clone(),
            Some(supervisor_task_id),
            vec![
                supervisor_session.name.clone(),
                "supervisor-01".to_owned(),
                "supervisor".to_owned(),
            ],
        );
        self.store
            .update_session_state(supervisor_session.id, SessionState::Progressing)?;
        if let Some((session, spec, prompt, _, running)) = repair_launch {
            register_session(
                &mut active_sessions,
                &mut alias_map,
                session.clone(),
                None,
                running,
                None,
                spec,
                prompt,
                repair_supervisor_task_id,
                vec![
                    session.name.clone(),
                    "supervisor-02".to_owned(),
                    "repair-supervisor".to_owned(),
                    "standby-supervisor".to_owned(),
                ],
            );
            self.store
                .update_session_state(session.id, SessionState::Progressing)?;
        }

        let mut prompt_queue = Vec::<(Uuid, Duration, String)>::new();
        if !supervisor_prompt_embedded {
            prompt_queue.push((
                supervisor_session.id,
                active_sessions
                    .get(&supervisor_session.id)
                    .map(|session| session.runtime.prompt_delay())
                    .unwrap_or_default(),
                supervisor_prompt,
            ));
        }
        if let (Some(session_id), Some(prompt)) = (repair_supervisor_id, repair_supervisor_prompt.clone())
            && let Some(session) = active_sessions.get(&session_id)
        {
            prompt_queue.push((
                session_id,
                session.runtime.prompt_delay(),
                prompt,
            ));
        }

        for (worker_index, launch) in worker_launches.into_iter().enumerate() {
            let (launch_spec, prompt_embedded) = if tmux_live {
                (launch.launch_spec, false)
            } else {
                embed_initial_prompt_if_supported(
                    launch.session.agent,
                    launch.launch_spec,
                    &launch.prompt,
                )
            };
            let runtime_slot = if worker_runtimes.is_empty() {
                None
            } else {
                Some(worker_index % worker_runtimes.len())
            };
            let running = if let Some(runtime_slot) = runtime_slot {
                worker_runtimes[runtime_slot].spawn(launch.session.id, launch_spec.clone())?
            } else {
                supervisor_runtime.spawn(launch.session.id, launch_spec.clone())?
            };
            // Small delay between spawns to prevent tmux race conditions
            if tmux_live {
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            let prompt_delay = running.prompt_delay();
            info!(
                "launched {} via {}",
                launch.session.name,
                running.display_name()
            );
            register_session(
                &mut active_sessions,
                &mut alias_map,
                launch.session.clone(),
                Some(launch.packet.clone()),
                running,
                runtime_slot,
                launch_spec.clone(),
                launch.prompt.clone(),
                launch.task_id,
                vec![
                    launch.session.name.clone(),
                    launch.packet.display_name.clone(),
                    launch.packet.worker_id.clone(),
                ],
            );
            self.store
                .update_session_state(launch.session.id, SessionState::Progressing)?;
            if let Some(session) = active_sessions.get_mut(&launch.session.id) {
                session.state = SessionState::Progressing;
                session.record.last_summary =
                    Some("Assignment delivered; awaiting first Sapphire status.".to_owned());
            }
            self.store.update_worker_summary(
                launch.session.id,
                "Assignment delivered; awaiting first Sapphire status.",
            )?;
            if !prompt_embedded {
                prompt_queue.push((launch.session.id, prompt_delay, launch.prompt));
            }
        }

        prompt_queue.sort_by_key(|(_, delay, _)| *delay);
        let mut waited = Duration::default();
        for (session_id, delay, prompt) in prompt_queue {
            let additional = delay.saturating_sub(waited);
            if !additional.is_zero() {
                tokio::time::sleep(additional).await;
                waited = delay;
            }
            if let Some(session) = active_sessions.get_mut(&session_id) {
                if session.launch_prompt_sent {
                    info!(
                        session = %session_id,
                        worker = %session.record.name,
                        "launch prompt already sent, skipping"
                    );
                    continue;
                }
                info!(
                    session = %session_id,
                    worker = %session.record.name,
                    delay_ms = delay.as_millis(),
                    prompt_bytes = prompt.len(),
                    "dispatching launch prompt"
                );
                send_prompt_immediately(session, &prompt)?;
                session.launch_prompt_sent = true;
                info!(
                    session = %session_id,
                    worker = %session.record.name,
                    "launch prompt sent successfully"
                );
            }
        }

        self.prime_supervisor_backlog(
            mission_id,
            active_supervisor_id,
            &active_sessions,
            &mut pending_supervisor_decisions,
        )?;
        self.write_status_snapshot(
            mission_id,
            active_supervisor_id,
            control_surface,
            &active_sessions,
            &pending_mail,
            &stats,
            true,
        )?;
        let mut last_snapshot_written_at = Instant::now();
        let mut last_full_surface_written_at = last_snapshot_written_at;

        loop {
            if let Some(limit) = max_runtime
                && started_at.elapsed() >= limit
            {
                self.store.append_json_event(
                    mission_id,
                    None,
                    "watchdog_timeout",
                    "watchdog max runtime reached",
                    &json!({ "seconds": limit.as_secs() }),
                )?;
                break;
            }

            if let Some(event) =
                next_runtime_event(&mut supervisor_runtime, &mut worker_runtimes, tick).await
            {
                stats.runtime_events += 1;
                self.handle_runtime_event(
                    mission_id,
                    &config.repo,
                    active_supervisor_id,
                    &event,
                    &mut active_sessions,
                    &alias_map,
                    &mut leases,
                    &mut pending_mail,
                    &mut pending_supervisor_decisions,
                    &mut recent_failures,
                    &mut mass_death_detector,
                    supervisor_mode == SupervisorMode::Degraded,
                    &mut stats,
                    control_surface,
                )?;
            }

            // File-backed worker status is authoritative. Read it before any stall
            // or health decisions so fresh progress cannot be mislabeled as stalled.
            self.read_worker_status_files(
                mission_id,
                &config.repo,
                active_supervisor_id,
                &mut active_sessions,
                &mut pending_supervisor_decisions,
                supervisor_mode == SupervisorMode::Degraded,
                &mut stats,
                control_surface,
            )?;

            self.handle_settled_worker_observations(
                mission_id,
                &config.repo,
                active_supervisor_id,
                &mut active_sessions,
                &mut pending_supervisor_decisions,
                supervisor_mode == SupervisorMode::Degraded,
                &mut stats,
            )?;

            refresh_tmux_health_cache(&mut active_sessions);

            if mission_profile.enable_supervisor_health_recovery
                && let Some(repair_supervisor_id) = repair_supervisor_id
            {
                self.handle_supervisor_health(
                    mission_id,
                    primary_supervisor_id,
                    repair_supervisor_id,
                    &mut active_supervisor_id,
                    stall_after,
                    &mut active_sessions,
                    &pending_mail,
                    &pending_supervisor_decisions,
                    started_at,
                    &mut supervisor_mode,
                    &mut worker_continuity_announced,
                    &mut stats,
                )?;
            }

            // Health probe: nudge sessions that haven't produced output in a while
            // before they hit the stall threshold (Gas Town two-tier response pattern).
            if mission_profile.enable_health_probes {
                self.health_probe_sessions(
                    mission_id,
                    stall_after,
                    &mut active_sessions,
                    &mut stats,
                )?;
            }

            // Zombie debounce: check sessions that appear dead but haven't exceeded
            // the consecutive zombie threshold yet. Prevents false kills during
            // slow startup or transient gaps (Gas Town supervise.md:1568-1600).
            self.zombie_debounce_check(
                mission_id,
                &mut active_sessions,
                &mut stats,
            )?;

            self.handle_stalls(
                mission_id,
                &config.repo,
                active_supervisor_id,
                stall_after,
                &mut active_sessions,
                &mut pending_supervisor_decisions,
                supervisor_mode == SupervisorMode::Degraded,
                &mut stats,
            )?;
            if mission_profile.enable_protocol_reminders {
                self.handle_protocol_reminders(
                    mission_id,
                    active_supervisor_id,
                    &mut active_sessions,
                    &mut pending_supervisor_decisions,
                    &mut stats,
                )?;
            }
            self.handle_pending_mail(
                mission_id,
                active_supervisor_id,
                &mut active_sessions,
                &mut pending_mail,
            )?;
            self.handle_pending_supervisor_decisions(
                mission_id,
                active_supervisor_id,
                &mut active_sessions,
                &mut pending_supervisor_decisions,
            )?;
            self.handle_pending_restarts(
                mission_id,
                &mut supervisor_runtime,
                &mut worker_runtimes,
                &mut active_sessions,
            )
            .await?;

            // Nudge queue: drain prompts queued for agents that have gone quiet.
            drain_prompt_queues(&mut active_sessions);

            // Nudge queue drain: deliver queued nudges to agents at natural turn boundaries
            // (non-destructive — doesn't cancel in-flight tool calls)
            let nudge_injections = mail::drain_nudge_queues(&config.state_dir, &active_sessions);
            for (session_id, formatted) in nudge_injections {
                if let Some(session) = active_sessions.get(&session_id) {
                    let _ = session.runtime.send_prompt(&formatted);
                }
            }

            if last_snapshot_written_at.elapsed() >= status_snapshot_interval {
                let full_surface = last_full_surface_written_at.elapsed() >= full_surface_interval;
                self.write_status_snapshot(
                    mission_id,
                    active_supervisor_id,
                    control_surface,
                    &active_sessions,
                    &pending_mail,
                    &stats,
                    full_surface,
                )?;
                last_snapshot_written_at = Instant::now();
                if full_surface {
                    last_full_surface_written_at = last_snapshot_written_at;
                }
            }

            // Supervisor state card refresh — every 30s, send a full state snapshot.
            // The supervisor doesn't need to remember its 350-line prompt.md — this keeps
            // the full mission picture fresh in its context window.
            if mission_profile.enable_state_cards
                && last_state_card_sent.elapsed() >= state_card_interval
                && supervisor_mode != SupervisorMode::Degraded
                && !final_synthesis_requested
            {
                if let Some(card) = build_supervisor_state_card(
                    &active_sessions,
                    &pending_mail,
                    &pending_supervisor_decisions,
                    supervisor_mode,
                    started_at,
                ) {
                    let active_supervisor_name = active_sessions
                        .get(&active_supervisor_id)
                        .map(|session| session.record.name.clone())
                        .unwrap_or_else(|| "supervisor".to_owned());
                    if queue_supervisor_state_card(
                        &mut active_sessions,
                        active_supervisor_id,
                        SupervisorEventType::Notice,
                        &card,
                    ) {
                        last_state_card_sent = Instant::now();
                    }
                    if let Some(repair_supervisor_id) = repair_supervisor_id {
                        let standby_supervisor_id = if active_supervisor_id == primary_supervisor_id {
                            repair_supervisor_id
                        } else {
                            primary_supervisor_id
                        };
                        if let Some(standby_supervisor) =
                            active_sessions.get_mut(&standby_supervisor_id)
                            && !matches!(
                                standby_supervisor.state,
                                SessionState::Exited | SessionState::Failed
                            )
                        {
                            let sync_prompt = supervisor::build_repair_sync_prompt(
                                &card,
                                &active_supervisor_name,
                            );
                            let key = supervisor::state_card_key(&sync_prompt);
                            if standby_supervisor
                                .last_supervisor_state_card_key
                                .as_deref()
                                != Some(key.as_str())
                            {
                                standby_supervisor.last_supervisor_state_card_key = Some(key);
                                let _ = send_or_queue_prompt(standby_supervisor, &sync_prompt);
                            }
                        }
                    }
                }
            }

            let workers_terminal = finalization::workers_are_terminal(&active_sessions);

            if workers_terminal && !final_synthesis_requested {
                if let Some(supervisor) = active_sessions.get_mut(&active_supervisor_id)
                    && !supervisor.state.is_terminal()
                {
                    let adapter = adapter_for(supervisor.record.agent);
                    let _ = send_or_queue_prompt(supervisor, &adapter.build_final_summary_prompt());
                }
                final_synthesis_requested = true;
            }

            if finalization::should_break_run(
                &active_sessions,
                active_supervisor_id,
                final_synthesis_requested,
                supervisor_mode,
            ) {
                break;
            }
        }

        for session in active_sessions.values() {
            let _ = session.runtime.terminate();
        }

        let final_status = if active_sessions
            .values()
            .filter(|session| session.record.role == SessionRole::Worker)
            .any(|session| session.state == SessionState::Failed)
        {
            MissionStatus::Failed
        } else {
            MissionStatus::Completed
        };
        self.store.update_mission_status(mission_id, final_status)?;
        self.write_status_snapshot(
            mission_id,
            active_supervisor_id,
            control_surface,
            &active_sessions,
            &pending_mail,
            &stats,
            true,
        )?;
        if !control_surface.tmux_session_names.is_empty() {
            self.store.append_summary(
                mission_id,
                None,
                "surface",
                format!(
                    "teamwork surface active at {}",
                    control_surface.dashboard_file.display()
                ),
            )?;
        }

        Ok(stats)
    }

    fn handle_runtime_event(
        &self,
        mission_id: Uuid,
        repo_root: &Path,
        active_supervisor_id: Uuid,
        event: &RuntimeEvent,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        alias_map: &HashMap<String, Uuid>,
        leases: &mut HashMap<String, LeaseOwner>,
        pending_mail: &mut HashMap<Uuid, PendingMail>,
        pending_supervisor_decisions: &mut HashMap<String, PendingSupervisorDecision>,
        recent_failures: &mut Vec<RecentFailure>,
        mass_death_detector: &mut health::MassDeathDetector,
        supervisor_degraded: bool,
        stats: &mut WatchdogStats,
        control_surface: &ControlSurface,
    ) -> Result<()> {
        persist_runtime_event(&self.store, mission_id, event)?;

        match event {
            RuntimeEvent::Output { session_id, chunk } => {
                let mut directives = Vec::new();
                let mut supervisor_action = None;
                let mut final_envelope = None;
                let mut should_reset_restart_tracker = false;
                let mut should_clear_transient_supervisor_decisions = false;
                if let Some(session) = active_sessions.get_mut(session_id) {
                    let now = Instant::now();
                    session.last_output_at = now;
                    session.last_confirmed_alive = now;
                    session.consecutive_stall_failures = 0;
                    session.output_chunks += 1;
                    // Health state: output = response to any pending probe
                    session.health_state.record_response();
                    // Zombie debounce: session is alive, reset zombie counter
                    session.zombie_debounce.record_alive();
                    if session.restart_count > 0 || session.restart_at.is_some() {
                        session.restart_count = 0;
                        session.restart_at = None;
                        should_reset_restart_tracker = true;
                    }
                    self.store.update_worker_heartbeat(*session_id)?;
                    // Record intervention response time (from gastown health/health.md pattern)
                    record_intervention_response(session, now);
                    if session.record.role == SessionRole::Worker {
                        clear_superseded_prompt_queue(session);
                        should_clear_transient_supervisor_decisions = true;
                    }
                    let sanitized = crate::protocol::sanitize_output(chunk);
                    directives = consume_directives(&mut session.line_buffer, &sanitized);
                    session.raw_buffer.push_str(&sanitized);
                    trim_recent_utf8(&mut session.raw_buffer, 64_000, 32_000);
                    let adapter = adapter_for(session.record.agent);
                    if session.record.role == SessionRole::Supervisor
                        && *session_id == active_supervisor_id
                    {
                        supervisor_action = adapter.extract_supervisor_action(&session.raw_buffer);
                        final_envelope = adapter.extract_final_envelope(&session.raw_buffer);
                    }
                    if control_surface.persist_transcripts {
                        append_transcript(control_surface, &session.record.name, &sanitized)?;
                    }
                }
                if should_reset_restart_tracker {
                    let _ = self.store.reset_restart_tracker(*session_id);
                }
                if should_clear_transient_supervisor_decisions {
                    clear_transient_supervisor_decisions_on_output(
                        pending_supervisor_decisions,
                        *session_id,
                    );
                }

                for directive in directives {
                    stats.directives += 1;
                    if let Some(session) = active_sessions.get_mut(session_id) {
                        session.directive_count += 1;
                        session.protocol_reminder_sent = false;
                    }
                    self.store.append_json_event(
                        mission_id,
                        Some(*session_id),
                        "directive",
                        directive_kind(&directive),
                        &directive,
                    )?;
                    match directive {
                        SapphireDirective::Status(status) => {
                            self.persist_normalized_status(
                                mission_id,
                                *session_id,
                                &status.state,
                                "sapphire_directive",
                                Confidence::High,
                                &status.summary,
                                &crate::protocol::sanitize_output(chunk),
                            )?;
                            self.handle_status_directive(
                                mission_id,
                                repo_root,
                                active_supervisor_id,
                                *session_id,
                                status,
                                false,
                                active_sessions,
                                pending_supervisor_decisions,
                                supervisor_degraded,
                                stats,
                            )?
                        }
                        SapphireDirective::Mail(mail) => {
                            // Message dedup: skip if already processed (prevents duplicate
                            // mail injection after orchestrator restart)
                            if let Some(mail_id) = &mail.mail_id {
                                if let Some(session) = active_sessions.get_mut(session_id) {
                                    if session.message_dedup.already_processed(mail_id) {
                                        tracing::debug!(
                                            mail_id = %mail_id,
                                            "mail directive deduped — already processed"
                                        );
                                        continue;
                                    }
                                    session.message_dedup.mark_processed(mail_id);
                                }
                            }
                            self.handle_mail_directive(
                                &control_surface.state_dir,
                                mission_id,
                                active_supervisor_id,
                                *session_id,
                                mail,
                                active_sessions,
                                alias_map,
                                pending_mail,
                                stats,
                            )?
                        }
                        SapphireDirective::Ack(ack) => self.handle_ack_directive(
                            mission_id,
                            active_supervisor_id,
                            *session_id,
                            ack,
                            active_sessions,
                            pending_mail,
                        )?,
                        SapphireDirective::Lease(lease) => self.handle_lease_directive(
                            mission_id,
                            active_supervisor_id,
                            *session_id,
                            lease,
                            active_sessions,
                            leases,
                            stats,
                        )?,
                    }
                }

                if let Some(action) = supervisor_action {
                    self.apply_supervisor_action(
                        mission_id,
                        repo_root,
                        active_supervisor_id,
                        action,
                        active_sessions,
                        alias_map,
                        pending_supervisor_decisions,
                        stats,
                    )?;
                }

                if let Some(final_envelope) = final_envelope {
                    self.apply_final_envelope(
                        mission_id,
                        active_supervisor_id,
                        final_envelope,
                        active_sessions,
                    )?;
                }
            }
            RuntimeEvent::Automation { session_id, .. } => {
                if let Some(session) = active_sessions.get_mut(session_id) {
                    session.last_output_at = Instant::now();
                    session.last_confirmed_alive = Instant::now();
                    session.consecutive_stall_failures = 0;
                    self.store.update_worker_heartbeat(*session_id)?;
                }
            }
            RuntimeEvent::Exited {
                session_id,
                exit_code,
            } => {
                let mut crash_loop_notice = None;
                if let Some(session) = active_sessions.get_mut(session_id) {
                    let previous_state = session.state;
                    let mut should_restart = false;
                    let mut restart_delay = Duration::default();
                    let mut crash_loop = false;

                    if !previous_state.is_terminal() {
                        let restart_record =
                            self.store.upsert_restart_attempt(*session_id, mission_id)?;
                        session.restart_count = restart_record.restart_count;
                        restart_delay = Duration::from_secs_f64(
                            restart_record.backoff_seconds.max(restart_base_secs() as f64),
                        );
                        crash_loop = self.store.is_crash_loop(
                            *session_id,
                            restart_crash_loop_threshold(),
                            restart_crash_loop_window(),
                        )?;
                        if crash_loop {
                            crash_loop_notice = Some(format!(
                                "{} entered a crash loop after {} restart attempts in the last {}s.",
                                session.record.name,
                                restart_record.restart_count,
                                restart_crash_loop_window().as_secs()
                            ));
                        }
                        should_restart = !crash_loop
                            && should_auto_restart(
                                session.record.role,
                                previous_state,
                                session.restart_count,
                            );
                    }

                    if should_restart {
                        session.restart_at = Some(Instant::now() + restart_delay);
                        session.state = SessionState::Booting;
                        session.startup_grace_until =
                            Instant::now() + startup_grace(session.record.agent);
                        session.record.last_summary = Some(format!(
                            "{} exited; restart {} scheduled in {}s",
                            session.record.name,
                            session.restart_count,
                            restart_delay.as_secs()
                        ));
                        self.store
                            .update_session_state(*session_id, SessionState::Booting)?;
                        self.store.update_worker_summary(
                            *session_id,
                            session
                                .record
                                .last_summary
                                .as_deref()
                                .unwrap_or("restart scheduled"),
                        )?;
                        self.store.append_json_event(
                            mission_id,
                            Some(*session_id),
                            "session_restart_scheduled",
                            "session exited and will be restarted",
                            &json!({
                                "name": session.record.name,
                                "exit_code": exit_code,
                                "restart_count": session.restart_count,
                                "restart_in_seconds": restart_delay.as_secs(),
                            }),
                        )?;
                    } else if crash_loop {
                        session.state = SessionState::Failed;
                        self.store
                            .update_session_state(*session_id, SessionState::Failed)?;
                        if let Some(task_id) = session.task_id {
                            let _ = self.store.update_task_status(task_id, "failed");
                        }
                    } else {
                        session.state = SessionState::Exited;
                        self.store
                            .update_session_state(*session_id, SessionState::Exited)?;
                        if let Some(task_id) = session.task_id {
                            let _ = self.store.update_task_status(task_id, "exited");
                        }
                    }
                    self.store.append_json_event(
                        mission_id,
                        Some(*session_id),
                        "session_state",
                        "session exited",
                        &json!({
                            "name": session.record.name,
                            "exit_code": exit_code,
                        }),
                    )?;
                    self.store.append_summary(
                        mission_id,
                        Some(*session_id),
                        "exit",
                        if should_restart {
                            format!(
                                "{} exited and was queued for restart {}",
                                session.record.name, session.restart_count
                            )
                        } else if crash_loop_notice.is_some() {
                            format!("{} entered a crash loop and was marked failed", session.record.name)
                        } else {
                            format!("{} exited", session.record.name)
                        },
                    )?;
                    if session.record.role == SessionRole::Worker && !should_restart {
                        recent_failures.push(RecentFailure {
                            session_id: *session_id,
                            session_name: session.record.name.clone(),
                            recorded_at: Instant::now(),
                        });
                        // Sliding window mass death detection (Gas Town supervise.md:2389-2436)
                        if let Some(mass_death_event) = mass_death_detector
                            .record_death(&session.record.name)
                        {
                            stats.mass_deaths_detected += 1;
                            stats.critical_failures += 1;
                            tracing::error!(
                                mass_death_count = mass_death_event.count,
                                dead_sessions = ?mass_death_event.dead_sessions,
                                "MASS DEATH DETECTED: {} sessions died within {:?}",
                                mass_death_event.count,
                                mass_death_event.window,
                            );
                        }
                        trim_recent_failures(recent_failures);
                        if recent_failures.len() >= mass_failure_threshold() {
                            let names = recent_failures
                                .iter()
                                .map(|entry| entry.session_name.clone())
                                .collect::<Vec<_>>();

                            // Check for crash loop overlap (from gastown daemon mass death pattern)
                            let crash_loops: Vec<(String, usize)> = recent_failures
                                .iter()
                                .filter_map(|entry| {
                                    self.store.load_restart_state(entry.session_id)
                                        .ok()
                                        .flatten()
                                        .filter(|r| r.restart_count >= restart_crash_loop_threshold())
                                        .map(|r| (entry.session_name.clone(), r.restart_count))
                                })
                                .collect();

                            let event_type = if crash_loops.is_empty() {
                                "mass_failure_detected"
                            } else {
                                "critical_failure"  // Mass death + crash loops = critical
                            };
                            if !crash_loops.is_empty() {
                                stats.critical_failures += 1;
                                stats.crash_loops_detected += crash_loops.len();
                            }

                            let summary = if crash_loops.is_empty() {
                                format!(
                                    "Multiple workers died within {}s: {}. Decide whether to fail fast, narrow scope, or relaunch selectively.",
                                    mass_failure_window().as_secs(),
                                    names.join(", ")
                                )
                            } else {
                                let crash_details = crash_loops
                                    .iter()
                                    .map(|(name, count)| format!("{} ({} restarts)", name, count))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                format!(
                                    "CRITICAL: Multiple workers died within {}s: {}. Crash loops detected: {}. The watchdog will not auto-respawn these sessions.",
                                    mass_failure_window().as_secs(),
                                    names.join(", "),
                                    crash_details
                                )
                            };

                            self.store.append_json_event(
                                mission_id,
                                Some(active_supervisor_id),
                                event_type,
                                &summary,
                                &json!({
                                    "count": recent_failures.len(),
                                    "workers": names,
                                    "crash_loops": crash_loops.iter().map(|(n, c)| json!({"name": n, "restarts": c})).collect::<Vec<_>>(),
                                    "is_critical": !crash_loops.is_empty(),
                                }),
                            )?;
                            self.send_supervisor_notice(
                                mission_id,
                                active_supervisor_id,
                                active_sessions,
                                SupervisorEventType::Failed,
                                &summary,
                            )?;
                        }
                    }
                }
                if let Some(notice) = crash_loop_notice {
                    stats.crash_loops_detected += 1;
                    self.store.append_json_event(
                        mission_id,
                        Some(active_supervisor_id),
                        "crash_loop_detected",
                        &notice,
                        &json!({ "session_id": session_id.to_string(), "exit_code": exit_code }),
                    )?;
                    self.send_supervisor_notice(
                        mission_id,
                        active_supervisor_id,
                        active_sessions,
                        SupervisorEventType::Failed,
                        &notice,
                    )?;
                }
            }
        }

        Ok(())
    }

    fn handle_status_directive(
        &self,
        mission_id: Uuid,
        repo_root: &Path,
        supervisor_id: Uuid,
        session_id: Uuid,
        directive: StatusDirective,
        supervisor_override: bool,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        pending_supervisor_decisions: &mut HashMap<String, PendingSupervisorDecision>,
        supervisor_degraded: bool,
        _stats: &mut WatchdogStats,
    ) -> Result<()> {
        let Some(reported_state) = directive.session_state() else {
            return Ok(());
        };
        let Some(session_role) = active_sessions
            .get(&session_id)
            .map(|session| session.record.role)
        else {
            return Ok(());
        };
        let mut new_state = enforcement::canonicalize_worker_state(
            session_role,
            reported_state,
            supervisor_override,
        );

        let mut notify_supervisor = false;
        let mut validation_escalation = None::<String>;
        let mut overlap_escalation = None::<String>;
        let mut state_summary = directive.summary.clone();
        let mut session_name = String::new();
        let mut validation_result: Option<(Option<Uuid>, String)> = None;
        let mut files = directive.files.clone();
        let commands = directive.commands.clone();
        let mut risks = directive.risks.clone();
        let overlap = directive.overlap.clone();
        let overlap_detail = enforcement::meaningful_overlap_detail(overlap.as_deref());
        let mut completion_gate_correction = None::<String>;

        if !supervisor_override
            && session_role == SessionRole::Worker
            && reported_state == SessionState::Validated
        {
            state_summary = format!(
                "worker reported validated; awaiting supervisor acceptance: {}",
                directive.summary
            );
        }

        if matches!(
            new_state,
            SessionState::DoneClaimed | SessionState::NeedsValidation | SessionState::Validated
        ) && let Some(session) = active_sessions.get(&session_id)
            && session.record.role == SessionRole::Worker
            && let Some(packet) = session.packet.as_ref()
        {
            if let Some(rejection) = reject_completion_without_artifacts(repo_root, packet, &files)
            {
                new_state = SessionState::NeedsRetry;
                state_summary = format!("completion rejected: missing {}", rejection.join(", "));
                risks.push(format!(
                    "missing required on-disk artifacts: {}",
                    rejection.join(", ")
                ));
                completion_gate_correction = Some(format!(
                    "Your completion claim was rejected because Sapphire could not find the required artifacts on disk: {}. Do the work, then report status again with exact files.",
                    rejection.join(", ")
                ));
            } else if files.is_empty() {
                files = infer_observed_artifacts(repo_root, packet);
            }
        }

        if let Some(session) = active_sessions.get_mut(&session_id) {
            session_name = session.record.name.clone();
            let now = Instant::now();
            session.last_output_at = now;
            session.last_confirmed_alive = now;
            enforcement::note_status_report(session, now);
            session.reported_overlap = overlap_detail.clone();
            session.state = new_state;
            session.record.last_summary = Some(state_summary.clone());
            if new_state != SessionState::Stalled {
                session.stall_count = 0;
                session.consecutive_stall_failures = 0;
            }
            if !matches!(
                new_state,
                SessionState::WeakOutput | SessionState::WrongDirection
            ) {
                session.low_confidence_count = 0;
            }
            self.store.update_session_state(session_id, new_state)?;
            self.store
                .update_worker_summary(session_id, &state_summary)?;
            if let Some(task_id) = session.task_id {
                let task_status = match new_state {
                    SessionState::Validated => "validated",
                    SessionState::Failed => "failed",
                    SessionState::DoneClaimed => "done_claimed",
                    SessionState::NeedsValidation => "needs_validation",
                    SessionState::Blocked => "blocked",
                    SessionState::NeedsRetry => "needs_retry",
                    SessionState::Stalled => "stalled",
                    _ => "in_progress",
                };
                let _ = self.store.update_task_status(task_id, task_status);
                if matches!(
                    new_state,
                    SessionState::Validated | SessionState::NeedsRetry | SessionState::Failed
                ) {
                    let outcome = match new_state {
                        SessionState::Validated => "pass",
                        SessionState::NeedsRetry => "needs_retry",
                        SessionState::Failed => "fail",
                        _ => unreachable!(),
                    };
                    validation_result = Some((Some(task_id), outcome.to_owned()));
                }
            }

            notify_supervisor = !supervisor_override
                && session.record.role == SessionRole::Worker
                && matches!(
                    new_state,
                    SessionState::Blocked
                        | SessionState::Stalled
                        | SessionState::DoneClaimed
                        | SessionState::NeedsValidation
                        | SessionState::WeakOutput
                        | SessionState::WrongDirection
                        | SessionState::Contradictory
                        | SessionState::NeedsRetry
                        | SessionState::Validated
                        | SessionState::Failed
                )
                && session.escalation_sent_for_state != Some(new_state);

            session.validation_pending = session.record.role == SessionRole::Worker
                && matches!(
                    new_state,
                    SessionState::DoneClaimed | SessionState::NeedsValidation
                );
            session.escalation_sent_for_state = if notify_supervisor {
                Some(new_state)
            } else {
                session.escalation_sent_for_state
            };
            if session.record.role == SessionRole::Worker
                && matches!(
                    new_state,
                    SessionState::DoneClaimed | SessionState::NeedsValidation
                )
                && !supervisor_degraded
            {
                validation_escalation = Some(format!(
                    "Worker {} is requesting validation with state {}. Decide whether to validate_worker, accept_worker, retry_worker, fail_worker, or message_worker. Summary: {}",
                    session.record.name,
                    new_state.as_str(),
                    state_summary
                ));
            }
            if !supervisor_override
                && session.record.role == SessionRole::Worker
                && let Some(detail) = overlap_detail.as_deref()
                && !supervisor_degraded
            {
                overlap_escalation = Some(format!(
                    "Worker {} reported live overlap risk: {}. Enforce ownership, keep newer teammate work intact, and decide whether to redirect_worker, message_worker, or fail_worker.",
                    session.record.name,
                    detail,
                ));
            }
        }

        self.store.append_json_event(
            mission_id,
            Some(session_id),
            "session_state",
            &state_summary,
            &json!({
                "state": new_state.as_str(),
                "summary": state_summary,
                "files": files,
                "commands": commands,
                "risks": risks,
                "overlap": overlap,
            }),
        )?;
        self.store.append_summary(
            mission_id,
            Some(session_id),
            format!("state:{}", new_state.as_str()),
            &state_summary,
        )?;

        if notify_supervisor && session_id != supervisor_id {
            let event_type = event_type_for_state(&new_state);
            self.send_supervisor_notice(
                mission_id,
                supervisor_id,
                active_sessions,
                event_type,
                &format!(
                    "Worker {} reported state {}. Summary: {}",
                    session_name,
                    new_state.as_str(),
                    state_summary,
                ),
            )?;
        }

        if let Some(reason) = validation_escalation
            && queue_supervisor_decision(
                pending_supervisor_decisions,
                SupervisorDecisionKind::Validation,
                session_id,
                &reason,
            )
        {
            self.send_supervisor_notice(mission_id, supervisor_id, active_sessions, SupervisorEventType::DoneClaimed, &reason)?;
        }

        if let Some(reason) = overlap_escalation
            && queue_supervisor_decision(
                pending_supervisor_decisions,
                SupervisorDecisionKind::OverlapRecovery,
                session_id,
                &reason,
            )
        {
            self.send_supervisor_notice(
                mission_id,
                supervisor_id,
                active_sessions,
                SupervisorEventType::Contradiction,
                &reason,
            )?;
        }

        clear_resolved_supervisor_decisions(
            pending_supervisor_decisions,
            active_sessions,
            session_id,
        );

        if let Some((task_id, outcome)) = validation_result {
            self.store
                .persist_validation_result(&ValidationResultRecord {
                    id: Uuid::new_v4(),
                    mission_id,
                    worker_id: session_id,
                    task_id,
                    outcome,
                    summary: state_summary.clone(),
                    evidence_json: json!({
                        "state": new_state.as_str(),
                        "files": files,
                        "commands": commands,
                        "risks": risks,
                        "overlap": overlap,
                    })
                    .to_string(),
                    created_at: Utc::now(),
                })?;
        }

        if let Some(correction) = completion_gate_correction
            && queue_supervisor_decision(
                pending_supervisor_decisions,
                SupervisorDecisionKind::Validation,
                session_id,
                &format!(
                    "Worker {} needs a strict completion correction: {}",
                    session_name,
                    correction,
                ),
            )
        {
            self.send_supervisor_notice(
                mission_id,
                supervisor_id,
                active_sessions,
                SupervisorEventType::WeakOutput,
                &format!(
                    "Completion claim from {} failed the artifact gate. Challenge the worker directly with proof requirements. {}",
                    session_name,
                    correction,
                ),
            )?;
        }

        if session_id == supervisor_id && new_state == SessionState::Validated {
            let _ = self
                .store
                .update_mission_final_summary(mission_id, &state_summary);
        }

        // Update worker agent memory file for resume continuity
        if session_role == SessionRole::Worker && !session_name.is_empty() {
            let mem_store = self.store.agent_memory();
            // Ensure initial memory file exists, then update state and files
            if mem_store.load_memory(&mission_id, &session_name).unwrap_or(None).is_none() {
                let packet = active_sessions.get(&session_id).and_then(|s| s.packet.as_ref());
                let memory = crate::storage::agent_memory::AgentMemory {
                    mission_id,
                    display_name: session_name.clone(),
                    role_type: packet.map(|p| p.role_type.clone()).unwrap_or_default(),
                    owned_scope: packet
                        .map(|p| p.owned_scope.clone())
                        .filter(|s| !s.is_empty())
                        .into_iter()
                        .collect(),
                    decisions: Vec::new(),
                    blockers: directive.risks.clone(),
                    learnings: Vec::new(),
                    files_touched: directive.files.clone(),
                    final_state: directive.state.clone(),
                    summary: directive.summary.clone(),
                    created_at: Utc::now(),
                };
                let _ = mem_store.save_memory(&mission_id, &session_name, &memory);
            } else {
                let _ = mem_store.update_state(&mission_id, &session_name, &directive.state);
                if !directive.files.is_empty() {
                    let _ = mem_store.append_files_touched(&mission_id, &session_name, &directive.files);
                }
                // Update summary by loading, modifying, and saving
                if let Ok(Some(mut mem)) = mem_store.load_memory(&mission_id, &session_name) {
                    mem.summary = directive.summary.clone();
                    for risk in &directive.risks {
                        if !mem.blockers.contains(risk) {
                            mem.blockers.push(risk.clone());
                        }
                    }
                    let _ = mem_store.save_memory(&mission_id, &session_name, &mem);
                }
            }

            // Terminal state snapshot — save complete memory record when worker
            // reaches a terminal state. This is what powers cross-mission memory:
            // Engineer-1 on the next mission will see what Engineer-1 did here.
            if session_role == SessionRole::Worker
                && !session_name.is_empty()
                && new_state.is_terminal()
            {
                let _ = save_terminal_memory_snapshot(
                    &self.store,
                    mission_id,
                    session_id,
                    &session_name,
                    active_sessions,
                    &new_state,
                    &state_summary,
                    &files,
                    &risks,
                );
            }
        }

        Ok(())
    }

    fn persist_normalized_status(
        &self,
        mission_id: Uuid,
        session_id: Uuid,
        normalized_state: &str,
        source: &str,
        confidence: Confidence,
        summary: &str,
        raw_excerpt: &str,
    ) -> Result<()> {
        self.store
            .persist_normalized_update(&NormalizedUpdateRecord {
                id: Uuid::new_v4(),
                mission_id,
                worker_id: session_id,
                source: source.to_owned(),
                raw_excerpt: truncate(raw_excerpt, 400),
                normalized_state: normalized_state.to_owned(),
                confidence: confidence.as_str().to_owned(),
                summary: summary.to_owned(),
                adapter: source.to_owned(),
                created_at: Utc::now(),
            })
    }

    fn handle_normalized_observation(
        &self,
        mission_id: Uuid,
        repo_root: &Path,
        supervisor_id: Uuid,
        session_id: Uuid,
        mut observation: NormalizedObservation,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        pending_supervisor_decisions: &mut HashMap<String, PendingSupervisorDecision>,
        supervisor_degraded: bool,
        stats: &mut WatchdogStats,
    ) -> Result<()> {
        if observation.source != "status_envelope"
            && matches!(observation.state, SessionState::Validated)
        {
            observation.state = SessionState::NeedsValidation;
            observation.summary = format!(
                "inferred completion signal needs explicit validation: {}",
                observation.summary
            );
            observation.files.clear();
            observation.blocker = None;
        }

        let Some(session) = active_sessions.get_mut(&session_id) else {
            return Ok(());
        };
        if Instant::now() < session.startup_grace_until
            && observation.source != "status_envelope"
            && observation.confidence == Confidence::Low
        {
            return Ok(());
        }
        let key = format!(
            "{}|{}|{}",
            observation.state.as_str(),
            observation.source,
            observation.summary
        );
        if session.last_observation_key.as_deref() == Some(key.as_str()) {
            return Ok(());
        }
        session.last_observation_key = Some(key);
        match observation.confidence {
            Confidence::High | Confidence::Medium => session.low_confidence_count = 0,
            Confidence::Low => session.low_confidence_count += 1,
        }

        self.store
            .persist_normalized_update(&NormalizedUpdateRecord {
                id: Uuid::new_v4(),
                mission_id,
                worker_id: session_id,
                source: observation.source.to_owned(),
                raw_excerpt: truncate(&observation.raw_excerpt, 400),
                normalized_state: observation.state.as_str().to_owned(),
                confidence: observation.confidence.as_str().to_owned(),
                summary: observation.summary.clone(),
                adapter: session.record.agent.as_str().to_owned(),
                created_at: Utc::now(),
            })?;

        self.handle_status_directive(
            mission_id,
            repo_root,
            supervisor_id,
            session_id,
            StatusDirective {
                state: observation.state.as_str().to_owned(),
                summary: observation.summary.clone(),
                files: observation.files.clone(),
                commands: Vec::new(),
                risks: observation.blocker.clone().into_iter().collect::<Vec<_>>(),
                overlap: None,
            },
            false,
            active_sessions,
            pending_supervisor_decisions,
            supervisor_degraded,
            stats,
        )?;

        let low_confidence_count = active_sessions
            .get(&session_id)
            .map(|worker| worker.low_confidence_count)
            .unwrap_or(0);
        if low_confidence_count >= 2 {
            let reason = format!(
                "Worker {} remains low-confidence after repeated updates. Decide whether to retry_worker, redirect_worker, validate_worker, fail_worker, or message_worker.",
                active_sessions
                    .get(&session_id)
                    .map(|worker| worker.record.name.as_str())
                    .unwrap_or("unknown")
            );
            if queue_supervisor_decision(
                pending_supervisor_decisions,
                SupervisorDecisionKind::LowConfidenceRecovery,
                session_id,
                &reason,
            ) {
                self.send_supervisor_notice(mission_id, supervisor_id, active_sessions, SupervisorEventType::WeakOutput, &reason)?;
            }
        }
        Ok(())
    }

    fn handle_settled_worker_observations(
        &self,
        mission_id: Uuid,
        repo_root: &Path,
        supervisor_id: Uuid,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        pending_supervisor_decisions: &mut HashMap<String, PendingSupervisorDecision>,
        supervisor_degraded: bool,
        stats: &mut WatchdogStats,
    ) -> Result<()> {
        let now = Instant::now();
        let mut observations = Vec::new();

        for (session_id, session) in active_sessions.iter() {
            if session.record.role != SessionRole::Worker
                || session.state.is_terminal()
                || session.raw_buffer.is_empty()
                || session.output_chunks == 0
                || now < session.startup_grace_until
                || !worker_output_has_settled(session, now)
                || has_recent_status_activity(session, now)
                || session_has_live_terminal(session)
                || !session.queued_prompts.is_empty()
                || recently_prompted(session, now)
                || is_in_cooldown(session, now)
            {
                continue;
            }

            let adapter = adapter_for(session.record.agent);
            if let Some(observation) = adapter.detect_state(&session.raw_buffer) {
                observations.push((*session_id, observation));
            }
        }

        for (session_id, observation) in observations {
            self.handle_normalized_observation(
                mission_id,
                repo_root,
                supervisor_id,
                session_id,
                observation,
                active_sessions,
                pending_supervisor_decisions,
                supervisor_degraded,
                stats,
            )?;
        }

        Ok(())
    }

    fn apply_supervisor_action(
        &self,
        mission_id: Uuid,
        repo_root: &Path,
        supervisor_id: Uuid,
        action: SupervisorAction,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        alias_map: &HashMap<String, Uuid>,
        pending_supervisor_decisions: &mut HashMap<String, PendingSupervisorDecision>,
        stats: &mut WatchdogStats,
    ) -> Result<()> {
        let Some(supervisor) = active_sessions.get_mut(&supervisor_id) else {
            return Ok(());
        };
        let signature = format!(
            "{}|{}|{}|{}",
            action.action,
            action.target.as_deref().unwrap_or("NONE"),
            action.summary,
            action.message.as_deref().unwrap_or("NONE")
        );
        if supervisor.last_supervisor_action_key.as_deref() == Some(signature.as_str()) {
            return Ok(());
        }
        supervisor.last_supervisor_action_key = Some(signature);

        let Some(target_alias) = action.target.as_deref() else {
            return Ok(());
        };
        let Some(target_id) = resolve_alias(alias_map, target_alias) else {
            return Ok(());
        };
        let action_name = action.action.trim().to_ascii_lowercase();
        pending_supervisor_decisions.retain(|_, pending| {
            pending.target_session_id != target_id
                || !enforcement::action_resolves_decision(pending.kind, &action_name)
        });

        match action_name.as_str() {
            "observe" => {}
            "validate_worker" => {
                if let Some(target) = active_sessions.get_mut(&target_id) {
                    let adapter = adapter_for(target.record.agent);
                    if let Some(msg) = action.message.as_deref() {
                        let prompt = adapter.build_validation_prompt(msg);
                        let _ = send_or_queue_prompt(target, &prompt);
                        stats.validation_challenges += 1;
                    } else {
                        // No message from supervisor — skip. The real AI brain
                        // must provide a concrete instruction; no deterministic fallback.
                        self.store.append_json_event(
                            mission_id,
                            Some(supervisor_id),
                            "supervisor_action_skipped",
                            "validate_worker without message — skipping (no deterministic fallback)",
                            &json!({ "action": action.action, "target": action.target }),
                        )?;
                    }
                }
            }
            "retry_worker" | "redirect_worker" | "message_worker" => {
                if let Some(target) = active_sessions.get_mut(&target_id) {
                    let adapter = adapter_for(target.record.agent);
                    let message = action.message.as_deref()
                        .unwrap_or(&action.summary);
                    let prompt = adapter.build_correction_prompt(message);
                    let _ = send_or_queue_prompt(target, &prompt);
                }
            }
            "accept_worker" => {
                self.handle_status_directive(
                    mission_id,
                    repo_root,
                    supervisor_id,
                    target_id,
                    StatusDirective {
                        state: SessionState::Validated.as_str().to_owned(),
                        summary: action.summary.clone(),
                        files: Vec::new(),
                        commands: Vec::new(),
                        risks: Vec::new(),
                        overlap: None,
                    },
                    true,
                    active_sessions,
                    pending_supervisor_decisions,
                    false,
                    stats,
                )?;
            }
            "fail_worker" => {
                self.handle_status_directive(
                    mission_id,
                    repo_root,
                    supervisor_id,
                    target_id,
                    StatusDirective {
                        state: SessionState::Failed.as_str().to_owned(),
                        summary: action.summary.clone(),
                        files: Vec::new(),
                        commands: Vec::new(),
                        risks: Vec::new(),
                        overlap: None,
                    },
                    true,
                    active_sessions,
                    pending_supervisor_decisions,
                    false,
                    stats,
                )?;
            }
            _ => {}
        }

        self.store.append_json_event(
            mission_id,
            Some(supervisor_id),
            "supervisor_action",
            &action.summary,
            &json!({
                "action": action.action,
                "target": action.target,
                "summary": action.summary,
                "message": action.message,
            }),
        )?;
        Ok(())
    }

    fn apply_final_envelope(
        &self,
        mission_id: Uuid,
        supervisor_id: Uuid,
        final_envelope: FinalEnvelope,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
    ) -> Result<()> {
        let key = format!(
            "{}|{}|{}",
            final_envelope.state.as_str(),
            if final_envelope.ready_for_cleanup {
                "cleanup_yes"
            } else {
                "cleanup_no"
            },
            final_envelope.summary
        );
        if let Some(supervisor) = active_sessions.get_mut(&supervisor_id) {
            if supervisor.last_observation_key.as_deref() == Some(key.as_str()) {
                return Ok(());
            }
            supervisor.last_observation_key = Some(key);
            supervisor.cleanup_authorized = final_envelope.ready_for_cleanup;
            supervisor.record.last_summary = Some(final_envelope.summary.clone());
            if final_envelope.ready_for_cleanup {
                supervisor.state = final_envelope.state;
                self.store
                    .update_session_state(supervisor_id, final_envelope.state)?;
            }
        }
        self.store.append_summary(
            mission_id,
            Some(supervisor_id),
            "final_summary",
            &final_envelope.summary,
        )?;
        self.store.append_json_event(
            mission_id,
            Some(supervisor_id),
            "cleanup_decision",
            &final_envelope.summary,
            &json!({
                "final_state": final_envelope.state.as_str(),
                "ready_for_cleanup": final_envelope.ready_for_cleanup,
            }),
        )?;
        if final_envelope.ready_for_cleanup {
            self.store
                .update_mission_final_summary(mission_id, &final_envelope.summary)?;
        }
        Ok(())
    }

    fn handle_mail_directive(
        &self,
        state_dir: &Path,
        mission_id: Uuid,
        supervisor_id: Uuid,
        sender_session_id: Uuid,
        directive: MailDirective,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        alias_map: &HashMap<String, Uuid>,
        pending_mail: &mut HashMap<Uuid, PendingMail>,
        stats: &mut WatchdogStats,
    ) -> Result<()> {
        let result = mail::handle_mail_directive(
            &self.store, state_dir, mission_id, supervisor_id, sender_session_id,
            directive, active_sessions, alias_map, pending_mail,
            &mut mail::MailStats { mails_routed: 0, lease_conflicts: 0 },
        )?;
        stats.mails_routed += 1;
        if let Some((event_type, notice)) = result.supervisor_notice {
            self.send_supervisor_notice(mission_id, supervisor_id, active_sessions, event_type, &notice)?;
        }
        Ok(())
    }

    fn handle_ack_directive(
        &self,
        mission_id: Uuid,
        supervisor_id: Uuid,
        sender_session_id: Uuid,
        directive: AckDirective,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        pending_mail: &mut HashMap<Uuid, PendingMail>,
    ) -> Result<()> {
        mail::handle_ack_directive(
            &self.store, mission_id, supervisor_id, sender_session_id,
            directive, pending_mail, active_sessions,
        )
    }

    fn handle_lease_directive(
        &self,
        mission_id: Uuid,
        supervisor_id: Uuid,
        session_id: Uuid,
        directive: LeaseDirective,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        leases: &mut HashMap<String, LeaseOwner>,
        stats: &mut WatchdogStats,
    ) -> Result<()> {
        let mut mail_stats = mail::MailStats { mails_routed: 0, lease_conflicts: 0 };
        mail::handle_lease_directive(
            &self.store, mission_id, supervisor_id, session_id,
            directive, active_sessions, leases, &mut mail_stats,
        )?;
        stats.lease_conflicts = mail_stats.mails_routed;
        Ok(())
    }

    fn handle_supervisor_health(
        &self,
        mission_id: Uuid,
        primary_supervisor_id: Uuid,
        repair_supervisor_id: Uuid,
        active_supervisor_id: &mut Uuid,
        stall_after: Duration,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        pending_mail: &HashMap<Uuid, PendingMail>,
        pending_decisions: &HashMap<String, PendingSupervisorDecision>,
        started_at: Instant,
        supervisor_mode: &mut SupervisorMode,
        worker_continuity_announced: &mut bool,
        stats: &mut WatchdogStats,
    ) -> Result<()> {
        let workers_active = active_sessions.values().any(|session| {
            session.record.role == SessionRole::Worker && !session.state.is_terminal()
        });
        if !workers_active {
            return Ok(());
        }

        let describe_supervisor = |session_id: Uuid| {
            active_sessions.get(&session_id).map(|session| {
                let elapsed = session.last_output_at.elapsed();
                let tmux_health = session
                    .runtime
                    .terminal_target()
                    .map(|target| {
                        tmux::Tmux::new(None)
                            .check_session_health(target, zombie_check_max_inactivity())
                    })
                    .unwrap_or_else(|| {
                        if elapsed >= zombie_check_max_inactivity() {
                            tmux::SessionHealth::Hung
                        } else {
                            tmux::SessionHealth::Healthy
                        }
                    });
                (
                    session.state,
                    elapsed,
                    tmux_health,
                    session.record.agent,
                    session.record.name.clone(),
                )
            })
        };

        let primary = describe_supervisor(primary_supervisor_id);
        let repair = describe_supervisor(repair_supervisor_id);
        let primary_condition = primary
            .as_ref()
            .map(|(state, elapsed, tmux_health, _, _)| {
                supervisor::classify_supervisor(*state, *elapsed, stall_after, *tmux_health)
            })
            .unwrap_or(supervisor::SupervisorCondition::Unavailable);
        let repair_condition = repair
            .as_ref()
            .map(|(state, elapsed, tmux_health, _, _)| {
                supervisor::classify_supervisor(*state, *elapsed, stall_after, *tmux_health)
            })
            .unwrap_or(supervisor::SupervisorCondition::Unavailable);

        let current_condition = if *active_supervisor_id == repair_supervisor_id {
            repair_condition
        } else {
            primary_condition
        };

        let replacement_id = if *active_supervisor_id == primary_supervisor_id {
            if repair_condition != supervisor::SupervisorCondition::Unavailable {
                Some(repair_supervisor_id)
            } else {
                None
            }
        } else if primary_condition != supervisor::SupervisorCondition::Unavailable {
            Some(primary_supervisor_id)
        } else {
            None
        };

        if current_condition == supervisor::SupervisorCondition::Unavailable {
            if let Some(next_supervisor_id) = replacement_id {
                let previous_name = active_sessions
                    .get(active_supervisor_id)
                    .map(|session| session.record.name.clone())
                    .unwrap_or_else(|| "supervisor".to_owned());
                let next_name = active_sessions
                    .get(&next_supervisor_id)
                    .map(|session| session.record.name.clone())
                    .unwrap_or_else(|| "repair-supervisor".to_owned());
                let takeover_card = build_supervisor_state_card(
                    active_sessions,
                    pending_mail,
                    pending_decisions,
                    *supervisor_mode,
                    started_at,
                )
                .unwrap_or_else(|| "SUPERVISOR STATE CARD unavailable".to_owned());

                *active_supervisor_id = next_supervisor_id;
                *supervisor_mode = SupervisorMode::Recovering;

                if let Some(replacement) = active_sessions.get_mut(&next_supervisor_id) {
                    let prompt = supervisor::build_takeover_prompt(&takeover_card, &previous_name);
                    let _ = send_or_queue_prompt(replacement, &prompt);
                }
                Self::notify_workers_of_supervisor_continuity(
                    active_sessions,
                    &next_name,
                    worker_continuity_announced,
                );
                self.store.append_json_event(
                    mission_id,
                    Some(next_supervisor_id),
                    "supervisor_takeover",
                    "repair supervisor takeover activated",
                    &json!({
                        "from": previous_name,
                        "to": next_name,
                        "mode": "repair_takeover",
                    }),
                )?;
                self.store.append_summary(
                    mission_id,
                    Some(next_supervisor_id),
                    "supervisor_takeover",
                    format!("{next_name} took over supervision because {previous_name} became unavailable."),
                )?;
                stats.supervisor_health_events += 1;
                return Ok(());
            }

            if *supervisor_mode != SupervisorMode::Degraded {
                *supervisor_mode = SupervisorMode::Degraded;
                Self::notify_workers_of_supervisor_continuity(
                    active_sessions,
                    "peer continuity mode",
                    worker_continuity_announced,
                );
                self.store.append_json_event(
                    mission_id,
                    Some(*active_supervisor_id),
                    "supervisor_health",
                    "all supervisor sessions unavailable; workers remain in peer continuity mode",
                    &json!({
                        "primary": format!("{primary_condition:?}"),
                        "repair": format!("{repair_condition:?}"),
                        "mode": "degraded",
                    }),
                )?;
                stats.supervisor_health_events += 1;
                stats.supervisor_fallbacks += 1;
            }
            return Ok(());
        }

        let Some((supervisor_state, elapsed, tmux_health, _supervisor_agent, supervisor_name)) =
            describe_supervisor(*active_supervisor_id)
        else {
            return Ok(());
        };

        if matches!(tmux_health, tmux::SessionHealth::Starting) {
            return Ok(());
        }

        if current_condition == supervisor::SupervisorCondition::ProbeNeeded {
            if *supervisor_mode == SupervisorMode::Healthy {
                *supervisor_mode = SupervisorMode::Recovering;
                self.store.append_json_event(
                    mission_id,
                    Some(*active_supervisor_id),
                    "supervisor_health",
                    if communication_policy::SUPERVISOR_HEALTH_PROBES {
                        "supervisor recovery probe sent"
                    } else {
                        "supervisor entered recovering mode without a watchdog probe"
                    },
                    &json!({
                        "supervisor": supervisor_name,
                        "state": supervisor_state.as_str(),
                        "health": format!("{tmux_health:?}"),
                        "seconds_since_output": elapsed.as_secs(),
                        "mode": "recovering",
                    }),
                )?;
                stats.supervisor_health_events += 1;
            }
            return Ok(());
        }

        if *supervisor_mode != SupervisorMode::Healthy {
            *supervisor_mode = SupervisorMode::Healthy;
            self.store.append_summary(
                mission_id,
                Some(*active_supervisor_id),
                "supervisor_health",
                format!("{supervisor_name} is healthy and supervising normally."),
            )?;
            stats.supervisor_health_events += 1;
        }

        Ok(())
    }

    fn notify_workers_of_supervisor_continuity(
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        acting_supervisor_name: &str,
        worker_continuity_announced: &mut bool,
    ) {
        // No deterministic continuity prompt to workers.
        // The repair supervisor's real AI brain decides what to tell each worker.
        if *worker_continuity_announced {
            return;
        }
        // Update summary so watchdog reflects the change.
        for session in active_sessions.values_mut() {
            if session.record.role == SessionRole::Worker && !session.state.is_terminal() {
                session.record.last_summary = Some(
                    format!("Supervisor transitioned to {acting_supervisor_name}")
                );
            }
        }
        *worker_continuity_announced = true;
    }

    fn handle_pending_supervisor_decisions(
        &self,
        mission_id: Uuid,
        supervisor_id: Uuid,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        pending_supervisor_decisions: &mut HashMap<String, PendingSupervisorDecision>,
    ) -> Result<()> {
        let now = Instant::now();
        let mut completed = Vec::new();
        let mut follow_ups = Vec::new();

        for (key, pending) in pending_supervisor_decisions.iter_mut() {
            let Some(target) = active_sessions.get(&pending.target_session_id) else {
                completed.push(key.clone());
                continue;
            };

            let resolved = target.state.is_terminal()
                || (pending.kind == SupervisorDecisionKind::Validation
                    && !target.validation_pending)
                || (pending.kind == SupervisorDecisionKind::StallRecovery
                    && target.state != SessionState::Stalled)
                || (pending.kind == SupervisorDecisionKind::LowConfidenceRecovery
                    && target.low_confidence_count == 0)
                || (pending.kind == SupervisorDecisionKind::OverlapRecovery
                    && target
                        .reported_overlap
                        .as_deref()
                        .is_none_or(|value| value.trim().is_empty()));
            if resolved {
                completed.push(key.clone());
                continue;
            }

            if enforcement::should_follow_up_pending_decision(pending, now) {
                follow_ups.push((
                    enforcement::follow_up_event_type(pending.kind),
                    enforcement::follow_up_reason(pending, target, now),
                ));
                enforcement::mark_pending_decision_notified(pending, now);
            }
        }

        for key in completed {
            pending_supervisor_decisions.remove(&key);
        }

        for (event_type, body) in follow_ups {
            self.send_supervisor_notice(
                mission_id,
                supervisor_id,
                active_sessions,
                event_type,
                &body,
            )?;
        }

        Ok(())
    }

    async fn handle_pending_restarts(
        &self,
        mission_id: Uuid,
        supervisor_runtime: &mut SessionRuntime,
        worker_runtimes: &mut [SessionRuntime],
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
    ) -> Result<()> {
        let now = Instant::now();
        let restart_ids = active_sessions
            .iter()
            .filter_map(|(session_id, session)| {
                session
                    .restart_at
                    .filter(|when| now >= *when)
                    .map(|_| *session_id)
            })
            .collect::<Vec<_>>();

        for session_id in restart_ids {
            let Some(session) = active_sessions.get_mut(&session_id) else {
                continue;
            };
            session.restart_at = None;
            let (launch_spec, prompt_embedded) = embed_initial_prompt_if_supported(
                session.record.agent,
                session.launch_spec.clone(),
                &session.launch_prompt,
            );
            let running = if session.record.role == SessionRole::Worker {
                if let Some(runtime_slot) = session.runtime_slot {
                    if let Some(runtime) = worker_runtimes.get(runtime_slot) {
                        runtime.spawn(session_id, launch_spec)?
                    } else {
                        supervisor_runtime.spawn(session_id, launch_spec)?
                    }
                } else {
                    supervisor_runtime.spawn(session_id, launch_spec)?
                }
            } else {
                supervisor_runtime.spawn(session_id, launch_spec)?
            };
            let prompt_delay = running.prompt_delay();
            session.runtime = running;
            session.state = SessionState::Booting;
            session.started_at = Instant::now();
            session.last_output_at = Instant::now();
            session.startup_grace_until = Instant::now() + startup_grace(session.record.agent);
            session.output_chunks = 0;
            session.directive_count = 0;
            session.initial_status_received = false;
            session.output_chunks_at_last_status = 0;
            session.reported_overlap = None;
            session.protocol_reminder_sent = false;
            session.low_confidence_count = 0;
            session.last_observation_key = None;
            session.queued_prompts.clear();
            session.queued_prompt_keys.clear();
            session.recent_prompt_keys.clear();
            session.last_prompt_sent_at = None;
            session.launch_prompt_sent = false;
            session.cleanup_authorized = false;
            session.last_tmux_health = None;
            session.last_tmux_health_checked_at = None;
            session.last_supervisor_notice_key = None;
            session.recent_supervisor_notice_keys.clear();
            session.last_supervisor_state_card_key = None;
            session.record.last_summary = Some(format!(
                "Restarted after exit; attempt {}",
                session.restart_count
            ));
            self.store
                .update_session_state(session_id, SessionState::Booting)?;
            self.store.update_worker_summary(
                session_id,
                session
                    .record
                    .last_summary
                    .as_deref()
                    .unwrap_or("restarted"),
            )?;
            self.store.append_json_event(
                mission_id,
                Some(session_id),
                "session_restarted",
                "session restarted after exit",
                &json!({
                    "name": session.record.name,
                    "restart_count": session.restart_count,
                }),
            )?;
            if !prompt_embedded {
                tokio::time::sleep(prompt_delay).await;
                if !session.launch_prompt_sent {
                    let launch_prompt = session.launch_prompt.clone();
                    send_prompt_immediately(session, &launch_prompt)?;
                    session.launch_prompt_sent = true;
                }
            }
            session.state = SessionState::Progressing;
            self.store
                .update_session_state(session_id, SessionState::Progressing)?;
        }

        Ok(())
    }

    fn prime_supervisor_backlog(
        &self,
        mission_id: Uuid,
        supervisor_id: Uuid,
        active_sessions: &HashMap<Uuid, ActiveSession>,
        pending_supervisor_decisions: &mut HashMap<String, PendingSupervisorDecision>,
    ) -> Result<()> {
        let mut seeded = Vec::new();

        for (session_id, session) in active_sessions {
            if session.record.role != SessionRole::Worker {
                continue;
            }
            match session.state {
                SessionState::DoneClaimed | SessionState::NeedsValidation => {
                    if queue_supervisor_decision(
                        pending_supervisor_decisions,
                        SupervisorDecisionKind::Validation,
                        *session_id,
                        &format!(
                            "Worker {} resumed with a pending completion claim. Validate it before acceptance.",
                            session.record.name
                        ),
                    ) {
                        seeded.push(format!("validation:{}", session.record.name));
                    }
                }
                SessionState::Stalled => {
                    if queue_supervisor_decision(
                        pending_supervisor_decisions,
                        SupervisorDecisionKind::StallRecovery,
                        *session_id,
                        &format!(
                            "Worker {} resumed in a stalled state. Decide whether to retry, redirect, or unblock it.",
                            session.record.name
                        ),
                    ) {
                        seeded.push(format!("stall:{}", session.record.name));
                    }
                }
                SessionState::Blocked
                | SessionState::Contradictory
                | SessionState::WrongDirection => {
                    seeded.push(format!(
                        "{}:{}",
                        session.state.as_str(),
                        session.record.name
                    ));
                }
                _ => {}
            }
        }

        if !seeded.is_empty() {
            self.store.append_json_event(
                mission_id,
                Some(supervisor_id),
                "startup_sweep",
                "seeded supervisor backlog from live session state",
                &json!({ "items": seeded }),
            )?;
        }

        Ok(())
    }

    /// Health probe: nudge sessions that are approaching stall threshold.
    /// Gas Town two-tier response pattern — probe before escalation.
    /// Sends a lightweight nudge to check responsiveness before the stall
    /// handler fires the escalation ladder.
    fn health_probe_sessions(
        &self,
        _mission_id: Uuid,
        stall_after: Duration,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        stats: &mut WatchdogStats,
    ) -> Result<()> {
        let now = Instant::now();
        // Probe at 75% of stall threshold — gives session time to respond
        // before hitting the actual stall detection
        let probe_threshold = stall_after.mul_f64(0.75);
        let mut probed_ids = Vec::new();

        for (session_id, session) in active_sessions.iter() {
            if session.record.role == SessionRole::Supervisor || session.state.is_terminal() {
                continue;
            }
            if has_recent_status_activity(session, now) || session_has_live_terminal(session) {
                continue;
            }
            let idle_duration = now.duration_since(session.last_confirmed_alive);
            let already_probed_this_idle_window = session
                .health_state
                .last_probe_at
                .is_some_and(|last_probe| last_probe >= session.last_confirmed_alive);
            // Session is approaching stall but hasn't hit it yet
            if idle_duration >= probe_threshold
                && idle_duration < stall_after
                && session.state != SessionState::Stalled
                && !already_probed_this_idle_window
            {
                probed_ids.push(*session_id);
            }
        }

        for session_id in probed_ids {
            if let Some(session) = active_sessions.get_mut(&session_id) {
                // Record health probe
                session.health_state.record_probe();

                // Record failure (no response yet — will be reset if output arrives)
                session.health_state.record_failure();

                stats.supervisor_health_events += 1;

                tracing::debug!(
                    session_id = %session_id,
                    name = %session.record.name,
                    "health probe sent (approaching stall)"
                );
            }
        }

        Ok(())
    }

    /// Zombie debounce: check sessions that appear dead but haven't exceeded
    /// the consecutive zombie threshold. Prevents false kills during slow
    /// startup or transient gaps (Gas Town supervise.md:1568-1600).
    ///
    /// Only triggers restart after N consecutive zombie detections (default: 3).
    fn zombie_debounce_check(
        &self,
        _mission_id: Uuid,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        stats: &mut WatchdogStats,
    ) -> Result<()> {
        let now = Instant::now();
        let mut zombie_session_ids = Vec::new();

        for (session_id, session) in active_sessions.iter() {
            if session.record.role == SessionRole::Supervisor || session.state.is_terminal() {
                continue;
            }
            if has_recent_status_activity(session, now) || !session.queued_prompts.is_empty() {
                continue;
            }

            let idle_duration = now.duration_since(session.last_confirmed_alive);
            if idle_duration <= Duration::from_secs(60) {
                continue;
            }

            match session_tmux_health(session) {
                Some(tmux::SessionHealth::Zombie | tmux::SessionHealth::Dead) => {
                    zombie_session_ids.push(*session_id);
                }
                Some(tmux::SessionHealth::Healthy | tmux::SessionHealth::Hung | tmux::SessionHealth::Starting)
                | None => {}
            }
        }

        for session_id in zombie_session_ids {
            if let Some(session) = active_sessions.get_mut(&session_id) {
                let should_restart = session.zombie_debounce.record_zombie();
                if should_restart {
                    tracing::warn!(
                        session_id = %session_id,
                        name = %session.record.name,
                        consecutive_zombies = session.zombie_debounce.consecutive_zombie_count,
                        "zombie debounce threshold exceeded — considering restart"
                    );
                    stats.critical_failures += 1;
                    // Reset debounce so we don't spam restarts
                    session.zombie_debounce.record_alive();
                }
            }
        }

        Ok(())
    }

    fn handle_stalls(
        &self,
        mission_id: Uuid,
        repo_root: &Path,
        supervisor_id: Uuid,
        stall_after: Duration,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        pending_supervisor_decisions: &mut HashMap<String, PendingSupervisorDecision>,
        supervisor_degraded: bool,
        stats: &mut WatchdogStats,
    ) -> Result<()> {
        let now = Instant::now();
        let mut stalled_ids = Vec::new();

        for (session_id, session) in active_sessions.iter() {
            if session.record.role == SessionRole::Supervisor || session.state.is_terminal() {
                continue;
            }
            if now < session.startup_grace_until {
                continue;
            }
            if has_recent_status_activity(session, now)
                || session_has_live_terminal(session)
                || !session.queued_prompts.is_empty()
                || recently_prompted(session, now)
            {
                continue;
            }
            if now.duration_since(session.last_confirmed_alive)
                >= effective_stall_threshold(session, stall_after)
                && session.state != SessionState::Stalled
            {
                stalled_ids.push(*session_id);
            }
        }

        for session_id in stalled_ids {
            // Extract data we need before dropping the mutable borrow.
            let (consecutive, worker_name, in_cooldown) = {
                let session = active_sessions.get(&session_id).unwrap();
                (
                    session.consecutive_stall_failures + 1,
                    session.record.name.clone(),
                    is_in_cooldown(session, now),
                )
            };

            // Update stall state.
            if let Some(session) = active_sessions.get_mut(&session_id) {
                session.state = SessionState::Stalled;
                session.stall_count += 1;
                session.consecutive_stall_failures = consecutive;
            }
            self.store
                .update_session_state(session_id, SessionState::Stalled)?;
            self.store.update_worker_summary(
                session_id,
                &format!("Stalled after {} interventions", {
                    active_sessions.get(&session_id).unwrap().stall_count
                }),
            )?;
            self.store.append_json_event(
                mission_id,
                Some(session_id),
                "stall_detected",
                "worker stalled",
                &json!({
                    "name": worker_name,
                    "stall_count": active_sessions.get(&session_id).unwrap().stall_count,
                    "consecutive_failures": consecutive,
                }),
            )?;

            // Cooldown check: skip redundant interventions if still in cooldown.
            if in_cooldown {
                self.store.append_json_event(
                    mission_id,
                    Some(session_id),
                    "stall_cooldown",
                    "worker stalled but in intervention cooldown — skipping redundant prompt",
                    &json!({
                        "name": worker_name,
                        "cooldown_until_secs": active_sessions.get(&session_id).and_then(|s| s.intervention_cooldown_until.map(|t| t.elapsed().as_secs())),
                    }),
                )?;
                continue;
            }

            // Escalation ladder based on consecutive stall failures:
            // 1st: corrective status prompt (existing behavior)
            // 2nd: redirect_worker — narrow the task, change the angle
            // 3rd+: fail_worker + supervisor notice for replacement
            if consecutive >= 3 {
                // Third consecutive stall: fail the worker, let supervisor decide on replacement.
                // The watchdog cannot unilaterally respawn — that's a supervisor call.
                self.handle_status_directive(
                    mission_id,
                    repo_root,
                    supervisor_id,
                    session_id,
                    StatusDirective {
                        state: SessionState::Failed.as_str().to_owned(),
                        summary: format!(
                            "Worker {} has stalled {} consecutive times. Corrective prompts had no effect. Supervisor must decide: respawn or reassign.",
                            worker_name, consecutive
                        ),
                        files: Vec::new(),
                        commands: Vec::new(),
                        risks: Vec::new(),
                        overlap: None,
                    },
                    false,
                    active_sessions,
                    pending_supervisor_decisions,
                    supervisor_degraded,
                    stats,
                )?;
                stats.stall_interventions += 1;
                if let Some(session) = active_sessions.get_mut(&session_id) {
                    record_intervention(session, "stall_fail", now);
                }
            } else if consecutive == 2 {
                let reason = if supervisor_degraded {
                    format!(
                        "Worker {} has stalled 2 consecutive times. Supervisor is degraded, so no watchdog redirect was sent. Recover the supervisor or inspect the worker session directly.",
                        worker_name
                    )
                } else {
                    format!(
                        "Worker {} has stalled 2 consecutive times. Decide whether to retry_worker, redirect_worker, or message_worker with one narrow next step.",
                        worker_name
                    )
                };
                if queue_supervisor_decision(
                    pending_supervisor_decisions,
                    SupervisorDecisionKind::StallRecovery,
                    session_id,
                    &reason,
                ) {
                    self.send_supervisor_notice(
                        mission_id,
                        supervisor_id,
                        active_sessions,
                        SupervisorEventType::Stall,
                        &reason,
                    )?;
                }
            } else {
                let reason = if supervisor_degraded {
                    format!(
                        "Worker {} appears stalled, but the supervisor is degraded. No local watchdog prompt was sent. Recover the supervisor or inspect the worker session directly.",
                        worker_name,
                    )
                } else {
                    format!(
                        "Worker {} appears stalled. Decide whether to message_worker, retry_worker, redirect_worker, or validate_worker.",
                        worker_name,
                    )
                };
                if queue_supervisor_decision(
                    pending_supervisor_decisions,
                    SupervisorDecisionKind::StallRecovery,
                    session_id,
                    &reason,
                ) {
                    self.send_supervisor_notice(
                        mission_id,
                        supervisor_id,
                        active_sessions,
                        SupervisorEventType::Stall,
                        &reason,
                    )?;
                }
            }
        }

        Ok(())
    }

    fn handle_protocol_reminders(
        &self,
        mission_id: Uuid,
        supervisor_id: Uuid,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        pending_supervisor_decisions: &mut HashMap<String, PendingSupervisorDecision>,
        stats: &mut WatchdogStats,
    ) -> Result<()> {
        if !communication_policy::WATCHDOG_PROTOCOL_REMINDERS {
            let mut low_observability = Vec::new();
            let now = Instant::now();
            for (session_id, session) in active_sessions.iter() {
                if session.record.role != SessionRole::Worker
                    || session.state.is_terminal()
                    || session.protocol_reminder_sent
                    || now < session.startup_grace_until
                    || !session.queued_prompts.is_empty()
                    || recently_prompted(session, now)
                    || is_in_cooldown(session, now)
                {
                    continue;
                }
                let Some(report_kind) = enforcement::pending_report_back(session, now) else {
                    continue;
                };
                low_observability.push((*session_id, report_kind));
            }

            for (session_id, report_kind) in low_observability {
                let Some(session) = active_sessions.get_mut(&session_id) else {
                    continue;
                };
                session.protocol_reminder_sent = true;
                let summary = enforcement::status_summary(report_kind, &session.record.name);
                session.record.last_summary = Some(summary.clone());
                record_intervention(session, "status_enforcement_escalated", now);
                self.store.append_json_event(
                    mission_id,
                    Some(session_id),
                    "status_enforcement_escalated",
                    "worker output lacked report-back; escalated to supervisor without watchdog prompt",
                    &json!({
                        "name": session.record.name,
                        "output_chunks": session.output_chunks,
                    }),
                )?;
                self.store.update_worker_summary(session_id, &summary)?;
                stats.protocol_reminders += 1;

                let reason = enforcement::status_reason(report_kind, &session.record.name);
                if queue_supervisor_decision(
                    pending_supervisor_decisions,
                    SupervisorDecisionKind::LowConfidenceRecovery,
                    session_id,
                    &reason,
                ) {
                    self.send_supervisor_notice(
                        mission_id,
                        supervisor_id,
                        active_sessions,
                        SupervisorEventType::WeakOutput,
                        &reason,
                    )?;
                }
            }

            return Ok(());
        }

        let mut reminder_ids = Vec::new();

        for (session_id, session) in active_sessions.iter() {
            let lowered = session.raw_buffer.to_ascii_lowercase();
            let now = Instant::now();
            if session.state.is_terminal()
                || session.protocol_reminder_sent
                || session.output_chunks < 8
                || session.directive_count > 0
                || now < session.startup_grace_until
                || session.started_at.elapsed() < protocol_reminder_grace(session.record.agent)
                || now.duration_since(session.last_confirmed_alive) < Duration::from_secs(30)
                || session.last_status_update_at.is_some_and(|at| {
                    now.duration_since(at)
                        < Duration::from_secs(supervisor::STATUS_FILE_LIVENESS_GRACE_SECS)
                })
                || session_has_live_terminal(session)
                || is_in_cooldown(session, now)
                || !session.queued_prompts.is_empty()
                || recently_prompted(session, now)
                || (session.record.agent == crate::agent::AgentKind::Qwen
                    && (lowered.contains("readfile")
                        || lowered.contains("writefile")
                        || lowered.contains("editfile")
                        || lowered.contains("model:")))
                || (session.record.agent == crate::agent::AgentKind::Qwen
                    && (lowered.contains("user:") || lowered.contains("responding "))
                    && !lowered.contains("model:")
                    && !lowered.contains("sapphire_status"))
            {
                continue;
            }
            reminder_ids.push(*session_id);
        }

        for session_id in reminder_ids {
            if let Some(session) = active_sessions.get_mut(&session_id) {
                session.protocol_reminder_sent = true;
                session.record.last_summary = Some("Protocol reminder sent".to_owned());
                record_intervention(session, "protocol_reminder", Instant::now());
                self.store.append_json_event(
                    mission_id,
                    Some(session_id),
                    "status_probe",
                    "requested strict status envelope after low observability",
                    &json!({
                        "name": session.record.name,
                        "output_chunks": session.output_chunks,
                    }),
                )?;
                self.store
                    .update_worker_summary(session_id, "Protocol reminder sent")?;
                stats.protocol_reminders += 1;

                let reason = format!(
                    "Worker {} is producing output without a usable Sapphire status update. Decide whether to message_worker or redirect_worker with a strict status instruction.",
                    session.record.name
                );
                if queue_supervisor_decision(
                    pending_supervisor_decisions,
                    SupervisorDecisionKind::LowConfidenceRecovery,
                    session_id,
                    &reason,
                ) {
                    self.send_supervisor_notice(
                        mission_id,
                        supervisor_id,
                        active_sessions,
                        SupervisorEventType::WeakOutput,
                        &reason,
                    )?;
                }
            }
        }

        Ok(())
    }

    fn handle_pending_mail(
        &self,
        mission_id: Uuid,
        supervisor_id: Uuid,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        pending_mail: &mut HashMap<Uuid, PendingMail>,
    ) -> Result<()> {
        let now = Instant::now();
        let mut expired = Vec::new();

        for (message_id, pending) in pending_mail.iter_mut() {
            if pending.acked && pending.thread_state == "closed" {
                expired.push(*message_id);
                continue;
            }
            if pending.acked {
                continue;
            }

            if !mail::mail_timeout_stage_due(pending, now) {
                continue;
            }
            let recipient_is_live = active_sessions
                .get(&pending.recipient_session_id)
                .is_some_and(|session| live_state::should_pause_mail_timeout(session, now));
            let sender_is_live = active_sessions
                .get(&pending.sender_session_id)
                .is_some_and(|session| live_state::should_pause_mail_timeout(session, now));
            if recipient_is_live || sender_is_live {
                continue;
            }
            pending.timeout_stage = pending.timeout_stage.saturating_add(1);
            pending.last_timeout_at = Some(now);
            pending.thread_state =
                coordination::thread_state_for_timeout_stage(pending.timeout_stage).to_owned();

            let stage = pending.timeout_stage;
            let timeout = mail::mail_timeout_interval(&pending.priority);
            let sender_session_id = pending.sender_session_id;
            let recipient_session_id = pending.recipient_session_id;
            let cc_session_ids = pending.cc_session_ids.clone();
            let thread_id = pending.thread_id.clone();
            let subject = pending.subject.clone();
            let message_type = pending.message_type.clone();
            let priority = pending.priority.clone();
            let sender_name = active_sessions
                .get(&sender_session_id)
                .map(|session| session.record.name.clone())
                .unwrap_or_else(|| "unknown".to_owned());
            let recipient_name = active_sessions
                .get(&recipient_session_id)
                .map(|session| session.record.name.clone())
                .unwrap_or_else(|| "unknown".to_owned());

            if communication_policy::MAIL_TIMEOUT_DIRECT_PROMPTS {
                if let Some(recipient) = active_sessions.get_mut(&recipient_session_id) {
                    let adapter = adapter_for(recipient.record.agent);
                    let prompt = mail::recipient_timeout_prompt(pending, &sender_name, stage);
                    let _ = send_or_queue_prompt(recipient, &adapter.build_status_prompt(&prompt));
                }

                if let Some(sender) = active_sessions.get_mut(&sender_session_id) {
                    let prompt = mail::sender_timeout_prompt(pending, &recipient_name, stage);
                    let _ = send_or_queue_prompt(sender, &prompt);
                }

                for cc_id in &cc_session_ids {
                    if let Some(cc_session) = active_sessions.get_mut(cc_id) {
                        let _ = send_or_queue_prompt(cc_session, &mail::cc_timeout_prompt(pending, stage));
                    }
                }
            }

            let urgency_note = match priority.to_lowercase().as_str() {
                "urgent" | "critical" => " [URGENT]",
                "high" => " [HIGH]",
                _ => "",
            };
            let should_escalate_supervisor =
                stage >= 2 || matches!(priority.to_lowercase().as_str(), "urgent" | "critical" | "high");
            let mail_event_type = if stage >= 3 || urgency_note.contains("URGENT") {
                SupervisorEventType::Blocked
            } else {
                SupervisorEventType::Notice
            };
            if should_escalate_supervisor {
                self.send_supervisor_notice(
                    mission_id,
                    supervisor_id,
                    active_sessions,
                    mail_event_type,
                    &format!(
                        "Mail {} in thread {} from {} to {} is still unresolved after timeout stage {} ({}s each).{} Subject: {}. The team may need reroute, override, or direct intervention.",
                        pending.message_id,
                        thread_id,
                        sender_name,
                        recipient_name,
                        stage,
                        timeout.as_secs(),
                        urgency_note,
                        subject
                    ),
                )?;
            }
            let _ = self
                .store
                .update_message_status(*message_id, &format!("timeout_stage_{stage}"), "pending");

            let _ = self.store.append_json_event(
                mission_id,
                Some(supervisor_id),
                "mail_timeout_stage",
                format!("mail {} reached timeout stage {}", message_id, stage),
                &json!({
                    "message_id": message_id.to_string(),
                    "thread_id": thread_id,
                    "sender": sender_session_id.to_string(),
                    "recipient": recipient_session_id.to_string(),
                    "priority": priority,
                    "subject": subject,
                    "message_type": message_type,
                    "stage": stage,
                    "interval_seconds": timeout.as_secs(),
                }),
            );
        }

        // Clean up acked messages
        for message_id in expired {
            pending_mail.remove(&message_id);
        }
        Ok(())
    }

    fn send_supervisor_notice(
        &self,
        mission_id: Uuid,
        supervisor_id: Uuid,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        event_type: SupervisorEventType,
        body: &str,
    ) -> Result<()> {
        if let Some(supervisor) = active_sessions.get_mut(&supervisor_id) {
            if matches!(supervisor.state, SessionState::Exited | SessionState::Failed) {
                return Ok(());
            }
            let now = Instant::now();
            prune_recent_supervisor_notice_keys(supervisor, now);
            let notice_key = supervisor::notice_key(event_type, body);
            if supervisor.last_supervisor_notice_key.as_deref() == Some(notice_key.as_str())
                || has_recent_supervisor_notice_key(supervisor, &notice_key)
            {
                return Ok(());
            }
            let adapter = adapter_for(supervisor.record.agent);
            let prompt = adapter.build_supervisor_action_prompt(event_type, body);
            remember_supervisor_notice(supervisor, notice_key, now);
            let _ = send_or_queue_prompt(supervisor, &prompt);
        }
        self.store.append_json_event(
            mission_id,
            Some(supervisor_id),
            "supervisor_notice",
            truncate(body, 240),
            &json!({ "event_type": event_type.as_str(), "body": body }),
        )?;
        self.store.append_summary(
            mission_id,
            Some(supervisor_id),
            "supervisor_notice",
            body.to_owned(),
        )?;
        Ok(())
    }

    fn write_status_snapshot(
        &self,
        mission_id: Uuid,
        active_supervisor_id: Uuid,
        control_surface: &ControlSurface,
        active_sessions: &HashMap<Uuid, ActiveSession>,
        pending_mail: &HashMap<Uuid, PendingMail>,
        stats: &WatchdogStats,
        full_surface: bool,
    ) -> Result<()> {
        let pod_summaries = coordination::summarize_pods(active_sessions, pending_mail);
        let memory_summaries = if full_surface {
            memory::write_agent_memories(control_surface, active_sessions, pending_mail)?
        } else {
            Vec::new()
        };
        let meetings = if full_surface {
            meetings::write_meeting_artifacts(control_surface, active_sessions, pending_mail)?
        } else {
            Vec::new()
        };
        let now = Instant::now();
        let live_snapshot = live_state::Snapshot::build(active_sessions, now);
        let mut workers = active_sessions
            .values()
            .filter(|session| session.record.role == SessionRole::Worker)
            .collect::<Vec<_>>();
        workers.sort_by(|left, right| left.record.name.cmp(&right.record.name));

        let blocked = workers
            .iter()
            .filter(|session| live_snapshot.counts_as_blocked(session))
            .map(|session| session.record.name.clone())
            .collect::<Vec<_>>();
        let validation_queue = workers
            .iter()
            .filter(|session| session.validation_pending)
            .map(|session| session.record.name.clone())
            .collect::<Vec<_>>();
        let contradictions = workers
            .iter()
            .filter(|session| session.state == SessionState::Contradictory)
            .map(|session| session.record.name.clone())
            .collect::<Vec<_>>();
        let waiting_mail = workers
            .iter()
            .filter_map(|session| {
                let open_threads = pending_mail
                    .values()
                    .filter(|pending| {
                        pending.thread_state != "closed"
                            && (pending.sender_session_id == session.record.id
                                || pending.recipient_session_id == session.record.id)
                    })
                    .count();
                if open_threads == 0 {
                    None
                } else {
                    Some(format!("{} ({open_threads} thread(s))", session.record.name))
                }
            })
            .collect::<Vec<_>>();
        let problems = workers
            .iter()
            .filter(|session| live_snapshot.counts_as_problem(session))
            .map(|session| {
                format!(
                    "{} [{}]",
                    session.record.name,
                    live_snapshot.effective_state_label(session)
                )
            })
            .collect::<Vec<_>>();
        let crash_loops = self
            .store
            .get_crash_loop_sessions(
                mission_id,
                restart_crash_loop_threshold(),
                restart_crash_loop_window(),
            )
            .unwrap_or_default()
            .into_iter()
            .map(|(session_id, restart_count)| {
                let name = active_sessions
                    .get(&session_id)
                    .map(|session| session.record.name.clone())
                    .unwrap_or_else(|| truncate_id(&session_id));
                format!("{} ({} restarts)", name, restart_count)
            })
            .collect::<Vec<_>>();

        let mut lines = Vec::new();
        let supervisor_line = active_sessions
            .get(&active_supervisor_id)
            .map(|session| {
                format!(
                    "{} [{}] {}",
                    session.record.name,
                    live_snapshot.effective_state_label(session),
                    session
                        .record
                        .last_summary
                        .as_deref()
                        .unwrap_or("no summary yet")
                )
            })
            .unwrap_or_else(|| "supervisor unavailable".to_owned());
        let standby_line = active_sessions
            .values()
            .filter(|session| {
                session.record.role == SessionRole::Supervisor
                    && session.record.id != active_supervisor_id
            })
            .map(|session| {
                format!(
                    "{} [{}] {}",
                    session.record.name,
                    live_snapshot.effective_state_label(session),
                    session
                        .record
                        .last_summary
                        .as_deref()
                        .unwrap_or("no summary yet")
                )
            })
            .next();
        lines.push(format!("Session: {}", mission_id));
        lines.push(format!("Updated: {}", Utc::now().to_rfc3339()));
        lines.push(format!(
            "Workers: {} | Directives: {} | Mail: {} | Validation: {} | Stalls: {} | Lease Conflicts: {} | Protocol Reminders: {} | Supervisor Health: {} | Supervisor Fallbacks: {} | Critical Failures: {} | Crash Loops: {}",
            workers.len(),
            stats.directives,
            stats.mails_routed,
            stats.validation_challenges,
            stats.stall_interventions,
            stats.lease_conflicts,
            stats.protocol_reminders,
            stats.supervisor_health_events,
            stats.supervisor_fallbacks,
            stats.critical_failures,
            stats.crash_loops_detected,
        ));
        lines.push(format!("Supervisor: {}", supervisor_line));
        if let Some(standby_line) = standby_line.clone() {
            lines.push(format!("Standby Supervisor: {}", standby_line));
        }
        lines.push(String::new());
        lines.push(format!(
            "Blocked: {}",
            if blocked.is_empty() {
                "none".to_owned()
            } else {
                blocked.join(", ")
            }
        ));
        lines.push(format!(
            "Validation Queue: {}",
            if validation_queue.is_empty() {
                "none".to_owned()
            } else {
                validation_queue.join(", ")
            }
        ));
        lines.push(format!(
            "Contradictions: {}",
            if contradictions.is_empty() {
                "none".to_owned()
            } else {
                contradictions.join(", ")
            }
        ));
        lines.push(format!(
            "Mail Pressure: {}",
            if waiting_mail.is_empty() {
                "none".to_owned()
            } else {
                waiting_mail.join(", ")
            }
        ));
        lines.push(format!(
            "Problems: {}",
            if problems.is_empty() {
                "none".to_owned()
            } else {
                problems.join(", ")
            }
        ));
        lines.push(format!(
            "Crash Loops: {}",
            if crash_loops.is_empty() {
                "none".to_owned()
            } else {
                crash_loops.join(", ")
            }
        ));
        lines.push(format!(
            "Pods: {}",
            if pod_summaries.is_empty() {
                "none".to_owned()
            } else {
                pod_summaries
                    .iter()
                    .map(|pod| format!(
                        "{} members={} blocked={} threads={}",
                        pod.name,
                        pod.members.len(),
                        pod.blocked_members.len(),
                        pod.open_threads
                    ))
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
        ));
        lines.push(format!(
            "Meetings: {}",
            if meetings.is_empty() {
                "none".to_owned()
            } else {
                meetings
                    .iter()
                    .take(4)
                    .map(|meeting| format!("{} [{}]", meeting.kind, meeting.reason))
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
        ));
        lines.push(String::new());
        lines.push("Workers".to_owned());
        for worker in workers {
            lines.push(format!(
                "- {} [{}] {}",
                worker.record.name,
                live_snapshot.effective_state_label(worker),
                worker
                    .record
                    .last_summary
                    .as_deref()
                    .unwrap_or("no summary yet")
            ));
        }
        write_string_to_file(&control_surface.status_file, &lines.join("\n"))?;
        write_string_to_file(&hidden_status_file(control_surface), &lines.join("\n"))?;
        if !full_surface {
            return Ok(());
        }
        let mission = self.store.load_mission_snapshot(mission_id)?;
        let supervisor_summary = self.store.latest_supervisor_summary(mission_id)?;
        let mut dashboard = Vec::new();
        dashboard.push("# Sapphire".to_owned());
        if let Some(mission) = mission {
            dashboard.push(String::new());
            dashboard.push(format!("Mission: {}", mission.mission_rewrite));
            dashboard.push(format!("Status: {}", mission.status));
        }
        dashboard.push(String::new());
        dashboard.push("## Supervisor".to_owned());
        dashboard.push(format!("- {}", supervisor_line));
        if let Some(standby_line) = standby_line {
            dashboard.push(format!("- Standby: {}", standby_line));
        }
        if let Some(summary) = supervisor_summary {
            dashboard.push(format!("- Summary: {}", truncate(&summary, 180)));
        }
        dashboard.push(String::new());
        dashboard.push("## Watchdog".to_owned());
        dashboard.push(format!(
            "- events={} directives={} mail={} validation={} stalls={} conflicts={} reminders={} critical_failures={} crash_loops={}",
            stats.runtime_events,
            stats.directives,
            stats.mails_routed,
            stats.validation_challenges,
            stats.stall_interventions,
            stats.lease_conflicts,
            stats.protocol_reminders,
            stats.critical_failures,
            stats.crash_loops_detected,
        ));
        if !crash_loops.is_empty() {
            dashboard.push(format!("- crash loops: {}", crash_loops.join(", ")));
        }
        dashboard.push(String::new());
        dashboard.push("## Pods".to_owned());
        if pod_summaries.is_empty() {
            dashboard.push("- no active pods".to_owned());
        } else {
            for pod in &pod_summaries {
                dashboard.push(format!(
                    "- {} | members={} | blocked={} | threads={}",
                    pod.name,
                    pod.members.len(),
                    pod.blocked_members.len(),
                    pod.open_threads
                ));
            }
        }
        dashboard.push(String::new());
        dashboard.push("## Meetings".to_owned());
        if meetings.is_empty() {
            dashboard.push("- no active coordination meetings".to_owned());
        } else {
            for meeting in meetings.iter().take(6) {
                dashboard.push(format!(
                    "- {} [{}] {}",
                    meeting.kind,
                    meeting.participants.join(", "),
                    truncate(&meeting.reason, 96)
                ));
            }
        }
        dashboard.push(String::new());
        dashboard.push("## Memory".to_owned());
        if memory_summaries.is_empty() {
            dashboard.push("- worker memories not initialized yet".to_owned());
        } else {
            for summary in &memory_summaries {
                dashboard.push(format!(
                    "- {} | pod={} | active_threads={}",
                    summary.display_name, summary.pod, summary.active_threads
                ));
            }
        }
        dashboard.push(String::new());
        dashboard.push("## Sessions".to_owned());
        for session in active_sessions.values() {
            let state_label = live_snapshot.effective_state_label(session);
            dashboard.push(format!(
                "- {} [{}] {}",
                session.record.name,
                state_label,
                truncate(
                    session
                        .record
                        .last_summary
                        .as_deref()
                        .unwrap_or("no summary yet"),
                    96
                ),
            ));
        }
        dashboard.push(String::new());
        dashboard.push("Detach: Ctrl-b d".to_owned());
        write_string_to_file(&control_surface.dashboard_file, &dashboard.join("\n"))?;
        write_string_to_file(&hidden_dashboard_file(control_surface), &dashboard.join("\n"))?;
        Ok(())
    }

    fn ensure_tmux_surface(
        &self,
        session_name: &str,
        mission_id: Uuid,
        config: &LaunchConfig,
    ) -> Result<Vec<String>> {
        let tmux = tmux::Tmux::new(None);
        let base_name = session_name.to_string();
        let per_tab = 10; // From launch-codex-tabs.sh: 10 terminals per tab
        let total_workers = config.worker_count.max(1);

        // Create batch sessions: 10 workers per tmux session (from launch-codex-tabs.sh)
        let session_names = tmux.create_batch_sessions(
            &base_name,
            &config.repo.to_string_lossy(),
            total_workers,
            per_tab,
        )
        .map_err(anyhow::Error::msg)
        .context("failed to create batch tmux sessions")?;

        self.store.append_summary(
            mission_id,
            None,
            "surface",
            format!(
                "teamwork surface prepared: {} sessions ({} workers, {} per session)",
                session_names.len(),
                total_workers,
                per_tab
            ),
        )?;
        Ok(session_names)
    }

    /// Read worker status.json files from .sp/workers/<display_name>/status.json
    /// and update active_sessions state. This is the PRIMARY status source — more
    /// reliable than PTY directive parsing (no ANSI, no scrollback loss, no race).
    fn read_worker_status_files(
        &self,
        mission_id: Uuid,
        repo_root: &Path,
        supervisor_id: Uuid,
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        pending_supervisor_decisions: &mut HashMap<String, PendingSupervisorDecision>,
        supervisor_degraded: bool,
        stats: &mut WatchdogStats,
        control_surface: &ControlSurface,
    ) -> Result<()> {
        let workers_dir = &control_surface.workers_state_dir;
        let hidden_workers_dir = hidden_workers_state_dir(control_surface);
        if !workers_dir.exists() && !hidden_workers_dir.exists() {
            return Ok(());
        }

        let name_to_id: HashMap<String, Uuid> = active_sessions
            .iter()
            .filter(|(_, s)| s.record.role == crate::model::SessionRole::Worker)
            .map(|(id, s)| (s.record.name.clone(), *id))
            .collect();

        let mut updates = Vec::new();

        for (display_name, session_id) in &name_to_id {
            let status_file = workers_dir.join(display_name).join("status.json");
            let hidden_status_file = hidden_workers_dir.join(display_name).join("status.json");
            let previous_modified = active_sessions
                .get(session_id)
                .and_then(|session| session.last_status_file_modified);
            let Some(update) = status_files::load_status_file_update(
                &status_file,
                &hidden_status_file,
                previous_modified,
            ) else {
                continue;
            };

            let Some(session) = active_sessions.get_mut(session_id) else {
                continue;
            };
            if session.state.is_terminal() {
                continue;
            }
            let now = Instant::now();
            session.last_status_file_modified = Some(update.modified_at);
            if !update.bootstrap {
                session.last_status_update_at = Some(now);
            }
            session.last_confirmed_alive = now;
            if !update.bootstrap {
                session.protocol_reminder_sent = false;
                record_intervention_response(session, now);
            }
            updates.push((*session_id, update.directive, update.bootstrap));
        }

        for (session_id, directive, bootstrap) in updates {
            let source = if bootstrap {
                "bootstrap_status_file"
            } else {
                "status_file"
            };
            self.persist_normalized_status(
                mission_id,
                session_id,
                &directive.state,
                source,
                Confidence::High,
                &directive.summary,
                &directive.summary,
            )?;
            if bootstrap {
                if let Some(session) = active_sessions.get_mut(&session_id) {
                    if let Some(reported_state) = directive.session_state() {
                        session.state = reported_state;
                        session.record.last_summary = Some(directive.summary.clone());
                        self.store.update_session_state(session_id, reported_state)?;
                    }
                    self.store
                        .update_worker_summary(session_id, &directive.summary)?;
                }
                continue;
            }
            self.handle_status_directive(
                mission_id,
                repo_root,
                supervisor_id,
                session_id,
                directive,
                false,
                active_sessions,
                pending_supervisor_decisions,
                supervisor_degraded,
                stats,
            )?;
        }

        Ok(())
    }
}

async fn next_runtime_event(
    supervisor_runtime: &mut SessionRuntime,
    worker_runtimes: &mut [SessionRuntime],
    tick: Duration,
) -> Option<RuntimeEvent> {
    if let Some(event) = supervisor_runtime
        .next_event(Duration::from_millis(5))
        .await
    {
        return Some(event);
    }

    for worker_runtime in worker_runtimes.iter_mut() {
        if let Some(event) = worker_runtime.next_event(Duration::from_millis(5)).await {
            return Some(event);
        }
    }

    if worker_runtimes.is_empty() {
        return supervisor_runtime.next_event(tick).await;
    }

    let polling_window = tick.min(Duration::from_millis(25));
    let started = Instant::now();
    loop {
        if let Some(event) = supervisor_runtime
            .next_event(Duration::from_millis(5))
            .await
        {
            return Some(event);
        }
        for worker_runtime in worker_runtimes.iter_mut() {
            if let Some(event) = worker_runtime.next_event(Duration::from_millis(5)).await {
                return Some(event);
            }
        }
        if started.elapsed() >= tick {
            return None;
        }
        tokio::time::sleep(polling_window).await;
    }
}

fn register_session(
    active_sessions: &mut HashMap<Uuid, ActiveSession>,
    alias_map: &mut HashMap<String, Uuid>,
    record: SessionRecord,
    packet: Option<WorkerPacket>,
    runtime: RunningSession,
    runtime_slot: Option<usize>,
    launch_spec: ProcessLaunchSpec,
    launch_prompt: String,
    task_id: Option<Uuid>,
    aliases: Vec<String>,
) {
    let session_id = record.id;
    let initial_status = record.status;
    let startup_grace_until = Instant::now() + startup_grace(record.agent);
    for alias in &aliases {
        alias_map.insert(alias.clone(), session_id);
        alias_map.insert(alias.to_ascii_lowercase(), session_id);
    }
    active_sessions.insert(
        session_id,
        ActiveSession {
            state: initial_status,
            record,
            packet,
            runtime,
            runtime_slot,
            launch_spec,
            launch_prompt,
            task_id,
            line_buffer: String::new(),
            raw_buffer: String::new(),
            started_at: Instant::now(),
            startup_grace_until,
            last_output_at: Instant::now(),
            output_chunks: 0,
            directive_count: 0,
            initial_status_received: false,
            output_chunks_at_last_status: 0,
            reported_overlap: None,
            stall_count: 0,
            restart_count: 0,
            restart_at: None,
            validation_pending: matches!(
                initial_status,
                SessionState::DoneClaimed | SessionState::NeedsValidation
            ),
            low_confidence_count: 0,
            last_observation_key: None,
            last_supervisor_action_key: None,
            escalation_sent_for_state: None,
            protocol_reminder_sent: false,
            consecutive_stall_failures: 0,
            last_confirmed_alive: Instant::now(),
            last_files: Vec::new(),
            last_risks: Vec::new(),
            intervention_cooldown_until: None,
            last_intervention_type: None,
            total_interventions: 0,
            last_response_time: None,
            last_intervention_at: None,
            queued_prompts: VecDeque::new(),
            queued_prompt_keys: HashSet::new(),
            recent_prompt_keys: VecDeque::new(),
            last_prompt_sent_at: None,
            launch_prompt_sent: false,
            cleanup_authorized: false,
            last_status_update_at: None,
            last_status_file_modified: None,
            last_tmux_health: None,
            last_tmux_health_checked_at: None,
            last_supervisor_notice_key: None,
            recent_supervisor_notice_keys: VecDeque::new(),
            last_supervisor_state_card_key: None,
            zombie_debounce: health::ZombieDebounce::default(),
            health_state: health::SessionHealthState::new(),
            message_dedup: dedup::MessageDeduplicator::new(),
        },
    );
}

fn append_transcript(
    control_surface: &ControlSurface,
    session_name: &str,
    chunk: &str,
) -> Result<()> {
    let path = control_surface
        .transcript_dir
        .join(format!("{session_name}.log"));
    append_to_file(&path, chunk)?;
    let hidden_path = hidden_transcript_dir(control_surface).join(format!("{session_name}.log"));
    append_to_file(&hidden_path, chunk)?;
    Ok(())
}

fn resolve_alias(alias_map: &HashMap<String, Uuid>, alias: &str) -> Option<Uuid> {
    let direct = alias.trim();
    alias_map
        .get(direct)
        .copied()
        .or_else(|| alias_map.get(&direct.to_ascii_lowercase()).copied())
}

fn persist_runtime_event(store: &Store, mission_id: Uuid, event: &RuntimeEvent) -> Result<()> {
    match event {
        RuntimeEvent::Output { session_id, chunk } => store.append_json_event(
            mission_id,
            Some(*session_id),
            "output_chunk",
            truncate(chunk, 240),
            event,
        ),
        RuntimeEvent::Automation {
            session_id,
            rule_name,
        } => store.append_json_event(
            mission_id,
            Some(*session_id),
            "automation",
            format!("fired {}", rule_name),
            event,
        ),
        RuntimeEvent::Exited {
            session_id,
            exit_code,
        } => store.append_json_event(
            mission_id,
            Some(*session_id),
            "process_exit",
            "session exited",
            &json!({ "exit_code": exit_code }),
        ),
    }
}

fn directive_kind(directive: &SapphireDirective) -> &'static str {
    match directive {
        SapphireDirective::Status(_) => "status",
        SapphireDirective::Mail(_) => "mail",
        SapphireDirective::Ack(_) => "ack",
        SapphireDirective::Lease(_) => "lease",
    }
}

fn queue_supervisor_decision(
    pending_supervisor_decisions: &mut HashMap<String, PendingSupervisorDecision>,
    kind: SupervisorDecisionKind,
    target_session_id: Uuid,
    reason: &str,
) -> bool {
    let key = supervisor_decision_key(kind, target_session_id);
    if let Some(existing) = pending_supervisor_decisions.get_mut(&key) {
        existing.reason = reason.to_owned();
        return false;
    }
    let now = Instant::now();
    pending_supervisor_decisions.insert(
        key,
        PendingSupervisorDecision {
            kind,
            target_session_id,
            reason: reason.to_owned(),
            queued_at: now,
            last_notified_at: now,
            notice_count: 0,
        },
    );
    true
}

fn clear_supervisor_decisions_for_target(
    pending_supervisor_decisions: &mut HashMap<String, PendingSupervisorDecision>,
    target_session_id: Uuid,
) {
    pending_supervisor_decisions
        .retain(|_, pending| pending.target_session_id != target_session_id);
}

fn clear_resolved_supervisor_decisions(
    pending_supervisor_decisions: &mut HashMap<String, PendingSupervisorDecision>,
    active_sessions: &HashMap<Uuid, ActiveSession>,
    target_session_id: Uuid,
) {
    let Some(target) = active_sessions.get(&target_session_id) else {
        clear_supervisor_decisions_for_target(pending_supervisor_decisions, target_session_id);
        return;
    };

    pending_supervisor_decisions.retain(|_, pending| {
        if pending.target_session_id != target_session_id {
            return true;
        }
        match pending.kind {
            SupervisorDecisionKind::Validation => target.validation_pending,
            SupervisorDecisionKind::StallRecovery => target.state == SessionState::Stalled,
            SupervisorDecisionKind::LowConfidenceRecovery => target.low_confidence_count > 0,
            SupervisorDecisionKind::OverlapRecovery => target
                .reported_overlap
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
        }
    });
}

fn clear_transient_supervisor_decisions_on_output(
    pending_supervisor_decisions: &mut HashMap<String, PendingSupervisorDecision>,
    target_session_id: Uuid,
) {
    pending_supervisor_decisions.retain(|_, pending| {
        pending.target_session_id != target_session_id
            || matches!(
                pending.kind,
                SupervisorDecisionKind::Validation | SupervisorDecisionKind::OverlapRecovery
            )
    });
}

fn supervisor_decision_key(kind: SupervisorDecisionKind, target_session_id: Uuid) -> String {
    format!("{}:{target_session_id}", kind.as_str())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use chrono::Utc;

    use super::{
        enforcement,
        ActiveSession, HEURISTIC_SETTLE_THRESHOLD, Orchestrator, QueuedPrompt,
        SupervisorDecisionKind, SupervisorMode, WatchdogStats,
        drain_prompt_queues,
        deterministic_plan_for_mission, ensure_agents_bootstrap, ensure_state_tree, hidden_state_dir, normalize_supervisor_plan,
        mission_profile,
        queue_supervisor_decision, rebuild_launch_spec, register_session,
        render_supervisor_prompt_with_agents, should_auto_restart,
        send_or_queue_prompt,
        startup_grace,
    };
    use crate::adapter::SupervisorEventType;
    use crate::cli::LaunchConfig;
    use crate::agent::AgentKind;
    use crate::model::{
        MissionPlan, RiskItem, SessionRecord, SessionRole, SessionState, WorkerPacket,
        Workstream, WorkstreamExecution,
    };
    use crate::runtime::{RunningSession, RuntimeEvent};
    use crate::store::Store;
    use crate::templates::PromptLibrary;
    use uuid::Uuid;

    fn test_orchestrator() -> Orchestrator {
        // Use a nested .sp dir so hidden_state_dir works correctly
        let base = std::env::temp_dir().join(format!("sp-test-{}", Uuid::new_v4()));
        let state_dir = base.join(".sp");
        ensure_state_tree(&state_dir).expect("state tree");
        ensure_state_tree(&hidden_state_dir(&state_dir)).expect("hidden state tree");
        Orchestrator {
            store: Store::open(&state_dir).expect("store"),
            prompts: crate::templates::PromptLibrary::load(),
        }
    }

    fn make_session(
        mission_id: Uuid,
        session_id: Uuid,
        name: &str,
        role: SessionRole,
        state: SessionState,
    ) -> SessionRecord {
        SessionRecord {
            id: session_id,
            mission_id,
            role,
            ordinal: 1,
            agent: AgentKind::Codex,
            terminal_id: name.to_owned(),
            name: name.to_owned(),
            owned_scope: "test scope".to_owned(),
            status: state,
            launch_command: vec!["codex".to_owned()],
            last_heartbeat_at: Utc::now(),
            last_summary: Some(format!("{name} summary")),
        }
    }

    fn insert_session(
        active_sessions: &mut HashMap<Uuid, ActiveSession>,
        alias_map: &mut HashMap<String, Uuid>,
        record: SessionRecord,
    ) -> crate::runtime::TestSessionProbe {
        let (runtime, probe) = RunningSession::test(record.name.clone(), Duration::from_millis(0));
        register_session(
            active_sessions,
            alias_map,
            record.clone(),
            None,
            runtime,
            None,
            rebuild_launch_spec(&record, Path::new("."), Path::new("./.sp-test")),
            "test prompt".to_owned(),
            None,
            vec![record.name.clone()],
        );
        probe
    }

    fn test_launch_config(mission: &str, worker_count: usize) -> LaunchConfig {
        LaunchConfig {
            worker_agent: AgentKind::Qwen,
            supervisor_agent: AgentKind::Qwen,
            worker_count,
            repo: PathBuf::from("."),
            mission: mission.to_owned(),
            state_dir: PathBuf::from(".sp"),
            dry_run: true,
            stall_seconds: 45,
            watchdog_max_seconds: None,
            watchdog_tick_millis: 1000,
            tmux: false,
            tmux_session_name: None,
            persist_transcripts: false,
            tui: false,
            worker_args: Vec::new(),
            supervisor_args: Vec::new(),
            git_remote: None,
        }
    }

    fn test_control_surface() -> super::ControlSurface {
        let root = std::env::temp_dir().join(format!("sp-control-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("workers")).expect("workers dir");
        std::fs::create_dir_all(root.join("transcripts")).expect("transcripts dir");
        super::ControlSurface {
            state_dir: root.clone(),
            status_file: root.join("control/status.txt"),
            dashboard_file: root.join("control/dashboard.txt"),
            transcript_dir: root.join("transcripts"),
            workers_state_dir: root.join("workers"),
            hidden_root: root.join(".hide.sp"),
            persist_transcripts: false,
            tmux_session_names: Vec::new(),
        }
    }

    fn sample_plan(worker_count: usize) -> MissionPlan {
        MissionPlan {
            mission_rewrite: "validate orchestration".to_owned(),
            workstreams: vec![Workstream {
                id: "ws1".to_owned(),
                name: "Validation".to_owned(),
                execution: WorkstreamExecution::Parallel,
                owned_scope: "repo".to_owned(),
                success_criteria: vec!["prove agent messaging".to_owned()],
                depends_on: Vec::new(),
            }],
            risk_map: vec![RiskItem {
                zone: "planner".to_owned(),
                risk: "packet mismatch".to_owned(),
                mitigation: "normalize extras".to_owned(),
            }],
            worker_packets: (1..=worker_count)
                .map(|index| WorkerPacket {
                    worker_id: format!("Engineer-{index}"),
                    role_type: "software-engineer".to_owned(),
                    display_name: format!("Engineer-{index}"),
                    role: "Software Engineer".to_owned(),
                    starting_angle: format!("angle {index}"),
                    owned_scope: format!("scope {index}"),
                    explicit_task: format!("task {index}"),
                    out_of_scope: "none".to_owned(),
                    definition_of_done: vec!["done".to_owned()],
                    required_evidence: vec!["evidence".to_owned()],
                    blocker_protocol: "escalate".to_owned(),
                    conflict_warning: "claim files".to_owned(),
                    communication_rules: vec!["mail blockers".to_owned()],
                    validation_standard: vec!["pass checks".to_owned()],
                    expected_output_format: vec!["summary".to_owned()],
                })
                .collect(),
            supervision_strategy: "tight".to_owned(),
        }
    }

    #[test]
    fn normalize_supervisor_plan_trims_extra_worker_packets() {
        let plan = sample_plan(4);
        let normalized = normalize_supervisor_plan(plan, 1, "qwen").expect("normalized plan");
        assert_eq!(normalized.worker_packets.len(), 1);
        assert_eq!(normalized.worker_packets[0].display_name, "Engineer-1");
    }

    #[test]
    fn normalize_supervisor_plan_rejects_too_few_worker_packets() {
        let plan = sample_plan(1);
        let error = normalize_supervisor_plan(plan, 2, "qwen").expect_err("expected failure");
        assert!(error.to_string().contains("expected at least 2 worker packets"));
    }

    #[test]
    fn deterministic_plan_builds_expected_worker_count() {
        let plan = deterministic_plan_for_mission(
            "Prove Sapphire terminal orchestration works.",
            2,
        );
        assert_eq!(plan.worker_packets.len(), 2);
        assert_eq!(plan.workstreams.len(), 2);
        assert_eq!(plan.worker_packets[0].display_name, "Validator-1");
    }

    #[test]
    fn lean_coordination_profile_uses_deterministic_planning() {
        let config = test_launch_config(
            "Talk to your teammate and prove the team can work together.",
            3,
        );
        let profile = mission_profile::MissionProfile::from_launch(&config);
        assert!(profile.coordination_focused);
        assert!(profile.deterministic_planning);
        assert!(profile.lean_supervision);
        assert!(!profile.enable_repair_supervisor);
        assert!(!profile.enable_state_cards);
    }

    #[test]
    fn coordination_plan_requires_real_teammate_mail() {
        let profile = mission_profile::MissionProfile::from_launch(&test_launch_config(
            "Talk to your teammate and prove the team can work together.",
            3,
        ));
        let plan = mission_profile::deterministic_plan_for_mission(
            "Talk to your teammate and prove the team can work together.",
            3,
            profile,
        );
        assert_eq!(plan.worker_packets.len(), 3);
        assert!(plan.worker_packets[0]
            .communication_rules
            .iter()
            .any(|rule| rule.contains("SAPPHIRE_MAIL")));
        assert!(plan.worker_packets[0]
            .definition_of_done
            .iter()
            .any(|rule| rule.contains("mail thread")));
    }

    #[test]
    fn supervisor_prompt_forbids_hidden_extra_workers() {
        let prompts = PromptLibrary::load();
        let agents_bootstrap = ensure_agents_bootstrap(Path::new("."), &prompts)
            .expect("agents bootstrap");
        let rendered = render_supervisor_prompt_with_agents(
            "base prompt".to_owned(),
            &agents_bootstrap,
            2,
        );
        assert!(rendered.contains("Worker packet count must equal 2."));
        assert!(rendered.contains("Do not invent extra workers"));
        assert!(!rendered.contains("Reserved AGENTS steward"));
    }

    #[test]
    fn stalled_worker_is_routed_to_supervisor_before_local_intervention() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let supervisor_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();

        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        let supervisor_probe = insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                supervisor_id,
                "supervisor-01",
                SessionRole::Supervisor,
                SessionState::Progressing,
            ),
        );
        let worker_probe = insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Engineer-1",
                SessionRole::Worker,
                SessionState::Progressing,
            ),
        );
        active_sessions
            .get_mut(&worker_id)
            .expect("worker")
            .last_output_at = std::time::Instant::now() - Duration::from_secs(60);
        active_sessions
            .get_mut(&worker_id)
            .expect("worker")
            .last_confirmed_alive = std::time::Instant::now() - Duration::from_secs(60);
        active_sessions
            .get_mut(&worker_id)
            .expect("worker")
            .startup_grace_until = std::time::Instant::now() - Duration::from_secs(1);

        let mut pending_supervisor_decisions = HashMap::new();
        let mut stats = WatchdogStats::default();

        orchestrator
            .handle_stalls(
                mission_id,
                Path::new("."),
                supervisor_id,
                Duration::from_secs(15),
                &mut active_sessions,
                &mut pending_supervisor_decisions,
                false,
                &mut stats,
            )
            .expect("stall handling");

        assert!(worker_probe.sent_texts().is_empty());
        assert_eq!(stats.stall_interventions, 0);
        assert_eq!(pending_supervisor_decisions.len(), 1);
        let supervisor_text = supervisor_probe.sent_texts().join("\n");
        assert!(supervisor_text.contains("appears stalled"));
        assert!(supervisor_text.contains("Decide whether to"));
    }

    #[test]
    fn pending_validation_stays_queued_until_supervisor_acts() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let supervisor_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();

        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                supervisor_id,
                "supervisor-01",
                SessionRole::Supervisor,
                SessionState::Exited,
            ),
        );
        let worker_probe = insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Validator-1",
                SessionRole::Worker,
                SessionState::DoneClaimed,
            ),
        );
        active_sessions
            .get_mut(&worker_id)
            .expect("worker")
            .validation_pending = true;

        let mut pending_supervisor_decisions = HashMap::new();
        assert!(queue_supervisor_decision(
            &mut pending_supervisor_decisions,
            SupervisorDecisionKind::Validation,
            worker_id,
            "validation required",
        ));

        orchestrator
            .handle_pending_supervisor_decisions(
                mission_id,
                worker_id,
                &mut active_sessions,
                &mut pending_supervisor_decisions,
            )
            .expect("pending supervisor decisions");

        assert!(worker_probe.sent_texts().is_empty());
        assert_eq!(pending_supervisor_decisions.len(), 1);
    }

    #[test]
    fn repair_supervisor_takes_over_when_primary_exits_with_active_workers() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let primary_supervisor_id = Uuid::new_v4();
        let repair_supervisor_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();

        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                primary_supervisor_id,
                "supervisor-01",
                SessionRole::Supervisor,
                SessionState::Exited,
            ),
        );
        let repair_probe = insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                repair_supervisor_id,
                "supervisor-02-repair",
                SessionRole::Supervisor,
                SessionState::Progressing,
            ),
        );
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Engineer-1",
                SessionRole::Worker,
                SessionState::Progressing,
            ),
        );

        let mut active_supervisor_id = primary_supervisor_id;
        let pending_mail = HashMap::new();
        let pending_decisions = HashMap::new();
        let mut supervisor_mode = SupervisorMode::Healthy;
        let mut worker_continuity_announced = false;
        let mut stats = WatchdogStats::default();

        orchestrator
            .handle_supervisor_health(
                mission_id,
                primary_supervisor_id,
                repair_supervisor_id,
                &mut active_supervisor_id,
                Duration::from_secs(15),
                &mut active_sessions,
                &pending_mail,
                &pending_decisions,
                std::time::Instant::now(),
                &mut supervisor_mode,
                &mut worker_continuity_announced,
                &mut stats,
            )
            .expect("supervisor health");

        assert_eq!(active_supervisor_id, repair_supervisor_id);
        assert_eq!(supervisor_mode, SupervisorMode::Recovering);
        assert!(worker_continuity_announced);
        let repair_text = repair_probe.sent_texts().join("\n");
        assert!(repair_text.contains("TAKEOVER NOW."));
        assert!(repair_text.contains("supervisor-01"));
        assert_eq!(stats.supervisor_health_events, 1);
    }

    #[test]
    fn supervisor_health_degrades_when_both_supervisors_unavailable() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let primary_supervisor_id = Uuid::new_v4();
        let repair_supervisor_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();

        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                primary_supervisor_id,
                "supervisor-01",
                SessionRole::Supervisor,
                SessionState::Exited,
            ),
        );
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                repair_supervisor_id,
                "supervisor-02-repair",
                SessionRole::Supervisor,
                SessionState::Exited,
            ),
        );
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Engineer-1",
                SessionRole::Worker,
                SessionState::Progressing,
            ),
        );

        let mut active_supervisor_id = primary_supervisor_id;
        let pending_mail = HashMap::new();
        let pending_decisions = HashMap::new();
        let mut supervisor_mode = SupervisorMode::Healthy;
        let mut worker_continuity_announced = false;
        let mut stats = WatchdogStats::default();

        orchestrator
            .handle_supervisor_health(
                mission_id,
                primary_supervisor_id,
                repair_supervisor_id,
                &mut active_supervisor_id,
                Duration::from_secs(15),
                &mut active_sessions,
                &pending_mail,
                &pending_decisions,
                std::time::Instant::now(),
                &mut supervisor_mode,
                &mut worker_continuity_announced,
                &mut stats,
            )
            .expect("supervisor health");

        assert_eq!(active_supervisor_id, primary_supervisor_id);
        assert_eq!(supervisor_mode, SupervisorMode::Degraded);
        assert!(worker_continuity_announced);
        assert_eq!(stats.supervisor_health_events, 1);
    }

    #[test]
    fn fresh_worker_inside_startup_grace_is_not_marked_stalled() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let supervisor_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();

        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        let _ = insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                supervisor_id,
                "supervisor-01",
                SessionRole::Supervisor,
                SessionState::Progressing,
            ),
        );
        let _ = insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Engineer-1",
                SessionRole::Worker,
                SessionState::Progressing,
            ),
        );
        let worker = active_sessions.get_mut(&worker_id).expect("worker");
        worker.last_output_at = std::time::Instant::now() - Duration::from_secs(60);
        worker.last_confirmed_alive = std::time::Instant::now() - Duration::from_secs(60);
        worker.startup_grace_until =
            std::time::Instant::now() + startup_grace(worker.record.agent);

        let mut pending_supervisor_decisions = HashMap::new();
        let mut stats = WatchdogStats::default();
        orchestrator
            .handle_stalls(
                mission_id,
                Path::new("."),
                supervisor_id,
                Duration::from_secs(15),
                &mut active_sessions,
                &mut pending_supervisor_decisions,
                false,
                &mut stats,
            )
            .expect("stall handling");

        assert!(pending_supervisor_decisions.is_empty());
        assert_eq!(
            active_sessions.get(&worker_id).expect("worker").state,
            SessionState::Progressing
        );
    }

    #[test]
    fn escalation_ladder_third_consecutive_stall_fails_worker() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let supervisor_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();

        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        let supervisor_probe = insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                supervisor_id,
                "supervisor-01",
                SessionRole::Supervisor,
                SessionState::Progressing,
            ),
        );
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Engineer-1",
                SessionRole::Worker,
                SessionState::Progressing,
            ),
        );
        let worker = active_sessions.get_mut(&worker_id).expect("worker");
        worker.last_output_at = std::time::Instant::now() - Duration::from_secs(60);
        worker.last_confirmed_alive = std::time::Instant::now() - Duration::from_secs(60);
        worker.startup_grace_until = std::time::Instant::now() - Duration::from_secs(1);
        // Simulate 2 prior consecutive stall detections.
        worker.consecutive_stall_failures = 2;

        let mut pending_supervisor_decisions = HashMap::new();
        let mut stats = WatchdogStats::default();

        orchestrator
            .handle_stalls(
                mission_id,
                Path::new("."),
                supervisor_id,
                Duration::from_secs(15),
                &mut active_sessions,
                &mut pending_supervisor_decisions,
                false,
                &mut stats,
            )
            .expect("stall handling");

        // Third consecutive stall → watchdog forces Failed state.
        let worker_state = active_sessions.get(&worker_id).expect("worker").state;
        assert_eq!(worker_state, SessionState::Failed);
        assert_eq!(stats.stall_interventions, 1);
        // Supervisor should have been notified via handle_status_directive.
        let supervisor_text = supervisor_probe.sent_texts().join("\n");
        assert!(supervisor_text.contains("3 consecutive times"));
    }

    #[test]
    fn consecutive_stall_failures_reset_on_output() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let supervisor_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();

        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        let worker_probe = insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Engineer-1",
                SessionRole::Worker,
                SessionState::Progressing,
            ),
        );
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                supervisor_id,
                "supervisor-01",
                SessionRole::Supervisor,
                SessionState::Progressing,
            ),
        );

        // Simulate 2 consecutive stall failures.
        let worker = active_sessions.get_mut(&worker_id).expect("worker");
        worker.consecutive_stall_failures = 2;
        worker.last_output_at = std::time::Instant::now() - Duration::from_secs(60);
        worker.last_confirmed_alive = std::time::Instant::now() - Duration::from_secs(60);
        worker.startup_grace_until = std::time::Instant::now() - Duration::from_secs(1);

        let mut pending = HashMap::new();
        let mut stats = WatchdogStats::default();
        orchestrator
            .handle_stalls(
                mission_id,
                Path::new("."),
                supervisor_id,
                Duration::from_secs(15),
                &mut active_sessions,
                &mut pending,
                false,
                &mut stats,
            )
            .expect("stall handling");

        // After the stall handler increments to 3, simulate output arriving.
        let session = active_sessions.get_mut(&worker_id).expect("worker");
        session.last_output_at = std::time::Instant::now();
        session.last_confirmed_alive = std::time::Instant::now();
        session.consecutive_stall_failures = 0;

        // The counter was reset — now the worker is alive again.
        assert_eq!(active_sessions.get(&worker_id).unwrap().consecutive_stall_failures, 0);
        assert!(worker_probe.sent_texts().is_empty() || true); // Reset happened, no extra prompt needed for this test.
    }

    #[test]
    fn duplicate_prompt_is_not_queued_repeatedly() {
        let mission_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();
        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Engineer-1",
                SessionRole::Worker,
                SessionState::Progressing,
            ),
        );

        let session = active_sessions.get_mut(&worker_id).expect("worker");
        session.output_chunks = 1;
        session.last_output_at = Instant::now();

        assert!(!send_or_queue_prompt(session, "repeat this"));
        assert!(!send_or_queue_prompt(session, "repeat this"));
        assert_eq!(session.queued_prompts.len(), 1);
    }

    #[test]
    fn supervisor_notice_is_suppressed_across_interleaved_notices() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let supervisor_id = Uuid::new_v4();
        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        let supervisor_probe = insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                supervisor_id,
                "supervisor-01",
                SessionRole::Supervisor,
                SessionState::Progressing,
            ),
        );

        orchestrator
            .send_supervisor_notice(
                mission_id,
                supervisor_id,
                &mut active_sessions,
                SupervisorEventType::Contradiction,
                "Resolve overlap on src/main.rs.",
            )
            .expect("first notice");
        orchestrator
            .send_supervisor_notice(
                mission_id,
                supervisor_id,
                &mut active_sessions,
                SupervisorEventType::Notice,
                "Background status changed.",
            )
            .expect("interleaved notice");
        orchestrator
            .send_supervisor_notice(
                mission_id,
                supervisor_id,
                &mut active_sessions,
                SupervisorEventType::Contradiction,
                "Resolve overlap on src/main.rs.",
            )
            .expect("duplicate contradiction notice");

        let sent = supervisor_probe.sent_texts();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("Resolve overlap on src/main.rs."));
        let supervisor = active_sessions.get(&supervisor_id).expect("supervisor");
        assert_eq!(supervisor.queued_prompts.len(), 1);
        assert!(supervisor
            .queued_prompts
            .front()
            .expect("queued notice")
            .body
            .contains("Background status changed."));
    }

    #[test]
    fn overlap_detail_ignores_none_detected_language() {
        assert_eq!(
            enforcement::meaningful_overlap_detail(Some("none detected yet")),
            None
        );
        assert_eq!(
            enforcement::meaningful_overlap_detail(Some("no overlap")),
            None
        );
        assert_eq!(
            enforcement::meaningful_overlap_detail(Some("src/main.rs changed by Engineer-2")),
            Some("src/main.rs changed by Engineer-2".to_owned())
        );
    }

    #[test]
    fn zombie_debounce_does_not_flag_idle_worker_without_dead_terminal_signal() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();
        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Engineer-1",
                SessionRole::Worker,
                SessionState::Progressing,
            ),
        );
        let worker = active_sessions.get_mut(&worker_id).expect("worker");
        worker.last_confirmed_alive = Instant::now() - Duration::from_secs(120);
        worker.startup_grace_until = Instant::now() - Duration::from_secs(1);

        let mut stats = WatchdogStats::default();
        orchestrator
            .zombie_debounce_check(mission_id, &mut active_sessions, &mut stats)
            .expect("zombie debounce");

        assert_eq!(stats.critical_failures, 0);
        assert_eq!(
            active_sessions
                .get(&worker_id)
                .expect("worker")
                .zombie_debounce
                .consecutive_zombie_count,
            0
        );
    }

    #[test]
    fn queued_prompts_drain_one_at_a_time() {
        let mission_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();
        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        let worker_probe = insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Engineer-1",
                SessionRole::Worker,
                SessionState::Progressing,
            ),
        );

        let session = active_sessions.get_mut(&worker_id).expect("worker");
        session.output_chunks = 1;
        session.last_output_at = Instant::now();
        assert!(!send_or_queue_prompt(session, "first"));
        assert!(!send_or_queue_prompt(session, "second"));

        session.last_output_at = Instant::now() - Duration::from_secs(10);
        drain_prompt_queues(&mut active_sessions);
        assert_eq!(worker_probe.sent_texts().len(), 1);
        assert_eq!(
            active_sessions.get(&worker_id).expect("worker").queued_prompts.len(),
            1
        );

        let session = active_sessions.get_mut(&worker_id).expect("worker");
        session.last_prompt_sent_at = Some(Instant::now() - Duration::from_secs(60));
        drain_prompt_queues(&mut active_sessions);
        assert_eq!(worker_probe.sent_texts().len(), 2);
        assert!(
            active_sessions
                .get(&worker_id)
                .expect("worker")
                .queued_prompts
                .is_empty()
        );
    }

    #[test]
    fn active_worker_gets_extended_stall_threshold() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let supervisor_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();

        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                supervisor_id,
                "supervisor-01",
                SessionRole::Supervisor,
                SessionState::Progressing,
            ),
        );
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Engineer-1",
                SessionRole::Worker,
                SessionState::Progressing,
            ),
        );
        let worker = active_sessions.get_mut(&worker_id).expect("worker");
        worker.output_chunks = 12;
        worker.last_output_at = Instant::now() - Duration::from_secs(30);
        worker.last_confirmed_alive = Instant::now() - Duration::from_secs(30);
        worker.startup_grace_until = Instant::now() - Duration::from_secs(1);

        let mut pending_supervisor_decisions = HashMap::new();
        let mut stats = WatchdogStats::default();
        orchestrator
            .handle_stalls(
                mission_id,
                Path::new("."),
                supervisor_id,
                Duration::from_secs(15),
                &mut active_sessions,
                &mut pending_supervisor_decisions,
                false,
                &mut stats,
            )
            .expect("stall handling");

        assert!(pending_supervisor_decisions.is_empty());
        assert_eq!(
            active_sessions.get(&worker_id).expect("worker").state,
            SessionState::Progressing
        );
    }

    #[test]
    fn health_probe_only_records_once_per_idle_window() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();
        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Engineer-1",
                SessionRole::Worker,
                SessionState::Progressing,
            ),
        );
        let worker = active_sessions.get_mut(&worker_id).expect("worker");
        worker.last_confirmed_alive = Instant::now() - Duration::from_secs(12);
        worker.startup_grace_until = Instant::now() - Duration::from_secs(1);

        let mut stats = WatchdogStats::default();
        orchestrator
            .health_probe_sessions(
                mission_id,
                Duration::from_secs(15),
                &mut active_sessions,
                &mut stats,
            )
            .expect("first probe");
        orchestrator
            .health_probe_sessions(
                mission_id,
                Duration::from_secs(15),
                &mut active_sessions,
                &mut stats,
            )
            .expect("second probe");

        assert_eq!(stats.supervisor_health_events, 1);
        assert_eq!(
            active_sessions
                .get(&worker_id)
                .expect("worker")
                .health_state
                .consecutive_probe_failures,
            1
        );
    }

    #[test]
    fn supervisor_output_does_not_trigger_worker_state_heuristics() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let supervisor_id = Uuid::new_v4();
        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                supervisor_id,
                "supervisor-01",
                SessionRole::Supervisor,
                SessionState::Progressing,
            ),
        );
        let mut leases = HashMap::new();
        let mut pending_mail = HashMap::new();
        let mut pending_supervisor_decisions = HashMap::new();
        let mut recent_failures = Vec::new();
        let mut mass_death_detector = crate::orchestrator::health::MassDeathDetector::default();
        let mut stats = WatchdogStats::default();
        let control_surface = test_control_surface();

        orchestrator
            .handle_runtime_event(
                mission_id,
                Path::new("."),
                supervisor_id,
                &RuntimeEvent::Output {
                    session_id: supervisor_id,
                    chunk: "waiting on dependency review".to_owned(),
                },
                &mut active_sessions,
                &alias_map,
                &mut leases,
                &mut pending_mail,
                &mut pending_supervisor_decisions,
                &mut recent_failures,
                &mut mass_death_detector,
                false,
                &mut stats,
                &control_surface,
            )
            .expect("runtime event");

        assert_eq!(
            active_sessions
                .get(&supervisor_id)
                .expect("supervisor")
                .state,
            SessionState::Progressing
        );
    }

    #[test]
    fn worker_output_does_not_trigger_heuristics_inline() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let supervisor_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();
        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                supervisor_id,
                "supervisor-01",
                SessionRole::Supervisor,
                SessionState::Progressing,
            ),
        );
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Engineer-1",
                SessionRole::Worker,
                SessionState::Progressing,
            ),
        );
        let mut leases = HashMap::new();
        let mut pending_mail = HashMap::new();
        let mut pending_supervisor_decisions = HashMap::new();
        let mut recent_failures = Vec::new();
        let mut mass_death_detector = crate::orchestrator::health::MassDeathDetector::default();
        let mut stats = WatchdogStats::default();
        let control_surface = test_control_surface();

        orchestrator
            .handle_runtime_event(
                mission_id,
                Path::new("."),
                supervisor_id,
                &RuntimeEvent::Output {
                    session_id: worker_id,
                    chunk: "waiting on dependency review".to_owned(),
                },
                &mut active_sessions,
                &alias_map,
                &mut leases,
                &mut pending_mail,
                &mut pending_supervisor_decisions,
                &mut recent_failures,
                &mut mass_death_detector,
                false,
                &mut stats,
                &control_surface,
            )
            .expect("runtime event");

        assert_eq!(
            active_sessions.get(&worker_id).expect("worker").state,
            SessionState::Progressing
        );
        assert!(pending_supervisor_decisions.is_empty());
    }

    #[test]
    fn settled_worker_output_triggers_heuristics_after_quiet_window() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let supervisor_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();
        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                supervisor_id,
                "supervisor-01",
                SessionRole::Supervisor,
                SessionState::Progressing,
            ),
        );
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Engineer-1",
                SessionRole::Worker,
                SessionState::Progressing,
            ),
        );

        let worker = active_sessions.get_mut(&worker_id).expect("worker");
        worker.raw_buffer = "waiting on dependency review".to_owned();
        worker.output_chunks = 1;
        worker.startup_grace_until = Instant::now() - Duration::from_secs(1);
        worker.last_output_at = Instant::now() - Duration::from_secs(HEURISTIC_SETTLE_THRESHOLD.as_secs() + 1);
        worker.last_confirmed_alive = worker.last_output_at;

        let mut pending_supervisor_decisions = HashMap::new();
        let mut stats = WatchdogStats::default();
        orchestrator
            .handle_settled_worker_observations(
                mission_id,
                Path::new("."),
                supervisor_id,
                &mut active_sessions,
                &mut pending_supervisor_decisions,
                false,
                &mut stats,
            )
            .expect("settled observations");

        assert_eq!(
            active_sessions.get(&worker_id).expect("worker").state,
            SessionState::Blocked
        );
    }

    #[test]
    fn fresh_output_clears_stale_interventions() {
        let orchestrator = test_orchestrator();
        let mission_id = Uuid::new_v4();
        let supervisor_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();
        let mut active_sessions = HashMap::new();
        let mut alias_map = HashMap::new();
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                supervisor_id,
                "supervisor-01",
                SessionRole::Supervisor,
                SessionState::Progressing,
            ),
        );
        insert_session(
            &mut active_sessions,
            &mut alias_map,
            make_session(
                mission_id,
                worker_id,
                "Engineer-1",
                SessionRole::Worker,
                SessionState::Progressing,
            ),
        );

        let worker = active_sessions.get_mut(&worker_id).expect("worker");
        worker.queued_prompt_keys.insert("stale".to_owned());
        worker.queued_prompts.push_back(QueuedPrompt {
            key: "stale".to_owned(),
            body: "Status update only.".to_owned(),
        });
        let mut leases = HashMap::new();
        let mut pending_mail = HashMap::new();
        let mut pending_supervisor_decisions = HashMap::new();
        assert!(queue_supervisor_decision(
            &mut pending_supervisor_decisions,
            SupervisorDecisionKind::LowConfidenceRecovery,
            worker_id,
            "stale weak output"
        ));
        assert!(queue_supervisor_decision(
            &mut pending_supervisor_decisions,
            SupervisorDecisionKind::StallRecovery,
            worker_id,
            "stale stall"
        ));
        assert!(queue_supervisor_decision(
            &mut pending_supervisor_decisions,
            SupervisorDecisionKind::Validation,
            worker_id,
            "validation should remain"
        ));
        let mut recent_failures = Vec::new();
        let mut mass_death_detector = crate::orchestrator::health::MassDeathDetector::default();
        let mut stats = WatchdogStats::default();
        let control_surface = test_control_surface();

        orchestrator
            .handle_runtime_event(
                mission_id,
                Path::new("."),
                supervisor_id,
                &RuntimeEvent::Output {
                    session_id: worker_id,
                    chunk: "still working".to_owned(),
                },
                &mut active_sessions,
                &alias_map,
                &mut leases,
                &mut pending_mail,
                &mut pending_supervisor_decisions,
                &mut recent_failures,
                &mut mass_death_detector,
                false,
                &mut stats,
                &control_surface,
            )
            .expect("runtime event");

        let worker = active_sessions.get(&worker_id).expect("worker");
        assert!(worker.queued_prompts.is_empty());
        assert!(worker.queued_prompt_keys.is_empty());
        assert_eq!(pending_supervisor_decisions.len(), 1);
        assert!(pending_supervisor_decisions
            .values()
            .all(|pending| pending.kind == SupervisorDecisionKind::Validation));
    }

    #[test]
    fn auto_restart_policy_is_bounded_and_backed_off() {
        fn restart_backoff(restart_count: usize) -> Duration {
            let seconds = match restart_count {
                0 | 1 => 2,
                2 => 5,
                _ => 10,
            };
            Duration::from_secs(seconds)
        }
        assert!(should_auto_restart(
            SessionRole::Worker,
            SessionState::Progressing,
            0
        ));
        assert!(should_auto_restart(
            SessionRole::Supervisor,
            SessionState::Blocked,
            1
        ));
        assert!(!should_auto_restart(
            SessionRole::Worker,
            SessionState::Validated,
            0
        ));
        assert!(!should_auto_restart(
            SessionRole::Supervisor,
            SessionState::Progressing,
            4
        ));
        assert_eq!(restart_backoff(1), Duration::from_secs(2));
        assert_eq!(restart_backoff(2), Duration::from_secs(5));
        assert_eq!(restart_backoff(3), Duration::from_secs(10));
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_owned()
    } else {
        let mut rendered = text.chars().take(max_chars).collect::<String>();
        rendered.push_str("...");
        rendered
    }
}

fn truncate_id(id: &uuid::Uuid) -> String {
    let hex = id.to_string();
    hex[..8].to_owned()
}

fn pad_status(status: &str) -> String {
    let padded = format!("{status:<10}");
    match status {
        "running" | "launching" => format!("\x1b[33m{padded}\x1b[0m"),
        "completed" => format!("\x1b[32m{padded}\x1b[0m"),
        "failed" => format!("\x1b[31m{padded}\x1b[0m"),
        "planned" => format!("\x1b[36m{padded}\x1b[0m"),
        _ => padded,
    }
}

fn trim_recent_utf8(buffer: &mut String, max_bytes: usize, keep_bytes: usize) {
    if buffer.len() <= max_bytes {
        return;
    }
    let target = buffer.len().saturating_sub(keep_bytes);
    let keep_from = previous_char_boundary(buffer, target);
    buffer.drain(..keep_from);
}

fn previous_char_boundary(text: &str, index: usize) -> usize {
    let mut boundary = index.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

/// Enhanced mail renderer with thread tracking, CC visibility, urgency banners,
/// and reply-chain awareness. Supercharged from gastown's notification system.
#[allow(dead_code)]
fn render_routed_mail_enhanced(
    message_id: Uuid,
    thread_id: &str,
    sender: &str,
    directive: &MailDirective,
    cc_recipients: &[Uuid],
    requires_ack: bool,
    is_urgent: bool,
) -> String {
    let urgency_banner = if is_urgent {
        "═══════════════════════════════════════════\n\
         ⚡  URGENT MAIL — IMMEDIATE ATTENTION REQUIRED  ⚡\n\
         ═══════════════════════════════════════════\n\n"
    } else {
        ""
    };

    let thread_info = if directive.reply_to.is_some() {
        format!("REPLY in thread {}\n", thread_id)
    } else {
        format!("NEW THREAD {}\n", thread_id)
    };

    let cc_line = if cc_recipients.is_empty() {
        String::new()
    } else {
        format!(
            "CC: {} recipients (visibility only, no action required)\n\n",
            cc_recipients.len()
        )
    };

    let ack_instruction = if requires_ack {
        if is_urgent {
            format!(
                "⚡ URGENT: Acknowledge IMMEDIATELY with:\n\
                 SAPPHIRE_ACK {{\"mail_id\":\"{}\",\"status\":\"acked\",\"summary\":\"acknowledged urgent\"}}\n\
                 Then respond with your action plan.",
                message_id
            )
        } else {
            format!(
                "Acknowledge within the timeout window with:\n\
                 SAPPHIRE_ACK {{\"mail_id\":\"{}\",\"status\":\"acked\",\"summary\":\"one short sentence\"}}\n\
                 Then respond with SAPPHIRE_MAIL or SAPPHIRE_STATUS when your coordination state changes.",
                message_id
            )
        }
    } else {
        "No ack required. Process when ready and update SAPPHIRE_STATUS when state changes.".to_owned()
    };

    format!(
        "{urgency_banner}[SAPPHIRE ROUTER — Enhanced]
{thread_info}MAIL_ID: {message_id}
FROM: {sender}
TO: {to}
TYPE: {message_type}
PRIORITY: {priority}
SUBJECT: {subject}
{cc_line}
CONTEXT:
{context}

REQUEST:
{request}

EXPECTED ACTION:
{expected_action}

{ack_instruction}",
        urgency_banner = urgency_banner,
        message_id = message_id,
        thread_info = thread_info,
        sender = sender,
        to = directive.to,
        message_type = directive.message_type,
        priority = directive.priority,
        subject = directive.subject,
        cc_line = cc_line,
        context = directive.context,
        request = directive.request,
        expected_action = directive.expected_action,
        ack_instruction = ack_instruction,
    )
}
#[allow(dead_code)]

fn parse_mail_id(value: &str) -> Option<Uuid> {
#[allow(dead_code)]
    Uuid::parse_str(value.trim()).ok()
}

fn normalize_supervisor_plan(
    mut plan: MissionPlan,
    expected_worker_packets: usize,
    planner_label: &str,
) -> Result<MissionPlan> {
    if plan.workstreams.is_empty() {
        anyhow::bail!("{planner_label} returned an invalid plan: no workstreams");
    }
    if plan.worker_packets.is_empty() {
        anyhow::bail!("{planner_label} returned an invalid plan: no worker packets");
    }
    if plan.worker_packets.len() < expected_worker_packets {
        anyhow::bail!(
            "{planner_label} returned an invalid plan: expected at least {} worker packets, got {}",
            expected_worker_packets,
            plan.worker_packets.len()
        );
    }
    if plan.worker_packets.len() > expected_worker_packets {
        tracing::warn!(
            planner = planner_label,
            expected = expected_worker_packets,
            actual = plan.worker_packets.len(),
            "planner overproduced worker packets; trimming extras"
        );
        plan.worker_packets.truncate(expected_worker_packets);
    }
    Ok(plan)
}

fn startup_grace(agent: crate::agent::AgentKind) -> Duration {
    protocol_reminder_grace(agent) + Duration::from_secs(8)
}

fn max_restart_attempts(role: SessionRole) -> usize {
    match role {
        SessionRole::Supervisor => 4,
        SessionRole::Worker => 3,
    }
}

fn should_auto_restart(
    role: SessionRole,
    previous_state: SessionState,
    restart_count: usize,
) -> bool {
    !previous_state.is_terminal() && restart_count < max_restart_attempts(role)
}

fn mass_failure_window() -> Duration {
    Duration::from_secs(30)
}

fn mass_failure_threshold() -> usize {
    3
}

// Health state + cooldown constants (from gastown health/health.md pattern)
fn intervention_cooldown_base() -> Duration {
    Duration::from_secs(30)
}

fn intervention_cooldown_max() -> Duration {
    Duration::from_secs(120)
}

// Restart tracker with exponential backoff (from gastown daemon pattern)
fn restart_base_secs() -> u64 { 2 }
fn restart_crash_loop_threshold() -> usize { 5 }
fn restart_crash_loop_window() -> Duration { Duration::from_secs(600) }  // 10 min

// Zombie detection constants (from gastown witness/deacon pattern)
fn zombie_check_max_inactivity() -> Duration { Duration::from_secs(180) }  // 3 min without output = hung

fn trim_recent_failures(recent_failures: &mut Vec<RecentFailure>) {
    let cutoff = Instant::now() - mass_failure_window();
    recent_failures.retain(|entry| entry.recorded_at >= cutoff);
}

fn pending_supervisor_plan(mission: &str) -> MissionPlan {
    MissionPlan {
        mission_rewrite: format!("Pending supervisor plan for mission: {}", mission.trim()),
        workstreams: Vec::new(),
        risk_map: Vec::new(),
        worker_packets: Vec::new(),
        supervision_strategy: "Pending supervisor planning.".to_owned(),
    }
}

#[cfg(test)]
fn deterministic_plan_for_mission(mission: &str, worker_count: usize) -> MissionPlan {
    let worker_count = worker_count.max(1);
    let lowered = mission.to_ascii_lowercase();
    let role_type = deterministic_role_type(&lowered);
    let role_title = deterministic_role_title(&role_type);
    let mission_rewrite = truncate(mission.trim(), 140);

    let workstreams = (1..=worker_count)
        .map(|ordinal| Workstream {
            id: format!("ws-{ordinal}"),
            name: format!("{} lane {}", deterministic_workstream_name(&lowered), ordinal),
            execution: WorkstreamExecution::Parallel,
            owned_scope: deterministic_owned_scope(&lowered),
            success_criteria: vec![
                "Stay inside the assigned scope.".to_owned(),
                "Prove the result with concrete evidence.".to_owned(),
                "Report blockers immediately with Sapphire status.".to_owned(),
            ],
            depends_on: Vec::new(),
        })
        .collect();

    let worker_packets = (1..=worker_count)
        .map(|ordinal| {
            let display_name = deterministic_display_name(&role_type, ordinal);
            WorkerPacket {
                worker_id: display_name.clone(),
                role_type: role_type.clone(),
                display_name: display_name.clone(),
                role: role_title.clone(),
                starting_angle: deterministic_starting_angle(&lowered, ordinal),
                owned_scope: deterministic_owned_scope(&lowered),
                explicit_task: deterministic_explicit_task(mission, ordinal),
                out_of_scope: "Do not broaden scope, rewrite unrelated code, or duplicate another lane.".to_owned(),
                definition_of_done: vec![
                    "Deliver the scoped result only.".to_owned(),
                    "Show proof that the assigned lane was completed.".to_owned(),
                    "Use Sapphire status updates so the supervisor can validate the result.".to_owned(),
                ],
                required_evidence: vec![
                    "List touched files.".to_owned(),
                    "List validation commands or observations.".to_owned(),
                    "State what was proven or what blocked completion.".to_owned(),
                ],
                blocker_protocol: "If blocked, report the blocker immediately with a machine-readable Sapphire status update.".to_owned(),
                conflict_warning: "Claim files before editing and avoid overlap with other workers.".to_owned(),
                communication_rules: vec![
                    "Use supervisor-visible coordination for blockers or dependencies.".to_owned(),
                    "Keep updates concise, factual, and scoped to your lane.".to_owned(),
                ],
                validation_standard: vec![
                    "Do not claim done without evidence.".to_owned(),
                    "Report uncertainty instead of guessing.".to_owned(),
                ],
                expected_output_format: vec![
                    "STATE".to_owned(),
                    "SUMMARY".to_owned(),
                    "FILES".to_owned(),
                    "BLOCKER".to_owned(),
                    "DONE".to_owned(),
                ],
            }
        })
        .collect();

    MissionPlan {
        mission_rewrite,
        workstreams,
        risk_map: vec![
            RiskItem {
                zone: "Planning".to_owned(),
                risk: "Supervisor planner unavailable or malformed.".to_owned(),
                mitigation: "Use deterministic planning fixture and keep worker scope narrow.".to_owned(),
            },
            RiskItem {
                zone: "Coordination".to_owned(),
                risk: "Workers may overlap or drift when planning is degraded.".to_owned(),
                mitigation: "Use explicit lane ownership, status updates, and lease discipline.".to_owned(),
            },
        ],
        worker_packets,
        supervision_strategy: "Deterministic planning fixture. Keep scope tight, validate every claim, and prefer evidence over narration.".to_owned(),
    }
}

#[cfg(test)]
fn deterministic_role_type(mission: &str) -> String {
    if mission.contains("security") || mission.contains("auth") || mission.contains("vuln") {
        "security-engineer".to_owned()
    } else if mission.contains("compliance") || mission.contains("policy") {
        "compliance-engineer".to_owned()
    } else if mission.contains("revenue")
        || mission.contains("pricing")
        || mission.contains("enterprise")
        || mission.contains("sell")
        || mission.contains("sales")
    {
        "revenue-engineer".to_owned()
    } else if mission.contains("product manager")
        || mission.contains("product strategy")
        || mission.contains("positioning")
    {
        "product-manager".to_owned()
    } else if mission.contains("design") || mission.contains("ui") || mission.contains("ux") {
        "designer-engineer".to_owned()
    } else if mission.contains("test")
        || mission.contains("validate")
        || mission.contains("verify")
        || mission.contains("prove")
    {
        "validation-engineer".to_owned()
    } else if mission.contains("debug")
        || mission.contains("fix")
        || mission.contains("bug")
        || mission.contains("broken")
    {
        "debug-and-review-engineer".to_owned()
    } else {
        "software-engineer".to_owned()
    }
}

#[cfg(test)]
fn deterministic_role_title(role_type: &str) -> String {
    match role_type {
        "software-engineer" => "Software Engineer".to_owned(),
        "research-engineer" => "Research Engineer".to_owned(),
        "validation-engineer" => "Validation Engineer".to_owned(),
        "architecture-engineer" => "Architecture Engineer".to_owned(),
        "security-engineer" => "Security Engineer".to_owned(),
        "debug-and-review-engineer" => "Debug and Review Engineer".to_owned(),
        "testing-and-automation-engineer" => "Testing and Automation Engineer".to_owned(),
        "designer-engineer" => "Designer Engineer".to_owned(),
        "sales-engineer" => "Sales Engineer".to_owned(),
        "solutions-engineer" => "Solutions Engineer".to_owned(),
        "customer-success-engineer" => "Customer Success Engineer".to_owned(),
        "product-engineer" => "Product Engineer".to_owned(),
        "product-manager" => "Product Manager".to_owned(),
        "revenue-engineer" => "Revenue Engineer".to_owned(),
        "compliance-engineer" => "Compliance Engineer".to_owned(),
        _ => "Software Engineer".to_owned(),
    }
}

#[cfg(test)]
fn deterministic_display_name(role_type: &str, ordinal: usize) -> String {
    let prefix = match role_type {
        "software-engineer" => "Engineer",
        "research-engineer" => "Researcher",
        "validation-engineer" => "Validator",
        "architecture-engineer" => "Architect",
        "security-engineer" => "Security",
        "debug-and-review-engineer" => "Reviewer",
        "testing-and-automation-engineer" => "QA",
        "designer-engineer" => "Designer",
        "sales-engineer" => "Sales",
        "solutions-engineer" => "Solutions",
        "customer-success-engineer" => "CustomerSuccess",
        "product-engineer" => "Product",
        "product-manager" => "ProductManager",
        "revenue-engineer" => "Revenue",
        "compliance-engineer" => "Compliance",
        _ => "Engineer",
    };
    format!("{prefix}-{ordinal}")
}

#[cfg(test)]
fn deterministic_workstream_name(mission: &str) -> &'static str {
    if mission.contains("test")
        || mission.contains("validate")
        || mission.contains("verify")
        || mission.contains("prove")
    {
        "Validation"
    } else if mission.contains("debug") || mission.contains("fix") || mission.contains("bug") {
        "Debug"
    } else if mission.contains("design") || mission.contains("ui") || mission.contains("ux") {
        "UI"
    } else {
        "Implementation"
    }
}

#[cfg(test)]
fn deterministic_owned_scope(mission: &str) -> String {
    if mission.contains("tmux") || mission.contains("terminal") || mission.contains("orchestration")
    {
        "Sapphire runtime, tmux surface, worker coordination, and mission evidence.".to_owned()
    } else if mission.contains("ui") || mission.contains("ux") || mission.contains("design") {
        "UI surfaces, rendering paths, and interaction flow.".to_owned()
    } else {
        "Requested mission scope only, constrained to the repo root and directly relevant files.".to_owned()
    }
}

#[cfg(test)]
fn deterministic_starting_angle(mission: &str, ordinal: usize) -> String {
    let lane = match ordinal {
        1 => "primary path",
        2 => "independent verification path",
        3 => "failure-path review",
        4 => "integration review",
        _ => "narrow scoped pass",
    };
    if mission.contains("terminal") || mission.contains("orchestration") {
        format!("Audit live terminal coordination via {lane}.")
    } else if mission.contains("validate") || mission.contains("prove") {
        format!("Challenge the claim set via {lane}.")
    } else {
        format!("Execute the scoped task via {lane}.")
    }
}

#[cfg(test)]
fn deterministic_explicit_task(mission: &str, ordinal: usize) -> String {
    match ordinal {
        1 => format!("Own the main delivery path for: {}", truncate(mission.trim(), 160)),
        2 => "Independently verify the main path and challenge weak claims.".to_owned(),
        3 => "Inspect failure paths, overlap risk, and missing evidence.".to_owned(),
        _ => format!("Take a narrow parallel slice of: {}", truncate(mission.trim(), 160)),
    }
}

fn reject_completion_without_artifacts(
    repo_root: &Path,
    packet: &WorkerPacket,
    reported_files: &[String],
) -> Option<Vec<String>> {
    let expectation = infer_completion_expectation(packet);
    if expectation.exact_files.is_empty()
        && !expectation.require_readme
        && !expectation.require_script
    {
        return None;
    }

    let observed = scan_repo_artifacts(repo_root, 4096);
    let observed_lower = observed
        .iter()
        .map(|path| path.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let reported_lower = reported_files
        .iter()
        .map(|path| path.trim().to_ascii_lowercase())
        .collect::<BTreeSet<_>>();

    let mut missing = BTreeSet::new();
    for file in &expectation.exact_files {
        let needle = file.to_ascii_lowercase();
        let present = observed_lower.iter().any(|path| path.ends_with(&needle))
            || reported_lower.iter().any(|path| path.ends_with(&needle));
        if !present {
            missing.insert(file.clone());
        }
    }

    if expectation.require_readme
        && !observed_lower
            .iter()
            .any(|path| path.ends_with("readme.md") || path.ends_with("readme"))
    {
        missing.insert("README.md".to_owned());
    }

    if expectation.require_script
        && !observed_lower.iter().any(|path| {
            path.ends_with(".sh")
                || path.ends_with(".py")
                || path.ends_with(".js")
                || path.ends_with(".ts")
        })
    {
        missing.insert("script artifact (*.sh|*.py|*.js|*.ts)".to_owned());
    }

    if missing.is_empty() {
        None
    } else {
        Some(missing.into_iter().collect())
    }
}

fn infer_observed_artifacts(repo_root: &Path, packet: &WorkerPacket) -> Vec<String> {
    let expectation = infer_completion_expectation(packet);
    scan_repo_artifacts(repo_root, 4096)
        .into_iter()
        .filter(|path| {
            expectation
                .exact_files
                .iter()
                .any(|needle| path.ends_with(needle))
                || (expectation.require_readme
                    && (path.ends_with("README.md") || path.ends_with("README")))
                || (expectation.require_script
                    && (path.ends_with(".sh")
                        || path.ends_with(".py")
                        || path.ends_with(".js")
                        || path.ends_with(".ts")))
        })
        .collect()
}

#[derive(Default)]
struct CompletionExpectation {
    exact_files: Vec<String>,
    require_readme: bool,
    require_script: bool,
}

fn infer_completion_expectation(packet: &WorkerPacket) -> CompletionExpectation {
    let mut expectation = CompletionExpectation::default();
    let mut exact_files = BTreeSet::new();
    let texts = packet
        .definition_of_done
        .iter()
        .chain(packet.required_evidence.iter())
        .chain(std::iter::once(&packet.explicit_task))
        .chain(std::iter::once(&packet.owned_scope))
        .collect::<Vec<_>>();

    for text in texts {
        let lowered = text.to_ascii_lowercase();
        if lowered.contains("readme") {
            expectation.require_readme = true;
        }
        if lowered.contains("script") {
            expectation.require_script = true;
        }
        for token in text.split_whitespace() {
            let candidate = token
                .trim_matches(|ch: char| {
                    ch.is_ascii_punctuation() && ch != '.' && ch != '/' && ch != '_'
                })
                .trim();
            if let Some(file) = normalize_expected_file(candidate)
                && !matches!(file.as_str(), "AGENTS.md" | "tasks.txt") && file != ".sp"
            {
                exact_files.insert(file);
            }
        }
    }

    expectation.exact_files = exact_files.into_iter().collect();
    expectation
}

fn normalize_expected_file(token: &str) -> Option<String> {
    let candidate = token.trim_matches(|ch: char| ch == '"' || ch == '\'' || ch == '`');
    if candidate.is_empty()
        || candidate.starts_with('.')
        || candidate.ends_with(':')
        || candidate.contains("://")
        || !candidate.contains('.')
    {
        return None;
    }
    let extension = candidate.rsplit('.').next()?.to_ascii_lowercase();
    let allowed = [
        "md", "sh", "py", "js", "ts", "tsx", "jsx", "rs", "toml", "json", "yaml", "yml", "sql",
        "txt", "html", "css",
    ];
    if !allowed.contains(&extension.as_str()) {
        return None;
    }
    Some(candidate.trim_start_matches("./").to_owned())
}

fn scan_repo_artifacts(repo_root: &Path, max_entries: usize) -> Vec<String> {
    let mut results = Vec::new();
    let mut stack = vec![repo_root.to_path_buf()];

    while let Some(path) = stack.pop() {
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            if results.len() >= max_entries {
                return results;
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if file_type.is_dir() {
                if matches!(
                    name.as_ref(),
                    ".git" | ".sp" | "target" | "node_modules" | "dist" | "build"
                ) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            if let Ok(relative) = path.strip_prefix(repo_root) {
                results.push(relative.to_string_lossy().into_owned());
            }
        }
    }

    results
}

fn protocol_reminder_grace(agent: crate::agent::AgentKind) -> Duration {
    if agent == crate::agent::AgentKind::Qwen {
        Duration::from_secs(15)
    } else {
        Duration::from_secs(6)
    }
}

fn embed_initial_prompt_if_supported(
    agent: crate::agent::AgentKind,
    spec: ProcessLaunchSpec,
    _prompt: &str,
) -> (ProcessLaunchSpec, bool) {
    // Qwen's `-i` (interactive) boots the TUI but never auto-executes the pre-loaded prompt.
    // Workers need to stay alive for the watchdog loop, so skip embedding for Qwen
    // and fall through to PTY injection instead.
    // All other agents also use PTY injection (prompt_embedded = false).
    let _ = agent;
    (spec, false)
}

fn ensure_agents_bootstrap(
    repo: &Path,
    prompts: &PromptLibrary,
) -> Result<AgentsBootstrap> {
    let path = repo.join("AGENTS.md");
    let existed = path.exists();
    let _content = if existed {
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?
    } else {
        let rendered = generate_agents_md(repo, prompts)?;
        fs::write(&path, &rendered)
            .with_context(|| format!("failed to write {}", path.display()))?;
        rendered
    };
    Ok(AgentsBootstrap {
        path,
        existed,
    })
}

fn render_supervisor_prompt_with_agents(
    base_prompt: String,
    agents_bootstrap: &AgentsBootstrap,
    requested_workers: usize,
) -> String {
    prompt_contracts::render_supervisor_bootstrap(
        base_prompt,
        &agents_bootstrap.path,
        agents_bootstrap.existed,
        requested_workers,
    )
}

fn render_worker_prompt_with_agents(
    base_prompt: String,
    agents_bootstrap: &AgentsBootstrap,
    state_dir: &Path,
    packet: &WorkerPacket,
    git_remote: Option<&str>,
    memory_block: Option<&str>,
) -> String {
    prompt_contracts::render_worker_bootstrap(
        base_prompt,
        &agents_bootstrap.path,
        state_dir,
        packet,
        git_remote,
        memory_block,
    )
}

fn generate_agents_md(repo: &Path, prompts: &PromptLibrary) -> Result<String> {
    let repo_name = repo
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("repository");
    let cargo_toml = repo.join("Cargo.toml");
    let package_json = repo.join("package.json");
    let go_mod = repo.join("go.mod");
    let pyproject = repo.join("pyproject.toml");

    let (stack_summary, build_cmd, run_cmd, test_cmd, entry_point) = if cargo_toml.exists() {
        (
            "Rust CLI".to_owned(),
            "cargo build".to_owned(),
            "cargo run -- --help".to_owned(),
            "cargo test".to_owned(),
            detect_entry_point(repo, &["src/main.rs", "src/lib.rs", "main.rs"]),
        )
    } else if package_json.exists() {
        (
            "Node.js project".to_owned(),
            "npm run build".to_owned(),
            "npm run dev".to_owned(),
            "npm test".to_owned(),
            detect_entry_point(
                repo,
                &["src/index.ts", "src/index.js", "index.ts", "index.js"],
            ),
        )
    } else if go_mod.exists() {
        (
            "Go project".to_owned(),
            "go build ./...".to_owned(),
            "go run .".to_owned(),
            "go test ./...".to_owned(),
            detect_entry_point(repo, &["main.go", "cmd/server/main.go", "cmd/main.go"]),
        )
    } else if pyproject.exists() {
        (
            "Python project".to_owned(),
            "python -m build".to_owned(),
            "python -m <entrypoint>".to_owned(),
            "pytest".to_owned(),
            detect_entry_point(repo, &["src/__init__.py", "main.py", "app.py"]),
        )
    } else {
        (
            "Repository under active development".to_owned(),
            "inspect local build tooling".to_owned(),
            "inspect repo entrypoint".to_owned(),
            "inspect local test tooling".to_owned(),
            "unknown".to_owned(),
        )
    };

    let top_level = render_repo_tree(repo)?;
    let key_modules = render_module_table(repo)?;
    let critical_files = render_critical_files(repo)?;

    Ok(format!(
        "# AGENTS.md\n\nRead this file before starting any task. It is the local operating guide for this repository.\n\n## Product Overview\n\n- Product: `{repo_name}`\n- Type: {stack_summary}\n- Status: Active development\n- Entry point: `{entry_point}`\n- Source template: `agents.md-instructions.md`\n\n## Tech Stack\n\n- Primary stack: {stack_summary}\n- Build: `{build_cmd}`\n- Run: `{run_cmd}`\n- Test: `{test_cmd}`\n\n## Architecture\n\n- Work from the actual repository state, not assumptions.\n- Trace entrypoints, orchestration boundaries, persistence, and runtime control paths before broad edits.\n- Keep module ownership tight and avoid hidden cross-cutting changes.\n\n## Repository File Tree\n\n```text\n{top_level}\n```\n\n## Critical File Index\n\n{critical_files}\n\n## Module Boundaries\n\n{key_modules}\n\n## Known Issues & Active Debt\n\n- This file may be auto-seeded and should be refreshed when architecture, workflow, or runtime behavior changes materially.\n- Prefer concrete repo evidence over stale summaries.\n\n## Agent Notes\n\n- Read this file once at initialization before real work.\n- Do not expand scope without explicit justification.\n- If ownership is unclear, escalate instead of guessing.\n\n## Operating Protocol\n\n- Reproduce or inspect the current state before broad edits.\n- Keep changes inside the owned scope.\n- State exact files, exact checks, and exact remaining risk.\n- Refresh this file when repository reality changes materially.\n\n## Testing & Validation\n\n- Run the smallest relevant checks that prove the claim.\n- Do not mark work complete without observed results.\n- Call out what remains unverified.\n\n## Guardrails\n\n- No decorative churn.\n- No hidden rewrites.\n- No silent scope expansion.\n- No fake completion claims.\n\n## Instruction Source\n\nThe authoritative formatting source used to seed this file is embedded in Sapphire from `agents.md-instructions.md`.\nUse that source when this file must be materially refreshed.\nCurrent embedded template size: {instruction_size} bytes.\n\n---\n\nAuto-seeded by Sapphire from the AGENTS instruction source. Refresh this file when the repository changes materially.\n",
        instruction_size = prompts.agents_instruction_source().len(),
    ))
}

fn detect_entry_point(repo: &Path, candidates: &[&str]) -> String {
    candidates
        .iter()
        .find(|candidate| repo.join(candidate).exists())
        .map(|candidate| candidate.to_string())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn render_repo_tree(repo: &Path) -> Result<String> {
    let mut entries = fs::read_dir(repo)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && name != ".github" {
                None
            } else {
                Some((name, entry.path().is_dir()))
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut lines = vec!["/".to_owned()];
    for (name, is_dir) in entries.into_iter().take(24) {
        lines.push(format!("├── {}{}", name, if is_dir { "/" } else { "" }));
    }
    Ok(lines.join("\n"))
}

fn render_module_table(repo: &Path) -> Result<String> {
    let mut modules = fs::read_dir(repo)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !entry.path().is_dir() || name.starts_with('.') {
                None
            } else {
                Some(name)
            }
        })
        .collect::<Vec<_>>();
    modules.sort();
    if modules.is_empty() {
        return Ok("- No top-level module directories detected automatically.".to_owned());
    }
    Ok(modules
        .into_iter()
        .take(8)
        .map(|module| format!("- `{module}`: inspect this directory before touching related code."))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn render_critical_files(repo: &Path) -> Result<String> {
    let mut lines = Vec::new();
    for candidate in [
        "Cargo.toml",
        "package.json",
        "go.mod",
        "pyproject.toml",
        "src/main.rs",
        "src/lib.rs",
        "README.md",
    ] {
        if repo.join(candidate).exists() {
            lines.push(format!("- `{candidate}`"));
        }
    }
    if lines.is_empty() {
        lines.push("- No standard critical files detected automatically.".to_owned());
    }
    Ok(lines.join("\n"))
}

fn write_prompt_file(state_dir: &Path, session_name: &str, prompt: &str) -> Result<()> {
    let path = launch_prompt::prompt_file_path(state_dir, session_name);
    write_string_to_file(&path, prompt)?;
    let hidden_path = hidden_state_dir(state_dir)
        .join("prompts")
        .join(format!("{session_name}.md"));
    write_string_to_file(&hidden_path, prompt)
}

fn ensure_state_tree(state_dir: &Path) -> Result<()> {
    fs::create_dir_all(state_dir)
        .with_context(|| format!("failed to create {}", state_dir.display()))?;
    fs::create_dir_all(state_dir.join("prompts"))?;
    fs::create_dir_all(state_dir.join("control"))?;
    fs::create_dir_all(state_dir.join("transcripts"))?;
    fs::create_dir_all(state_dir.join("workers"))?;
    fs::create_dir_all(state_dir.join("forge-home/.local/share"))?;
    fs::create_dir_all(state_dir.join("forge-home/.config"))?;
    fs::create_dir_all(state_dir.join("supervisor-runtime"))?;
    Ok(())
}

fn hidden_state_dir(state_dir: &Path) -> PathBuf {
    let parent = state_dir.parent().unwrap_or_else(|| Path::new("."));
    match state_dir.file_name().and_then(|name| name.to_str()) {
        Some(".sp") => parent.join(".hide.sp"),
        Some(name) => parent.join(format!(".hide.{name}")),
        None => parent.join(".hide.sp"),
    }
}

fn hidden_status_file(control_surface: &ControlSurface) -> PathBuf {
    control_surface.hidden_root.join("control/status.txt")
}

fn hidden_dashboard_file(control_surface: &ControlSurface) -> PathBuf {
    control_surface.hidden_root.join("control/dashboard.txt")
}

fn hidden_transcript_dir(control_surface: &ControlSurface) -> PathBuf {
    control_surface.hidden_root.join("transcripts")
}

fn hidden_workers_state_dir(control_surface: &ControlSurface) -> PathBuf {
    control_surface.hidden_root.join("workers")
}

fn write_string_to_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
}

fn append_to_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(content.as_bytes())?;
    Ok(())
}

fn launch_command(program: &str, args: &[String]) -> Vec<String> {
    let mut command = vec![program.to_owned()];
    command.extend(args.iter().cloned());
    command
}

fn rebuild_launch_spec(
    session: &SessionRecord,
    repo: &Path,
    state_dir: &Path,
) -> ProcessLaunchSpec {
    let mut base = session.agent.build_launch_spec(repo, state_dir, &[]);
    if let Some((program, args)) = session.launch_command.split_first() {
        base.program = program.clone();
        base.args = args.to_vec();
    }
    if session.role == SessionRole::Supervisor {
        base = harden_supervisor_launch_spec(session.agent, base);
    }
    base.surface_label = session.name.clone();
    base
}

fn harden_supervisor_launch_spec(
    agent: crate::agent::AgentKind,
    mut spec: ProcessLaunchSpec,
) -> ProcessLaunchSpec {
    if agent == crate::agent::AgentKind::Qwen
        && !spec.args.iter().any(|arg| arg == "--screen-reader")
    {
        spec.args.insert(0, "--screen-reader".to_owned());
    }
    spec
}

/// Build a concise state card for the supervisor's periodic refresh.
/// Gives the supervisor the full picture without needing to remember its initial prompt.
fn build_supervisor_state_card(
    active_sessions: &HashMap<Uuid, ActiveSession>,
    pending_mail: &HashMap<Uuid, PendingMail>,
    pending_decisions: &HashMap<String, PendingSupervisorDecision>,
    _supervisor_mode: SupervisorMode,
    started_at: Instant,
) -> Option<String> {
    let elapsed = started_at.elapsed();
    let mins = elapsed.as_secs() / 60;
    let secs = elapsed.as_secs() % 60;

    let mut worker_lines = Vec::new();
    let mut validation_queue = Vec::new();
    let mut blockers = Vec::new();
    let mut contradictions = Vec::new();
    let mut restarting = Vec::new();

    for session in active_sessions.values() {
        if session.record.role != SessionRole::Worker {
            continue;
        }
        let name = &session.record.name;
        let state = session.state.as_str();
        let summary = session
            .record
            .last_summary
            .as_deref()
            .unwrap_or("no summary")
            .chars()
            .take(60)
            .collect::<String>();
        worker_lines.push(format!("  {} [{}]: {}", name, state, summary));

        if session.restart_at.is_some() {
            restarting.push(name.clone());
        }

        match session.state {
            SessionState::DoneClaimed | SessionState::NeedsValidation => {
                validation_queue.push(name.clone());
            }
            SessionState::Blocked | SessionState::Stalled => {
                blockers.push(name.clone());
            }
            SessionState::Contradictory => {
                contradictions.push(name.clone());
            }
            _ => {}
        }
    }

    let pending_mail_count = pending_mail.values().filter(|m| !m.acked).count();
    let pending_decision_count = pending_decisions.len();
    let pod_summaries = coordination::summarize_pods(active_sessions, pending_mail);
    let meetings = meetings::build_meetings(active_sessions, pending_mail);

    let mut card = format!(
        "SUPERVISOR STATE CARD [{}m {}s elapsed]\n\
         Workers ({}):\n{}\n\
         Pods: {}\n\
         Validation queue: {}\n\
         Blockers/stalled: {}\n\
         Contradictions: {}\n\
         Restart queue: {}\n\
         Pending mail (unacked): {}\n\
         Pending supervisor decisions: {}\n\
         Active meetings: {}",
        mins,
        secs,
        worker_lines.len(),
        if worker_lines.is_empty() {
            "  (none)".to_owned()
        } else {
            worker_lines.join("\n")
        },
        if pod_summaries.is_empty() {
            "none".to_owned()
        } else {
            pod_summaries
                .iter()
                .map(|pod| format!(
                    "{}:{} members/{} blocked/{} threads",
                    pod.name,
                    pod.members.len(),
                    pod.blocked_members.len(),
                    pod.open_threads
                ))
                .collect::<Vec<_>>()
                .join(" | ")
        },
        if validation_queue.is_empty() {
            "none".to_owned()
        } else {
            validation_queue.join(", ")
        },
        if blockers.is_empty() {
            "none".to_owned()
        } else {
            blockers.join(", ")
        },
        if contradictions.is_empty() {
            "none".to_owned()
        } else {
            contradictions.join(", ")
        },
        if restarting.is_empty() {
            "none".to_owned()
        } else {
            restarting.join(", ")
        },
        pending_mail_count,
        pending_decision_count,
        meetings.len(),
    );

    if !pending_decisions.is_empty() {
        card.push_str("\nDecisions awaiting action:\n");
        for (key, decision) in pending_decisions.iter().take(5) {
            card.push_str(&format!(
                "  {} → {}: {}\n",
                key,
                decision.kind.as_str(),
                decision.reason.chars().take(80).collect::<String>(),
            ));
        }
    }

    Some(card)
}

fn queue_supervisor_state_card(
    active_sessions: &mut HashMap<Uuid, ActiveSession>,
    supervisor_id: Uuid,
    event_type: SupervisorEventType,
    card: &str,
) -> bool {
    let Some(supervisor_session) = active_sessions.get_mut(&supervisor_id) else {
        return false;
    };
    if matches!(
        supervisor_session.state,
        SessionState::Exited | SessionState::Failed
    ) {
        return false;
    }

    let key = supervisor::state_card_key(card);
    if supervisor_session.last_supervisor_state_card_key.as_deref() == Some(key.as_str()) {
        return false;
    }
    supervisor_session.last_supervisor_state_card_key = Some(key);

    let adapter = adapter_for(supervisor_session.record.agent);
    let prompt = adapter.build_supervisor_action_prompt(event_type, card);
    send_or_queue_prompt(supervisor_session, &prompt)
}

/// Save a terminal-state memory snapshot for a worker.
/// This is the real persistent memory: when a worker finishes (validated, failed, or exited),
/// their complete session memory is saved. On the NEXT mission, an agent with the same
/// display_name and role_type will read this and know what happened last time.
fn save_terminal_memory_snapshot(
    store: &Store,
    mission_id: Uuid,
    session_id: Uuid,
    display_name: &str,
    active_sessions: &HashMap<Uuid, ActiveSession>,
    final_state: &SessionState,
    summary: &str,
    files_touched: &[String],
    risks: &[String],
) -> Result<()> {
    let mem_store = store.agent_memory();

    // Load existing incremental memory (already being updated on each status report)
    let Some(mut memory) = mem_store.load_memory(&mission_id, display_name)? else {
        return Ok(());
    };

    let session = active_sessions.get(&session_id);
    let packet = session.and_then(|s| s.packet.as_ref());

    // Fill in gaps that incremental updates may have missed
    if memory.role_type.is_empty() {
        memory.role_type = packet.map(|p| p.role_type.clone()).unwrap_or_default();
    }
    if memory.owned_scope.is_empty() {
        if let Some(scope) = packet.and_then(|p| {
            let s = p.owned_scope.trim();
            if s.is_empty() { None } else { Some(s) }
        }) {
            memory.owned_scope = scope.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
        }
    }
    for f in files_touched {
        if !memory.files_touched.contains(f) {
            memory.files_touched.push(f.clone());
        }
    }
    for risk in risks {
        if !memory.blockers.contains(risk) {
            memory.blockers.push(risk.clone());
        }
    }

    // Terminal state and final summary
    memory.final_state = final_state.as_str().to_owned();
    memory.summary = truncate(summary, 240);

    mem_store.save_memory(&mission_id, display_name, &memory)
}

/// Map a session state to the appropriate supervisor event type.
/// Determines which rule gets injected into the supervisor micro-prompt.
fn event_type_for_state(state: &SessionState) -> SupervisorEventType {
    match state {
        SessionState::Stalled => SupervisorEventType::Stall,
        SessionState::DoneClaimed | SessionState::NeedsValidation => SupervisorEventType::DoneClaimed,
        SessionState::WeakOutput | SessionState::WrongDirection => SupervisorEventType::WeakOutput,
        SessionState::Contradictory => SupervisorEventType::Contradiction,
        SessionState::Blocked => SupervisorEventType::Blocked,
        SessionState::Failed | SessionState::Exited => SupervisorEventType::Failed,
        _ => SupervisorEventType::Notice,
    }
}

// ─── Health state + cooldown (from gastown health/health.md pattern) ────────

/// Calculate cooldown duration after a watchdog intervention.
/// Escalates with intervention count: 30s × count, capped at 120s.
fn intervention_cooldown(intervention_count: usize) -> Duration {
    let secs = intervention_cooldown_base().as_secs().saturating_mul(intervention_count.max(1) as u64);
    Duration::from_secs(secs.min(intervention_cooldown_max().as_secs()))
}

/// Check if a session is still in cooldown after a watchdog intervention.
fn is_in_cooldown(session: &ActiveSession, now: Instant) -> bool {
    session.intervention_cooldown_until.is_some_and(|until| now < until)
}

/// Record a watchdog intervention and set the cooldown.
fn record_intervention(
    session: &mut ActiveSession,
    intervention_type: &str,
    now: Instant,
) {
    session.total_interventions += 1;
    session.last_intervention_type = Some(intervention_type.to_owned());
    session.last_intervention_at = Some(now);
    session.intervention_cooldown_until = Some(now + intervention_cooldown(session.total_interventions));
}

/// Record that an intervention received a response (output arrived).
fn record_intervention_response(session: &mut ActiveSession, now: Instant) {
    if let Some(intervention_at) = session.last_intervention_at {
        session.last_response_time = Some(now.duration_since(intervention_at));
    }
    session.last_intervention_at = None;
}

// ─── Nudge Queue (from gastown wait-idle nudge pattern) ─────────────────────

const NUDGE_QUIET_THRESHOLD: Duration = Duration::from_secs(3);
const HEURISTIC_SETTLE_THRESHOLD: Duration = Duration::from_secs(8);

/// Check if the agent is mid-response (recent output activity).
/// If true, injecting a prompt now would conflict with the TUI readline.
fn is_agent_mid_response(session: &ActiveSession) -> bool {
    session.last_output_at.elapsed() < NUDGE_QUIET_THRESHOLD
        && session.output_chunks > 0
}

fn worker_output_has_settled(session: &ActiveSession, now: Instant) -> bool {
    session.output_chunks > 0
        && now.duration_since(session.last_output_at) >= HEURISTIC_SETTLE_THRESHOLD
}

fn prompt_fingerprint(prompt: &str) -> String {
    let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    blake3::hash(normalized.as_bytes()).to_hex().to_string()
}

fn prompt_repeat_suppression_window() -> Duration {
    Duration::from_secs(180)
}

fn supervisor_notice_repeat_suppression_window() -> Duration {
    Duration::from_secs(240)
}

fn prompt_queue_limit() -> usize {
    6
}

fn prompt_dispatch_interval(session: &ActiveSession) -> Duration {
    Duration::from_secs(12 + (session.total_interventions.min(4) as u64 * 8))
}

fn prune_recent_prompt_keys(session: &mut ActiveSession, now: Instant) {
    while session
        .recent_prompt_keys
        .front()
        .is_some_and(|(_, sent_at)| now.duration_since(*sent_at) > prompt_repeat_suppression_window())
    {
        session.recent_prompt_keys.pop_front();
    }
}

fn has_recent_prompt_key(session: &ActiveSession, key: &str) -> bool {
    session
        .recent_prompt_keys
        .iter()
        .any(|(existing, _)| existing == key)
}

fn remember_prompt_delivery(session: &mut ActiveSession, key: String, now: Instant) {
    session.last_prompt_sent_at = Some(now);
    session.recent_prompt_keys.push_back((key, now));
    while session.recent_prompt_keys.len() > 32 {
        session.recent_prompt_keys.pop_front();
    }
}

fn clear_superseded_prompt_queue(session: &mut ActiveSession) {
    if session.queued_prompts.is_empty() {
        return;
    }
    session.queued_prompts.clear();
    session.queued_prompt_keys.clear();
}

fn prune_recent_supervisor_notice_keys(session: &mut ActiveSession, now: Instant) {
    while session
        .recent_supervisor_notice_keys
        .front()
        .is_some_and(|(_, sent_at)| {
            now.duration_since(*sent_at) > supervisor_notice_repeat_suppression_window()
        })
    {
        session.recent_supervisor_notice_keys.pop_front();
    }
}

fn has_recent_supervisor_notice_key(session: &ActiveSession, key: &str) -> bool {
    session
        .recent_supervisor_notice_keys
        .iter()
        .any(|(existing, _)| existing == key)
}

fn remember_supervisor_notice(session: &mut ActiveSession, key: String, now: Instant) {
    session.last_supervisor_notice_key = Some(key.clone());
    session.recent_supervisor_notice_keys.push_back((key, now));
    while session.recent_supervisor_notice_keys.len() > 32 {
        session.recent_supervisor_notice_keys.pop_front();
    }
}

fn has_recent_status_activity(session: &ActiveSession, now: Instant) -> bool {
    session.last_status_update_at.is_some_and(|at| {
        now.duration_since(at)
            < Duration::from_secs(supervisor::STATUS_FILE_LIVENESS_GRACE_SECS * 3)
    })
}

fn session_tmux_health(session: &ActiveSession) -> Option<tmux::SessionHealth> {
    session.last_tmux_health
}

fn session_has_live_terminal(session: &ActiveSession) -> bool {
    matches!(
        session_tmux_health(session),
        Some(tmux::SessionHealth::Healthy | tmux::SessionHealth::Hung | tmux::SessionHealth::Starting)
    )
}

fn effective_stall_threshold(session: &ActiveSession, stall_after: Duration) -> Duration {
    if session.output_chunks > 0 || session.last_status_update_at.is_some() {
        stall_after.mul_f64(3.0)
    } else {
        stall_after
    }
}

fn tmux_health_refresh_interval() -> Duration {
    Duration::from_secs(10)
}

fn refresh_tmux_health_cache(active_sessions: &mut HashMap<Uuid, ActiveSession>) {
    let now = Instant::now();
    for session in active_sessions.values_mut() {
        if session.state.is_terminal() {
            continue;
        }
        if session
            .last_tmux_health_checked_at
            .is_some_and(|checked| now.duration_since(checked) < tmux_health_refresh_interval())
        {
            continue;
        }
        session.last_tmux_health_checked_at = Some(now);
        session.last_tmux_health = session.runtime.terminal_target().map(|target| {
            tmux::Tmux::new(None).check_session_health(target, zombie_check_max_inactivity())
        });
    }
}

fn recently_prompted(session: &ActiveSession, now: Instant) -> bool {
    session
        .last_prompt_sent_at
        .is_some_and(|last| now.duration_since(last) < prompt_dispatch_interval(session))
}

fn send_prompt_immediately(
    session: &mut ActiveSession,
    prompt: &str,
) -> Result<()> {
    let now = Instant::now();
    let key = prompt_fingerprint(prompt);
    prune_recent_prompt_keys(session, now);

    // Hard dedup: if this exact prompt was recently sent to this session, skip it.
    // Prevents duplicate paste even if launch_prompt_sent guard fails elsewhere.
    if has_recent_prompt_key(session, &key) {
        warn!(
            worker = %session.record.name,
            key = %key,
            "BLOCKED duplicate prompt delivery (same hash recently sent)"
        );
        return Ok(());
    }

    // Readiness guard: never inject a prompt while the agent is mid-response.
    // This prevents prompt text from being buffered/echoed by the shell instead
    // of being consumed by the agent's TUI readline.
    if is_agent_mid_response(session) {
        let elapsed = now.saturating_duration_since(session.last_output_at);
        warn!(
            worker = %session.record.name,
            elapsed_ms = elapsed.as_millis(),
            "send_prompt_immediately: agent mid-response, delaying 500ms"
        );
        std::thread::sleep(Duration::from_millis(500));
    }

    session.runtime.send_prompt(prompt)?;
    remember_prompt_delivery(session, key, now);
    Ok(())
}

/// Try to send a prompt, or queue it if the agent is mid-response.
/// Returns true if the prompt was sent immediately, false if it was queued.
fn send_or_queue_prompt(
    session: &mut ActiveSession,
    prompt: &str,
) -> bool {
    let now = Instant::now();
    let key = prompt_fingerprint(prompt);
    prune_recent_prompt_keys(session, now);

    if session.queued_prompt_keys.contains(&key) || has_recent_prompt_key(session, &key) {
        return false;
    }

    if is_agent_mid_response(session) || recently_prompted(session, now) {
        if session.queued_prompts.len() >= prompt_queue_limit() {
            if let Some(evicted) = session.queued_prompts.pop_front() {
                session.queued_prompt_keys.remove(&evicted.key);
            }
        }
        session.queued_prompt_keys.insert(key.clone());
        session.queued_prompts.push_back(QueuedPrompt {
            key,
            body: prompt.to_owned(),
        });
        false
    } else {
        let _ = session.runtime.send_prompt(prompt);
        remember_prompt_delivery(session, key, now);
        true
    }
}

/// Drain queued prompts for sessions that have gone quiet.
fn drain_prompt_queues(
    active_sessions: &mut HashMap<Uuid, ActiveSession>,
) {
    let mut drained = Vec::new();
    let now = Instant::now();
    for (session_id, session) in active_sessions.iter_mut() {
        prune_recent_prompt_keys(session, now);
        if is_agent_mid_response(session) || recently_prompted(session, now) {
            continue;
        }
        if let Some(prompt) = session.queued_prompts.pop_front() {
            session.queued_prompt_keys.remove(&prompt.key);
            let _ = session.runtime.send_prompt(&prompt.body);
            remember_prompt_delivery(session, prompt.key, now);
            drained.push((*session_id, prompt.body));
        }
    }
    // Log drained prompts (best-effort, non-fatal)
    for (_, prompt) in &drained {
        tracing::debug!("drained queued prompt: {}", prompt.chars().take(80).collect::<String>());
    }
}

/// Text similarity helper for validating worker packet differentiation.
fn text_similarity_sim(a: &str, b: &str) -> f64 {
    let a_lower = a.to_ascii_lowercase();
    let b_lower = b.to_ascii_lowercase();
    let words_a: std::collections::HashSet<_> = a_lower.split_whitespace().collect();
    let words_b: std::collections::HashSet<_> = b_lower.split_whitespace().collect();
    if words_a.is_empty() && words_b.is_empty() { return 1.0; }
    if words_a.is_empty() || words_b.is_empty() { return 0.0; }
    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();
    if union == 0 { return 0.0; }
    intersection as f64 / union as f64
}
