use std::path::Path;

use crate::model::WorkerPacket;

use super::communication_policy;
use super::coordination;
use super::launch_prompt::preferred_state_roots;

pub fn render_supervisor_bootstrap(
    base_prompt: String,
    agents_path: &Path,
    agents_existed: bool,
    requested_workers: usize,
) -> String {
    format!(
        "{base_prompt}\n\n---\n\n# AGENTS.md PROTOCOL\n\n- AGENTS path: {path}\n- AGENTS status at session start: {status}\n- Total worker terminals requested: {requested_workers}\n- Worker packet count must equal {requested_workers}. Do not invent extra workers, stewards, alternates, or helper terminals.\n- Every newly launched worker must read AGENTS.md on first initialization only, before real work.\n- AGENTS.md is repository guidance, not an implicit reserved worker role.\n- If AGENTS.md must be created or refreshed, use Sapphire's embedded `agents.md-instructions.md` source. Do not dump that source back into the session unless the refresh is actually required.\n\n# SUPERVISOR TEAM RULES\n\n- Git push is operator-only. Workers and supervisors must never run `git push`. The human push path is `sp push`.\n- Workers and supervisors must never run `git restore` or `git reset`.\n- Treat a dirty git tree as normal multi-agent reality. Multiple workers commit in parallel. Do not trigger cleanup theater because `git status` is noisy.\n- If a worker reports a dirty tree, reroute it back to owned scope unless `git status --porcelain` itself fails.\n- Prefer direct worker-to-worker coordination over supervisor chatter when a peer can answer faster.\n- Demand concrete `SAPPHIRE_MAIL` threads for dependencies, reviews, contract questions, handoffs, and unblock requests.\n- Do not classify active streaming workers as stalled based on guesswork. Prefer fresh status files, recent output, and real teammate mail evidence.\n- When workers report overlap, preserve newer teammate work, settle ownership explicitly, and forbid cleanup theater. Resolve the collision; do not let both workers rewrite the same file blind.\n- Approve cleanup only after every worker is resolved and no more reroute, retry, or proof challenge is needed.\n{enforcement_rules}\n",
        path = agents_path.display(),
        status = if agents_existed {
            "already present"
        } else {
            "created before worker launch"
        },
        enforcement_rules = communication_policy::supervisor_enforcement_rules(),
    )
}

pub fn render_worker_bootstrap(
    base_prompt: String,
    agents_path: &Path,
    state_dir: &Path,
    packet: &WorkerPacket,
    _git_remote: Option<&str>,
    memory_block: Option<&str>,
) -> String {
    let display_name = &packet.display_name;
    let (primary_root, secondary_root) = preferred_state_roots(state_dir);
    let prompt_path = primary_root.join("prompts").join(format!("{display_name}.md"));
    let secondary_prompt_path = secondary_root.join("prompts").join(format!("{display_name}.md"));
    let status_path = primary_root
        .join("workers")
        .join(display_name)
        .join("status.json");
    let secondary_status_path = secondary_root
        .join("workers")
        .join(display_name)
        .join("status.json");
    let memory_path = primary_root
        .join("workers")
        .join(display_name)
        .join("memory.json");
    let secondary_memory_path = secondary_root
        .join("workers")
        .join(display_name)
        .join("memory.json");
    let preferred_counterparts = coordination::preferred_counterparts(&packet.role_type).join(", ");

    // Inject persistent memory block if available — this is what makes agents
    // remember what they did last session, last mission. No amnesia on reboot.
    let memory_section = memory_block
        .map(|m| format!("{m}\n\n"))
        .unwrap_or_default();

    format!(
        "File paths for this worker:\n- Role assignment: {prompt_path} (fallback: {secondary_prompt_path})\n- Status file: {status_path} (fallback: {secondary_status_path})\n- Memory file: {memory_path} (fallback: {secondary_memory_path})\n- AGENTS.md: {agents}\n\nPreferred coordination lanes: {preferred_counterparts}.\n\n{memory_section}{base_prompt}",
        agents = agents_path.display(),
        prompt_path = prompt_path.display(),
        secondary_prompt_path = secondary_prompt_path.display(),
        status_path = status_path.display(),
        secondary_status_path = secondary_status_path.display(),
        memory_path = memory_path.display(),
        secondary_memory_path = secondary_memory_path.display(),
        preferred_counterparts = preferred_counterparts,
    )
}
