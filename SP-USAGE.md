# sp — Sapphire CLI

Launch, supervise, and audit multiple coding agents from one command.

## Launch

```
sp <agent> <count> "mission text"
```

- **agent**: `qwen`, `codex`, `claude`, `forge`
- **count**: number of worker terminals (1–∞)
- **mission**: what the agents should do

### Examples

```bash
# 2 Codex workers audit this repo
sp codex 2 "audit this repo and identify the top 3 risks"

# 8 Qwen workers fix all failing tests
sp qwen 8 "fix all failing tests"

# 4 Claude workers build a CLI tool (dry run)
sp claude 4 "build a CLI task runner" --dry-run

# Qwen workers with Claude supervisor
sp qwen 4 "refactor the payment module" --supervisor-agent claude
```

### Flags

| Flag | What it does |
|---|---|
| `--tui` | Force the fallback single-terminal TUI dashboard instead of the default split teamwork surface |
| `--tmux-session-name <name>` | Override the default teamwork session name |
| `--dry-run` | Plan only, no agents spawned |
| `--repo <path>` | Target repo (default: `.`) |
| `--stall-seconds <n>` | Stall threshold (default: 45) |
| `--supervisor-agent <agent>` | Different supervisor than workers |

## TUI Controls

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Switch pane / cycle tabs |
| `1` | Workers tab |
| `2` | Watchdog tab |
| `3` | Events tab |
| `4` | Supervisor tab |
| `j/k` or `↑/↓` | Scroll |
| `q` | Quit (when done) |

## Inspect

```bash
sp status              # Active missions at a glance
sp sessions            # Full history
sp replay <id>         # Replay events
sp summary <id>        # Supervisor summary
sp resume <id>         # Resume stalled mission
sp watch <id> <worker> # Single worker journey
```

## How it works

1. **Plan** — Decomposes mission into workstreams
2. **Launch** — Keeps the current terminal as Sapphire control UI and opens a second terminal window for the tmux teamwork grid
3. **Coordinate** — Leases, mail, status markers
4. **Supervise** — LLM validates, resolves conflicts
5. **Persist** — Everything to `.sp/sapphire.sqlite3`

## Host Behavior

- If launched from **Ghostty**, Sapphire first tries to open the teamwork grid in a new Ghostty tab.
- If macOS blocks Ghostty tab automation, Sapphire falls back to a new **Ghostty window**, not Terminal.app.
- If launched from **VS Code** or another terminal host, Sapphire opens the teamwork grid in a separate external terminal window.
