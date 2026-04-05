//! Unit tests for the mail subsystem.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::types::*;
use super::timeouts::*;
use super::PendingMail;

fn sample_pending(priority: &str) -> PendingMail {
    PendingMail {
        message_id: Uuid::new_v4(),
        thread_id: "thread-1".to_owned(),
        intent: "dependency".to_owned(),
        thread_state: "open".to_owned(),
        duplicate_key: "dup-1".to_owned(),
        sender_session_id: Uuid::new_v4(),
        recipient_session_id: Uuid::new_v4(),
        cc_session_ids: Vec::new(),
        sender_pod: "build".to_owned(),
        recipient_pod: "platform".to_owned(),
        routing_class: "cross_pod".to_owned(),
        subject: "Need dependency answer".to_owned(),
        message_type: "task".to_owned(),
        priority: priority.to_owned(),
        routed_at: Instant::now(),
        acked: false,
        timeout_stage: 0,
        last_timeout_at: None,
        reply_count: 0,
    }
}

#[test]
fn timeout_interval_respects_priority() {
    assert_eq!(mail_timeout_interval("urgent"), Duration::from_secs(10));
    assert_eq!(mail_timeout_interval("high"), Duration::from_secs(15));
    assert_eq!(mail_timeout_interval("normal"), Duration::from_secs(20));
    assert_eq!(mail_timeout_interval("low"), Duration::from_secs(45));
}

#[test]
fn timeout_stage_due_uses_last_timeout_marker() {
    let mut pending = sample_pending("normal");
    pending.routed_at = Instant::now() - Duration::from_secs(25);
    assert!(mail_timeout_stage_due(&pending, Instant::now()));

    pending.last_timeout_at = Some(Instant::now());
    assert!(!mail_timeout_stage_due(&pending, Instant::now()));
}

#[test]
fn recipient_timeout_prompt_demands_explicit_ack_status() {
    let pending = sample_pending("high");
    let prompt = recipient_timeout_prompt(&pending, "Engineer-1", 1);
    assert!(prompt.contains("SAPPHIRE_ACK"));
    assert!(prompt.contains("cannot_comply"));
    assert!(prompt.contains("done"));
}

#[test]
fn sender_timeout_prompt_advises_independent_work() {
    let pending = sample_pending("normal");
    let prompt = sender_timeout_prompt(&pending, "Engineer-2", 1);
    assert!(prompt.contains("independent work"));
}

#[test]
fn cc_timeout_prompt_is_monitor_only() {
    let pending = sample_pending("high");
    let prompt = cc_timeout_prompt(&pending, 1);
    assert!(prompt.contains("CC NOTICE"));
    assert!(prompt.contains("Monitor only"));
}

#[test]
fn normalize_message_type_passes_clean_types() {
    assert_eq!(normalize_message_type("task"), "task");
    assert_eq!(normalize_message_type("reply"), "reply");
    assert_eq!(normalize_message_type("notification"), "notification");
    assert_eq!(normalize_message_type("escalation"), "escalation");
    assert_eq!(normalize_message_type("scavenge"), "scavenge");
}

#[test]
fn normalize_message_type_maps_legacy_to_task() {
    assert_eq!(normalize_message_type("dependency_request"), "task");
    assert_eq!(normalize_message_type("dependency_response"), "task");
    assert_eq!(normalize_message_type("review_request"), "task");
    assert_eq!(normalize_message_type("review_response"), "task");
    assert_eq!(normalize_message_type("handoff"), "task");
    assert_eq!(normalize_message_type("collision_warning"), "task");
}

#[test]
fn normalize_message_type_maps_legacy_to_notification() {
    assert_eq!(normalize_message_type("completion_notice"), "notification");
}

#[test]
fn normalize_message_type_maps_legacy_to_escalation() {
    assert_eq!(normalize_message_type("blocker"), "escalation");
    assert_eq!(normalize_message_type("architecture_concern"), "escalation");
    assert_eq!(normalize_message_type("supervisor_directive"), "escalation");
}

#[test]
fn normalize_message_type_unknown_defaults_to_notification() {
    assert_eq!(normalize_message_type("random_gibberish"), "notification");
}

#[test]
fn derive_delivery_mode_respects_explicit() {
    assert_eq!(derive_delivery_mode("low", "interrupt"), "interrupt");
    assert_eq!(derive_delivery_mode("low", "queue"), "queue");
    assert_eq!(derive_delivery_mode("urgent", ""), "interrupt");
}

#[test]
fn derive_delivery_mode_derives_from_priority() {
    assert_eq!(derive_delivery_mode("urgent", ""), "interrupt");
    assert_eq!(derive_delivery_mode("critical", ""), "interrupt");
    assert_eq!(derive_delivery_mode("high", ""), "queue");
    assert_eq!(derive_delivery_mode("normal", ""), "queue");
    assert_eq!(derive_delivery_mode("low", ""), "queue");
}

#[test]
fn requires_ack_respects_explicit_flag() {
    assert!(requires_ack("reply", "low", true));
}

#[test]
fn requires_ack_for_task_escalation_scavenge() {
    assert!(requires_ack("task", "low", false));
    assert!(requires_ack("escalation", "low", false));
    assert!(requires_ack("scavenge", "low", false));
}

#[test]
fn requires_ack_for_urgent_notification() {
    assert!(requires_ack("notification", "urgent", false));
    assert!(requires_ack("notification", "high", false));
    assert!(!requires_ack("notification", "normal", false));
    assert!(!requires_ack("notification", "low", false));
}

#[test]
fn requires_ack_false_for_reply() {
    assert!(!requires_ack("reply", "urgent", false));
    assert!(!requires_ack("reply", "normal", false));
}

#[test]
fn parse_mail_id_valid_uuid() {
    let id = parse_mail_id("00000000-0000-0000-0000-000000000000");
    assert!(id.is_some());
}

#[test]
fn parse_mail_id_invalid_uuid() {
    let id = parse_mail_id("not-a-uuid");
    assert!(id.is_none());
}

#[test]
fn parse_mail_id_trims_whitespace() {
    let id = parse_mail_id("  00000000-0000-0000-0000-000000000000  ");
    assert!(id.is_some());
}

#[test]
fn resolve_alias_direct_match() {
    let mut map = HashMap::new();
    let id = Uuid::new_v4();
    map.insert("Engineer-1".to_owned(), id);
    assert_eq!(resolve_alias(&map, "Engineer-1"), Some(id));
}

#[test]
fn resolve_alias_case_insensitive() {
    let mut map = HashMap::new();
    let id = Uuid::new_v4();
    map.insert("Engineer-1".to_owned(), id);
    assert_eq!(resolve_alias(&map, "engineer-1"), Some(id));
}

#[test]
fn resolve_alias_not_found() {
    let map: HashMap<String, Uuid> = HashMap::new();
    assert_eq!(resolve_alias(&map, "Nobody"), None);
}
