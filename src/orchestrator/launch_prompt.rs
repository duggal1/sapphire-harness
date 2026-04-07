use std::path::{Path, PathBuf};

use crate::agent::AgentKind;
use crate::model::WorkerPacket;

pub fn prompt_file_path(state_dir: &Path, session_name: &str) -> PathBuf {
    state_dir.join("prompts").join(format!("{session_name}.md"))
}

fn hidden_state_dir(state_dir: &Path) -> PathBuf {
    let parent = state_dir.parent().unwrap_or_else(|| Path::new("."));
    match state_dir.file_name().and_then(|name| name.to_str()) {
        Some(".sp") => parent.join(".hide.sp"),
        Some(name) => parent.join(format!(".hide.{name}")),
        None => parent.join(".hide.sp"),
    }
}

pub fn preferred_state_roots(state_dir: &Path) -> (PathBuf, PathBuf) {
    let hidden_root = hidden_state_dir(state_dir);
    let prefers_hidden = state_dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'));
    if prefers_hidden {
        (hidden_root, state_dir.to_path_buf())
    } else {
        (state_dir.to_path_buf(), hidden_root)
    }
}

pub fn worker_terminal_prompt(
    state_dir: &Path,
    _prompt_path: &Path,
    mission: &str,
    packet: &WorkerPacket,
) -> String {
    let (primary_root, secondary_root) = preferred_state_roots(state_dir);
    let prompt_path = primary_root
        .join("prompts")
        .join(format!("{}.md", packet.display_name));
    let secondary_prompt_path = secondary_root
        .join("prompts")
        .join(format!("{}.md", packet.display_name));
    let status_path = primary_root
        .join("workers")
        .join(&packet.display_name)
        .join("status.json");
    let secondary_status_path = secondary_root
        .join("workers")
        .join(&packet.display_name)
        .join("status.json");
    // Short injection prompt — ONLY file paths and mandatory first action.
    // Role brief, task, git rules, coordination rules, report-back contract
    // are ALL in the prompt file. Do NOT repeat them here.
    let lines = vec![
        format!("You are {} ({}) for this mission.", packet.display_name, packet.role),
        format!("Full assignment source of truth: {}", prompt_path.display()),
        format!(
            "Fallback assignment path if the first path is unavailable: {}",
            secondary_prompt_path.display()
        ),
        format!("Mission: {mission}"),
        "- Read that file once now. It contains your full role description, task, and rules.".to_owned(),
        "- Do not restate that file back to me.".to_owned(),
        "- Do not ask what to do if the file exists.".to_owned(),
        String::new(),
        "Critical team awareness:".to_owned(),
        "- You are NOT alone. Teammates (Engineers, Designers, Security, Architects) edit the same repo concurrently.".to_owned(),
        "- If you see file changes you didn't make, that is NORMAL and EXPECTED. Do NOT panic, revert, or delete.".to_owned(),
        "- Iterate over teammate changes. Merge useful work. Adapt your implementation on top.".to_owned(),
        "- Never run `git push`, `git restore`, or `git reset`. If using worktrees, stay inside your own tree.".to_owned(),
        String::new(),
        "Mandatory first action:".to_owned(),
        format!("- A bootstrap status file already exists at: {}", status_path.display()),
        format!("- If that path is unavailable, use: {}", secondary_status_path.display()),
        "- Overwrite that existing status file immediately with your real current state before broad repo work.".to_owned(),
        "- Only if both fail, print one raw `SAPPHIRE_STATUS` line.".to_owned(),
        "- Do not inspect repo files before this first status write.".to_owned(),
        "- Do not replace the first status with narration about what you plan to do.".to_owned(),
        String::new(),
        "Begin now.".to_owned(),
    ];
    lines.join("\n")
}

pub fn supervisor_loader_prompt(
    agent: AgentKind,
    prompt_path: &Path,
    display_name: &str,
) -> String {
    let prompt_ref = prompt_reference(agent, prompt_path);
    format!(
        "You are {display_name}. Read and fully ingest this supervisor brief now:\n\
{prompt_ref}\n\
\n\
Rules:\n\
- That file is the source of truth for mission supervision.\n\
- Ingest it once. Do not paraphrase the brief back to me.\n\
- Do not spend turns narrating the brief.\n\
- After reading it, start supervising immediately with the next concrete worker-facing action."
    )
}

fn prompt_reference(agent: AgentKind, prompt_path: &Path) -> String {
    let absolute = prompt_path.display().to_string();
    match agent {
        AgentKind::Qwen => format!("@{absolute}\n{absolute}"),
        _ => absolute,
    }
}
