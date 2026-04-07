//! Mail subsystem for the Sapphire orchestrator.
//!
//! Handles SAPPHIRE_MAIL routing, ack processing, validation, auto-archival,
//! and engineering-semantic rendering. Extracted from mod.rs for modularity.

mod types;
mod contract;
mod render;
mod nudge_queue;
mod scavenge;
mod timeouts;
mod handlers;
#[cfg(test)]
mod tests;

// ─── Re-exports (same paths as the original single-file module) ──────────────
// These are consumed by orchestrator/mod.rs via `mail::` paths.
// The compiler sees them as "unused" within this module, but they ARE the public API.

#[allow(unused_imports)]
pub use types::{
    normalize_message_type,
    derive_delivery_mode,
    requires_ack,
    validate_mail,
    MailStats,
    resolve_alias,
    parse_mail_id,
    MailHandlingResult,
    QueuedNudge,
    nudge_from_mail,
};

#[allow(unused_imports)]
pub use contract::validate_team_mail;

#[allow(unused_imports)]
pub use render::{
    render_mail_for_delivery,
    render_cc_notice,
};

#[allow(unused_imports)]
pub use nudge_queue::{
    nudge_enqueue,
    nudge_drain,
    nudge_pending_count,
    nudge_format_for_injection,
    drain_nudge_queues,
};

#[allow(unused_imports)]
pub use scavenge::{
    attempt_scavenge_claim,
    release_scavenge,
    ClaimResult,
};

#[allow(unused_imports)]
pub use timeouts::{
    mail_timeout_interval,
    mail_timeout_stage_due,
    recipient_timeout_prompt,
    sender_timeout_prompt,
    cc_timeout_prompt,
    probe_pending_mail,
};

// Re-export the orchestrator's PendingMail to avoid duplication
pub use super::PendingMail;

#[allow(unused_imports)]
pub use handlers::{
    handle_mail_directive,
    handle_ack_directive,
    handle_lease_directive,
    auto_archive_resolved_mail,
};
