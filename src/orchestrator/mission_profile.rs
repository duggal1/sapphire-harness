use crate::cli::LaunchConfig;
use crate::model::{MissionPlan, RiskItem, WorkerPacket, Workstream, WorkstreamExecution};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissionProfile {
    pub coordination_focused: bool,
    pub deterministic_planning: bool,
    pub lean_supervision: bool,
    pub enable_repair_supervisor: bool,
    pub enable_supervisor_health_recovery: bool,
    pub enable_state_cards: bool,
    pub enable_protocol_reminders: bool,
    pub enable_health_probes: bool,
}

impl MissionProfile {
    pub fn from_launch(config: &LaunchConfig) -> Self {
        let lowered = config.mission.to_ascii_lowercase();
        let word_count = lowered.split_whitespace().count();
        let coordination_focused = contains_any(
            &lowered,
            &[
                "teammate",
                "teammate",
                "coordinate",
                "coordination",
                "work together",
                "collaborate",
                "talk to your teammate",
                "prove",
                "mail",
                "agent-to-agent",
                "inter-agent",
            ],
        );
        let code_heavy = contains_any(
            &lowered,
            &[
                "fix",
                "bug",
                "implement",
                "write code",
                "edit",
                "refactor",
                "compile",
                "cargo",
                "build",
                "test",
                "ui",
                "ux",
                "render",
                "style",
                "theme",
                "file",
                "repo",
                "rust",
            ],
        );
        let trivial_coordination = coordination_focused && !code_heavy;
        let lean_supervision = config.worker_count <= 3
            && word_count <= 60
            && (trivial_coordination || !code_heavy);
        let deterministic_planning = lean_supervision;

        Self {
            coordination_focused,
            deterministic_planning,
            lean_supervision,
            enable_repair_supervisor: !lean_supervision && config.worker_count >= 4,
            enable_supervisor_health_recovery: !lean_supervision,
            enable_state_cards: !lean_supervision && config.worker_count >= 4,
            enable_protocol_reminders: !lean_supervision,
            enable_health_probes: !lean_supervision,
        }
    }
}

pub fn deterministic_plan_for_mission(
    mission: &str,
    worker_count: usize,
    profile: MissionProfile,
) -> MissionPlan {
    if profile.coordination_focused && worker_count >= 2 {
        return coordination_plan(mission, worker_count);
    }
    general_plan(mission, worker_count, profile)
}

fn coordination_plan(mission: &str, worker_count: usize) -> MissionPlan {
    let worker_count = worker_count.max(1);
    let mission_rewrite = truncate(mission.trim(), 140);
    let workstreams = (1..=worker_count)
        .map(|ordinal| Workstream {
            id: format!("coord-{ordinal}"),
            name: match ordinal {
                1 => "Coordination bootstrap".to_owned(),
                2 => "Peer response and challenge".to_owned(),
                3 => "Coordination verification".to_owned(),
                _ => format!("Supporting lane {ordinal}"),
            },
            execution: WorkstreamExecution::Parallel,
            owned_scope: "Teammate coordination, shared plan quality, and proof of collaboration."
                .to_owned(),
            success_criteria: vec![
                "Use real Sapphire mail between workers.".to_owned(),
                "Keep the task narrow and coordination-first.".to_owned(),
                "Report only material blockers or proof.".to_owned(),
            ],
            depends_on: Vec::new(),
        })
        .collect();

    let worker_packets = (1..=worker_count)
        .map(|ordinal| coordination_packet(mission, ordinal))
        .collect();

    MissionPlan {
        mission_rewrite,
        workstreams,
        risk_map: vec![
            RiskItem {
                zone: "Supervision".to_owned(),
                risk: "Supervisor over-directs a simple teammate exercise.".to_owned(),
                mitigation: "Use lean supervision and let workers coordinate directly first."
                    .to_owned(),
            },
            RiskItem {
                zone: "Coordination".to_owned(),
                risk: "Workers narrate collaboration without actually using mail.".to_owned(),
                mitigation: "Require one real mail thread with ack or reply evidence.".to_owned(),
            },
        ],
        worker_packets,
        supervision_strategy:
            "Lean supervision. Workers coordinate directly. Supervisor intervenes only for real blockers, contradictions, or final validation."
                .to_owned(),
    }
}

fn coordination_packet(mission: &str, ordinal: usize) -> WorkerPacket {
    let (role_type, role, display_name) = match ordinal {
        3 => (
            "validation-engineer".to_owned(),
            "Validation Engineer".to_owned(),
            "Validator-1".to_owned(),
        ),
        _ => (
            "software-engineer".to_owned(),
            "Software Engineer".to_owned(),
            format!("Engineer-{ordinal}"),
        ),
    };
    let explicit_task = match ordinal {
        1 => format!(
            "Open the first teammate thread, propose the execution plan, and drive the main path for: {}",
            truncate(mission.trim(), 160)
        ),
        2 => {
            let mission_snippet = truncate(mission.trim(), 160);
            format!(
                "Challenge the main path for: {mission_snippet}. Find at least two concrete weaknesses. Mail Engineer-1 your review. Do not write code — review and validate only."
            )
        }
        3 => {
            let mission_snippet = truncate(mission.trim(), 160);
            format!(
                "Verify coordination evidence for: {mission_snippet}. Check that Engineer-1 and Engineer-2 actually used Sapphire mail with real ack replies. Report if coordination was real or theater."
            )
        }
        _ => format!(
            "Take a narrow supporting slice of the shared plan for: {}",
            truncate(mission.trim(), 160)
        ),
    };

    WorkerPacket {
        worker_id: display_name.clone(),
        role_type,
        display_name: display_name.clone(),
        role,
        starting_angle: match ordinal {
            1 => "Initiate direct teammate coordination and keep the plan concrete.".to_owned(),
            2 => "Act as the peer responder who turns coordination into proof.".to_owned(),
            3 => "Act as the verifier who checks whether the coordination was real.".to_owned(),
            _ => "Support the shared plan with one narrow parallel pass.".to_owned(),
        },
        owned_scope: "Teammate coordination, evidence of collaboration, and the shared execution plan."
            .to_owned(),
        explicit_task,
        out_of_scope:
            "Do not invent code work, rewrite the mission, or spam repeated supervisor prompts."
                .to_owned(),
        definition_of_done: vec![
            "Show at least one real teammate mail thread.".to_owned(),
            "Produce a shared plan or a concrete blocker.".to_owned(),
            "Keep status updates factual and minimal.".to_owned(),
        ],
        required_evidence: vec![
            "Reference the teammate mail thread or ack.".to_owned(),
            "Summarize what the teammate proved or challenged.".to_owned(),
            "State the resulting plan or blocker.".to_owned(),
        ],
        blocker_protocol:
            "Mail the teammate first for dependencies, clarification, or proof requests. Escalate only after failed peer coordination."
                .to_owned(),
        conflict_warning:
            "Do not duplicate the same ask repeatedly. One clear thread beats repeated prompts."
                .to_owned(),
        communication_rules: vec![
            "Send at least one concrete SAPPHIRE_MAIL to a teammate before escalating."
                .to_owned(),
            "Acknowledge teammate mail explicitly and reply with a concrete next step."
                .to_owned(),
            "Use the supervisor only for rulings, contradictions, or failed teammate coordination."
                .to_owned(),
        ],
        validation_standard: vec![
            "Do not claim coordination unless the mail trail exists.".to_owned(),
            "Do not call a teammate stalled while they are visibly active.".to_owned(),
        ],
        expected_output_format: vec![
            "STATE".to_owned(),
            "SUMMARY".to_owned(),
            "FILES".to_owned(),
            "BLOCKER".to_owned(),
            "DONE".to_owned(),
        ],
    }
}

fn general_plan(mission: &str, worker_count: usize, profile: MissionProfile) -> MissionPlan {
    let worker_count = worker_count.max(1);
    let lowered = mission.to_ascii_lowercase();
    let role_type = deterministic_role_type(&lowered);
    let role_title = deterministic_role_title(&role_type);
    let mission_rewrite = truncate(mission.trim(), 140);

    let workstreams = (1..=worker_count)
        .map(|ordinal| Workstream {
            id: format!("ws-{ordinal}"),
            name: match ordinal {
                1 => format!("{} — primary path", deterministic_workstream_name(&lowered)),
                2 => format!("{} — independent verification", deterministic_workstream_name(&lowered)),
                3 => format!("{} — failure path review", deterministic_workstream_name(&lowered)),
                4 => format!("{} — integration review", deterministic_workstream_name(&lowered)),
                _ => format!("{} — lane {ordinal}", deterministic_workstream_name(&lowered)),
            },
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
                out_of_scope:
                    "Do not broaden scope, rewrite unrelated code, or duplicate another lane."
                        .to_owned(),
                definition_of_done: vec![
                    "Deliver the scoped result only.".to_owned(),
                    "Show proof that the assigned lane was completed.".to_owned(),
                    "Use Sapphire status updates so the supervisor can validate the result."
                        .to_owned(),
                ],
                required_evidence: vec![
                    "List touched files.".to_owned(),
                    "List validation commands or observations.".to_owned(),
                    "State what was proven or what blocked completion.".to_owned(),
                ],
                blocker_protocol:
                    "If blocked, report the blocker immediately with a machine-readable Sapphire status update."
                        .to_owned(),
                conflict_warning:
                    "Claim files before editing and avoid overlap with other workers.".to_owned(),
                communication_rules: vec![
                    "Mail teammates for real dependencies or reviews before escalating."
                        .to_owned(),
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
                risk: "Supervisor planner unavailable or overcomplicated for the task."
                    .to_owned(),
                mitigation: "Use deterministic planning and keep worker scope narrow."
                    .to_owned(),
            },
            RiskItem {
                zone: "Coordination".to_owned(),
                risk: "Workers may overlap or drift when planning is degraded.".to_owned(),
                mitigation:
                    "Use explicit lane ownership, teammate-first coordination, and lease discipline."
                        .to_owned(),
            },
        ],
        worker_packets,
        supervision_strategy: if profile.lean_supervision {
            "Lean supervision. Keep the mission narrow, minimize interventions, and prefer direct worker coordination."
                .to_owned()
        } else {
            "Deterministic planning fallback. Keep scope tight, validate every claim, and prefer evidence over narration."
                .to_owned()
        },
    }
}

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

fn deterministic_owned_scope(mission: &str) -> String {
    if mission.contains("tmux") || mission.contains("terminal") || mission.contains("orchestration")
    {
        "Sapphire runtime, tmux surface, worker coordination, and mission evidence."
            .to_owned()
    } else if mission.contains("ui") || mission.contains("ux") || mission.contains("design") {
        "UI surfaces, rendering paths, and interaction flow.".to_owned()
    } else {
        "Requested mission scope only, constrained to the repo root and directly relevant files."
            .to_owned()
    }
}

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
    } else if mission.contains("security") || mission.contains("vuln") {
        format!("Attack surface audit via {lane}.")
    } else if mission.contains("ui") || mission.contains("design") {
        format!("Visual implementation via {lane}.")
    } else if mission.contains("test") || mission.contains("debug") {
        format!("Bug investigation via {lane}.")
    } else {
        format!("Execute the scoped task via {lane}.")
    }
}

fn deterministic_explicit_task(mission: &str, ordinal: usize) -> String {
    let mission_snippet = truncate(mission.trim(), 160);
    match ordinal {
        1 => format!("Own the main delivery path for: {mission_snippet}. Produce the primary implementation. Report concrete progress with files touched."),
        2 => format!("Independent parallel pass for: {mission_snippet}. Take a different starting angle than Engineer-1. Find gaps the main path missed."),
        3 => format!("Inspect failure paths and missing evidence for: {mission_snippet}. Focus on what could go wrong, edge cases, and untested assumptions."),
        4 => format!("Integration review for: {mission_snippet}. Verify that lanes 1-3 produce compatible results. Identify integration gaps."),
        _ => format!("Narrow scoped pass for: {mission_snippet}. Stay inside your lane. Do not duplicate work from lanes 1-4."),
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
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
