use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::cli::NoSupervisorLaunchConfig;
use crate::runtime::SessionRuntime;
use crate::tmux::Tmux;

const PANES_PER_SESSION: usize = 10;
const STARTUP_SETTLE_BUFFER: Duration = Duration::from_millis(250);

pub async fn launch(config: NoSupervisorLaunchConfig) -> Result<String> {
    if !Tmux::is_available() {
        anyhow::bail!("tmux is required for `sp ns`");
    }

    let tmux = Tmux::new(None);
    let session_names = tmux
        .create_batch_sessions(
            &config.session_name,
            &config.repo.to_string_lossy(),
            config.count,
            PANES_PER_SESSION,
        )
        .map_err(anyhow::Error::msg)
        .context("failed to create tmux sessions for `sp ns`")?;

    let runtime_root = ns_runtime_root(&config);
    let transcript_dir = runtime_root.join("transcripts");
    std::fs::create_dir_all(&transcript_dir)?;

    let runtimes = session_names
        .iter()
        .map(|session_name| SessionRuntime::with_tmux(session_name.clone(), transcript_dir.clone()))
        .collect::<Vec<_>>();

    let mut launches = Vec::with_capacity(config.count);
    let mut prompt_targets = Vec::with_capacity(config.count);
    for index in 0..config.count {
        let runtime_index = index / PANES_PER_SESSION;
        let prompt = config.prompts[index].clone();
        let mut spec =
            config
                .agent
                .build_launch_spec(&config.repo, &runtime_root, &config.worker_args);
        spec.surface_label = format!("{}-{}", config.agent.as_str(), index + 1);
        let prompt_delay = spec.prompt_delay;
        let running = runtimes[runtime_index]
            .spawn(Uuid::new_v4(), spec)
            .with_context(|| format!("failed to spawn terminal {}", index + 1))?;
        if let Some(pane_id) = running.terminal_target() {
            prompt_targets.push((pane_id.to_owned(), prompt.clone()));
        }
        launches.push((running, prompt, prompt_delay));
    }

    if let Some(delay) = launches.iter().map(|(_, _, delay)| *delay).max() {
        tokio::time::sleep(delay + STARTUP_SETTLE_BUFFER).await;
    } else {
        tokio::time::sleep(Duration::from_millis(250) + STARTUP_SETTLE_BUFFER).await;
    }

    dispatch_prompts_parallel(&prompt_targets).await?;

    tokio::time::sleep(Duration::from_millis(350)).await;
    open_ghostty_batch_tabs_if_supported(&tmux, &session_names);

    Ok(render_summary(&config, &session_names))
}

fn ns_runtime_root(config: &NoSupervisorLaunchConfig) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    std::env::temp_dir().join(format!(
        "sp-ns-{}-{}-{stamp}",
        config.agent.as_str(),
        std::process::id()
    ))
}

#[cfg(target_os = "macos")]
fn open_ghostty_batch_tabs_if_supported(tmux: &Tmux, session_names: &[String]) {
    let _ = tmux.open_ghostty_batch_tabs(session_names);
}

#[cfg(not(target_os = "macos"))]
fn open_ghostty_batch_tabs_if_supported(_tmux: &Tmux, _session_names: &[String]) {}

fn render_summary(config: &NoSupervisorLaunchConfig, session_names: &[String]) -> String {
    format!(
        "launched {} {} terminal(s) without a supervisor\nrepo: {}\nsession base: {}\ntmux session(s): {}",
        config.count,
        config.agent.as_str(),
        config.repo.display(),
        config.session_name,
        session_names.join(", ")
    )
}

async fn dispatch_prompts_parallel(prompt_targets: &[(String, String)]) -> Result<()> {
    let mut tasks = JoinSet::new();
    for (pane_id, prompt) in prompt_targets {
        let pane_id = pane_id.clone();
        let prompt = prompt.clone();
        tasks.spawn_blocking(move || submit_prompt_fast(&pane_id, &prompt));
    }

    while let Some(result) = tasks.join_next().await {
        result
            .context("parallel prompt dispatch task failed")?
            .context("failed to submit no-supervisor prompt")?;
    }

    Ok(())
}

fn submit_prompt_fast(pane_id: &str, prompt: &str) -> Result<()> {
    let tmux = Tmux::new(None);
    let body = prompt.trim_end_matches(['\r', '\n']);
    if body.is_empty() {
        tmux.send_enter(pane_id).map_err(anyhow::Error::msg)?;
        return Ok(());
    }

    if body.contains('\n') || body.contains('\r') {
        tmux.paste_text_via_buffer(pane_id, body)
            .map_err(anyhow::Error::msg)?;
    } else {
        tmux.send_keys_literal(pane_id, body)
            .map_err(anyhow::Error::msg)?;
    }
    tmux.send_enter(pane_id).map_err(anyhow::Error::msg)?;
    Ok(())
}
