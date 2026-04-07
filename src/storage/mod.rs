pub mod history;

pub mod agent_memory;
pub mod live_state;
pub mod transcripts;
pub mod mail_store;
pub mod lease_store;

mod types;

pub use types::*;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use parking_lot::RwLock;
use uuid::Uuid;

use crate::agent::AgentKind;
use crate::model::{
    EventRecord, LeaseRecord, MailRecord, MissionPlan, MissionRecord, MissionSnapshot,
    MissionStatus, NormalizedUpdateRecord, ReplayEntry, RestartRecord, SessionListItem, SessionRecord,
    SessionRole, SessionState, SummaryRecord, TaskRecord, ValidationResultRecord, WorkerPacket,
    WorkerSnapshot,
};

use self::history::{SessionHistory, HistoryIndex};
use self::agent_memory::AgentMemoryStore;
use self::live_state::LiveState;
use self::transcripts::TranscriptStore;
use self::mail_store::MailStore;
use self::lease_store::LeaseStore;

/// Unified Store facade — backwards-compatible API, zero SQLite.
///
/// Architecture:
/// - Session history: JSONL files (`.sp/history/<mission_id>/session.jsonl`)
/// - Agent memory: JSON files (`.sp/memory/<mission_id>/<display_name>.json`)
/// - Live state: in-memory HashMaps (parking_lot RwLock)
/// - Transcripts: zstd-compressed, rotating (`.sp/transcripts/<mission_id>/`)
/// - Mail: JSON files (`.sp/mail/<mission_id>/`)
/// - Leases: JSON files (`.sp/leases/<mission_id>/`)
pub struct Store {
    state_dir: PathBuf,
    history: SessionHistory,
    history_index: HistoryIndex,
    agent_memory: AgentMemoryStore,
    live_state: Arc<RwLock<LiveState>>,
    transcripts: TranscriptStore,
    mail: MailStore,
    leases: LeaseStore,
}

impl Store {
    pub fn open(state_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(state_dir)
            .with_context(|| format!("failed to create state dir {}", state_dir.display()))?;

        let history = SessionHistory::open(state_dir.join("history"))?;
        let history_index = HistoryIndex::open(state_dir.join("history").join("index.jsonl"))?;
        let agent_memory = AgentMemoryStore::open(state_dir.join("memory"))?;
        let live_state = Arc::new(RwLock::new(LiveState::new()));
        let transcripts = TranscriptStore::open(state_dir.join("transcripts"))?;
        let mail = MailStore::open(state_dir.join("mail"))?;
        let leases = LeaseStore::open(state_dir.join("leases"))?;

        Ok(Self {
            state_dir: state_dir.to_owned(),
            history,
            history_index,
            agent_memory,
            live_state,
            transcripts,
            mail,
            leases,
        })
    }

    pub fn state_dir(&self) -> &Path { &self.state_dir }
    pub fn live_state(&self) -> Arc<RwLock<LiveState>> { self.live_state.clone() }
    pub fn transcripts(&self) -> &TranscriptStore { &self.transcripts }
    pub fn mail_store(&self) -> &MailStore { &self.mail }
    pub fn lease_store(&self) -> &LeaseStore { &self.leases }
    pub fn agent_memory(&self) -> &AgentMemoryStore { &self.agent_memory }

    // ─── Mission Persistence ─────────────────────────────────────────────

    pub fn persist_mission(&self, mission: &MissionRecord, plan: &MissionPlan) -> Result<()> {
        self.history.write_mission(&mission.id, mission, plan)?;
        self.history_index.upsert(&mission.id, &mission.repo_path, &mission.mission, &mission.status)?;
        Ok(())
    }

    pub fn update_mission_status(&self, mission_id: Uuid, status: MissionStatus) -> Result<()> {
        self.history.update_mission_status(&mission_id, &status)?;
        self.history_index.update_status(&mission_id, &status)?;
        Ok(())
    }

    pub fn update_mission_final_summary(&self, mission_id: Uuid, summary: &str) -> Result<()> {
        self.history.update_final_summary(&mission_id, summary)?;
        Ok(())
    }

    pub fn replace_mission_plan(&self, mission_id: Uuid, plan: &MissionPlan) -> Result<()> {
        self.history.update_plan(&mission_id, plan)?;
        Ok(())
    }

    // ─── Session/Worker Persistence ──────────────────────────────────────

    pub fn persist_session(
        &self,
        session: &SessionRecord,
        packet: Option<&WorkerPacket>,
    ) -> Result<()> {
        self.history.write_worker(&session.mission_id, session, packet)?;
        Ok(())
    }

    pub fn update_session_state(&self, worker_id: Uuid, status: SessionState) -> Result<()> {
        self.history.update_worker_state(&worker_id, &status)?;
        Ok(())
    }

    pub fn update_worker_heartbeat(&self, _worker_id: Uuid) -> Result<()> {
        // No-op: heartbeats are tracked in live_state (in-memory)
        Ok(())
    }

    pub fn update_worker_summary(&self, worker_id: Uuid, summary: &str) -> Result<()> {
        self.history.update_worker_summary(&worker_id, summary)?;
        Ok(())
    }

    // ─── Tasks ───────────────────────────────────────────────────────────

    pub fn persist_task(&self, task: &TaskRecord) -> Result<()> {
        self.history.write_task(&task.mission_id, task)?;
        Ok(())
    }

    pub fn update_task_status(&self, task_id: Uuid, status: &str) -> Result<()> {
        self.history.update_task_status(&task_id, status)?;
        Ok(())
    }

    pub fn find_task_id(&self, mission_id: Uuid, worker_id: Uuid) -> Result<Option<Uuid>> {
        self.history.find_task_id(&mission_id, &worker_id)
    }

    // ─── Events (Discardable — no persistent event log by default) ──────

    pub fn persist_event(&self, _event: &EventRecord) -> Result<()> {
        // Events are NOT persisted by default. They're terminal chatter.
        // If debug mode is needed, override to write to .sp/debug/events.jsonl
        Ok(())
    }

    pub fn append_json_event<T: serde::Serialize>(
        &self,
        mission_id: Uuid,
        worker_id: Option<Uuid>,
        kind: impl Into<String>,
        body: impl Into<String>,
        payload: &T,
    ) -> Result<()> {
        // Write significant events to session history as structured lines
        let event_line = serde_json::json!({
            "type": "event",
            "event_type": kind.into(),
            "worker_id": worker_id.map(|id| id.to_string()),
            "body": body.into(),
            "payload": serde_json::to_value(payload)?,
            "created_at": chrono::Utc::now(),
        });
        self.history.append_event(&mission_id, &event_line)?;
        Ok(())
    }

    // ─── Messages ────────────────────────────────────────────────────────

    pub fn persist_message(&self, message: &MailRecord) -> Result<()> {
        self.mail.persist_message(&message.mission_id, message)
    }

    pub fn update_message_status(
        &self,
        message_id: Uuid,
        status: &str,
        ack_state: &str,
    ) -> Result<()> {
        self.mail.update_message_status(&message_id, status, ack_state)
    }

    pub fn archive_resolved_mail(&self, mission_id: Uuid, older_than_secs: u64) -> Result<usize> {
        self.mail.archive_resolved_mail(&mission_id, older_than_secs)
    }

    pub fn search_mail(&self, mission_id: Uuid, query: Option<&str>, from_worker: Option<Uuid>, msg_type: Option<&str>, limit: usize) -> Result<Vec<serde_json::Value>> {
        self.mail.search_mail(&mission_id, query, from_worker, msg_type, limit)
    }

    pub fn claim_scavenge_mail(&self, mail_id: Uuid, claimer_id: Uuid, claimer_name: &str) -> Result<usize> {
        self.mail.claim_scavenge_mail(&mail_id, &claimer_id, claimer_name)
    }

    pub fn release_scavenge_mail(&self, mail_id: Uuid, releaser_id: Uuid) -> Result<usize> {
        self.mail.release_scavenge_mail(&mail_id, &releaser_id)
    }

    pub fn get_mail_body(&self, mail_id: Uuid) -> Result<Option<String>> {
        self.mail.get_mail_body(&mail_id)
    }

    pub fn get_mail_by_id(&self, mail_id: Uuid) -> Result<Option<MailRecord>> {
        self.mail.get_mail_by_id(&mail_id)
    }

    // ─── Leases ──────────────────────────────────────────────────────────

    pub fn upsert_lease(&self, lease: &LeaseRecord) -> Result<()> {
        self.leases.upsert_lease(&lease.mission_id, lease)
    }

    pub fn get_existing_lease(&self, mission_id: Uuid, path: &str) -> Result<Option<LeaseRecord>> {
        self.leases.get_lease(&mission_id, path)
    }

    // ─── Summaries ───────────────────────────────────────────────────────

    pub fn persist_summary(&self, summary: &SummaryRecord) -> Result<()> {
        self.history.write_summary(&summary.mission_id, summary)
    }

    pub fn append_summary(
        &self,
        mission_id: Uuid,
        worker_id: Option<Uuid>,
        summary_type: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<()> {
        let content = content.into();
        let summary = SummaryRecord {
            id: Uuid::new_v4(),
            mission_id,
            worker_id,
            summary_type: summary_type.into(),
            content: content.clone(),
            created_at: chrono::Utc::now(),
        };
        self.persist_summary(&summary)?;
        if let Some(wid) = worker_id {
            let _ = self.update_worker_summary(wid, &content);
        }
        Ok(())
    }

    // ─── Normalized Updates (Discardable — debug-only) ───────────────────

    pub fn persist_normalized_update(&self, _update: &NormalizedUpdateRecord) -> Result<()> {
        // Normalized updates are debug traces. Not persisted by default.
        Ok(())
    }

    // ─── Validation Results ──────────────────────────────────────────────

    pub fn persist_validation_result(&self, result: &ValidationResultRecord) -> Result<()> {
        self.history.write_validation_result(&result.mission_id, result)
    }

    // ─── Restart Tracking (in live_state for crash loop detection) ───────

    pub fn upsert_restart_attempt(
        &self,
        session_id: Uuid,
        mission_id: Uuid,
    ) -> Result<RestartRecord> {
        let mut state = self.live_state.write();
        state.upsert_restart_attempt(session_id, mission_id)
    }

    pub fn load_restart_state(&self, session_id: Uuid) -> Result<Option<RestartRecord>> {
        let state = self.live_state.read();
        Ok(state.load_restart_state(&session_id).map(|s| crate::model::RestartRecord {
            id: Uuid::new_v4(),
            session_id: s.session_id,
            mission_id: s.mission_id,
            restart_count: s.restart_count,
            first_restart_at: s.first_restart_at,
            last_restart_at: s.last_restart_at,
            backoff_seconds: s.backoff_seconds,
        }))
    }

    pub fn reset_restart_tracker(&self, session_id: Uuid) -> Result<()> {
        let mut state = self.live_state.write();
        state.reset_restart_tracker(&session_id);
        Ok(())
    }

    pub fn is_crash_loop(
        &self,
        session_id: Uuid,
        threshold: usize,
        window: Duration,
    ) -> Result<bool> {
        let state = self.live_state.read();
        Ok(state.is_crash_loop(&session_id, threshold, window))
    }

    pub fn get_crash_loop_sessions(
        &self,
        mission_id: Uuid,
        threshold: usize,
        window: Duration,
    ) -> Result<Vec<(Uuid, usize)>> {
        let state = self.live_state.read();
        Ok(state.get_crash_loop_sessions(&mission_id, threshold, window))
    }

    // ─── Query/Read Operations ───────────────────────────────────────────

    pub fn list_sessions(&self) -> Result<Vec<SessionListItem>> {
        self.history_index.list_all()
    }

    pub fn latest_session_for_repo(
        &self,
        repo_path: &Path,
        not_before: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<SessionListItem>> {
        self.history_index.latest_for_repo(repo_path, not_before)
    }

    pub fn load_mission_snapshot(&self, mission_id: Uuid) -> Result<Option<MissionSnapshot>> {
        self.history.load_mission_snapshot(&mission_id)
    }

    pub fn load_workers(&self, mission_id: Uuid) -> Result<Vec<WorkerSnapshot>> {
        self.history.load_workers(&mission_id)
    }

    pub fn recent_replay_entries(
        &self,
        mission_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ReplayEntry>> {
        self.history.recent_replay_entries(&mission_id, limit)
    }

    pub fn recent_worker_replay(
        &self,
        mission_id: Uuid,
        worker_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ReplayEntry>> {
        self.history.recent_worker_replay(&mission_id, &worker_id, limit)
    }

    pub fn latest_supervisor_summary(&self, mission_id: Uuid) -> Result<Option<String>> {
        self.history.latest_supervisor_summary(&mission_id)
    }

    pub fn recent_worker_summary(&self, worker_id: Uuid) -> Result<Option<String>> {
        self.history.recent_worker_summary(&worker_id)
    }
}

// Helper functions for role parsing (ported from old store)
fn role_name(role: SessionRole) -> &'static str {
    match role {
        SessionRole::Supervisor => "supervisor",
        SessionRole::Worker => "worker",
    }
}

fn parse_role(value: &str) -> Option<SessionRole> {
    match value.trim().to_ascii_lowercase().as_str() {
        "supervisor" => Some(SessionRole::Supervisor),
        "worker" => Some(SessionRole::Worker),
        _ => None,
    }
}

// Restart tracking constants
fn restart_base_secs() -> u64 { 2 }
fn restart_max_secs() -> u64 { 300 }
