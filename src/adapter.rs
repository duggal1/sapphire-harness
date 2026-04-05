use std::sync::OnceLock;

use anyhow::Result;
use regex::Regex;
use serde::Deserialize;

use crate::agent::AgentKind;
use crate::model::{
    MissionPlan, RiskItem, SessionState, WorkerPacket, Workstream, WorkstreamExecution,
};
use crate::templates::PromptLibrary;

/// Confidence level for adapter-normalized state observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// Clear keyword match or explicit directive
    High,
    /// Heuristic inference from output patterns
    Medium,
    /// Weak signal, ambiguous keywords
    Low,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

/// Event types that trigger supervisor intervention.
/// Each type maps to a specific rule injection in the micro-prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorEventType {
    /// Worker produced no output beyond stall_seconds threshold.
    Stall,
    /// Worker claimed completion — supervisor must validate.
    DoneClaimed,
    /// Worker output is vague, shallow, or suspiciously fast.
    WeakOutput,
    /// Two workers report conflicting changes to the same surface.
    Contradiction,
    /// Worker cannot proceed due to dependency or ambiguity.
    Blocked,
    /// Worker process exited with non-zero or unexpected state.
    Failed,
    /// General notice — mail routed, state change, or watchdog observation.
    Notice,
}

impl SupervisorEventType {
    /// The rule text to inject into the supervisor micro-prompt.
    /// Each rule is the single relevant instruction the supervisor needs for this event.
    pub fn injected_rule(self) -> &'static str {
        match self {
            Self::Stall => "STALLED WORKER RULE: Send a corrective prompt immediately. Force concrete next steps. If repeated, use retry_worker or redirect_worker. Never ignore a stall.",
            Self::DoneClaimed => "DONE CLAIM RULE: Never accept claims without proof. Demand exact files changed, commands run, results observed, remaining risk, and overlap report. Reject vague completions.",
            Self::WeakOutput => "WEAK OUTPUT RULE: Ask sharper follow-ups. Demand evidence. Narrow the task again. Do not accept cosmetic or generic progress.",
            Self::Contradiction => "CONTRADICTION RULE: Identify the exact collision. Determine ownership. Preserve the better/newer/more-validated work. Redirect the losing worker.",
            Self::Blocked => "BLOCKED WORKER RULE: Identify the exact blocker. Determine whether clarification, rerouting, or dependency resolution is needed. Do not let a worker sit blocked.",
            Self::Failed => "FAILED WORKER RULE: Preserve useful partials. Reassign or continue around the failure. Do not treat failure as total loss.",
            Self::Notice => "SUPERVISION RULE: Stay active. Monitor all workers. Catch drift, overlap, and fake completion. Maximize throughput, not noise.",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stall => "stall",
            Self::DoneClaimed => "done_claimed",
            Self::WeakOutput => "weak_output",
            Self::Contradiction => "contradiction",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Notice => "notice",
        }
    }
}

/// Adapter-normalized observation — inferred state from raw PTY output
/// when no explicit SAPPHIRE_STATUS directive was found.
#[derive(Debug, Clone)]
pub struct NormalizedObservation {
    /// Inferred session state
    pub state: SessionState,
    /// How confident the adapter is in this inference
    pub confidence: Confidence,
    /// Brief summary of what the worker appears to be doing
    pub summary: String,
    /// Raw output excerpt that triggered this observation
    pub raw_excerpt: String,
    /// Files mentioned in the output
    pub files: Vec<String>,
    /// Blocker reason if state is Blocked
    pub blocker: Option<String>,
    /// Which adapter produced this (qwen, codex, claude, forge)
    pub source: &'static str,
}

/// Status envelope extracted from freeform output via regex.
/// Parses `STATE/SUMMARY/FILES/BLOCKER/DONE` blocks from agent output.
#[derive(Debug, Clone)]
pub struct StatusEnvelope {
    /// Raw state string from the envelope
    pub state: String,
    /// Summary of what the worker is doing
    pub summary: String,
    /// Files the worker touched
    pub files: Vec<String>,
    /// Blocker reason if present
    pub blocker: Option<String>,
    /// Done claim description if present
    pub done: Option<String>,
    /// The raw output block this was extracted from
    pub raw_excerpt: String,
}

/// Structured action the supervisor can issue to control workers.
/// Parsed from supervisor output and executed by the watchdog.
#[derive(Debug, Clone)]
pub struct SupervisorAction {
    /// Action type: observe, validate_worker, retry_worker, redirect_worker,
    /// message_worker, accept_worker, fail_worker
    pub action: String,
    /// Target worker display name (e.g. "Engineer-1")
    pub target: Option<String>,
    /// Brief summary of why this action is being taken
    pub summary: String,
    /// Additional message to send to the worker (for retry/redirect/message)
    pub message: Option<String>,
}

/// Final state summary extracted from worker output.
/// Used for mission completion reporting.
#[derive(Debug, Clone)]
pub struct FinalEnvelope {
    /// Final session state
    pub state: SessionState,
    /// Summary of final result
    pub summary: String,
}

#[allow(dead_code)]
pub trait CliAdapter: Send + Sync {
    fn kind(&self) -> AgentKind;

    fn detect_state(&self, raw_output: &str) -> Option<NormalizedObservation>;

    fn build_assignment_prompt(
        &self,
        prompts: &PromptLibrary,
        mission: &str,
        packet: &WorkerPacket,
    ) -> String;

    fn build_validation_prompt(&self) -> String;

    fn build_correction_prompt(&self, reason: &str) -> String;

    fn build_status_prompt(&self, reason: &str) -> String;

    fn extract_summary(&self, raw_output: &str) -> Option<String>;

    fn detect_done_claim(&self, raw_output: &str) -> bool;

    fn build_supervisor_plan_prompt(
        &self,
        mission: &str,
        worker_count: usize,
    ) -> String;

    fn extract_supervisor_plan(&self, raw_output: &str) -> Option<MissionPlan>;

    fn build_supervisor_action_prompt(&self, event_type: SupervisorEventType, context: &str) -> String;

    fn extract_supervisor_action(&self, raw_output: &str) -> Option<SupervisorAction>;

    fn build_final_summary_prompt(&self) -> String;

    fn extract_final_envelope(&self, raw_output: &str) -> Option<FinalEnvelope>;
}

pub fn adapter_for(kind: AgentKind) -> Box<dyn CliAdapter> {
    match kind {
        AgentKind::Qwen => Box::new(QwenAdapter),
        AgentKind::Forge => Box::new(ForgeAdapter),
        AgentKind::Codex => Box::new(CodexAdapter),
        AgentKind::Claude => Box::new(ClaudeAdapter),
    }
}

struct QwenAdapter;
struct CodexAdapter;
struct ClaudeAdapter;
struct ForgeAdapter;

macro_rules! impl_standard_adapter {
    ($name:ident, $kind:expr) => {
        impl CliAdapter for $name {
            fn kind(&self) -> AgentKind {
                $kind
            }

            fn detect_state(&self, raw_output: &str) -> Option<NormalizedObservation> {
                detect_state_impl($kind, raw_output)
            }

            fn build_assignment_prompt(
                &self,
                prompts: &PromptLibrary,
                mission: &str,
                packet: &WorkerPacket,
            ) -> String {
                build_assignment_prompt_impl(prompts, mission, packet)
            }

            fn build_validation_prompt(&self) -> String {
                build_validation_prompt_impl()
            }

            fn build_correction_prompt(&self, reason: &str) -> String {
                build_correction_prompt_impl(reason)
            }

            fn build_status_prompt(&self, reason: &str) -> String {
                build_status_prompt_impl(reason)
            }

            fn extract_summary(&self, raw_output: &str) -> Option<String> {
                extract_summary_impl(raw_output)
            }

            fn detect_done_claim(&self, raw_output: &str) -> bool {
                detect_done_claim_impl(raw_output)
            }

            fn build_supervisor_plan_prompt(
                &self,
                mission: &str,
                worker_count: usize,
            ) -> String {
                build_supervisor_plan_prompt_impl(mission, worker_count)
            }

            fn extract_supervisor_plan(&self, raw_output: &str) -> Option<MissionPlan> {
                extract_supervisor_plan_impl(raw_output)
            }

            fn build_supervisor_action_prompt(
                &self,
                event_type: SupervisorEventType,
                context: &str,
            ) -> String {
                build_supervisor_action_prompt_impl(event_type, context)
            }

            fn extract_supervisor_action(&self, raw_output: &str) -> Option<SupervisorAction> {
                extract_supervisor_action_impl(raw_output)
            }

            fn build_final_summary_prompt(&self) -> String {
                build_final_summary_prompt_impl()
            }

            fn extract_final_envelope(&self, raw_output: &str) -> Option<FinalEnvelope> {
                extract_final_envelope_impl(raw_output)
            }
        }
    };
}

impl_standard_adapter!(CodexAdapter, AgentKind::Codex);
impl_standard_adapter!(ClaudeAdapter, AgentKind::Claude);
impl_standard_adapter!(ForgeAdapter, AgentKind::Forge);
impl_standard_adapter!(QwenAdapter, AgentKind::Qwen);

fn build_assignment_prompt_impl(
    prompts: &PromptLibrary,
    mission: &str,
    packet: &WorkerPacket,
) -> String {
    let envelope = status_envelope_contract();
    format!(
        "{}\n\nStatus contract:\nUse the exact 5-line envelope below whenever Sapphire asks for status or validation.\n\n{}",
        prompts.render_worker_prompt(mission, packet),
        envelope
    )
}

fn build_validation_prompt_impl() -> String {
    let lead = "Validation challenge.\nDo not continue broad explanation.";
    format!(
        "{lead}\nReply only with:\n{}\nUse STATE as one of validated, needs_retry, blocked, failed.",
        status_envelope_contract()
    )
}

fn build_correction_prompt_impl(reason: &str) -> String {
    let lead = format!("Your last reply did not follow the required status format: {reason}\nDo not continue broad explanation.");
    format!("{lead}\nReply only with:\n{}", status_envelope_contract())
}

fn build_status_prompt_impl(reason: &str) -> String {
    let tail = format!("Status update only.\n{reason}\nReply exactly with:");
    format!("{tail}\n{}", status_envelope_contract())
}

fn build_supervisor_plan_prompt_impl(
    mission: &str,
    worker_count: usize,
) -> String {
    format!(
        "You are the supervisor brain. Convert one messy mission into the execution plan Sapphire will use.\n\
        \n\
        STRICT JSON OUTPUT RULES (VIOLATE ANY = INVALID):\n\
        - Output MUST be raw JSON only. NO markdown, NO code fences, NO backticks, NO explanation.\n\
        - MUST start with BEGIN_SAPPHIRE_PLAN_JSON on its own line and end with END_SAPPHIRE_PLAN_JSON on its own line.\n\
        - NOTHING outside those two markers. Not even a single word before or after.\n\
        - All strings use double quotes only. No single quotes. No trailing commas.\n\
        - NO newlines inside string values. Use spaces only.\n\
        - All arrays are actual JSON arrays [...], never strings.\n\
        - The entire block between markers MUST parse as valid JSON on first try.\n\
        - Do NOT wrap JSON in ```json or ``` or any markdown formatting.\n\
        \n\
        LENGTH RULES:\n\
        - EVERY string field must be 10 words or fewer.\n\
        - Total JSON must stay under 2000 characters.\n\
        - Be dense and actionable. No filler words.\n\
        \n\
        EXACT JSON EXAMPLE (follow this structure, NOT the specific roles — choose roles based on YOUR analysis of the mission):\n\
        BEGIN_SAPPHIRE_PLAN_JSON\n\
        {{\"mission_rewrite\":\"<10 word mission summary>\",\"team_activation\":[{{\"role_type\":\"<analyze mission — pick from 15 roles>\",\"display_name\":\"<Role-Type-N>\",\"reason\":\"<why THIS role for THIS mission>\"}}],\"workstreams\":[{{\"id\":\"<short id>\",\"name\":\"<workstream name>\",\"execution\":\"parallel|dependent|validation|integration\",\"owned_scope\":\"<what files/areas>\",\"success_criteria\":[\"<criterion>\"],\"depends_on\":[]}}],\"risk_map\":[{{\"zone\":\"<area>\",\"risk\":\"<risk type>\",\"mitigation\":\"<how to prevent>\"}}],\"worker_packets\":[{{\"worker_id\":\"<display name>\",\"role\":\"<Role Title>\",\"role_type\":\"<same as team_activation>\",\"display_name\":\"<same as team_activation>\",\"starting_angle\":\"<unique approach>\",\"owned_scope\":\"<files/areas>\",\"explicit_task\":\"<what to do>\",\"out_of_scope\":\"<what NOT to touch>\",\"definition_of_done\":[\"<criteria>\"],\"required_evidence\":[\"<proof>\"],\"blocker_protocol\":\"report blockers immediately\",\"conflict_warning\":\"claim files first\",\"communication_rules\":[\"mail blockers\"],\"validation_standard\":[\"pass checks\"],\"expected_output_format\":[\"summary\"]}}],\"supervision_strategy\":\"<tight|normal|loose>\"}}\n\
        END_SAPPHIRE_PLAN_JSON\n\
        \n\
        CRITICAL: The JSON example above shows the STRUCTURE only. Do NOT copy the role_types from the example. Analyze the actual mission and activate ONLY the roles that the mission genuinely needs. A docs-only mission might need only software-engineer. A security audit needs security-engineer. A full feature needs multiple roles. YOU decide based on the mission, not the example.\n\
        \n\
        Rules:\n\
        - You are planning for EXACTLY {worker_count} visible worker terminals.\n\
        - Worker packet count must equal {worker_count}.\n\
        - Do NOT invent extra workers, stewards, docs custodians, alternates, backups, or helper slots.\n\
        - There are exactly 15 role TYPES available. Each type can have MULTIPLE instances. For example: 5 software-engineers = Engineer-1, Engineer-2, Engineer-3, Engineer-4, Engineer-5. The numeric suffix means you can spawn as many of any role type as the mission needs.\n\
        - role_type must be one of these 15 types: software-engineer, research-engineer, validation-engineer, architecture-engineer, security-engineer, debug-and-review-engineer, testing-and-automation-engineer, designer-engineer, sales-engineer, solutions-engineer, customer-success-engineer, product-engineer, product-manager, revenue-engineer, compliance-engineer\n\
        - display_name uses function-first naming with numeric identity: Engineer-N (software-engineer), Designer-N (designer-engineer), Reviewer-N (debug-and-review-engineer), Architect-N (architecture-engineer), Security-N (security-engineer), Validator-N (validation-engineer), QA-N (testing-and-automation-engineer), Researcher-N (research-engineer), Sales-N (sales-engineer), Solutions-N (solutions-engineer), CustomerSuccess-N (customer-success-engineer), Product-N (product-engineer), ProductManager-N (product-manager), Revenue-N (revenue-engineer), Compliance-N (compliance-engineer)\n\
        - For go-to-market work, bias toward enterprise B2B, high-ticket, high-margin selling. Reject B2C fluff, low-value volume motions, and vague sales theater.\n\
        - Supervisor analyzes the mission and activates ONLY the role types the mission genuinely needs. You can pick 1 role type and spawn all {worker_count} workers as that type, OR pick 5 types and distribute workers across them. YOU decide the mix.\n\
        - A coding-heavy mission might need multiple software-engineers. A security audit might need security-engineers plus software-engineers. A documentation mission might need software-engineers plus a designer-engineer.\n\
        - Each packet has materially different starting_angle — even same-type workers must attack different slices.\n\
        - execution must be one of: parallel, dependent, validation, integration.\n\
        - Output EXACTLY {worker_count} worker_packets. Not {worker_count}+1. Not extra alternates. Not optional backups.\n\
        - If you think of extra workers, DO NOT include them. Trim the plan to EXACTLY {worker_count} worker_packets.\n\
        - If the mission could use more workers than {worker_count}, compress the plan. Keep only the highest-leverage {worker_count} worker_packets.\n\
        \n\
        Mission:\n\
        {mission}\n",
    )
}

fn build_supervisor_action_prompt_impl(
    event_type: SupervisorEventType,
    context: &str,
) -> String {
    let lead = "Supervisor intervention required.";
    let rule = event_type.injected_rule();

    format!(
        "{lead}\n\n{rule}\n\nDecision discipline:\n- Treat fresh .sp/.hide.sp worker status JSON as authoritative.\n- Do not call a worker stalled if it has a fresh status update.\n- Do not repeat the same action unless the worker produced a NEW status update after your last action.\n- If the workers already finished the task, stop issuing worker actions and move to final synthesis.\n- Keep one target per reply. No multi-action prose.\n\nIssue:\n{context}\n\nReply with four lines in this order.\nAllowed ACTION values: observe, validate_worker, retry_worker, redirect_worker, message_worker, accept_worker, fail_worker.\nTARGET must be a worker display name or NONE.\nSUMMARY must be one short sentence.\nMESSAGE must be one short instruction or NONE.\nFields:\nACTION:\nTARGET:\nSUMMARY:\nMESSAGE:"
    )
}

fn build_final_summary_prompt_impl() -> String {
    "All workers are terminal. Stop issuing worker actions. Produce the final concise synthesis of what happened, how the workers coordinated, and the final result.\nReply only with:\nFINAL_STATE: validated or failed\nFINAL_SUMMARY: one concise paragraph".to_owned()
}

// ─── Shared keyword lists for heuristic state detection ───────────────────────

const KEYWORDS_VALIDATED: &[&str] = &["validation passed", "validated", "all checks passed"];
const KEYWORDS_WEAK_OUTPUT: &[&str] = &[
    "probably fixed",
    "should work now",
    "didn't run tests",
    "cannot verify",
    "can't verify",
];
const KEYWORDS_WRONG_DIRECTION: &[&str] = &[
    "rewrote the architecture",
    "rewrote the whole",
    "full rewrite",
    "changed unrelated",
    "refactored broadly",
    "took over",
];
const KEYWORDS_BLOCKED: &[&str] = &[
    "blocked",
    "cannot proceed",
    "can't proceed",
    "waiting on",
    "need clarification",
    "dependency",
    "unclear owner",
];
const KEYWORDS_CONTRADICTION: &[&str] = &[
    "conflict",
    "overlap",
    "contradiction",
    "someone else changed",
    "merge collision",
];
const KEYWORDS_PROGRESSING: &[&str] = &[
    "investigating",
    "reproducing",
    "working on",
    "running tests",
    "profiling",
    "reviewing",
    "checking",
];
const KEYWORDS_CLI_FAILURE: &[&str] = &[
    "invalid number of stops",
    "traceback",
    "panic",
    "exception",
    "fatal:",
];
const KEYWORDS_CLI_FAILURE_QWEN: &[&str] = &[
    "invalid number of stops",
    "[api error:",
    "critical error",
    "traceback",
    "panic",
    "exception",
    "fatal:",
];
const KEYWORDS_VALIDATED_QWEN: &[&str] = &[
    "validation passed",
    "validated successfully",
    "all checks passed",
];
const KEYWORDS_QWEN_TOOL_SIGNAL: &[&str] = &[
    "readfile",
    "writefile",
    "editfile",
    "starting work",
    "now i'll create",
    "success: readfile",
    "success: writefile",
];
const KEYWORDS_PROMPT_ECHO: &[&str] = &[
    "definition of done",
    "required evidence",
    "expected output format",
    "communication rules",
    "validation standard",
    "sapphire control protocol",
    "repository bootstrap:",
    "you are worker-",
    "you are the **supervisor**",
    "# role",
    "primary objective",
    "mission:",
    "scope:",
    "task:",
    "out of scope:",
    "done:",
    "evidence:",
    "blocker protocol:",
    "output format:",
    "rules:",
];
const KEYWORDS_DONE_CLAIM: &[&str] = &[
    "i'm done",
    "im done",
    "completed the task",
    "finished the task",
    "task is complete",
    "implemented the requested",
    "done with",
];
const KEYWORDS_QWEN_SIGNAL: &[&str] = &[
    "model:",
    "success:",
    "readfile",
    "writefile",
    "editfile",
    "starting work",
    "execute",
    "bash",
];
const KEYWORDS_QWEN_PROMPT_ECHO_NEG: &[&str] = &[
    "model:",
    "success:",
    "readfile",
    "writefile",
    "editfile",
    "starting work",
    "sapphire_status",
    "sapphire_mail",
    "sapphire_ack",
    "sapphire_lease",
];

fn detect_state_impl(kind: AgentKind, raw_output: &str) -> Option<NormalizedObservation> {
    if let Some(envelope) = extract_status_envelope_impl(raw_output) {
        let mut state = map_state_token(&envelope.state)?;
        if matches!(state, SessionState::Progressing | SessionState::DoneClaimed)
            && envelope
                .done
                .as_deref()
                .map(|value| value.eq_ignore_ascii_case("yes"))
                .unwrap_or(false)
        {
            state = SessionState::NeedsValidation;
        }
        let summary = envelope.summary.clone();
        return Some(NormalizedObservation {
            state,
            confidence: Confidence::High,
            summary,
            raw_excerpt: envelope.raw_excerpt,
            files: envelope.files,
            blocker: envelope.blocker,
            source: "status_envelope",
        });
    }

    let heuristic_window = heuristic_window(kind, raw_output);
    let lowered = heuristic_window.to_ascii_lowercase();
    if kind == AgentKind::Qwen
        && contains_any(&lowered, KEYWORDS_PROMPT_ECHO)
        && !contains_any(&lowered, KEYWORDS_QWEN_PROMPT_ECHO_NEG)
    {
        return None;
    }
    let summary = truncate_summary(heuristic_window);
    let qwen_has_real_signal =
        kind != AgentKind::Qwen || contains_any(&lowered, KEYWORDS_QWEN_SIGNAL);

    // CLI failure detection
    let failure_keywords = if kind == AgentKind::Qwen {
        KEYWORDS_CLI_FAILURE_QWEN
    } else {
        KEYWORDS_CLI_FAILURE
    };
    if contains_any(&lowered, failure_keywords) {
        return Some(NormalizedObservation {
            state: SessionState::Failed,
            confidence: if kind == AgentKind::Qwen {
                Confidence::High
            } else {
                Confidence::Medium
            },
            summary,
            raw_excerpt: truncate_summary(heuristic_window),
            files: Vec::new(),
            blocker: Some("cli failure".to_owned()),
            source: "keyword_cli_failure",
        });
    }

    // Validated
    let validated_keywords = if kind == AgentKind::Qwen {
        KEYWORDS_VALIDATED_QWEN
    } else {
        KEYWORDS_VALIDATED
    };
    if qwen_has_real_signal && contains_any(&lowered, validated_keywords) {
        return Some(NormalizedObservation {
            state: SessionState::Validated,
            confidence: Confidence::Medium,
            summary,
            raw_excerpt: truncate_summary(heuristic_window),
            files: Vec::new(),
            blocker: None,
            source: "keyword_validated",
        });
    }

    // Weak output
    if qwen_has_real_signal && contains_any(&lowered, KEYWORDS_WEAK_OUTPUT) {
        return Some(NormalizedObservation {
            state: SessionState::WeakOutput,
            confidence: Confidence::Medium,
            summary,
            raw_excerpt: truncate_summary(heuristic_window),
            files: Vec::new(),
            blocker: None,
            source: "keyword_weak_output",
        });
    }

    // Wrong direction
    if qwen_has_real_signal && contains_any(&lowered, KEYWORDS_WRONG_DIRECTION) {
        return Some(NormalizedObservation {
            state: SessionState::WrongDirection,
            confidence: Confidence::Medium,
            summary,
            raw_excerpt: truncate_summary(heuristic_window),
            files: Vec::new(),
            blocker: None,
            source: "keyword_wrong_direction",
        });
    }

    // Done claim
    if qwen_has_real_signal && detect_done_claim_impl(heuristic_window) {
        return Some(NormalizedObservation {
            state: SessionState::NeedsValidation,
            confidence: if kind == AgentKind::Qwen {
                Confidence::Low
            } else {
                Confidence::Medium
            },
            summary,
            raw_excerpt: truncate_summary(heuristic_window),
            files: Vec::new(),
            blocker: None,
            source: "keyword_done_claim",
        });
    }

    // Blocked
    if qwen_has_real_signal && contains_any(&lowered, KEYWORDS_BLOCKED) {
        return Some(NormalizedObservation {
            state: SessionState::Blocked,
            confidence: Confidence::Medium,
            summary: summary.clone(),
            raw_excerpt: truncate_summary(heuristic_window),
            files: Vec::new(),
            blocker: Some(summary.clone()),
            source: "keyword_blocked",
        });
    }

    // Contradiction
    if qwen_has_real_signal && contains_any(&lowered, KEYWORDS_CONTRADICTION) {
        return Some(NormalizedObservation {
            state: SessionState::Contradictory,
            confidence: Confidence::Medium,
            summary,
            raw_excerpt: truncate_summary(heuristic_window),
            files: Vec::new(),
            blocker: None,
            source: "keyword_contradiction",
        });
    }

    // Qwen tool progression
    if kind == AgentKind::Qwen && contains_any(&lowered, KEYWORDS_QWEN_TOOL_SIGNAL) {
        return Some(NormalizedObservation {
            state: SessionState::Progressing,
            confidence: Confidence::Medium,
            summary,
            raw_excerpt: truncate_summary(heuristic_window),
            files: Vec::new(),
            blocker: None,
            source: "qwen_tool_progress",
        });
    }

    // Progressing (generic)
    if contains_any(&lowered, KEYWORDS_PROGRESSING) {
        return Some(NormalizedObservation {
            state: SessionState::Progressing,
            confidence: Confidence::Low,
            summary,
            raw_excerpt: truncate_summary(heuristic_window),
            files: Vec::new(),
            blocker: None,
            source: "keyword_progress",
        });
    }

    None
}

fn extract_summary_impl(raw_output: &str) -> Option<String> {
    if let Some(envelope) = extract_status_envelope_impl(raw_output) {
        return Some(envelope.summary);
    }
    let summary = truncate_summary(raw_output);
    if summary.is_empty() {
        None
    } else {
        Some(summary)
    }
}

fn detect_done_claim_impl(raw_output: &str) -> bool {
    let lowered = raw_output.to_ascii_lowercase();
    contains_any(&lowered, KEYWORDS_DONE_CLAIM)
}

fn extract_supervisor_plan_impl(raw_output: &str) -> Option<MissionPlan> {
    let json = extract_tagged_block(
        raw_output,
        "BEGIN_SAPPHIRE_PLAN_JSON",
        "END_SAPPHIRE_PLAN_JSON",
    )
    .or_else(|| extract_fenced_json(raw_output))
    .or_else(|| extract_plan_like_json(raw_output))?;
    let envelope = serde_json::from_str::<SupervisorPlanEnvelope>(&json).ok()?;
    envelope.try_into_plan().ok().map(sanitize_supervisor_plan)
}

fn extract_supervisor_action_impl(raw_output: &str) -> Option<SupervisorAction> {
    let captures = supervisor_action_regex().captures_iter(raw_output).last()?;
    let action = captures.name("action")?.as_str().trim().to_owned();
    let target = captures.name("target")?.as_str().trim().to_owned();
    let summary = captures.name("summary")?.as_str().trim().to_owned();
    let message = captures.name("message")?.as_str().trim().to_owned();
    if action_looks_like_prompt_example(&action)
        || action_looks_like_prompt_example(&target)
        || action_looks_like_prompt_example(&summary)
        || action_looks_like_prompt_example(&message)
    {
        return None;
    }
    Some(SupervisorAction {
        action,
        target: none_if_literal(&target),
        summary,
        message: none_if_literal(&message),
    })
}

fn extract_final_envelope_impl(raw_output: &str) -> Option<FinalEnvelope> {
    let captures = final_envelope_regex().captures_iter(raw_output).last()?;
    let state = map_state_token(captures.name("state")?.as_str())?;
    Some(FinalEnvelope {
        state,
        summary: captures.name("summary")?.as_str().trim().to_owned(),
    })
}

fn extract_status_envelope_impl(raw_output: &str) -> Option<StatusEnvelope> {
    let captures = status_envelope_regex()
        .captures_iter(raw_output)
        .last()
        .or_else(|| {
            relaxed_status_envelope_regex()
                .captures_iter(raw_output)
                .last()
        })?;
    let files_text = captures.name("files")?.as_str().trim();
    Some(StatusEnvelope {
        state: {
            let value = captures.name("state")?.as_str().trim().to_owned();
            if action_looks_like_prompt_example(&value) {
                return None;
            }
            value
        },
        summary: {
            let value = captures.name("summary")?.as_str().trim().to_owned();
            if action_looks_like_prompt_example(&value) {
                return None;
            }
            value
        },
        files: split_csv_or_none(files_text),
        blocker: none_if_literal(captures.name("blocker")?.as_str().trim()),
        done: none_if_literal(captures.name("done")?.as_str().trim()),
        raw_excerpt: captures.get(0)?.as_str().trim().to_owned(),
    })
}

fn action_looks_like_prompt_example(value: &str) -> bool {
    let lowered = value.trim().to_ascii_lowercase();
    lowered.is_empty()
        || lowered == "..."
        || lowered.contains('|')
        || lowered.contains('<')
        || lowered.contains('>')
        || lowered.contains("or none")
        || lowered.contains("one short sentence")
        || lowered.contains("one short instruction")
        || lowered.contains("comma-separated paths")
}

fn status_envelope_contract() -> String {
    "STATE: progressing|blocked|done_claimed|needs_validation|validated|needs_retry|wrong_direction|failed\nSUMMARY: one short sentence\nFILES: comma-separated paths or NONE\nBLOCKER: one short sentence or NONE\nDONE: yes or no".to_owned()
}

fn map_state_token(token: &str) -> Option<SessionState> {
    match token.trim().to_ascii_lowercase().as_str() {
        "progressing" => Some(SessionState::Progressing),
        "blocked" => Some(SessionState::Blocked),
        "done_claimed" => Some(SessionState::DoneClaimed),
        "needs_validation" => Some(SessionState::NeedsValidation),
        "validated" | "pass" => Some(SessionState::Validated),
        "needs_retry" | "partial" => Some(SessionState::NeedsRetry),
        "wrong_direction" => Some(SessionState::WrongDirection),
        "failed" | "fail" => Some(SessionState::Failed),
        "stalled" => Some(SessionState::Stalled),
        _ => SessionState::from_directive(token),
    }
}

fn split_csv_or_none(value: &str) -> Vec<String> {
    if value.eq_ignore_ascii_case("none") || value.is_empty() {
        Vec::new()
    } else {
        value
            .split(',')
            .map(|part| part.trim().to_owned())
            .collect()
    }
}

fn none_if_literal(value: &str) -> Option<String> {
    if value.eq_ignore_ascii_case("none") || value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

pub(crate) fn extract_tagged_block(text: &str, begin: &str, end: &str) -> Option<String> {
    let start = text.rfind(begin)?;
    let rest = &text[start + begin.len()..];
    let finish = rest.find(end)?;
    Some(rest[..finish].trim().to_owned())
}

pub(crate) fn extract_fenced_json(text: &str) -> Option<String> {
    let captures = fenced_json_regex().captures_iter(text).last()?;
    Some(captures.name("json")?.as_str().trim().to_owned())
}

pub(crate) fn extract_plan_like_json(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut depth = 0_i32;
    let mut start = None;
    let mut in_string = false;
    let mut escaped = false;

    for (index, byte) in bytes.iter().enumerate() {
        let ch = *byte as char;
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(index);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    if let Some(begin) = start {
                        let candidate = &text[begin..=index];
                        if candidate.contains("\"mission_rewrite\"")
                            && candidate.contains("\"worker_packets\"")
                            && candidate.contains("\"workstreams\"")
                        {
                            return Some(candidate.to_owned());
                        }
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }
    None
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn heuristic_window(kind: AgentKind, raw_output: &str) -> &str {
    let start = raw_output
        .char_indices()
        .rev()
        .nth(4000)
        .map(|(idx, _)| idx)
        .unwrap_or(0);
    let tail = &raw_output[start..];
    if kind != AgentKind::Qwen {
        return tail;
    }
    let markers = [
        "Model:",
        "Success:",
        "ReadFile",
        "WriteFile",
        "EditFile",
        "Execute",
    ];
    let mut best = 0;
    for marker in markers {
        if let Some(index) = tail.rfind(marker) {
            best = best.max(index);
        }
    }
    let trimmed = &tail[best..];
    strip_echoed_watchdog_prompts(trimmed)
}

fn strip_echoed_watchdog_prompts(text: &str) -> &str {
    let markers = [
        "\nStatus update only.",
        "\nSupervisor action only.",
        "\nYour last reply was not usable:",
    ];
    let mut cutoff = text.len();
    for marker in markers {
        if let Some(index) = text.find(marker) {
            cutoff = cutoff.min(index);
        }
    }
    &text[..cutoff]
}

fn truncate_summary(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = compact.chars().take(200).collect::<String>();
    if compact.chars().count() > 200 {
        out.push_str("...");
    }
    out
}

fn status_envelope_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?ms)STATE:\s*(?P<state>[^\n]+)\nSUMMARY:\s*(?P<summary>[^\n]+)\nFILES:\s*(?P<files>[^\n]+)\nBLOCKER:\s*(?P<blocker>[^\n]+)\nDONE:\s*(?P<done>[^\n]+)",
        )
        .expect("valid status envelope regex")
    })
}

fn relaxed_status_envelope_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?ms)STATE:\s*(?P<state>[A-Za-z_]+)\s+SUMMARY:\s*(?P<summary>.+?)\s+FILES:\s*(?P<files>.+?)\s+BLOCKER:\s*(?P<blocker>.+?)\s+DONE:\s*(?P<done>yes|no|[A-Za-z_]+)",
        )
        .expect("valid relaxed status envelope regex")
    })
}

fn supervisor_action_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?ms)ACTION:\s*(?P<action>[^\n]+)\nTARGET:\s*(?P<target>[^\n]+)\nSUMMARY:\s*(?P<summary>[^\n]+)\nMESSAGE:\s*(?P<message>[^\n]+)",
        )
        .expect("valid supervisor action regex")
    })
}

fn final_envelope_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?ms)FINAL_STATE:\s*(?P<state>[^\n]+)\nFINAL_SUMMARY:\s*(?P<summary>[^\n]+)")
            .expect("valid final envelope regex")
    })
}

fn fenced_json_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?ms)```(?:json)?\s*(?P<json>\{.*\})\s*```").expect("valid fenced json regex")
    })
}

#[derive(Debug, Deserialize)]
struct SupervisorPlanEnvelope {
    mission_rewrite: String,
    workstreams: Vec<SupervisorWorkstream>,
    risk_map: Vec<RiskItem>,
    worker_packets: Vec<SupervisorWorkerPacket>,
    supervision_strategy: String,
}

/// Lenient version of WorkerPacket for supervisor plan parsing.
/// Qwen sometimes puts strings where arrays are expected.
/// Now includes role_type (stable machine key) and display_name (runtime label).
#[derive(Debug, Deserialize)]
struct SupervisorWorkerPacket {
    worker_id: String,
    /// Human-readable role title from supervisor, e.g. "Software Engineer".
    #[serde(default)]
    role: String,
    /// Stable machine key for template lookup, e.g. "software-engineer".
    #[serde(default)]
    role_type: String,
    /// Runtime team label, e.g. "Engineer-1", "Designer-2".
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    starting_angle: String,
    owned_scope: String,
    explicit_task: String,
    out_of_scope: String,
    #[serde(deserialize_with = "deserialize_vec_or_string")]
    definition_of_done: Vec<String>,
    #[serde(deserialize_with = "deserialize_vec_or_string")]
    required_evidence: Vec<String>,
    blocker_protocol: String,
    conflict_warning: String,
    #[serde(deserialize_with = "deserialize_vec_or_string", default)]
    communication_rules: Vec<String>,
    #[serde(deserialize_with = "deserialize_vec_or_string", default)]
    validation_standard: Vec<String>,
    #[serde(deserialize_with = "deserialize_vec_or_string", default)]
    expected_output_format: Vec<String>,
}

fn deserialize_vec_or_string<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};

    struct VecOrString;

    impl<'de2> Visitor<'de2> for VecOrString {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string or array of strings")
        }

        fn visit_str<E>(self, value: &str) -> Result<Vec<String>, E> {
            // Split string by newlines, commas, or semicolons
            Ok(value
                .split(|c| matches!(c, '\n' | ',' | ';'))
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect())
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Vec<String>, A::Error>
        where
            A: de::SeqAccess<'de2>,
        {
            let mut vec = Vec::new();
            while let Some(elem) = seq.next_element::<String>()? {
                vec.push(elem);
            }
            Ok(vec)
        }
    }

    deserializer.deserialize_any(VecOrString)
}

impl SupervisorWorkerPacket {
    fn into_packet(self, ordinal: usize) -> WorkerPacket {
        // Derive role_type: prefer explicit role_type, fall back to inferring from role.
        let role_type = if !self.role_type.is_empty() {
            self.role_type.clone()
        } else if !self.role.is_empty() {
            // Infer from role name: "Software Engineer" -> "software-engineer"
            infer_role_type(&self.role)
        } else {
            "software-engineer".to_owned()
        };

        // Derive display_name: prefer explicit display_name, otherwise generate from role_type.
        let display_name = if !self.display_name.is_empty() {
            self.display_name.clone()
        } else {
            generate_display_name(&role_type, ordinal)
        };

        // Derive role from role_type for human readability.
        let role = if !self.role.is_empty() {
            self.role.clone()
        } else {
            role_type_to_title(&role_type)
        };

        WorkerPacket {
            worker_id: self.worker_id,
            role_type,
            display_name,
            role,
            starting_angle: self.starting_angle,
            owned_scope: self.owned_scope,
            explicit_task: self.explicit_task,
            out_of_scope: self.out_of_scope,
            definition_of_done: self.definition_of_done,
            required_evidence: self.required_evidence,
            blocker_protocol: self.blocker_protocol,
            conflict_warning: self.conflict_warning,
            communication_rules: self.communication_rules,
            validation_standard: self.validation_standard,
            expected_output_format: self.expected_output_format,
        }
    }
}

/// Infer role_type from a human-readable role name.
fn infer_role_type(role: &str) -> String {
    let r = role.to_ascii_lowercase();
    if r.contains("software") || r.contains("swe") { "software-engineer".to_owned() }
    else if r.contains("research") { "research-engineer".to_owned() }
    else if r.contains("validation") || r.contains("validat") { "validation-engineer".to_owned() }
    else if r.contains("architect") { "architecture-engineer".to_owned() }
    else if r.contains("security") { "security-engineer".to_owned() }
    else if r.contains("debug") || r.contains("review") { "debug-and-review-engineer".to_owned() }
    else if r.contains("test") || r.contains("automation") { "testing-and-automation-engineer".to_owned() }
    else if r.contains("design") { "designer-engineer".to_owned() }
    else if r.contains("revenue") || r.contains("go-to-market") || r.contains("go to market") || r.contains("gtm") || r.contains("account executive") { "revenue-engineer".to_owned() }
    else if r.contains("product manager") || r == "pm" || r.contains("product management") { "product-manager".to_owned() }
    else if r.contains("sales") { "sales-engineer".to_owned() }
    else if r.contains("solution") { "solutions-engineer".to_owned() }
    else if r.contains("customer") || r.contains("success") { "customer-success-engineer".to_owned() }
    else if r.contains("product") { "product-engineer".to_owned() }
    else if r.contains("compliance") { "compliance-engineer".to_owned() }
    else { "software-engineer".to_owned() }
}

/// Convert role_type to human-readable title.
fn role_type_to_title(role_type: &str) -> String {
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
        _ => role_type.replace('-', " ").split_whitespace().map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            }
        }).collect::<Vec<_>>().join(" "),
    }
}

/// Generate display_name from role_type and ordinal (1-based).
/// e.g. ("software-engineer", 1) -> "Engineer-1"
///      ("designer-engineer", 2) -> "Designer-2"
fn generate_display_name(role_type: &str, ordinal: usize) -> String {
    let short = match role_type {
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
    format!("{short}-{ordinal}")
}

#[derive(Debug, Deserialize)]
struct SupervisorWorkstream {
    id: String,
    name: String,
    execution: String,
    owned_scope: String,
    success_criteria: Vec<String>,
    #[serde(default)]
    depends_on: Vec<String>,
}

impl SupervisorPlanEnvelope {
    fn try_into_plan(self) -> Result<MissionPlan> {
        let workstreams = self
            .workstreams
            .into_iter()
            .map(|workstream| {
                Ok(Workstream {
                    id: workstream.id,
                    name: workstream.name,
                    execution: parse_execution(&workstream.execution)?,
                    owned_scope: workstream.owned_scope,
                    success_criteria: workstream.success_criteria,
                    depends_on: workstream.depends_on,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let worker_packets = self
            .worker_packets
            .into_iter()
            .enumerate()
            .map(|(i, p)| p.into_packet(i + 1))
            .collect();

        Ok(MissionPlan {
            mission_rewrite: self.mission_rewrite,
            workstreams,
            risk_map: self.risk_map,
            worker_packets,
            supervision_strategy: self.supervision_strategy,
        })
    }
}

fn parse_execution(value: &str) -> Result<WorkstreamExecution> {
    match value.trim().to_ascii_lowercase().as_str() {
        "parallel" => Ok(WorkstreamExecution::Parallel),
        "dependent" => Ok(WorkstreamExecution::Dependent),
        "validation" => Ok(WorkstreamExecution::Validation),
        "integration" => Ok(WorkstreamExecution::Integration),
        // Qwen sometimes invents values like "baseline", "review", "audit", etc.
        // Default to Parallel for any unknown value.
        _ => Ok(WorkstreamExecution::Parallel),
    }
}

fn sanitize_supervisor_plan(mut plan: MissionPlan) -> MissionPlan {
    plan.mission_rewrite = sanitize_plan_text(&plan.mission_rewrite);
    plan.supervision_strategy = sanitize_plan_text(&plan.supervision_strategy);
    for workstream in &mut plan.workstreams {
        workstream.name = sanitize_plan_text(&workstream.name);
        workstream.owned_scope = sanitize_plan_text(&workstream.owned_scope);
        workstream.success_criteria = workstream
            .success_criteria
            .iter()
            .map(|item| sanitize_plan_text(item))
            .collect();
        workstream.depends_on = workstream
            .depends_on
            .iter()
            .map(|item| sanitize_plan_text(item))
            .collect();
    }
    for risk in &mut plan.risk_map {
        risk.zone = sanitize_plan_text(&risk.zone);
        risk.risk = sanitize_plan_text(&risk.risk);
        risk.mitigation = sanitize_plan_text(&risk.mitigation);
    }
    for packet in &mut plan.worker_packets {
        packet.role = sanitize_plan_text(&packet.role);
        packet.owned_scope = sanitize_plan_text(&packet.owned_scope);
        packet.explicit_task = sanitize_plan_text(&packet.explicit_task);
        packet.out_of_scope = sanitize_plan_text(&packet.out_of_scope);
        packet.definition_of_done = packet
            .definition_of_done
            .iter()
            .map(|item| sanitize_plan_text(item))
            .collect();
        packet.required_evidence = packet
            .required_evidence
            .iter()
            .map(|item| sanitize_plan_text(item))
            .collect();
        packet.blocker_protocol = sanitize_plan_text(&packet.blocker_protocol);
        packet.conflict_warning = sanitize_plan_text(&packet.conflict_warning);
        packet.expected_output_format = packet
            .expected_output_format
            .iter()
            .map(|item| sanitize_plan_text(item))
            .collect();
    }
    plan
}

fn sanitize_plan_text(text: &str) -> String {
    text.replace("supervisor-runtime/", "")
        .replace("supervisor-runtime folder", "repo root")
        .replace("supervisor-runtime", "repo root")
        .replace(".sp/", "")
        .replace(".sp", "repo root")
        .replace("../tasks.txt", "tasks.txt")
        .replace("../", "")
}

#[cfg(test)]
mod tests {
    use super::{
        adapter_for, extract_final_envelope_impl, extract_status_envelope_impl,
        extract_supervisor_action_impl, extract_supervisor_plan_impl,
    };
    use crate::agent::AgentKind;
    use crate::model::WorkerPacket;
    use crate::templates::PromptLibrary;

    #[test]
    fn parses_status_envelope() {
        let raw = "noise\nSTATE: progressing\nSUMMARY: tracing root cause\nFILES: src/main.rs, src/lib.rs\nBLOCKER: NONE\nDONE: no\n";
        let envelope = extract_status_envelope_impl(raw).expect("envelope");
        assert_eq!(envelope.summary, "tracing root cause");
        assert_eq!(envelope.files.len(), 2);
    }

    #[test]
    fn parses_noisy_screen_reader_status_envelope() {
        let raw = concat!(
            "YOLO mode\n",
            "User:  Status update only.\n",
            "STATE: progressing|blocked|done_claimed|needs_validation|validated|needs_retry|wrong_direction|failed\n",
            "SUMMARY: one short sentence\n",
            "FILES: comma-separated paths or NONE\n",
            "BLOCKER: one short sentence or NONE\n",
            "DONE: yes or no\n",
            "Model:  STATE: progressing\n",
            "SUMMARY: Preflight check initiated for CLI health verification\n",
            "FILES: NONE\n",
            "BLOCKER: NONE\n",
            "DONE: no\n",
        );
        let envelope = extract_status_envelope_impl(raw).expect("noisy envelope");
        assert_eq!(envelope.state, "progressing");
        assert_eq!(
            envelope.summary,
            "Preflight check initiated for CLI health verification"
        );
    }

    #[test]
    fn parses_supervisor_action_envelope() {
        let raw = "ACTION: retry_worker\nTARGET: Engineer-2\nSUMMARY: evidence is weak\nMESSAGE: rerun tests and report exact output\n";
        let action = extract_supervisor_action_impl(raw).expect("action");
        assert_eq!(action.action, "retry_worker");
        assert_eq!(action.target.as_deref(), Some("Engineer-2"));
    }

    #[test]
    fn parses_final_envelope() {
        let raw = "FINAL_STATE: validated\nFINAL_SUMMARY: ship it";
        let final_state = extract_final_envelope_impl(raw).expect("final");
        assert_eq!(final_state.summary, "ship it");
    }

    #[test]
    fn parses_supervisor_plan_json_block() {
        let raw = concat!(
            "BEGIN_SAPPHIRE_PLAN_JSON\n",
            "{\"mission_rewrite\":\"tight mission\",\"workstreams\":[{\"id\":\"baseline\",\"name\":\"Baseline\",\"execution\":\"parallel\",\"owned_scope\":\"inspect repo\",\"success_criteria\":[\"facts gathered\"],\"depends_on\":[]}],\"risk_map\":[{\"zone\":\"repo\",\"risk\":\"unknown state\",\"mitigation\":\"inspect first\"}],\"worker_packets\":[{\"worker_id\":\"Engineer-1\",\"role\":\"Software Engineer\",\"role_type\":\"software-engineer\",\"display_name\":\"Engineer-1\",\"owned_scope\":\"inspect repo\",\"explicit_task\":\"inspect repo\",\"out_of_scope\":\"do not edit\",\"definition_of_done\":[\"facts gathered\"],\"required_evidence\":[\"files and commands\"],\"blocker_protocol\":\"escalate blockers\",\"conflict_warning\":\"avoid overlap\",\"expected_output_format\":[\"Summary\"]}],\"supervision_strategy\":\"tight control\"}\n",
            "END_SAPPHIRE_PLAN_JSON\n"
        );
        let plan = extract_supervisor_plan_impl(raw).expect("plan");
        assert_eq!(plan.mission_rewrite, "tight mission");
        assert_eq!(plan.worker_packets.len(), 1);
        assert_eq!(plan.worker_packets[0].display_name, "Engineer-1");
        assert_eq!(plan.worker_packets[0].role_type, "software-engineer");
    }

    #[test]
    fn qwen_assignment_prompt_uses_full_role_template() {
        let prompts = PromptLibrary::load();
        let adapter = adapter_for(AgentKind::Qwen);
        let packet = WorkerPacket {
            worker_id: "Engineer-1".to_owned(),
            role_type: "software-engineer".to_owned(),
            display_name: "Engineer-1".to_owned(),
            role: "Software Engineer".to_owned(),
            starting_angle: "primary path".to_owned(),
            owned_scope: "src".to_owned(),
            explicit_task: "Implement the fix".to_owned(),
            out_of_scope: "Do not rewrite the architecture".to_owned(),
            definition_of_done: vec!["Fix is implemented".to_owned()],
            required_evidence: vec!["Show touched files".to_owned()],
            blocker_protocol: "Report blockers immediately".to_owned(),
            conflict_warning: "Claim files first".to_owned(),
            communication_rules: vec!["Mail blockers".to_owned()],
            validation_standard: vec!["Pass checks".to_owned()],
            expected_output_format: vec!["Summary".to_owned()],
        };

        let prompt =
            adapter.build_assignment_prompt(&prompts, "Fix the launch flow.", &packet);

        assert!(prompt.contains("# Software Engineer"));
        assert!(prompt.contains("## Mission"));
        assert!(prompt.contains("## Core Responsibilities"));
        assert!(prompt.contains("Role Type: software-engineer"));
        assert!(prompt.contains("## Sapphire Control Protocol"));
        assert!(prompt.lines().count() <= 85);
    }
}
