//! Engineering-semantic rendering of mail for PTY delivery.
//! Each message type gets a distinct header format so agents instantly understand intent.

use uuid::Uuid;

use crate::protocol::MailDirective;
use super::types::{normalize_message_type};

/// Render mail for PTY injection with engineering-team semantics.
/// Each type gets a distinct header format so agents instantly understand intent.
pub fn render_mail_for_delivery(
    message_id: Uuid,
    thread_id: &str,
    sender_name: &str,
    directive: &MailDirective,
    cc_ids: &[Uuid],
    requires_ack: bool,
    is_urgent: bool,
) -> String {
    let normalized = normalize_message_type(&directive.message_type);

    let header = match normalized {
        "task" => format!(
            "[SAPPHIRE TASK — ACTION REQUIRED]\n\
             FROM: {sender}\nTO: {to}\n\
             THREAD: {thread}\nPRIORITY: {priority}\n\n\
             SUBJECT: {subject}\n\n\
             CONTEXT:\n{context}\n\n\
             REQUEST:\n{request}\n\n\
             EXPECTED ACTION:\n{expected}",
            sender = sender_name,
            to = directive.to,
            thread = thread_id,
            priority = directive.priority.to_uppercase(),
            subject = directive.subject,
            context = directive.context,
            request = directive.request,
            expected = directive.expected_action,
        ),
        "escalation" => format!(
            "⚠ [SAPPHIRE ESCALATION — BLOCKER]\n\
             FROM: {sender}\nTO: {to}\n\
             THREAD: {thread}\nPRIORITY: {priority}\n\n\
             SUBJECT: {subject}\n\n\
             CONTEXT:\n{context}\n\n\
             BLOCKER:\n{request}\n\n\
             ESCALATION REQUEST:\n{expected}",
            sender = sender_name,
            to = directive.to,
            thread = thread_id,
            priority = directive.priority.to_uppercase(),
            subject = directive.subject,
            context = directive.context,
            request = directive.request,
            expected = directive.expected_action,
        ),
        "scavenge" => format!(
            "[SAPPHIRE WORK AVAILABLE — FIRST TO CLAIM]\n\
             FROM: {sender}\nTO: {to}\n\
             THREAD: {thread}\n\n\
             SUBJECT: {subject}\n\n\
             CONTEXT:\n{context}\n\n\
             AVAILABLE WORK:\n{request}\n\n\
             To claim this work, acknowledge and take ownership.",
            sender = sender_name,
            to = directive.to,
            thread = thread_id,
            subject = directive.subject,
            context = directive.context,
            request = directive.request,
        ),
        "reply" => format!(
            "[SAPPHIRE REPLY]\n\
             FROM: {sender}\nTHREAD: {thread}\n\n\
             SUBJECT: Re: {subject}\n\n\
             {context}",
            sender = sender_name,
            thread = thread_id,
            subject = directive.subject,
            context = if directive.request.is_empty() { &directive.context } else { &directive.request },
        ),
        _ => format!(
            "[SAPPHIRE NOTICE]\n\
             FROM: {sender}\nTO: {to}\n\
             THREAD: {thread}\n\n\
             SUBJECT: {subject}\n\n\
             {body}",
            sender = sender_name,
            to = directive.to,
            thread = thread_id,
            subject = directive.subject,
            body = if directive.request.is_empty() { &directive.context } else { &directive.request },
        ),
    };

    let cc_line = if cc_ids.is_empty() {
        String::new()
    } else {
        format!("\nCC: {} recipients (visibility only)\n", cc_ids.len())
    };

    let ack_instruction = if requires_ack {
        if is_urgent {
            format!(
                "\n⚡ URGENT: Acknowledge IMMEDIATELY with:\n\
                 SAPPHIRE_ACK {{\"mail_id\":\"{}\",\"status\":\"acked\",\"summary\":\"acknowledged\"}}",
                message_id
            )
        } else {
            format!(
                "\nAcknowledge with:\n\
                 SAPPHIRE_ACK {{\"mail_id\":\"{}\",\"status\":\"acked\",\"summary\":\"one short sentence\"}}",
                message_id
            )
        }
    } else {
        "\nNo ack required. Respond via SAPPHIRE_MAIL or SAPPHIRE_STATUS when your state changes.".to_owned()
    };

    format!("{header}{cc_line}{ack_instruction}")
}

// ─── CC notice rendering ────────────────────────────────────────────────────

/// Render a CC visibility notice for non-primary recipients.
pub fn render_cc_notice(
    thread_id: &str,
    sender_name: &str,
    recipient_name: &str,
    directive: &MailDirective,
) -> String {
    format!(
        "[SAPPHIRE CC NOTICE]\n\
         You are CC'd on mail thread: {thread}\n\
         FROM: {from}\nTO: {to}\n\
         SUBJECT: {subject}\n\
         TYPE: {msg_type}\n\n\
         No action required. Monitor thread for context.",
        thread = thread_id,
        from = sender_name,
        to = recipient_name,
        subject = directive.subject,
        msg_type = normalize_message_type(&directive.message_type),
    )
}
