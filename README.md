# sp — Sapphire Agent Factory

A Rust CLI that orchestrates terminal-native coding agents (Codex, Claude, Qwen, Forge) as a local control plane. Launches one supervisor session plus multiple worker sessions, pushes prompts into PTYs, normalizes raw terminal output into Sapphire state, persists mission state into SQLite, and maintains a local control surface under `.sp/`.

## Phase Status

| Phase | Status | Key Artifact |
|---|---|---|
| 1 — Basic Launcher | ✅ Done | tmux splitting, interactive boot, prompt injection |
| 2 — Supervisor Dispatch | ✅ Done | JSON plan extraction + prompt decomposition |
| 3 — Full Orchestration | ✅ Done | End-to-end: 4 agents, interactive, `SAPPHIRE_STATUS` protocol |
| **4 — Live Watchdog & Role System** | **✅ Done** | Escalation ladder, supervisor health, 13 role templates, heuristic state detection, 3-layer prompt injection |
| 4.2 — Supervisor Behavioral Rules | ✅ Done | Propulsion Principle, Solo Artist Trap, Consecutive Failure Escalation |
| 4.3 — Prompt Injection Reliability | ✅ Done | PTY + startup nudge + durable file reference (all 4 agents) |
| 5 — Agent-to-Agent Communication | 🚧 TODO | 8-terminal mail, discussion, pushback, complex simulation |
| 6 — Scale to 16 | 🚧 TODO | 16-terminal orchestration, stress testing |
| 7 — Port to Rust Runtime | 🚧 TODO | Shell-script proven → `sp` binary integration |

## Table of Contents

- [Architecture](#architecture)
- [Quick Start](#quick-start)
- [CLI Commands](#cli-commands)
- [Agent Adapters](#agent-adapters)
- [Sapphire Control Protocol](#sapphire-control-protocol)
- [Session Lifecycle](#session-lifecycle)
  - [Watchdog Loop](#watchdog-loop)
  - [Supervisor Behavioral Rules](#supervisor-behavioral-rules)
  - [Role-Based Agent Activation](#role-based-agent-activation)
- [Security](#security)
  - [Heuristic State Detection](#heuristic-state-detection)
  - [Stall Escalation Ladder](#stall-escalation-ladder)
  - [Watchdog Stats](#watchdog-stats)
  - [Audit Scope](#audit-scope)
- [Phase 4 Features](#phase-4-features)
  - [Live Watchdog Dashboard](#live-watchdog-dashboard)
  - [Function-First Team Roles](#function-first-team-roles)
  - [Heuristic State Detection](#heuristic-state-detection-1)
  - [Supervisor Behavioral Rules](#supervisor-behavioral-rules-1)
  - [3-Layer Prompt Injection](#3-layer-prompt-injection)
- [Persistence Schema](#persistence-schema)
- [Directory Structure](#directory-structure)
- [Contributing](#contributing)

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                       sp (CLI)                          │
├──────────┬──────────────┬──────────────┬────────────────┤
│  CLI     │ Orchestrator │  Protocol    │  Store         │
│  (clap)  │ (watchdog)   │  (parser)    │  (SQLite)      │
├──────────┼──────────────┼──────────────┼────────────────┤
│  Agent   │ Runtime      │  Adapter     │  Templates     │
│  Kinds   │ (PTY/tmux)   │  (per-agent) │  (prompts)     │
├──────────┴──────────────┴──────────────┴────────────────┤
│                    TUI (ratatui)                         │
│              (startup/fallback dashboard)                │
└─────────────────────────────────────────────────────────┘
```

**Core components:**

| Component | File | Responsibility |
|---|---|---|
| CLI | `src/cli.rs` | Argument parsing via `clap`; defines `Run`, `Status`, `Sessions`, `Resume`, `Replay`, `Watch`, `Summary` actions |
| Orchestrator | `src/orchestrator/mod.rs` | Mission lifecycle: bootstrap, launch, watchdog loop, resume, introspection |
| Runtime | `src/runtime/mod.rs` | PTY and tmux session management, prompt injection, startup automation, buffer management |
| Protocol | `src/protocol.rs` | Parses `SAPPHIRE_STATUS`, `SAPPHIRE_MAIL`, `SAPPHIRE_ACK`, `SAPPHIRE_LEASE` directives from terminal output |
| Store | `src/store/mod.rs` | SQLite persistence with 9 tables, legacy migration, WAL mode |
| Adapter | `src/adapter.rs` | Per-agent behavior abstraction: state detection, prompt building, done-claim detection, heuristic inference |
| Agent | `src/agent/mod.rs` | Agent launch specs: executable, args, env, submit mode, automation rules, startup input nudge |
| Templates | `src/templates.rs` | Compiled prompt sources via `include_str!`; 13 enterprise role templates loaded at build time |
| TUI | `src/tui/` | Ratatui/crossterm dashboard (startup/fallback shell) |
| tmux | `src/tmux/` | tmux CLI wrapper, pane layout, control surface, Ghostty integration |
| Mail | `src/mail/` | Inter-worker mail with threading, priority, ack tracking, two-phase delivery |

## Quick Start

### Prerequisites

- Rust 1.94.1+ (stable)
- tmux (for teamwork surface)
- At least one supported agent CLI installed (Qwen, Claude, Codex, or Forge)

### Build

```bash
cargo build --release
```

### Verify

```bash
cargo test  # 77+ tests should pass
```

### Run a dry plan

```bash
cargo run --bin sp -- codex 2 --repo . --mission "debug and validate the repo" --dry-run
```

### Run live with 4 Claude workers

```bash
cargo run --bin sp -- claude 4 --repo . --mission "refactor the auth module"
```

### Run with tmux teamwork surface

```bash
cargo run --bin sp -- codex 4 --repo . --mission "implement feature X" --tmux-session-name sapphire
```

## CLI Commands

| Command | Description |
|---|---|
| `sp <agent> <count> --repo . --mission "..."` | Launch a new mission (default: `run`) |
| `sp status` | Show active/running missions |
| `sp sessions` | List all missions from SQLite |
| `sp resume <mission_id>` | Resume a mission from durable state |
| `sp replay <mission_id>` | Show recent events across all workers |
| `sp watch <mission_id> <worker>` | Show recent events for a specific worker |
| `sp summary <mission_id>` | Show supervisor's latest summary |

### Common flags

| Flag | Description |
|---|---|
| `--repo <path>` | Repository root (default: cwd) |
| `--mission <text>` | Mission description for planning |
| `--dry-run` | Plan only, don't launch workers |
| `--tmux-session-name <name>` | Name for the tmux teamwork surface |
| `--tui` | Force TUI mode |

## Agent Adapters

Four agent adapters are generated via the `impl_standard_adapter!` macro. Each has unique launch behavior:

| Agent | Default Args | Submit Mode | Startup Automation |
|---|---|---|---|
| **Qwen** | `--screen-reader` | CarriageReturn | Auto-dismisses "Do you want to connect IDE" (sends "2") |
| **Forge** | (none) | LineFeed | Sets isolated `HOME`/`XDG_DATA_HOME`/`XDG_CONFIG_HOME` to `.sp/forge-home/` |
| **Codex** | `--no-alt-screen -m gpt-5.4-mini -c model_reasoning_effort=low` | CarriageReturn | Auto-accepts directory trust (sends "1"); initial `\n` after 400ms |
| **Claude** | (none) | CarriageReturn | Auto-accepts "Yes, I trust this folder" prompt |

### Submit Modes

- **LineFeed**: Prompts terminated with `\n` (Forge)
- **CarriageReturn**: Prompts terminated with `\r\n` (Qwen, Codex, Claude), with a 250ms delay before the terminator

### State Detection

When no explicit `SAPPHIRE_STATUS` directive is found, the adapter layer infers state from output keywords:

| Keywords | Inferred State |
|---|---|
| `validation passed`, `validated`, `all checks passed` | `Validated` |
| `probably fixed`, `didn't run tests`, `can't verify` | `WeakOutput` |
| `rewrote the architecture`, `full rewrite`, `took over` | `WrongDirection` |
| `i'm done`, `completed the task`, `task is complete` | `NeedsValidation` |
| `blocked`, `cannot proceed`, `need clarification` | `Blocked` |
| `conflict`, `overlap`, `contradiction` | `Contradictory` |
| `investigating`, `working on`, `running tests` | `Progressing` |

## Sapphire Control Protocol

Workers communicate with the orchestrator through single-line JSON directives embedded in terminal output:

### SAPPHIRE_STATUS
```json
SAPPHIRE_STATUS {"state":"progressing","summary":"working on auth","files":["src/auth.rs"],"commands":["cargo test"],"risks":["regression risk"],"overlap":"none"}
```

### SAPPHIRE_MAIL
```json
SAPPHIRE_MAIL {"to":"worker-02","message_type":"dependency_request","priority":"high","subject":"confirm API","context":"...","request":"share response format","expected_action":"reply","requires_ack":true}
```

### SAPPHIRE_ACK
```json
SAPPHIRE_ACK {"mail_id":"m-1","status":"acked","summary":"will respond shortly"}
```

### SAPPHIRE_LEASE
```json
SAPPHIRE_LEASE {"paths":["src/auth.rs"],"intent":"edit","status":"claim"}
```

The protocol parser (`src/protocol.rs`) sanitizes ANSI escape codes, buffers partial lines, and extracts JSON objects using a brace-depth counter (not regex) for robustness with nested/multiline JSON.

## Session Lifecycle

### 16-State Lifecycle

```
Planned → Booting → NotStarted → Progressing → Blocked → Stalled →
DoneClaimed → NeedsValidation → WeakOutput → WrongDirection →
Contradictory → NeedsRetry → Validated → Failed → Exited
```

Terminal states: `Validated`, `Failed`, `Exited`.

### Watchdog Loop

The orchestrator runs a tick-based monitoring loop (configurable interval, default 1s):
- Collects runtime events from all PTYs
- Processes directives (status, mail, ack, lease)
- **Stall detection**: workers silent beyond `stall_seconds` are marked `Stalled` with a 3-tier escalation ladder:
  - **1st stall**: Corrective status prompt + supervisor decision queued
  - **2nd stall**: Redirect with narrowed scope sent immediately + supervisor notice
  - **3rd+ stall**: Force `Failed` state + supervisor decides respawn or reassign
- **Liveness tracking**: `last_confirmed_alive` timestamp resets on any output, distinguishing "stalled but was recently alive" from "dead". The `consecutive_stall_failures` counter resets on any liveness signal.
- **Protocol reminders**: workers producing output but no directives after 3+ chunks receive a strict status prompt
- **Supervisor health monitoring**: detects supervisor stalls, transitions through `Healthy → Recovering → Degraded` modes; in degraded mode the watchdog generates its own final synthesis as fallback
- **Supervisor decision queue**: pending decisions (validation, stall recovery, low-confidence recovery) are queued, deduplicated by signature, and applied with mode awareness (degraded mode skips actions requiring supervisor intelligence)
- **Max runtime enforcement**: optional `watchdog_max_seconds` hard timeout
- **Watchdog stats**: tracks and persists `runtime_events`, `directives`, `mails_routed`, `validation_challenges`, `stall_interventions`, `lease_conflicts`, `protocol_reminders`, `supervisor_health_events`, `supervisor_fallbacks`
- Writes status snapshots to `.sp/control/status.txt`

### Supervisor Behavioral Rules

Two behavioral rules baked into the supervisor prompt prevent common failure modes:

- **Propulsion Principle**: Supervisor auto-executes on restart — checks persisted state, acts on pending work, then summarizes (action first, summary second). Never waits for human approval. On supervisor restart, it immediately resumes from durable state rather than announcing and waiting.
- **Solo Artist Trap**: Supervisor must dispatch, not implement. Decision tree: coordination → do it directly, implementation → sling to worker, trivial → fix directly. Anti-pattern: reading code to "understand the issue" and fixing it burns context needed for team supervision.

### Role-Based Agent Activation

The supervisor analyzes the mission and activates relevant **function-first engineering team roles** — not generic "worker" personas. Each agent has:

- **`role_type`**: stable machine key for template lookup (e.g. `software-engineer`)
- **`display_name`**: runtime label for prompts/logs/UI (e.g. `Engineer-1`, `Designer-2`)

13 role templates are compiled into the binary at build time from `src/internal/agents/templetes/roles/job-roles/`. Each includes responsibilities, operating rules, pushback policy, coordination protocol, and validation standard. The supervisor decides which roles to deploy — coding-only tasks get `software-engineer` instances; multi-domain tasks activate `software-engineer` + `designer-engineer` + `security-engineer`, etc.

| Role Type | Display Name | When Activated |
|---|---|---|
| `software-engineer` | Engineer-1, Engineer-2, ... | Coding tasks, feature implementation |
| `research-engineer` | Researcher-1, ... | Investigation, analysis, technology survey |
| `validation-engineer` | Validator-1, ... | Testing, quality gates, claim verification |
| `architecture-engineer` | Architect-1, ... | System design, structural decisions |
| `security-engineer` | Security-1, ... | Auth, threat modeling, vulnerability review |
| `debug-and-review-engineer` | Reviewer-1, ... | Debugging, code review, contradiction resolution |
| `testing-and-automation-engineer` | QA-1, ... | Test suites, CI/CD, automation |
| `designer-engineer` | Designer-1, ... | UI/UX, visual design, frontend polish |
| `sales-engineer` | Sales-1, ... | Go-to-market, positioning, demo prep |
| `solutions-engineer` | Solutions-1, ... | Integration patterns, customer architecture |
| `customer-success-engineer` | CustomerSuccess-1, ... | Support workflows, onboarding, documentation |
| `product-engineer` | Product-1, ... | Product strategy, roadmap, feature prioritization |
| `compliance-engineer` | Compliance-1, ... | Regulatory, policy, standards alignment |

One agent slot is reserved as **Steward** for `AGENTS.md` maintenance when the run has more than one terminal.

### 3-Layer Prompt Injection Reliability (Phase 4.3)

Every agent receives its full role template + scoped assignment through three independent delivery layers — each alone would be sufficient, together they're bulletproof:

1. **PTY `send_prompt()`**: Full role template + assignment written to PTY master fd after per-agent boot delay (all agents).
2. **Startup `\n` nudge**: Timed Enter key wakes the TUI readline before the assignment arrives — Qwen 500ms, Codex 400ms, Forge 400ms, Claude 600ms.
3. **Durable file reference**: Full assignment saved to `.sp/prompts/<name>.md`; agents told to re-read if anything is unclear.

Each agent has per-agent launch specs with startup automation rules: Qwen dismisses IDE prompt, Codex accepts directory trust, Claude accepts folder trust, Forge sets isolated HOME/XDG sandboxing.

## Security

### Heuristic State Detection

When workers don't emit explicit `SAPPHIRE_STATUS` directives, the adapter layer infers state from output keywords:

| Keywords | Inferred State |
|---|---|
| `validation passed`, `validated`, `all checks passed` | `Validated` |
| `probably fixed`, `didn't run tests`, `can't verify` | `WeakOutput` |
| `rewrote the architecture`, `full rewrite`, `took over` | `WrongDirection` |
| `i'm done`, `completed the task`, `task is complete` | `NeedsValidation` |
| `blocked`, `cannot proceed`, `need clarification` | `Blocked` |
| `conflict`, `overlap`, `contradiction` | `Contradictory` |
| `investigating`, `working on`, `running tests` | `Progressing` |

Each inferred observation carries a confidence score (High/Medium/Low), raw excerpt, and source attribution. After 2+ low-confidence observations for the same key, the orchestrator escalates with a corrective prompt and supervisor notice.

### Stall Escalation Ladder

The watchdog tracks `consecutive_stall_failures` per worker (resets on any output/automation event):

| Stall # | Action |
|---|---|
| 1st | Corrective status prompt + supervisor decision queued |
| 2nd | Redirect with narrowed scope sent immediately + supervisor notice |
| 3rd+ | Force `Failed` state + supervisor decides respawn or reassign |

### Watchdog Stats

The orchestrator tracks and persists: `runtime_events`, `directives`, `mails_routed`, `validation_challenges`, `stall_interventions`, `lease_conflicts`, `protocol_reminders`, `supervisor_health_events`, `supervisor_fallbacks`. These are exposed via `sp replay` and `sp watch` commands.

### Audit Scope

The following areas were audited across all `src/` files:

### 1. Shell Quoting (`shell_quote()`)

**Location:** `src/runtime/mod.rs`, `src/tmux/mod.rs`

**Implementation:**
```rust
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\"'\"'"))
}
```

**Finding: SAFE.** This is the standard POSIX shell quoting pattern. It wraps the input in single quotes and escapes any embedded single quotes by closing the quote, adding an escaped single quote, and reopening the quote. This correctly handles all characters including spaces, backticks, `$()`, `;`, `|`, `&`, newlines, and other shell metacharacters.

**Usage locations:**
- `render_tmux_command()` — quotes environment variables and program paths
- `open_external_terminal_for_session()` — quotes tmux session names in AppleScript
- `open_ghostty_tab_for_session()` — quotes session names in AppleScript

### 2. Path Traversal

**Finding: LOW RISK (local tool).** All file operations use paths derived from:
- `.sp/` directory (created by the orchestrator under the repo root)
- `--repo` CLI argument (user-supplied, trusted)
- `transcript_dir` (derived from `.sp/transcripts/`)

Lease paths come from worker directives and are stored as-is in SQLite. Since this is a local CLI tool (not a network service), path traversal risk is bounded by the user's own filesystem permissions.

**Recommendation:** If this tool is ever exposed to untrusted input for lease paths, add path normalization (strip `..`, resolve symlinks, validate against repo root).

### 3. SQLite Parameterized Binding

**Finding: SAFE.** All SQL queries in `src/store/mod.rs` use parameterized binding via `params![]` macro:
```rust
connection.execute(
    "INSERT INTO sessions (...) VALUES (?1, ?2, ?3, ...)",
    params![mission.id, mission.created_at, ...],
)?;
```

No string concatenation is used for SQL values. JSON payloads are serialized via `serde_json::to_string()` before binding, which produces valid JSON strings (not SQL).

### 4. Command Injection in tmux

**Finding: MODERATE.** The `split_window_with_command()` function passes the `command` argument directly to tmux:
```rust
args.extend_from_slice(&["-t", target_pane, command]);
```

Since `command` is constructed internally by `render_tmux_command()` which uses `shell_quote()` for all components, the attack surface is limited to the program name and args from `ProcessLaunchSpec` (which are constructed from trusted `AgentKind` specs).

### 5. ANSI Escape Handling

**Finding: SAFE.** The `sanitize_output()` function strips ANSI escape sequences before directive parsing:
```rust
fn sanitize_output(text: &str) -> String {
    ansi_escape_regex().replace_all(text, "").replace('\r', "\n")
}
```

The regex `\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])` covers CSI, OSC, DCS, and single-byte escape sequences.

### 6. Buffer Management

**Finding: SAFE.** The `BufferManager` caps carry buffers at 24KB with trim-to-12KB, preserving UTF-8 character boundaries. The `previous_char_boundary()` function walks backward to find a valid char boundary, preventing partial multi-byte sequences.

### 7. AppleScript Injection

**Finding: MODERATE.** The `applescript_escape()` function only escapes `\` and `"`:
```rust
fn applescript_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}
```

This is used for session names in AppleScript. Session names are generated internally (not user-controlled), so this is acceptable. If session names ever become user-controllable, additional escaping (newlines, backticks, `$`) should be added.

### Security Findings Summary

| Area | Status | Notes |
|---|---|---|
| `shell_quote()` | **SAFE** | Standard POSIX quoting pattern |
| SQLite binding | **SAFE** | All queries use `params![]` macro |
| ANSI sanitization | **SAFE** | Comprehensive regex covers all escape sequences |
| Buffer management | **SAFE** | UTF-8 boundary preservation, bounded carry |
| Path traversal | **LOW RISK** | All paths are internal; bounded by user permissions |
| tmux command injection | **LOW RISK** | Commands built from trusted `AgentKind` specs |
| AppleScript escaping | **MODERATE** | Session names are internally generated |

## Phase 4 Features

### Live Watchdog Dashboard
Tick-based monitoring with configurable intervals. Workers silent beyond `stall_seconds` run through a 3-tier escalation ladder (corrective prompt → narrowed-scope redirect → force `Failed`). Liveness is tracked via `last_confirmed_alive` with a `consecutive_stall_failures` counter that resets on any output. The watchdog generates its own final synthesis when the supervisor is unavailable (degraded mode).

### Function-First Team Roles
Replaced generic `worker-01` IDs with 13 enterprise role templates compiled at build time. Each agent has a `role_type` (stable machine key like `software-engineer`) for template lookup and a `display_name` (runtime label like `Engineer-1`, `Designer-2`) for prompts/logs/UI. The supervisor decides which roles to activate based on mission analysis — no hardcoding.

### Heuristic State Detection
When agents don't emit explicit `SAPPHIRE_STATUS` directives, the adapter layer infers state from output keywords (8 states: `Validated`, `WeakOutput`, `WrongDirection`, `NeedsValidation`, `Blocked`, `Contradictory`, `Progressing`, `Failed`). Each inferred observation carries a confidence score, raw excerpt, and source attribution. After 2+ low-confidence observations for the same key, the orchestrator escalates with a corrective prompt and supervisor notice.

### Supervisor Behavioral Rules
- **Propulsion Principle**: Supervisor auto-executes on restart — checks persisted state, acts on pending work, then summarizes. Never waits for human approval.
- **Solo Artist Trap**: Supervisor must dispatch, not implement. Decision tree: coordination → do it directly, implementation → sling to worker, trivial → fix directly.
- **Consecutive Failure Escalation**: 3-tier escalation ladder enforced with `consecutive_stall_failures` counter that resets on any liveness signal.

### 3-Layer Prompt Injection
Every agent receives its assignment through three independent delivery layers: PTY `send_prompt()` after boot delay, timed startup `\n` nudge to wake the TUI readline, and durable file reference (`.sp/prompts/<name>.md`) for re-reading. Per-agent startup automation handles trust/setup prompts automatically.

## Persistence Schema

9 tables in SQLite with WAL mode:

| Table | Purpose | Key Columns |
|---|---|---|
| `sessions` | Mission records | `id`, `repo_path`, `status`, `plan_json` |
| `workers` | Session records (supervisor + workers) | `id`, `session_id`, `role`, `status`, `packet_json` |
| `tasks` | Task assignments per worker | `worker_id`, `title`, `status`, `depends_on` |
| `events` | All runtime events | `event_type`, `payload`, `worker_id` |
| `messages` | Inter-worker mail with ack tracking | `from_worker_id`, `to_worker_id`, `status`, `priority` |
| `summaries` | Freeform summaries | `summary_type`, `content`, `worker_id` |
| `ownership_leases` | File ownership claims | `session_id`, `path`, `owner_worker_id` |
| `normalized_updates` | Adapter-normalized state observations | `normalized_state`, `confidence`, `adapter` |
| `validation_results` | Validation challenge outcomes | `outcome`, `evidence`, `worker_id` |

### Legacy Migration

When schema columns don't match expected sets, the old table is renamed to `{name}_legacy_v0` and a new table is created. This is opportunistic — there is no data migration framework.

## Directory Structure

```
.
├── src/
│   ├── main.rs              # Entry point, CLI dispatch
│   ├── cli.rs               # CLI argument definitions
│   ├── model.rs             # Shared domain types
│   ├── orchestrator/mod.rs  # Core runtime coordinator
│   ├── runtime/mod.rs       # PTY/tmux session management
│   ├── protocol.rs          # Control protocol parser
│   ├── store/mod.rs         # SQLite persistence
│   ├── adapter.rs           # Per-agent behavior abstraction
│   ├── agent/mod.rs         # Agent launch specifications
│   ├── templates.rs         # Compiled prompt sources
│   ├── tmux/                # tmux CLI wrapper
│   ├── tui/                 # Ratatui dashboard
│   ├── mail/                # Inter-worker mail system
│   └── internal/            # UI theming infrastructure
├── .sp/                     # Runtime state directory
│   ├── sapphire.sqlite3     # SQLite database
│   ├── control/status.txt   # Live status snapshot
│   ├── transcripts/         # Per-worker transcript logs
│   └── prompts/             # Written prompt artifacts
├── AGENTS.md                # Agent instructions (seeded if missing)
├── Cargo.toml
└── product-direction.md     # Product direction document
```

## Contributing

### Change Rules

1. **Do not break the protocol contract.** If you change protocol wording or supported states, update `src/protocol.rs`, `src/model.rs`, `src/orchestrator/mod.rs`, `src/adapter.rs`, `src/templates.rs`, and any prompt markdown that teaches the protocol.

2. **Session state changes are global.** If you add a session state, update `SessionState::from_directive`, `is_terminal`, watchdog escalation logic, prompt instructions, and adapter heuristic keyword lists.

3. **Preserve migration compatibility.** Keep old runs readable or add a migration strategy when changing the persistence schema.

4. **Preserve agent launch safety.** If you change `AgentKind` launch behavior, maintain startup automation for trust/setup prompts.

5. **Keep worker packets narrow.** Ownership boundaries are a safety mechanism, not prompt decoration.

6. **Prefer surgical changes.** This project is small and tightly coupled.

### Commands

```bash
# Format
cargo fmt

# Test
cargo test

# Build (release)
cargo build --release

# Lint
cargo clippy
```

### Stack

- **Rust** 1.94.1, edition 2024
- **tokio** — async runtime
- **clap** — CLI parsing
- **rusqlite** (bundled SQLite) — persistence
- **portable-pty** — PTY/process control
- **ratatui** + **crossterm** — TUI
- **pulldown-cmark** — markdown rendering
- **serde** / **serde_json** — serialization
- **tracing** / **tracing-subscriber** — logging
- **parking_lot** — concurrency
- **uuid** — UUID generation
- **blake3** — content hashing
- **regex** — pattern matching
- **chrono** — time handling
- **anyhow** — error handling
