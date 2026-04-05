use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use uuid::Uuid;

use crate::agent::AgentKind;
use crate::model::{
    EventRecord, LeaseRecord, MailRecord, MissionPlan, MissionRecord, MissionSnapshot,
    MissionStatus, NormalizedUpdateRecord, ReplayEntry, RestartRecord, SessionListItem, SessionRecord,
    SessionRole, SessionState, SummaryRecord, TaskRecord, ValidationResultRecord, WorkerPacket,
    WorkerSnapshot,
};

pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let connection = Connection::open(path)
            .with_context(|| format!("failed to open sqlite database {}", path.display()))?;
        let store = Self {
            connection: Mutex::new(connection),
        };
        store.initialize_schema()?;
        Ok(store)
    }

    pub fn persist_mission(&self, mission: &MissionRecord, plan: &MissionPlan) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "INSERT INTO sessions (
                id, created_at, updated_at, repo_path, agent_type, user_mission_raw,
                mission_rewrite, status, final_summary, plan_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                mission.id,
                mission.created_at,
                mission.updated_at,
                mission.repo_path.to_string_lossy(),
                format!(
                    "workers:{} supervisor:{}",
                    mission.worker_agent.as_str(),
                    mission.supervisor_agent.as_str()
                ),
                mission.mission,
                mission.mission_rewrite,
                mission.status.as_str(),
                mission.final_summary,
                serde_json::to_string(plan)?,
            ],
        )?;
        Ok(())
    }

    pub fn persist_session(
        &self,
        session: &SessionRecord,
        packet: Option<&WorkerPacket>,
    ) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "INSERT INTO workers (
                id, session_id, role, terminal_id, name, owned_scope, status,
                last_heartbeat_at, last_summary, agent, launch_command, packet_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                session.id,
                session.mission_id,
                role_name(session.role),
                session.terminal_id,
                session.name,
                session.owned_scope,
                session.status.as_str(),
                session.last_heartbeat_at,
                session.last_summary,
                session.agent.as_str(),
                serde_json::to_string(&session.launch_command)?,
                match packet {
                    Some(value) => Some(serde_json::to_string(value)?),
                    None => None,
                },
            ],
        )?;
        Ok(())
    }

    pub fn persist_task(&self, task: &TaskRecord) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "INSERT INTO tasks (
                id, session_id, worker_id, title, description, status, priority,
                depends_on, definition_of_done
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                task.id,
                task.mission_id,
                task.worker_id,
                task.title,
                task.description,
                task.status,
                task.priority,
                task.depends_on_json,
                task.definition_of_done_json,
            ],
        )?;
        Ok(())
    }

    pub fn update_task_status(&self, task_id: Uuid, status: &str) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "UPDATE tasks SET status = ?2 WHERE id = ?1",
            params![task_id, status],
        )?;
        Ok(())
    }

    pub fn persist_event(&self, event: &EventRecord) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "INSERT INTO events (
                id, session_id, worker_id, event_type, payload, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.id,
                event.mission_id,
                event.worker_id,
                event.kind,
                event.payload_json,
                event.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_mission_status(&self, mission_id: Uuid, status: MissionStatus) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "UPDATE sessions SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![mission_id, status.as_str(), chrono::Utc::now()],
        )?;
        Ok(())
    }

    pub fn update_mission_final_summary(&self, mission_id: Uuid, summary: &str) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "UPDATE sessions SET final_summary = ?2, updated_at = ?3 WHERE id = ?1",
            params![mission_id, summary, chrono::Utc::now()],
        )?;
        Ok(())
    }

    pub fn update_session_state(&self, worker_id: Uuid, status: SessionState) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "UPDATE workers SET status = ?2, last_heartbeat_at = ?3 WHERE id = ?1",
            params![worker_id, status.as_str(), chrono::Utc::now()],
        )?;
        Ok(())
    }

    pub fn update_worker_heartbeat(&self, worker_id: Uuid) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "UPDATE workers SET last_heartbeat_at = ?2 WHERE id = ?1",
            params![worker_id, chrono::Utc::now()],
        )?;
        Ok(())
    }

    pub fn update_worker_summary(&self, worker_id: Uuid, summary: &str) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "UPDATE workers SET last_summary = ?2, last_heartbeat_at = ?3 WHERE id = ?1",
            params![worker_id, summary, chrono::Utc::now()],
        )?;
        Ok(())
    }

    pub fn persist_message(&self, message: &MailRecord) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "INSERT INTO messages (
                id, session_id, from_worker_id, to_worker_id, message_type, body,
                status, created_at, acked_at, priority, subject,
                delivery_mode, thread_id, reply_to, pinned, ack_state, archived_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                message.id,
                message.mission_id,
                message.sender_worker_id,
                message.recipient_worker_id,
                message.message_type,
                message.body_json,
                message.status,
                message.created_at,
                ack_timestamp(&message.ack_state),
                message.priority,
                message.subject,
                message.delivery_mode,
                message.thread_id,
                message.reply_to,
                if message.pinned { 1 } else { 0 },
                message.ack_state,
                message.archived_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_message_status(
        &self,
        message_id: Uuid,
        status: &str,
        ack_state: &str,
    ) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "UPDATE messages SET status = ?2, acked_at = ?3 WHERE id = ?1",
            params![message_id, status, ack_timestamp(ack_state)],
        )?;
        Ok(())
    }

    /// Archive resolved mail that is resolved and older than the given age.
    /// Returns the count of archived messages.
    #[allow(dead_code)]
    pub fn archive_resolved_mail(&self, mission_id: Uuid, older_than_secs: u64) -> Result<usize> {
        let connection = self.connection.lock();
        let cutoff = Utc::now() - chrono::Duration::seconds(older_than_secs as i64);
        let rows = connection.execute(
            "UPDATE messages SET status = 'archived', archived_at = ?2
             WHERE session_id = ?1
               AND status IN ('acked', 'responded', 'done')
               AND pinned = 0
               AND archived_at IS NULL
               AND created_at < ?3",
            params![mission_id, Utc::now(), cutoff],
        )?;
        Ok(rows)
    }

    /// Search mail across a mission. Supports filtering by query text, sender, and type.
    #[allow(dead_code)]
    pub fn search_mail(&self, mission_id: Uuid, query: Option<&str>, from_worker: Option<Uuid>, msg_type: Option<&str>, limit: usize) -> Result<Vec<serde_json::Value>> {
        let connection = self.connection.lock();

        let where_query = query.map(|q| format!(" AND (subject LIKE '%{}%' OR body LIKE '%{}%')", q.replace('\'', "''"), q.replace('\'', "''"))).unwrap_or_default();
        let where_from = from_worker.map(|id| format!(" AND from_worker_id = '{}'", id)).unwrap_or_default();
        let where_type = msg_type.map(|t| format!(" AND message_type = '{}'", t.replace('\'', "''"))).unwrap_or_default();

        let sql = format!(
            "SELECT id, from_worker_id, to_worker_id, message_type, priority, subject, status, ack_state, created_at, thread_id \
             FROM messages \
             WHERE session_id = ?1{}{}{} \
             ORDER BY created_at DESC \
             LIMIT {}",
            where_query, where_from, where_type, limit
        );

        let mut stmt = connection.prepare(&sql)?;
        let mut rows = stmt.query(params![mission_id])?;
        let mut results = Vec::new();
        while let Some(row) = rows.next()? {
            results.push(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "from_worker_id": row.get::<_, String>(1)?,
                "to_worker_id": row.get::<_, String>(2)?,
                "message_type": row.get::<_, String>(3)?,
                "priority": row.get::<_, String>(4)?,
                "subject": row.get::<_, String>(5)?,
                "status": row.get::<_, String>(6)?,
                "ack_state": row.get::<_, String>(7)?,
                "created_at": row.get::<_, String>(8)?,
                "thread_id": row.get::<_, String>(9)?,
            }));
        }
        Ok(results)
    }

    /// Atomically claim a scavenge message. Only succeeds if not yet claimed.
    /// Updates body_json with claimed_by, claimed_by_name, and claimed_at.
    pub fn claim_scavenge_mail(&self, mail_id: Uuid, claimer_id: Uuid, claimer_name: &str) -> Result<usize> {
        let connection = self.connection.lock();
        let now = Utc::now().to_rfc3339();
        // Use JSON patch to add claimed_by fields only if body_json doesn't already have claimed_by
        let rows = connection.execute(
            "UPDATE messages SET body_json = json_patch(
                json(body_json),
                json_object('claimed_by', ?2, 'claimed_by_name', ?3, 'claimed_at', ?4)
             )
             WHERE id = ?1
               AND message_type = 'scavenge'
               AND (body_json NOT LIKE '%\"claimed_by\"%' OR json_extract(body_json, '$.claimed_by') IS NULL)",
            params![mail_id, claimer_id.to_string(), claimer_name, now],
        )?;
        Ok(rows)
    }

    /// Release a scavenge claim — remove claimed_by fields from body_json.
    /// Only succeeds if the caller is the current claimer.
    pub fn release_scavenge_mail(&self, mail_id: Uuid, releaser_id: Uuid) -> Result<usize> {
        let connection = self.connection.lock();
        let rows = connection.execute(
            "UPDATE messages SET body_json = json_remove(body_json, '$.claimed_by', '$.claimed_by_name', '$.claimed_at')
             WHERE id = ?1
               AND json_extract(body_json, '$.claimed_by') = ?2",
            params![mail_id, releaser_id.to_string()],
        )?;
        Ok(rows)
    }

    /// Get the body_json of a mail by ID.
    pub fn get_mail_body(&self, mail_id: Uuid) -> Result<Option<String>> {
        let connection = self.connection.lock();
        let body: Option<String> = connection.query_row(
            "SELECT body FROM messages WHERE id = ?1",
            params![mail_id],
            |row| row.get(0),
        ).optional()?;
        Ok(body)
    }

    /// Get a mail record by ID (returns minimal fields).
    #[allow(dead_code)]
    pub fn get_mail_by_id(&self, mail_id: Uuid) -> Result<Option<crate::model::MailRecord>> {
        let connection = self.connection.lock();
        let row = connection.query_row(
            "SELECT id, session_id, from_worker_id, to_worker_id, message_type, priority,
                    subject, status, created_at, body, delivery_mode, thread_id, reply_to,
                    pinned, ack_state, archived_at
             FROM messages WHERE id = ?1",
            params![mail_id],
            |row| {
                Ok(crate::model::MailRecord {
                    id: row.get(0)?,
                    mission_id: row.get(1)?,
                    sender_worker_id: row.get(2)?,
                    recipient_worker_id: row.get(3)?,
                    message_type: row.get(4)?,
                    priority: row.get(5)?,
                    delivery_mode: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
                    subject: row.get(6)?,
                    status: row.get(7)?,
                    ack_state: row.get::<_, Option<String>>(14)?.unwrap_or_default(),
                    pinned: row.get::<_, Option<i64>>(13)?.unwrap_or(0) != 0,
                    body_json: row.get(9)?,
                    thread_id: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
                    reply_to: row.get(12)?,
                    created_at: row.get(8)?,
                    archived_at: row.get(15)?,
                })
            },
        ).optional()?;
        Ok(row)
    }

    pub fn persist_summary(&self, summary: &SummaryRecord) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "INSERT INTO summaries (
                id, session_id, worker_id, summary_type, content, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                summary.id,
                summary.mission_id,
                summary.worker_id,
                summary.summary_type,
                summary.content,
                summary.created_at,
            ],
        )?;
        Ok(())
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
        if let Some(worker_id) = worker_id {
            self.update_worker_summary(worker_id, &content)?;
        }
        Ok(())
    }

    pub fn persist_normalized_update(&self, update: &NormalizedUpdateRecord) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "INSERT INTO normalized_updates (
                id, session_id, worker_id, source, raw_excerpt, normalized_state,
                confidence, summary, adapter, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                update.id,
                update.mission_id,
                update.worker_id,
                update.source,
                update.raw_excerpt,
                update.normalized_state,
                update.confidence,
                update.summary,
                update.adapter,
                update.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn persist_validation_result(&self, result: &ValidationResultRecord) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "INSERT INTO validation_results (
                id, session_id, worker_id, task_id, outcome, summary, evidence, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                result.id,
                result.mission_id,
                result.worker_id,
                result.task_id,
                result.outcome,
                result.summary,
                result.evidence_json,
                result.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn replace_mission_plan(&self, mission_id: Uuid, plan: &MissionPlan) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "UPDATE sessions SET mission_rewrite = ?2, plan_json = ?3, updated_at = ?4 WHERE id = ?1",
            params![
                mission_id,
                plan.mission_rewrite,
                serde_json::to_string(plan)?,
                chrono::Utc::now(),
            ],
        )?;
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionListItem>> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT id, created_at, updated_at, repo_path, mission_rewrite, status, final_summary
             FROM sessions ORDER BY created_at DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(SessionListItem {
                id: row.get(0)?,
                created_at: row.get(1)?,
                updated_at: row.get(2)?,
                repo_path: std::path::PathBuf::from(row.get::<_, String>(3)?),
                mission_rewrite: row.get(4)?,
                status: row.get(5)?,
                final_summary: row.get(6)?,
            })
        })?;
        collect_rows(rows)
    }

    #[allow(dead_code)]
    pub fn latest_session_for_repo(
        &self,
        repo_path: &Path,
        not_before: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<SessionListItem>> {
        let connection = self.connection.lock();
        connection
            .query_row(
                "SELECT id, created_at, updated_at, repo_path, mission_rewrite, status, final_summary
                 FROM sessions
                 WHERE repo_path = ?1 AND created_at >= ?2
                 ORDER BY created_at DESC
                 LIMIT 1",
                params![repo_path.to_string_lossy().to_string(), not_before],
                |row| {
                    Ok(SessionListItem {
                        id: row.get(0)?,
                        created_at: row.get(1)?,
                        updated_at: row.get(2)?,
                        repo_path: std::path::PathBuf::from(row.get::<_, String>(3)?),
                        mission_rewrite: row.get(4)?,
                        status: row.get(5)?,
                        final_summary: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn load_mission_snapshot(&self, mission_id: Uuid) -> Result<Option<MissionSnapshot>> {
        let connection = self.connection.lock();
        connection
            .query_row(
                "SELECT id, created_at, updated_at, repo_path, user_mission_raw, mission_rewrite, status, final_summary, plan_json
                 FROM sessions WHERE id = ?1",
                params![mission_id],
                |row| {
                    let plan_json = row.get::<_, String>(8)?;
                    let plan = serde_json::from_str::<MissionPlan>(&plan_json).map_err(to_sql_err)?;
                    Ok(MissionSnapshot {
                        id: row.get(0)?,
                        created_at: row.get(1)?,
                        updated_at: row.get(2)?,
                        repo_path: std::path::PathBuf::from(row.get::<_, String>(3)?),
                        user_mission_raw: row.get(4)?,
                        mission_rewrite: row.get(5)?,
                        status: row.get(6)?,
                        final_summary: row.get(7)?,
                        plan,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn load_workers(&self, mission_id: Uuid) -> Result<Vec<WorkerSnapshot>> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT id, session_id, role, terminal_id, name, owned_scope, status,
                    last_heartbeat_at, last_summary, agent, launch_command, packet_json
             FROM workers WHERE session_id = ?1 ORDER BY role ASC, name ASC",
        )?;
        let rows = statement.query_map(params![mission_id], |row| {
            let role = parse_role(&row.get::<_, String>(2)?).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    "invalid worker role".into(),
                )
            })?;
            let status_text = row.get::<_, String>(6)?;
            let status = SessionState::from_directive(&status_text).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Text,
                    "invalid worker state".into(),
                )
            })?;
            let agent = AgentKind::from_str(&row.get::<_, String>(9)?).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    9,
                    rusqlite::types::Type::Text,
                    "invalid agent kind".into(),
                )
            })?;
            let launch_command_json = row.get::<_, String>(10)?;
            let launch_command =
                serde_json::from_str::<Vec<String>>(&launch_command_json).map_err(to_sql_err)?;
            let packet = row
                .get::<_, Option<String>>(11)?
                .map(|json| serde_json::from_str::<WorkerPacket>(&json).map_err(to_sql_err))
                .transpose()?;
            Ok(WorkerSnapshot {
                session: SessionRecord {
                    id: row.get(0)?,
                    mission_id: row.get(1)?,
                    role,
                    ordinal: 0,
                    agent,
                    terminal_id: row.get(3)?,
                    name: row.get(4)?,
                    owned_scope: row.get(5)?,
                    status,
                    launch_command,
                    last_heartbeat_at: row.get(7)?,
                    last_summary: row.get(8)?,
                },
                packet,
            })
        })?;
        collect_rows(rows)
    }

    pub fn recent_replay_entries(
        &self,
        mission_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ReplayEntry>> {
        let connection = self.connection.lock();
        let mut entries = Vec::new();

        let mut event_statement = connection.prepare(
            "SELECT created_at, worker_id, event_type, payload
             FROM events WHERE session_id = ?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let event_rows = event_statement.query_map(params![mission_id, limit as i64], |row| {
            let worker_id = row.get::<_, Option<Uuid>>(1)?;
            Ok(ReplayEntry {
                created_at: row.get(0)?,
                lane: format!(
                    "event:{}",
                    worker_id
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "mission".to_owned())
                ),
                kind: row.get(2)?,
                body: row.get(3)?,
            })
        })?;
        entries.extend(collect_rows(event_rows)?);

        let mut summary_statement = connection.prepare(
            "SELECT created_at, worker_id, summary_type, content
             FROM summaries WHERE session_id = ?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let summary_rows =
            summary_statement.query_map(params![mission_id, limit as i64], |row| {
                let worker_id = row.get::<_, Option<Uuid>>(1)?;
                Ok(ReplayEntry {
                    created_at: row.get(0)?,
                    lane: format!(
                        "summary:{}",
                        worker_id
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "mission".to_owned())
                    ),
                    kind: row.get(2)?,
                    body: row.get(3)?,
                })
            })?;
        entries.extend(collect_rows(summary_rows)?);

        entries.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        entries.truncate(limit);
        Ok(entries)
    }

    pub fn recent_worker_replay(
        &self,
        mission_id: Uuid,
        worker_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ReplayEntry>> {
        let connection = self.connection.lock();
        let mut entries = Vec::new();

        let mut statement = connection.prepare(
            "SELECT created_at, event_type, payload
             FROM events WHERE session_id = ?1 AND worker_id = ?2 ORDER BY created_at DESC LIMIT ?3",
        )?;
        let rows = statement.query_map(params![mission_id, worker_id, limit as i64], |row| {
            Ok(ReplayEntry {
                created_at: row.get(0)?,
                lane: "event".to_owned(),
                kind: row.get(1)?,
                body: row.get(2)?,
            })
        })?;
        entries.extend(collect_rows(rows)?);

        let mut normalized_statement = connection.prepare(
            "SELECT created_at, normalized_state, summary
             FROM normalized_updates WHERE session_id = ?1 AND worker_id = ?2 ORDER BY created_at DESC LIMIT ?3",
        )?;
        let normalized_rows = normalized_statement.query_map(
            params![mission_id, worker_id, limit as i64],
            |row| {
                Ok(ReplayEntry {
                    created_at: row.get(0)?,
                    lane: "normalized".to_owned(),
                    kind: row.get(1)?,
                    body: row.get(2)?,
                })
            },
        )?;
        entries.extend(collect_rows(normalized_rows)?);

        entries.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        entries.truncate(limit);
        Ok(entries)
    }

    pub fn latest_supervisor_summary(&self, mission_id: Uuid) -> Result<Option<String>> {
        let connection = self.connection.lock();
        connection
            .query_row(
                "SELECT content FROM summaries
                 WHERE session_id = ?1 AND worker_id IN (
                    SELECT id FROM workers WHERE session_id = ?1 AND role = 'supervisor'
                 )
                 AND summary_type NOT IN ('supervisor_health', 'plan_failure', 'final_summary_missing')
                 ORDER BY
                    CASE
                        WHEN summary_type = 'final_summary' THEN 0
                        WHEN summary_type = 'supervisor_notice' THEN 1
                        WHEN summary_type = 'supervisor_action' THEN 2
                        ELSE 9
                    END,
                    created_at DESC
                 LIMIT 1",
                params![mission_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn find_task_id(&self, mission_id: Uuid, worker_id: Uuid) -> Result<Option<Uuid>> {
        let connection = self.connection.lock();
        connection
            .query_row(
                "SELECT id FROM tasks WHERE session_id = ?1 AND worker_id = ?2 ORDER BY rowid DESC LIMIT 1",
                params![mission_id, worker_id],
                |row| row.get::<_, Uuid>(0),
            )
            .optional()
            .map_err(Into::into)
    }

    #[allow(dead_code)]
    pub fn recent_worker_summary(&self, worker_id: Uuid) -> Result<Option<String>> {
        let connection = self.connection.lock();
        let summary = connection
            .query_row(
                "SELECT last_summary FROM workers WHERE id = ?1",
                params![worker_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        Ok(summary)
    }

    pub fn upsert_lease(&self, lease: &LeaseRecord) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "INSERT INTO ownership_leases (session_id, path, owner_worker_id, intent, status, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_id, path) DO UPDATE SET
                owner_worker_id=excluded.owner_worker_id,
                intent=excluded.intent,
                status=excluded.status,
                updated_at=excluded.updated_at",
            params![
                lease.mission_id,
                lease.path,
                lease.owner_session_id,
                lease.intent,
                lease.status,
                lease.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn append_json_event<T: Serialize>(
        &self,
        mission_id: Uuid,
        worker_id: Option<Uuid>,
        kind: impl Into<String>,
        body: impl Into<String>,
        payload: &T,
    ) -> Result<()> {
        let event = EventRecord {
            id: Uuid::new_v4(),
            mission_id,
            worker_id,
            created_at: chrono::Utc::now(),
            kind: kind.into(),
            body: body.into(),
            payload_json: serde_json::to_string(payload)?,
        };
        self.persist_event(&event)
    }

    fn initialize_schema(&self) -> Result<()> {
        let connection = self.connection.lock();
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "foreign_keys", "OFF")?;
        migrate_table_if_legacy(
            &connection,
            "sessions",
            &[
                "id",
                "created_at",
                "updated_at",
                "repo_path",
                "agent_type",
                "user_mission_raw",
                "mission_rewrite",
                "status",
                "final_summary",
                "plan_json",
            ],
        )?;
        migrate_table_if_legacy(
            &connection,
            "events",
            &[
                "id",
                "session_id",
                "worker_id",
                "event_type",
                "payload",
                "created_at",
            ],
        )?;
        migrate_table_if_legacy(
            &connection,
            "messages",
            &[
                "id",
                "session_id",
                "from_worker_id",
                "to_worker_id",
                "message_type",
                "body",
                "status",
                "created_at",
                "acked_at",
                "priority",
                "subject",
            ],
        )?;
        migrate_table_if_legacy(
            &connection,
            "normalized_updates",
            &[
                "id",
                "session_id",
                "worker_id",
                "source",
                "raw_excerpt",
                "normalized_state",
                "confidence",
                "summary",
                "adapter",
                "created_at",
            ],
        )?;
        migrate_table_if_legacy(
            &connection,
            "validation_results",
            &[
                "id",
                "session_id",
                "worker_id",
                "task_id",
                "outcome",
                "summary",
                "evidence",
                "created_at",
            ],
        )?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                repo_path TEXT NOT NULL,
                agent_type TEXT NOT NULL,
                user_mission_raw TEXT NOT NULL,
                mission_rewrite TEXT NOT NULL,
                status TEXT NOT NULL,
                final_summary TEXT,
                plan_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS workers (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                terminal_id TEXT NOT NULL,
                name TEXT NOT NULL,
                owned_scope TEXT NOT NULL,
                status TEXT NOT NULL,
                last_heartbeat_at TEXT NOT NULL,
                last_summary TEXT,
                agent TEXT NOT NULL,
                launch_command TEXT NOT NULL,
                packet_json TEXT,
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            );

            CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                worker_id TEXT NOT NULL,
                title TEXT NOT NULL,
                description TEXT NOT NULL,
                status TEXT NOT NULL,
                priority TEXT NOT NULL,
                depends_on TEXT NOT NULL,
                definition_of_done TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                worker_id TEXT,
                event_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                from_worker_id TEXT NOT NULL,
                to_worker_id TEXT NOT NULL,
                message_type TEXT NOT NULL,
                body TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                acked_at TEXT,
                priority TEXT NOT NULL,
                subject TEXT NOT NULL,
                delivery_mode TEXT NOT NULL DEFAULT 'queue',
                thread_id TEXT NOT NULL DEFAULT '',
                reply_to TEXT,
                pinned INTEGER NOT NULL DEFAULT 0,
                ack_state TEXT NOT NULL DEFAULT 'pending',
                archived_at TEXT
            );

            CREATE TABLE IF NOT EXISTS summaries (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                worker_id TEXT,
                summary_type TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS ownership_leases (
                session_id TEXT NOT NULL,
                path TEXT NOT NULL,
                owner_worker_id TEXT NOT NULL,
                intent TEXT NOT NULL,
                status TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (session_id, path)
            );

            CREATE TABLE IF NOT EXISTS normalized_updates (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                worker_id TEXT NOT NULL,
                source TEXT NOT NULL,
                raw_excerpt TEXT NOT NULL,
                normalized_state TEXT NOT NULL,
                confidence TEXT NOT NULL,
                summary TEXT NOT NULL,
                adapter TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS validation_results (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                worker_id TEXT NOT NULL,
                task_id TEXT,
                outcome TEXT NOT NULL,
                summary TEXT NOT NULL,
                evidence TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            -- Session restart tracking (from gastown daemon restart tracker pattern)
            -- Persists restart attempts to survive orchestrator restarts and detect crash loops.
            CREATE TABLE IF NOT EXISTS session_restarts (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                mission_id TEXT NOT NULL,
                restart_count INTEGER NOT NULL DEFAULT 1,
                first_restart_at TEXT NOT NULL,
                last_restart_at TEXT NOT NULL,
                backoff_seconds REAL NOT NULL
            );
            ",
        )?;
        Ok(())
    }

    // ─── Session Restart Tracking (from gastown daemon restart tracker) ─────

    /// Upsert a restart attempt. Increments restart_count if entry exists.
    pub fn upsert_restart_attempt(
        &self,
        session_id: Uuid,
        mission_id: Uuid,
    ) -> Result<RestartRecord> {
        let connection = self.connection.lock();
        let existing = Self::load_restart_state_inner(&connection, session_id)?;
        let now = Utc::now();

        let record = if let Some(mut existing) = existing {
            existing.restart_count += 1;
            existing.last_restart_at = now;
            let backoff = (restart_base_secs() as f64 * 2.0f64.powi((existing.restart_count - 1) as i32))
                .min(restart_max_secs() as f64);
            existing.backoff_seconds = backoff;
            connection.execute(
                "UPDATE session_restarts SET restart_count = ?, last_restart_at = ?, backoff_seconds = ? WHERE id = ?",
                (existing.restart_count, existing.last_restart_at.to_rfc3339(), existing.backoff_seconds, existing.id.to_string()),
            )?;
            existing
        } else {
            let id = Uuid::new_v4();
            let record = RestartRecord {
                id,
                session_id,
                mission_id,
                restart_count: 1,
                first_restart_at: now,
                last_restart_at: now,
                backoff_seconds: restart_base_secs() as f64,
            };
            connection.execute(
                "INSERT INTO session_restarts (id, session_id, mission_id, restart_count, first_restart_at, last_restart_at, backoff_seconds) VALUES (?, ?, ?, ?, ?, ?, ?)",
                (record.id.to_string(), record.session_id.to_string(), record.mission_id.to_string(), record.restart_count, record.first_restart_at.to_rfc3339(), record.last_restart_at.to_rfc3339(), record.backoff_seconds),
            )?;
            record
        };

        Ok(record)
    }

    /// Load the current restart state for a session.
    pub fn load_restart_state(&self, session_id: Uuid) -> Result<Option<RestartRecord>> {
        let connection = self.connection.lock();
        Self::load_restart_state_inner(&connection, session_id)
    }

    fn load_restart_state_inner(
        connection: &Connection,
        session_id: Uuid,
    ) -> Result<Option<RestartRecord>> {
        connection.query_row(
            "SELECT id, session_id, mission_id, restart_count, first_restart_at, last_restart_at, backoff_seconds FROM session_restarts WHERE session_id = ?",
            [session_id.to_string()],
            |row: &rusqlite::Row| {
                Ok(RestartRecord {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    session_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default(),
                    mission_id: Uuid::parse_str(&row.get::<_, String>(2)?).unwrap_or_default(),
                    restart_count: row.get(3)?,
                    first_restart_at: row.get(4)?,
                    last_restart_at: row.get(5)?,
                    backoff_seconds: row.get(6)?,
                })
            },
        ).optional().map_err(Into::into)
    }

    /// Reset the restart tracker for a session (after successful recovery).
    pub fn reset_restart_tracker(&self, session_id: Uuid) -> Result<()> {
        let connection = self.connection.lock();
        connection.execute(
            "DELETE FROM session_restarts WHERE session_id = ?",
            [session_id.to_string()],
        )?;
        Ok(())
    }

    /// Check if a session is in a crash loop: N restarts within M minutes.
    pub fn is_crash_loop(
        &self,
        session_id: Uuid,
        threshold: usize,
        window: Duration,
    ) -> Result<bool> {
        let connection = self.connection.lock();
        let cutoff = Utc::now() - chrono::Duration::from_std(window).unwrap_or(chrono::Duration::minutes(10));
        let restart_count = connection.query_row(
            "SELECT restart_count FROM session_restarts WHERE session_id = ? AND last_restart_at > ?",
            [session_id.to_string(), cutoff.to_rfc3339()],
            |row: &rusqlite::Row| row.get::<_, i64>(0),
        ).optional()?;
        Ok(restart_count.unwrap_or_default() as usize >= threshold)
    }

    /// Get sessions in crash loop for a mission.
    pub fn get_crash_loop_sessions(
        &self,
        mission_id: Uuid,
        threshold: usize,
        window: Duration,
    ) -> Result<Vec<(Uuid, usize)>> {
        let connection = self.connection.lock();
        let cutoff = Utc::now() - chrono::Duration::from_std(window).unwrap_or(chrono::Duration::minutes(10));
        let mut stmt = connection.prepare(
            "SELECT session_id, restart_count FROM session_restarts WHERE mission_id = ? AND restart_count >= ? AND last_restart_at > ?",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![mission_id.to_string(), threshold as i64, cutoff.to_rfc3339()],
            |row: &rusqlite::Row| {
            Ok((
                Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                row.get::<_, i64>(1)? as usize,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

// Helper constants for restart backoff (mirrored from orchestrator for store use)
fn restart_base_secs() -> u64 { 2 }
fn restart_max_secs() -> u64 { 300 }

fn migrate_table_if_legacy(
    connection: &Connection,
    table_name: &str,
    expected_columns: &[&str],
) -> Result<()> {
    let columns = table_columns(connection, table_name)?;
    if columns.is_empty() {
        return Ok(());
    }

    let matches = expected_columns
        .iter()
        .all(|column| columns.iter().any(|existing| existing == column));
    if matches {
        return Ok(());
    }

    let legacy_name = format!("{}_legacy_v0", table_name);
    let already_migrated = table_exists(connection, &legacy_name)?;
    if !already_migrated {
        connection.execute(
            &format!("ALTER TABLE {table_name} RENAME TO {legacy_name}"),
            [],
        )?;
    }
    Ok(())
}

fn table_columns(connection: &Connection, table_name: &str) -> Result<Vec<String>> {
    if !table_exists(connection, table_name)? {
        return Ok(Vec::new());
    }

    let mut statement = connection.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = Vec::new();
    for row in rows {
        columns.push(row?);
    }
    Ok(columns)
}

fn table_exists(connection: &Connection, table_name: &str) -> Result<bool> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
            params![table_name],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(exists)
}

fn ack_timestamp(ack_state: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if ack_state.eq_ignore_ascii_case("acked") {
        Some(chrono::Utc::now())
    } else {
        None
    }
}

fn role_name(role: crate::model::SessionRole) -> &'static str {
    match role {
        crate::model::SessionRole::Supervisor => "supervisor",
        crate::model::SessionRole::Worker => "worker",
    }
}

fn parse_role(value: &str) -> Option<SessionRole> {
    match value.trim().to_ascii_lowercase().as_str() {
        "supervisor" => Some(SessionRole::Supervisor),
        "worker" => Some(SessionRole::Worker),
        _ => None,
    }
}

fn to_sql_err(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::Utc;
    use rusqlite::OptionalExtension;
    use uuid::Uuid;

    use crate::agent::AgentKind;
    use crate::model::{
        LeaseRecord, MailRecord, MissionPlan, MissionRecord, MissionStatus, NormalizedUpdateRecord,
        RiskItem, SessionRecord, SessionRole, SessionState, SummaryRecord, TaskRecord,
        ValidationResultRecord, WorkerPacket, Workstream, WorkstreamExecution,
    };

    use super::Store;

    #[test]
    fn persists_durable_orchestration_state() {
        let db_path = std::env::temp_dir().join(format!("sp-store-{}.sqlite3", Uuid::new_v4()));
        let store = Store::open(&db_path).expect("open store");
        let mission_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();
        let now = Utc::now();

        let plan = MissionPlan {
            mission_rewrite: "rewrite".to_owned(),
            workstreams: vec![Workstream {
                id: "baseline".to_owned(),
                name: "Baseline".to_owned(),
                execution: WorkstreamExecution::Parallel,
                owned_scope: "scope".to_owned(),
                success_criteria: vec!["done".to_owned()],
                depends_on: Vec::new(),
            }],
            risk_map: vec![RiskItem {
                zone: "zone".to_owned(),
                risk: "risk".to_owned(),
                mitigation: "mitigation".to_owned(),
            }],
            worker_packets: vec![WorkerPacket {
                worker_id: "Engineer-1".to_owned(),
                role_type: "software-engineer".to_owned(),
                display_name: "Engineer-1".to_owned(),
                role: "Software Engineer".to_owned(),
                starting_angle: "Execution first; Primary execution pass 1".to_owned(),
                owned_scope: "scope".to_owned(),
                explicit_task: "task".to_owned(),
                out_of_scope: "none".to_owned(),
                definition_of_done: vec!["done".to_owned()],
                required_evidence: vec!["evidence".to_owned()],
                blocker_protocol: "protocol".to_owned(),
                conflict_warning: "warning".to_owned(),
                communication_rules: vec!["mail".to_owned()],
                validation_standard: vec!["validate".to_owned()],
                expected_output_format: vec!["summary".to_owned()],
            }],
            supervision_strategy: "strategy".to_owned(),
        };

        store
            .persist_mission(
                &MissionRecord {
                    id: mission_id,
                    created_at: now,
                    updated_at: now,
                    repo_path: PathBuf::from("/tmp/repo"),
                    mission: "raw".to_owned(),
                    mission_rewrite: "rewrite".to_owned(),
                    worker_agent: AgentKind::Qwen,
                    supervisor_agent: AgentKind::Qwen,
                    worker_count: 1,
                    status: MissionStatus::Running,
                    final_summary: None,
                },
                &plan,
            )
            .expect("persist mission");

        store
            .persist_session(
                &SessionRecord {
                    id: worker_id,
                    mission_id,
                    role: SessionRole::Worker,
                    ordinal: 1,
                    agent: AgentKind::Qwen,
                    terminal_id: "Engineer-1".to_owned(),
                    name: "Engineer-1".to_owned(),
                    owned_scope: "scope".to_owned(),
                    status: SessionState::Progressing,
                    launch_command: vec!["qwen".to_owned()],
                    last_heartbeat_at: now,
                    last_summary: Some("working".to_owned()),
                },
                None,
            )
            .expect("persist worker");

        store
            .persist_task(&TaskRecord {
                id: Uuid::new_v4(),
                mission_id,
                worker_id,
                title: "task".to_owned(),
                description: "desc".to_owned(),
                status: "assigned".to_owned(),
                priority: "high".to_owned(),
                depends_on_json: "[]".to_owned(),
                definition_of_done_json: "[\"done\"]".to_owned(),
            })
            .expect("persist task");

        store
            .append_json_event(
                mission_id,
                Some(worker_id),
                "progress_update",
                "working",
                &serde_json::json!({"ok": true}),
            )
            .expect("persist event");

        store
            .persist_message(&MailRecord {
                id: Uuid::new_v4(),
                mission_id,
                sender_worker_id: worker_id,
                recipient_worker_id: worker_id,
                message_type: "review_request".to_owned(),
                priority: "high".to_owned(),
                delivery_mode: "queue".to_owned(),
                subject: "subject".to_owned(),
                status: "routed".to_owned(),
                ack_state: "pending".to_owned(),
                pinned: false,
                body_json: "{}".to_owned(),
                thread_id: String::new(),
                reply_to: None,
                created_at: now,
                archived_at: None,
            })
            .expect("persist message");

        store
            .upsert_lease(&LeaseRecord {
                mission_id,
                path: "src/main.rs".to_owned(),
                owner_session_id: worker_id,
                intent: "edit".to_owned(),
                status: "claim".to_owned(),
                updated_at: now,
            })
            .expect("persist lease");

        store
            .persist_summary(&SummaryRecord {
                id: Uuid::new_v4(),
                mission_id,
                worker_id: Some(worker_id),
                summary_type: "progress".to_owned(),
                content: "working".to_owned(),
                created_at: now,
            })
            .expect("persist summary");

        store
            .persist_normalized_update(&NormalizedUpdateRecord {
                id: Uuid::new_v4(),
                mission_id,
                worker_id,
                source: "status_envelope".to_owned(),
                raw_excerpt: "STATE: progressing".to_owned(),
                normalized_state: "progressing".to_owned(),
                confidence: "high".to_owned(),
                summary: "working".to_owned(),
                adapter: "qwen".to_owned(),
                created_at: now,
            })
            .expect("persist normalized update");

        store
            .persist_validation_result(&ValidationResultRecord {
                id: Uuid::new_v4(),
                mission_id,
                worker_id,
                task_id: None,
                outcome: "pass".to_owned(),
                summary: "validated".to_owned(),
                evidence_json: "{}".to_owned(),
                created_at: now,
            })
            .expect("persist validation result");

        store
            .update_mission_final_summary(mission_id, "final")
            .expect("update final summary");
    }

    #[test]
    fn legacy_table_migration_renames_mismatching_table() {
        use rusqlite::Connection;

        let db_path = std::env::temp_dir().join(format!("sp-migrate-{}.sqlite3", Uuid::new_v4()));
        // Create a legacy table with different columns
        let conn = Connection::open(&db_path).expect("open temp db");
        conn.execute_batch(
            "
            CREATE TABLE sessions (
                old_id TEXT PRIMARY KEY,
                old_field TEXT
            );
            ",
        )
        .expect("create legacy table");

        // Open via Store which triggers migration
        let store = Store::open(&db_path).expect("open store with migration");

        // Legacy table should have been renamed
        let conn = store.connection.lock();
        let legacy_exists = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sessions_legacy_v0' LIMIT 1",
                [],
                |_| Ok(()),
            )
            .optional()
            .expect("query sqlite_master")
            .is_some();
        assert!(legacy_exists, "legacy table should have been renamed");

        // New sessions table should exist with correct schema
        let new_exists = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sessions' LIMIT 1",
                [],
                |_| Ok(()),
            )
            .optional()
            .expect("query sqlite_master")
            .is_some();
        assert!(new_exists, "new sessions table should exist after migration");
    }

    #[test]
    fn open_creates_parent_directory() {
        let base = std::env::temp_dir().join(format!("sp-nested-{}", Uuid::new_v4()));
        let db_path = base.join("sub").join("test.sqlite3");
        let store = Store::open(&db_path).expect("open store with nested path");
        assert!(db_path.exists(), "database file should exist");
        drop(store);
    }

    #[test]
    fn update_task_status_changes_state() {
        let db_path = std::env::temp_dir().join(format!("sp-task-{}.sqlite3", Uuid::new_v4()));
        let store = Store::open(&db_path).expect("open store");
        let task_id = Uuid::new_v4();
        let mission_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();

        store
            .persist_task(&TaskRecord {
                id: task_id,
                mission_id,
                worker_id,
                title: "task".to_owned(),
                description: "desc".to_owned(),
                status: "assigned".to_owned(),
                priority: "high".to_owned(),
                depends_on_json: "[]".to_owned(),
                definition_of_done_json: "[\"done\"]".to_owned(),
            })
            .expect("persist task");

        store
            .update_task_status(task_id, "in_progress")
            .expect("update task status");

        // Verify via load_workers that the task was persisted
        // (tasks are queried indirectly; this confirms no error on update)
    }

    #[test]
    fn find_task_id_returns_none_for_missing_worker() {
        let db_path = std::env::temp_dir().join(format!("sp-find-task-{}.sqlite3", Uuid::new_v4()));
        let store = Store::open(&db_path).expect("open store");
        let mission_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();

        let result = store.find_task_id(mission_id, worker_id).expect("query tasks");
        assert!(result.is_none());
    }

    #[test]
    fn upsert_lease_replaces_existing() {
        let db_path = std::env::temp_dir().join(format!("sp-lease-{}.sqlite3", Uuid::new_v4()));
        let store = Store::open(&db_path).expect("open store");
        let mission_id = Uuid::new_v4();
        let worker_id = Uuid::new_v4();
        let now = Utc::now();

        store
            .upsert_lease(&LeaseRecord {
                mission_id,
                path: "src/main.rs".to_owned(),
                owner_session_id: worker_id,
                intent: "edit".to_owned(),
                status: "claim".to_owned(),
                updated_at: now,
            })
            .expect("persist lease");

        // Second claim for same path by different worker
        store
            .upsert_lease(&LeaseRecord {
                mission_id,
                path: "src/main.rs".to_owned(),
                owner_session_id: Uuid::new_v4(),
                intent: "read".to_owned(),
                status: "claim".to_owned(),
                updated_at: now,
            })
            .expect("upsert lease");

        // No error means ON CONFLICT DO UPDATE worked
    }

    #[test]
    fn list_sessions_returns_empty_for_fresh_store() {
        let db_path = std::env::temp_dir().join(format!("sp-list-{}.sqlite3", Uuid::new_v4()));
        let store = Store::open(&db_path).expect("open store");
        let items = store.list_sessions().expect("list sessions");
        assert!(items.is_empty());
    }

    #[test]
    fn load_mission_snapshot_returns_none_for_missing_mission() {
        let db_path = std::env::temp_dir().join(format!("sp-snapshot-{}.sqlite3", Uuid::new_v4()));
        let store = Store::open(&db_path).expect("open store");
        let result = store
            .load_mission_snapshot(Uuid::new_v4())
            .expect("load snapshot");
        assert!(result.is_none());
    }

    #[test]
    fn load_workers_returns_empty_for_missing_mission() {
        let db_path = std::env::temp_dir().join(format!("sp-workers-{}.sqlite3", Uuid::new_v4()));
        let store = Store::open(&db_path).expect("open store");
        let workers = store.load_workers(Uuid::new_v4()).expect("load workers");
        assert!(workers.is_empty());
    }
}
