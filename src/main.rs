mod adapter;
mod agent;
mod cli;
mod color;
mod git;
mod internal;
mod model;
mod orchestrator;
mod protocol;
mod runtime;
mod store;
mod templates;
mod terminal_palette;
mod tmux;
mod tui;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, CliAction};
use orchestrator::Orchestrator;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let action = cli.into_action()?;
    init_tracing(&action);

    match action {
        CliAction::Run(mut config) => {
            // ─── Git Pre-Flight Check ──────────────────────────────
            // Ultra-fast check: .git exists + git remote -v has output.
            let git_state = git::check_git_state(&config.repo);
            match git_state {
                git::GitState::Ready { .. } => {
                    // Remote exists — agents will handle commits. Skip prompt.
                }
                git::GitState::Unavailable => {
                    let _ = git::prompt_git_init(&config.repo, &git_state)?;
                }
                git::GitState::NoRemote => {
                    // Has git but no remote — prompt user
                    match git::prompt_git_init(&config.repo, &git_state)? {
                        git::GitInitResult::Initialized { remote_url } => {
                            config.git_remote = Some(remote_url);
                        }
                        git::GitInitResult::Declined => {
                            // User declined — continue without git
                            // Agents will be told there's no remote via their prompt
                        }
                    }
                }
                git::GitState::NotARepo => {
                    // No git repo at all — prompt user
                    match git::prompt_git_init(&config.repo, &git_state)? {
                        git::GitInitResult::Initialized { remote_url } => {
                            config.git_remote = Some(remote_url);
                        }
                        git::GitInitResult::Declined => {
                            // User declined — continue without git
                        }
                    }
                }
            }

            let db_path = config.db_path.clone().unwrap_or_else(|| {
                config.state_dir.clone().join("sapphire.sqlite3")
            });
            let control_status = config.state_dir.join("control/status.txt");
            let use_teamwork =
                config.tmux && tui::run_enabled_for_launch(config.dry_run) && tmux::Tmux::is_available();
            let use_tui = config.tui || (tui::run_enabled_for_launch(config.dry_run));

            if use_teamwork && !config.dry_run {
                let session_name = config
                    .tmux_session_name
                    .clone()
                    .expect("teamwork surface session name is always set");
                let config_clone = config.clone();
                let task = tokio::spawn(async move {
                    let orchestrator = Orchestrator::bootstrap(&config_clone)?;
                    orchestrator.launch(config_clone).await
                });
                let attach = tui::attach_for_repo(config.repo.clone(), chrono::Utc::now());
                let tmux_ready = tui::run_startup_dashboard_until_tmux(
                    db_path.clone(),
                    control_status.clone(),
                    attach.clone(),
                    &session_name,
                    &task,
                )
                .await?;
                // External terminal is opened by the orchestrator inside run_live_mission.
                // Do NOT open it here — that would create a duplicate window.
                let _ = tmux_ready;
                let summary =
                    tui::run_launch_dashboard(db_path, control_status, attach, task).await?;
                println!("{}", summary.render());
            } else if use_tui && !config.dry_run {
                let config_clone = config.clone();
                let task = tokio::spawn(async move {
                    let orchestrator = Orchestrator::bootstrap(&config_clone)?;
                    orchestrator.launch(config_clone).await
                });
                let attach = tui::attach_for_repo(config.repo.clone(), chrono::Utc::now());
                let summary = tui::run_launch_dashboard(db_path, control_status, attach, task).await?;
                println!("{}", summary.render());
            } else {
                let orchestrator = Orchestrator::bootstrap(&config)?;
                let summary = orchestrator.launch(config).await?;
                println!("{}", summary.render());
            }
        }
        CliAction::Status {
            db_path,
            repo,
            state_dir,
        } => {
            let db_path = db_path.unwrap_or_else(|| {
                state_dir
                    .unwrap_or_else(|| repo.join(".sp"))
                    .join("sapphire.sqlite3")
            });
            let orchestrator = Orchestrator::open(&db_path)?;
            println!("{}", orchestrator.render_status()?);
        }
        CliAction::Sessions {
            db_path,
            repo,
            state_dir,
        } => {
            let db_path = db_path.unwrap_or_else(|| {
                state_dir
                    .unwrap_or_else(|| repo.join(".sp"))
                    .join("sapphire.sqlite3")
            });
            let orchestrator = Orchestrator::open(&db_path)?;
            println!("{}", orchestrator.render_sessions()?);
        }
        CliAction::Resume(config) => {
            let db_path = config.db_path.clone().unwrap_or_else(|| {
                config
                    .state_dir
                    .clone()
                    .unwrap_or_else(|| std::env::current_dir().expect("cwd").join(".sp"))
                    .join("sapphire.sqlite3")
            });
            let control_status = config
                .state_dir
                .clone()
                .unwrap_or_else(|| std::env::current_dir().expect("cwd").join(".sp"))
                .join("control/status.txt");
            let use_teamwork =
                config.tmux && tmux::Tmux::is_available() && tui::run_enabled_for_launch(false);
            if use_teamwork {
                let session_name = config
                    .tmux_session_name
                    .clone()
                    .expect("teamwork surface session name is always set");
                let config_clone = config.clone();
                let task = tokio::spawn(async move {
                    let orchestrator = Orchestrator::open(
                        &config_clone
                            .db_path
                            .clone()
                            .unwrap_or_else(|| {
                                config_clone
                                    .state_dir
                                    .clone()
                                    .unwrap_or_else(|| std::env::current_dir().expect("cwd").join(".sp"))
                                    .join("sapphire.sqlite3")
                            }),
                    )?;
                    orchestrator.resume(config_clone).await
                });
                let attach = tui::attach_for_mission(config.mission_id);
                let tmux_ready = tui::run_startup_dashboard_until_tmux(
                    db_path.clone(),
                    control_status.clone(),
                    attach.clone(),
                    &session_name,
                    &task,
                )
                .await?;
                // External terminal is opened by the orchestrator inside run_live_mission.
                let _ = tmux_ready;
                let summary = tui::run_launch_dashboard(
                    db_path.clone(),
                    control_status,
                    attach,
                    task,
                )
                .await?;
                println!("{}", summary.render());
            } else {
                let orchestrator = Orchestrator::open(&db_path)?;
                let summary = orchestrator.resume(config).await?;
                println!("{}", summary.render());
            }
        }
        CliAction::Replay {
            mission_id,
            limit,
            db_path,
        } => {
            let db_path = db_path.unwrap_or_else(|| {
                std::env::current_dir()
                    .expect("cwd")
                    .join(".sp/sapphire.sqlite3")
            });
            let orchestrator = Orchestrator::open(&db_path)?;
            println!("{}", orchestrator.render_replay(mission_id, limit)?);
        }
        CliAction::Summary {
            mission_id,
            db_path,
        } => {
            let db_path = db_path.unwrap_or_else(|| {
                std::env::current_dir()
                    .expect("cwd")
                    .join(".sp/sapphire.sqlite3")
            });
            let orchestrator = Orchestrator::open(&db_path)?;
            println!("{}", orchestrator.render_supervisor_summary(mission_id)?);
        }
        CliAction::Watch {
            mission_id,
            worker,
            limit,
            db_path,
        } => {
            let db_path = db_path.unwrap_or_else(|| {
                std::env::current_dir()
                    .expect("cwd")
                    .join(".sp/sapphire.sqlite3")
            });
            let orchestrator = Orchestrator::open(&db_path)?;
            println!(
                "{}",
                orchestrator.render_worker_replay(mission_id, &worker, limit)?
            );
        }
    }
    Ok(())
}

fn init_tracing(action: &CliAction) {
    let interactive = match action {
        CliAction::Run(config) => !config.dry_run && (config.tmux || config.tui),
        CliAction::Resume(config) => config.tmux || config.tui,
        _ => false,
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact();
    if interactive {
        builder.with_writer(std::io::sink).init();
    } else {
        builder.init();
    }
}
