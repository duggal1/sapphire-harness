use std::num::NonZeroUsize;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use uuid::Uuid;

use crate::agent::AgentKind;

#[derive(Debug, Parser)]
#[command(
    name = "sp",
    about = "Sapphire: terminal-first orchestration for coding agents",
    subcommand_negates_reqs = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Agent: qwen, forge, codex, claude
    #[arg(value_enum)]
    pub agent: Option<AgentKind>,

    /// Number of worker terminals
    pub count: Option<NonZeroUsize>,

    /// Mission text — what the agents should do
    pub mission: Option<String>,

    #[command(flatten)]
    pub run: RunOptions,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show active/running missions at a glance
    Status,
    /// List all missions (history)
    Sessions,
    /// Replay events for a mission
    Replay(ReplayCommand),
    /// Supervisor's final summary for a mission
    Summary(MissionArg),
    /// Resume a stalled or partial mission
    Resume(ResumeCommand),
    /// Watch a single worker's journey
    Watch(WatchCommand),
}

#[derive(Debug, Args, Clone)]
pub struct RunOptions {
    #[arg(long, default_value = ".")]
    pub repo: PathBuf,

    #[arg(long, value_enum)]
    pub supervisor_agent: Option<AgentKind>,

    #[arg(long)]
    pub db_path: Option<PathBuf>,

    #[arg(long)]
    pub state_dir: Option<PathBuf>,

    #[arg(long)]
    pub dry_run: bool,

    #[arg(long, default_value_t = 45)]
    pub stall_seconds: u64,

    #[arg(long)]
    pub watchdog_max_seconds: Option<u64>,

    #[arg(long, default_value_t = 1000)]
    pub watchdog_tick_millis: u64,

    #[arg(long)]
    pub tmux_session_name: Option<String>,

    #[arg(long)]
    pub persist_transcripts: bool,

    #[arg(long)]
    pub tui: bool,

    #[arg(long)]
    pub worker_args: Vec<String>,

    #[arg(long = "supervisor-arg")]
    pub supervisor_args: Vec<String>,
}

#[derive(Debug, Args, Clone)]
pub struct ResumeCommand {
    pub mission_id: Uuid,

    #[command(flatten)]
    pub options: RuntimeOptions,

    #[arg(long)]
    pub db_path: Option<PathBuf>,

    #[arg(long)]
    pub state_dir: Option<PathBuf>,
}

#[derive(Debug, Args, Clone)]
pub struct ReplayCommand {
    pub mission_id: Uuid,

    #[arg(long, short = 'n', default_value_t = 40)]
    pub limit: usize,

    #[arg(long)]
    pub db_path: Option<PathBuf>,
}

#[derive(Debug, Args, Clone)]
pub struct MissionArg {
    pub mission_id: Uuid,

    #[arg(long)]
    pub db_path: Option<PathBuf>,
}

#[derive(Debug, Args, Clone)]
pub struct WatchCommand {
    pub mission_id: Uuid,
    pub worker: String,

    #[arg(long, short = 'n', default_value_t = 20)]
    pub limit: usize,

    #[arg(long)]
    pub db_path: Option<PathBuf>,
}

#[derive(Debug, Args, Clone)]
pub struct RuntimeOptions {
    #[arg(long, default_value_t = 45)]
    pub stall_seconds: u64,

    #[arg(long)]
    pub watchdog_max_seconds: Option<u64>,

    #[arg(long, default_value_t = 1000)]
    pub watchdog_tick_millis: u64,

    #[arg(long)]
    pub tmux_session_name: Option<String>,

    #[arg(long)]
    pub persist_transcripts: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LaunchConfig {
    pub worker_agent: AgentKind,
    pub supervisor_agent: AgentKind,
    pub worker_count: usize,
    pub repo: PathBuf,
    pub mission: String,
    pub db_path: Option<PathBuf>,
    pub state_dir: PathBuf,
    pub dry_run: bool,
    pub stall_seconds: u64,
    pub watchdog_max_seconds: Option<u64>,
    pub watchdog_tick_millis: u64,
    pub tmux: bool,
    pub tmux_session_name: Option<String>,
    pub persist_transcripts: bool,
    pub tui: bool,
    pub worker_args: Vec<String>,
    pub supervisor_args: Vec<String>,
    /// Git remote URL if repo has a remote, None otherwise.
    /// Used to decide whether to inject git commit rules into worker prompts.
    pub git_remote: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResumeConfig {
    pub mission_id: Uuid,
    pub db_path: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub stall_seconds: u64,
    pub watchdog_max_seconds: Option<u64>,
    pub watchdog_tick_millis: u64,
    pub tmux: bool,
    pub tmux_session_name: Option<String>,
    pub persist_transcripts: bool,
    pub tui: bool,
}

impl Cli {
    pub fn into_action(self) -> Result<CliAction> {
        let Cli {
            command,
            agent,
            count,
            mission,
            run,
        } = self;

        if let Some(command) = command {
            return match command {
                Command::Status => Ok(CliAction::Status {
                    db_path: run.db_path,
                    repo: run.repo,
                    state_dir: run.state_dir,
                }),
                Command::Sessions => Ok(CliAction::Sessions {
                    db_path: run.db_path,
                    repo: run.repo,
                    state_dir: run.state_dir,
                }),
                Command::Replay(cmd) => Ok(CliAction::Replay {
                    mission_id: cmd.mission_id,
                    limit: cmd.limit,
                    db_path: cmd.db_path,
                }),
                Command::Summary(cmd) => Ok(CliAction::Summary {
                    mission_id: cmd.mission_id,
                    db_path: cmd.db_path,
                }),
                Command::Resume(cmd) => Ok(CliAction::Resume(ResumeConfig {
                    mission_id: cmd.mission_id,
                    db_path: cmd.db_path,
                    state_dir: cmd.state_dir,
                    stall_seconds: cmd.options.stall_seconds,
                    watchdog_max_seconds: cmd.options.watchdog_max_seconds,
                    watchdog_tick_millis: cmd.options.watchdog_tick_millis,
                    tmux: true,
                    tmux_session_name: cmd
                        .options
                        .tmux_session_name
                        .or_else(|| Some(default_resume_session_name(cmd.mission_id))),
                    persist_transcripts: true,
                    tui: false,
                })),
                Command::Watch(cmd) => Ok(CliAction::Watch {
                    mission_id: cmd.mission_id,
                    worker: cmd.worker,
                    limit: cmd.limit,
                    db_path: cmd.db_path,
                }),
            };
        }

        Ok(CliAction::Run(Self::launch_config_from_parts(
            agent, count, mission, run,
        )?))
    }

    fn launch_config_from_parts(
        agent: Option<AgentKind>,
        count: Option<NonZeroUsize>,
        mission: Option<String>,
        run: RunOptions,
    ) -> Result<LaunchConfig> {
        let worker_agent =
            agent.context("missing agent — usage: sp <agent> <count> \"mission\"")?;
        let repo = run
            .repo
            .canonicalize()
            .with_context(|| format!("failed to resolve repo path {}", run.repo.display()))?;

        let state_dir = match run.state_dir {
            Some(path) => path,
            None => repo.join(".sp"),
        };

        let mission =
            mission.context("missing mission — usage: sp <agent> <count> \"mission text\"")?;
        let tmux_session_name = run
            .tmux_session_name
            .or_else(|| Some(default_launch_session_name(&repo, worker_agent)));

        Ok(LaunchConfig {
            worker_agent,
            supervisor_agent: run
                .supervisor_agent
                .unwrap_or(worker_agent),
            worker_count: count
                .context("missing count — usage: sp <agent> <count> \"mission\"")?
                .get(),
            repo: repo.clone(),
            mission: mission.trim().to_owned(),
            db_path: run.db_path,
            state_dir,
            dry_run: run.dry_run,
            stall_seconds: run.stall_seconds,
            watchdog_max_seconds: run.watchdog_max_seconds,
            watchdog_tick_millis: run.watchdog_tick_millis,
            tmux: !run.dry_run,
            tmux_session_name,
            persist_transcripts: true,
            tui: run.tui,
            worker_args: run.worker_args,
            supervisor_args: run.supervisor_args,
            git_remote: match crate::git::check_git_state(&repo) {
                crate::git::GitState::Ready { remote_url } => Some(remote_url),
                _ => None,
            },
        })
    }
}

pub enum CliAction {
    Run(LaunchConfig),
    Status {
        db_path: Option<PathBuf>,
        repo: PathBuf,
        state_dir: Option<PathBuf>,
    },
    Sessions {
        db_path: Option<PathBuf>,
        repo: PathBuf,
        state_dir: Option<PathBuf>,
    },
    Replay {
        mission_id: Uuid,
        limit: usize,
        db_path: Option<PathBuf>,
    },
    Summary {
        mission_id: Uuid,
        db_path: Option<PathBuf>,
    },
    Resume(ResumeConfig),
    Watch {
        mission_id: Uuid,
        worker: String,
        limit: usize,
        db_path: Option<PathBuf>,
    },
}

fn default_launch_session_name(repo: &std::path::Path, agent: AgentKind) -> String {
    let stem = repo
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo");
    let clean = stem
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => ch.to_ascii_lowercase(),
            _ => '-',
        })
        .collect::<String>();
    format!("sp-{}-{}-{}", clean, agent.as_str(), std::process::id())
}

fn default_resume_session_name(mission_id: Uuid) -> String {
    format!("sp-resume-{}", mission_id.simple())
}
