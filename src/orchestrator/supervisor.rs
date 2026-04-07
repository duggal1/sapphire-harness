use blake3::hash;

use crate::adapter::SupervisorEventType;
use crate::model::SessionState;
use crate::tmux::SessionHealth;

pub const STATE_CARD_INTERVAL_SECS: u64 = 90;
pub const STATUS_FILE_LIVENESS_GRACE_SECS: u64 = 20;

pub fn notice_key(event_type: SupervisorEventType, body: &str) -> String {
    let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "{}:{}",
        event_type.as_str(),
        hash(normalized.as_bytes()).to_hex()
    )
}

pub fn state_card_key(card: &str) -> String {
    hash(card.as_bytes()).to_hex().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorCondition {
    Healthy,
    ProbeNeeded,
    Unavailable,
}

pub fn classify_supervisor(
    state: SessionState,
    elapsed: std::time::Duration,
    stall_after: std::time::Duration,
    tmux_health: SessionHealth,
) -> SupervisorCondition {
    if matches!(state, SessionState::Failed | SessionState::Exited)
        || matches!(tmux_health, SessionHealth::Dead | SessionHealth::Zombie)
    {
        SupervisorCondition::Unavailable
    } else if matches!(tmux_health, SessionHealth::Healthy | SessionHealth::Starting) {
        SupervisorCondition::Healthy
    } else if matches!(tmux_health, SessionHealth::Hung) && elapsed >= stall_after.mul_f64(3.0) {
        SupervisorCondition::ProbeNeeded
    } else if elapsed >= stall_after.mul_f64(4.0) {
        SupervisorCondition::ProbeNeeded
    } else {
        SupervisorCondition::Healthy
    }
}

pub fn build_repair_supervisor_prompt(
    base_prompt: &str,
    primary_name: &str,
    repair_name: &str,
) -> String {
    format!(
        "{base_prompt}\n\n---\n\n# REPAIR SUPERVISOR MODE\n\n- You are {repair_name}, the standby repair supervisor.\n- The primary supervisor is {primary_name}.\n- Stay synchronized with mission state.\n- Do NOT issue worker actions while the primary supervisor is healthy.\n- When you receive a TAKEOVER prompt, become the acting supervisor immediately.\n- Once acting, supervise normally, drive cleanup, and produce the final concise markdown summary.\n- If the primary is unavailable, keep the company moving. Do not freeze the team.\n- Never direct any worker to run `git push`, `git restore`, or `git reset`.\n- Treat dirty git trees as normal multi-agent conditions unless git itself is broken.\n"
    )
}

pub fn build_repair_sync_prompt(card: &str, primary_name: &str) -> String {
    format!(
        "STANDBY SYNC ONLY.\nPrimary supervisor: {primary_name}.\nTrack this state quietly and be ready to take over if needed.\n\n{card}"
    )
}

pub fn build_takeover_prompt(card: &str, failed_supervisor_name: &str) -> String {
    format!(
        "TAKEOVER NOW.\nThe primary supervisor {failed_supervisor_name} is unavailable or unhealthy.\nYou are now the acting supervisor. Resume active supervision immediately, keep worker coordination moving, and provide concise supervisory markdown when the mission is complete.\n\n{card}"
    )
}
