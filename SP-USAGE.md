# sp — Sapphire CLI

Launch, supervise, and audit multiple coding agents from one command.

## Launch With Supervision

```
sp <agent> <count> "mission text"
```

- **agent**: `qwen`, `codex`, `claude`, `forge`
- **count**: number of worker terminals (1–∞)
- **mission**: what the agents should do

### Examples

```bash
# 2 Claude workers audit this repo
sp claude 2 "audit this repo and identify the top 3 risks"

# 4 Claude workers build a CLI tool (dry run)
sp claude 4 "build a CLI task runner" --dry-run

# Claude workers with Codex supervisor
sp claude 4 "refactor the payment module" --supervisor-agent codex
```

## Launch Without a Supervisor

```bash
sp ns <agent> <count> "<prompt 1>" "<prompt 2>" ... "<prompt N>"
```

- `ns` is the no-supervisor terminal launcher
- prompt count must exactly match `count`
- use this when you want direct terminal launch capability and you supervise manually

### Examples

```bash
# 3 Claude terminals with distinct prompts
sp ns claude 3 "audit auth" "audit billing" "audit tests"

# 20 Claude terminals launched directly
sp ns claude 20 "prompt 1" "prompt 2" "prompt 3" "prompt 4" "prompt 5" "prompt 6" "prompt 7" "prompt 8" "prompt 9" "prompt 10" "prompt 11" "prompt 12" "prompt 13" "prompt 14" "prompt 15" "prompt 16" "prompt 17" "prompt 18" "prompt 19" "prompt 20"
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
2. **Launch** — Keeps the current terminal as Sapphire control UI and opens the tmux teamwork grid in Ghostty tabs when available
3. **Coordinate** — Leases, mail, status markers
4. **Supervise** — LLM validates, resolves conflicts
5. **Persist** — Everything to `.sp/sapphire.sqlite3`

## Host Behavior

- If launched from **Ghostty**, Sapphire opens the teamwork grid in Ghostty tabs.
- If Ghostty is not open yet, Sapphire launches Ghostty once, uses the first tab, then adds tabs for the rest.
- Sapphire does not fall back to opening extra Ghostty windows for the tmux teamwork grid.
- If launched from **VS Code** or another terminal host, Sapphire opens the teamwork grid in a separate external terminal window.
