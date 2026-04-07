use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::model::{SessionRole, SessionState};
use crate::tmux::{SessionHealth, Tmux};
use uuid::Uuid;

use super::ActiveSession;

const TMUX_LIVENESS_WINDOW: Duration = Duration::from_secs(180);
const RECENT_OUTPUT_GRACE: Duration = Duration::from_secs(12);
const RECENT_STATUS_GRACE: Duration = Duration::from_secs(45);

pub struct Snapshot {
    labels: HashMap<Uuid, String>,
}

impl Snapshot {
    pub fn build(
        active_sessions: &HashMap<Uuid, ActiveSession>,
        now: Instant,
    ) -> Self {
        let terminal_live = active_sessions
            .iter()
            .map(|(session_id, session)| (*session_id, session_terminal_live(session)))
            .collect::<HashMap<_, _>>();
        let labels = active_sessions
            .iter()
            .map(|(session_id, session)| {
                (
                    *session_id,
                    effective_state_label_with_terminal_live(
                        session,
                        now,
                        *terminal_live.get(session_id).unwrap_or(&false),
                    ),
                )
            })
            .collect::<HashMap<_, _>>();
        Self { labels }
    }

    pub fn effective_state_label<'a>(&'a self, session: &ActiveSession) -> &'a str {
        self.labels
            .get(&session.record.id)
            .map(String::as_str)
            .unwrap_or_else(|| session.state.as_str())
    }

    pub fn counts_as_blocked(&self, session: &ActiveSession) -> bool {
        matches!(self.effective_state_label(session), "blocked" | "stalled")
    }

    pub fn counts_as_problem(&self, session: &ActiveSession) -> bool {
        matches!(
            self.effective_state_label(session),
            "failed" | "contradictory" | "blocked" | "stalled" | "wrong_direction" | "needs_retry"
        ) || session.validation_pending
    }
}

pub fn session_terminal_live(session: &ActiveSession) -> bool {
    session
        .runtime
        .terminal_target()
        .map(|target| Tmux::new(None).check_session_health(target, TMUX_LIVENESS_WINDOW))
        .is_some_and(|health| {
            matches!(
                health,
                SessionHealth::Healthy | SessionHealth::Hung | SessionHealth::Starting
            )
        })
}

pub fn session_has_recent_activity(session: &ActiveSession, now: Instant) -> bool {
    session.output_chunks > 0
        && now.duration_since(session.last_output_at) <= RECENT_OUTPUT_GRACE
            || session
                .last_status_update_at
                .is_some_and(|at| now.duration_since(at) <= RECENT_STATUS_GRACE)
}

pub fn session_is_live(session: &ActiveSession, now: Instant) -> bool {
    session_has_recent_activity(session, now) || session_terminal_live(session)
}

#[allow(dead_code)]
pub fn effective_state_label(session: &ActiveSession, now: Instant) -> String {
    effective_state_label_with_terminal_live(session, now, session_terminal_live(session))
}

fn effective_state_label_with_terminal_live(
    session: &ActiveSession,
    now: Instant,
    terminal_live: bool,
) -> String {
    if session.record.role == SessionRole::Supervisor {
        return effective_supervisor_state_label_with_terminal_live(session, now, terminal_live);
    }
    if session.state.is_terminal() {
        return session.state.as_str().to_owned();
    }
    if session.validation_pending {
        return SessionState::NeedsValidation.as_str().to_owned();
    }
    if (session_has_recent_activity(session, now) || terminal_live)
        && matches!(
            session.state,
            SessionState::Blocked
                | SessionState::Stalled
                | SessionState::NeedsRetry
                | SessionState::WrongDirection
        )
    {
        return "running".to_owned();
    }
    session.state.as_str().to_owned()
}

#[allow(dead_code)]
pub fn effective_supervisor_state_label(session: &ActiveSession, now: Instant) -> String {
    effective_supervisor_state_label_with_terminal_live(session, now, session_terminal_live(session))
}

fn effective_supervisor_state_label_with_terminal_live(
    session: &ActiveSession,
    now: Instant,
    terminal_live: bool,
) -> String {
    if session.state.is_terminal() {
        return session.state.as_str().to_owned();
    }
    if session_has_recent_activity(session, now) || terminal_live {
        "running".to_owned()
    } else {
        session.state.as_str().to_owned()
    }
}

#[allow(dead_code)]
pub fn counts_as_blocked(session: &ActiveSession, now: Instant) -> bool {
    matches!(effective_state_label(session, now).as_str(), "blocked" | "stalled")
}

#[allow(dead_code)]
pub fn counts_as_problem(session: &ActiveSession, now: Instant) -> bool {
    matches!(
        effective_state_label(session, now).as_str(),
        "failed" | "contradictory" | "blocked" | "stalled" | "wrong_direction" | "needs_retry"
    ) || session.validation_pending
}

pub fn should_pause_mail_timeout(session: &ActiveSession, now: Instant) -> bool {
    session_is_live(session, now)
        || !session.queued_prompts.is_empty()
        || session
            .last_prompt_sent_at
            .is_some_and(|at| now.duration_since(at) <= Duration::from_secs(20))
}
