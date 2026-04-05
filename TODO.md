# Sapphire — Implementation To-Do

> Status tracker for turning the shell-script prototype into a real `sp` orchestration layer.

---

## Phase 1 — Basic Terminal Launcher ✅ **DONE**

> Proven working via `launch-supervisor-grid.sh`

- [x] Create a simple script file for testing terminal launch → tmux session
- [x] Launch Qwen and Claude instances in tmux with vertical + horizontal splitting
- [x] Prompt injection: inject demo prompts into tmux panes running in interactive mode
- [x] Supervisor automation: replace fake demo prompts with real supervisor-generated prompts
- [x] JSON plan extraction from supervisor → per-worker prompt decomposition

## Phase 2 — Make Everything Real ✅ **DONE**

> Supervisor now plans, decomposes, and dispatches work autonomously

- [x] Supervisor takes mission breakdown and gives each agent its scoped prompt
- [x] Agent does the work and gives back the response to supervisor
- [x] Supervisor creates a concise summary of what happened in real time

## Phase 3 — Go Beyond: Full Orchestration Layer ✅ **DONE**

> Hardened from shell-script prototype into the real `sp` runtime. All subsystems verified working end-to-end.

### 3.1 Watchdog Loop ✅
- [x] Implement tick-based monitoring (1s interval) across all worker sessions
- [x] Stall detection: mark workers silent beyond threshold as `Stalled`, send corrective prompts
- [x] Protocol reminder: nudge workers that produce output but no `SAPPHIRE_STATUS` directives after 3+ chunks
- [x] Supervisor health monitoring: detect supervisor stalls, transition to `Degraded` mode
- [x] Status snapshot: write `.sp/control/status.txt` with live session states on each tick

### 3.2 Sapphire Control Protocol (4 directive types) ✅
- [x] `SAPPHIRE_STATUS` — parse worker state reports, update session state, trigger validation
- [x] `SAPPHIRE_MAIL` — durable inter-worker message routing with ack tracking and priority
- [x] `SAPPHIRE_ACK` — mail acknowledgment processing, sender/supervisor notification
- [x] `SAPPHIRE_LEASE` — file ownership claims, conflict detection, contradiction handling

### 3.3 Heuristic State Detection (Adapter Layer) ✅
- [x] Qwen adapter: keyword-based state inference from terminal output
- [x] Claude adapter: keyword-based state inference from terminal output
- [x] Codex adapter: keyword-based state inference from terminal output
- [x] Forge adapter: keyword-based state inference from terminal output
- [x] 16-state lifecycle: `Planned → Booting → NotStarted → Progressing → Blocked → Stalled → DoneClaimed → NeedsValidation → WeakOutput → WrongDirection → Contradictory → NeedsRetry → Validated → Failed → Exited`

### 3.4 Inter-Worker Communication (Mail System) ✅
- [x] Message types normalized to 5 durable coordination lanes: `task`, `reply`, `notification`, `escalation`, `scavenge`
- [x] Priority routing: urgent → high → normal → low
- [x] Ack timeout probing at 20s with supervisor escalation
- [x] Conversation threading with thread IDs and reply-to tracking
- [x] Supervisor visibility on all non-supervisor mail
- [x] Non-destructive mail delivery via filesystem nudge queue for normal priority mail
- [x] Explicit ack statuses: `acked`, `done`, `cannot_comply`
- [x] Multi-stage mail timeout ladder: recipient probe → reroute warning → coordination failure escalation
- [x] Worker prompt contract: teammate-first coordination before supervisor escalation when possible

### 3.5 File Ownership & Conflict Resolution ✅
- [x] Lease-based file ownership: workers claim files before editing
- [x] Conflict detection: two workers claim same path → challenger downgraded to `Contradictory`
- [x] Owner notification: notify current owner when another worker attempts to claim their path
- [x] Supervisor escalation on lease conflicts

### 3.6 Supervisor Action Execution ✅
- [x] `observe` — passive monitoring
- [x] `validate_worker` — send validation challenge to target
- [x] `retry_worker` / `redirect_worker` / `message_worker` — send correction prompt
- [x] `accept_worker` — force `Validated` state
- [x] `fail_worker` — force `Failed` state
- [x] Action deduplication: prevent repeated identical actions

### 3.7 Persistence Layer (SQLite) ✅
- [x] 9 tables: sessions, workers, tasks, events, messages, summaries, ownership_leases, normalized_updates, validation_results
- [x] Mission snapshot loading with worker packet deserialization
- [x] Replay query: recent events across all workers
- [x] Worker-specific replay: filtered event timeline per session
- [x] WAL mode + legacy migration via table rename

### 3.8 tmux Control Surface ✅
- [x] Pre-split all panes before launching agents (proven pattern from shell script)
- [x] Per-worker transcript tails
- [x] Live Sapphire control panel displaying `.sp/control/status.txt`
- [x] Ghostty integration: tab-first, window fallback (macOS)
- [x] External terminal opening for non-Ghostty hosts

### 3.9 Agent Adapters (4 implementations) ✅
- [x] Qwen: `--screen-reader` for supervisor (stdin pipe), interactive TUI for workers, startup automation (IDE prompt dismiss)
- [x] Claude: interactive TUI, startup automation (folder trust accept), CarriageReturn submit
- [x] Codex: `--no-alt-screen`, gpt-5.4-mini pinned, startup automation (directory trust accept), timed initial `\n`
- [x] Forge: isolated HOME/XDG sandboxing, LineFeed submit

### 3.10 TUI Dashboard ✅
- [x] Workers tab: per-worker state, progress, last event
- [x] Watchdog tab: stall counts, protocol reminders, supervisor health
- [x] Events tab: recent runtime events feed
- [x] Supervisor tab: strategy, risk map, latest actions
- [x] Keyboard navigation: Tab/Shift+Tab, j/k, number keys, q to quit

### 3.11 End-to-End Verification ✅
- [x] 4 Qwen workers launched in interactive mode via tmux 2×2 grid
- [x] All 4 workers emitted `SAPPHIRE_STATUS {"state":"done_claimed"}` independently
- [x] No overlap conflicts across workers (each scoped cleanly)
- [x] Builds pass (`cargo build`, `cargo build --release`)
- [x] Tests pass (80/80)
- [x] Protocol contract honored: workers emit status markers correctly
- [x] Ghostty attachment to tmux session working
- [x] Supervisor JSON plan extraction (with markdown fallback) working
- [x] Per-worker prompt injection via tmux buffer working

---

## Quick Reference

| Phase | Status | Key Artifact |
|---|---|---|
| 1 — Basic Launcher | ✅ Done | `launch-supervisor-grid.sh` |
| 2 — Supervisor Dispatch | ✅ Done | JSON plan extraction + prompt injection |
| 3 — Full Orchestration | ✅ Done | End-to-end: interactive orchestration, `SAPPHIRE_STATUS` protocol, 80 tests pass |
| 4 — Live Watchdog & Role System | ✅ Done | Escalation ladder, supervisor health monitoring, role naming (13 templates), heuristic state detection, file-based status reporting, human-readable supervisor snapshots |
| 4.2 — Supervisor Behavioral Rules | ✅ Done | Propulsion Principle, Solo Artist Trap, Consecutive Failure Escalation Ladder |
| 4.3 — Shell Script Proof of Reliability | ✅ Done | File-based status (primary), tmux capture (fallback), readline wake nudge, role-aware prompts, live supervisor snapshots with `SUPERVISOR_MD:` responses, `--test` mode for file-based state validation |
| 4.7 — Gas Town Reliability Patterns | ✅ Done | Zombie TOCTOU mitigation, zombie debounce (3-cycle), health probe before stall, message deduplication, sliding window mass death detection, session readiness verification. 71 new tests. |
| 4.8 — Supervisor Repair & Continuity | ✅ Done | Primary + repair supervisor, active-supervisor routing, peer continuity when CEO is down |
| 5 — Agent-to-Agent Communication | ✅ Done | Coordination topology, scoped worker memory, pod summaries, meeting artifacts, staged lifecycle mail handling |
| 6 — Scale to 16 | 🚧 TODO | 16-terminal orchestration, stress testing, full iteration |
| 7 — Port Shell Proofs to Rust | 🚧 IN PROGRESS | File-based status ✅, readline nudge ✅, supervisor snapshots ✅, prompt hardening ✅, TUI rewrite ✅, Ghostty AppleScript ✅, max 10/tab ✅, role awareness ✅, mail routing ✅, lease handling ✅. Remaining: integration tests, large-scale live validation |

---

## Phase 4 — Live Watchdog & Team Role System ✅ **DONE**

> Core watchdog loop, role naming system, heuristic state detection, and escalation ladder are fully implemented and tested in Rust. Remaining items are UI/dashboard wiring.

### 4.1 Live Watchdog Dashboard ✅
- [x] Tick-based monitoring in `run_live_mission()` — collects runtime events every tick (configurable, default 1s)
- [x] Stall detection with 3-rung escalation ladder: corrective prompt → redirect with narrowed scope → force `Failed`
- [x] Liveness tracking: `last_confirmed_alive` timestamp, `consecutive_stall_failures` counter (resets on any output)
- [x] Status snapshot: writes `.sp/control/status.txt` with live session states on each tick
- [x] Protocol reminders: workers producing output but no directives after 3+ chunks receive strict status prompt
- [x] Supervisor health monitoring: `Healthy → Recovering → Degraded` transitions with fallback synthesis
- [x] Supervisor decision queue: queued, deduplicated, mode-aware application
- [x] Max runtime enforcement: `watchdog_max_seconds` hard timeout
- [x] `WatchdogStats` tracking: runtime events, directives, mails routed, validation challenges, stall interventions, lease conflicts, protocol reminders, supervisor health events/fallbacks
- [ ] TUI dashboard wired as interactive dashboard (wired into `main.rs` as startup/fallback dashboard shell; individual modules carry `#![allow(dead_code)]` for unused helpers)

### 4.2 Structured Output Aggregation ✅
- [x] `SAPPHIRE_STATUS` parsing via `protocol.rs` — sanitizes ANSI, buffers partial lines, extracts JSON via brace-depth counter
- [x] Heuristic state detection in adapter layer: keyword-based inference for 8 states (`Validated`, `WeakOutput`, `WrongDirection`, `NeedsValidation`, `Blocked`, `Contradictory`, `Progressing`, `Failed`)
- [x] Per-observation metadata: confidence (High/Medium/Low), raw excerpt, files, blocker, source attribution
- [x] Status snapshots written to `.sp/control/status.txt` on each watchdog tick
- [ ] Live structured output aggregation beyond status.txt (e.g. `.sp/live-status.json` every tick)
- [ ] Final mission report generation when all agents reach terminal states (watchdog generates fallback synthesis in degraded mode)

### 4.3 Team Role Naming (Replace "Worker" Terminology) ✅
- [x] `role_type` field on `WorkerPacket`: stable machine key for template lookup
- [x] `display_name` field on `WorkerPacket`: runtime team label (e.g. `Engineer-1`, `Designer-2`)
- [x] 13 role templates compiled via `include_str!` at build time
- [x] Role assignment based on packet content with dynamic display naming:
  - `software-engineer` → Engineer-1, Engineer-2, ...
  - `designer-engineer` → Designer-1, Designer-2, ...
  - `debug-and-review-engineer` → Reviewer-1, ...
  - `validation-engineer` → Validator-1, ...
  - `architecture-engineer` → Architect-1, ...
  - `security-engineer` → Security-1, ...
  - `testing-and-automation-engineer` → QA-1, ...
  - `research-engineer` → Researcher-1, ...
  - `sales-engineer` → Sales-1, ...
  - `solutions-engineer` → Solutions-1, ...
  - `customer-success-engineer` → CustomerSuccess-1, ...
  - `product-engineer` → Product-1, ...
  - `compliance-engineer` → Compliance-1, ...
- [x] Supervisor prompt updated with role activation instructions
- [x] 13 role template `.md` files in `src/internal/agents/templetes/roles/job-roles/`
- [x] Backward compatible: old JSON without `role_type`/`display_name` infers from `role` field
- [x] "Steward" display name for AGENTS.md steward packet

### 4.4 Stuck/Error Detection ✅
- [x] Heuristic state detection infers `WeakOutput`, `WrongDirection`, `Blocked`, `Contradictory`, `Failed` from output keywords
- [x] Per-agent keyword filtering (Qwen-specific noise filtering for prompt echo)
- [x] Escalation ladder: stuck workers receive corrective prompts, narrowed-scope redirects, or forced `Failed` state
- [x] Supervisor decision queue for stalled workers
- [ ] Deeper stuck detection beyond keyword heuristics (e.g. panic/compilation error detection, same-output-for-3-polls detection)

### 4.5 Live Logging ✅
- [x] `tracing` / `tracing-subscriber` for structured logging throughout the codebase
- [x] Event persistence to SQLite `events` table: all runtime events with timestamps, payloads, worker IDs
- [x] Watchdog stats tracked: `runtime_events`, `directives`, `mails_routed`, `validation_challenges`, `stall_interventions`, `lease_conflicts`, `protocol_reminders`, `supervisor_health_events`, `supervisor_fallbacks`
- [x] Replay query: `sp replay <mission_id>` shows recent events across all workers
- [x] Worker-specific replay: `sp watch <mission_id> <worker>` shows filtered event timeline
- [ ] Dedicated live log viewer / tail command (e.g. `tail -f .sp/watchdog-log.jsonl`)

---

## Phase 4.2 — Supervisor Behavioral Rules + Escalation Ladder ✅ **DONE**

> Extracted from Gas Town reference. Stripped all over-engineering. Kept only what materially improves supervision quality.

### 4.2.1 Propulsion Principle
- [x] Added explicit startup/restart rules to `supervisor-templates/prompt.md`
- [x] Documented failure mode: "supervisor restarts → announces → waits for human → workers idle"
- [x] Rule: check persisted state → act on pending work → then summarize (action first, summary second)

### 4.2.2 Solo Artist Trap
- [x] Added decision tree to supervisor prompt: coordination → do it, implementation → dispatch, trivial → fix directly
- [x] Anti-pattern documented: reading code to "understand the issue" and fixing it burns context needed for team supervision
- [x] Added to Failure Conditions: "doing implementation work yourself while workers sit idle"

### 4.2.3 Escalation Ladder (Consecutive Stall Failures)
- [x] `ActiveSession.consecutive_stall_failures: usize` — tracks consecutive stalls without recovery
- [x] `ActiveSession.last_confirmed_alive: Instant` — liveness timestamp, updated on any output/automation event
- [x] Counter resets on any Output or Automation event (worker responded = not dead, just slow)
- [x] **1st stall**: Corrective status prompt + supervisor decision queued
- [x] **2nd stall**: Redirect with narrowed scope sent immediately + supervisor notice
- [x] **3rd+ stall**: Force `Failed` state + supervisor decides respawn or reassign
- [x] Persisted in stall event JSON (`consecutive_failures` field)
- [x] Test: `escalation_ladder_third_consecutive_stall_fails_worker` — verifies 3rd stall forces Failed
- [x] Test: `consecutive_stall_failures_reset_on_output` — verifies counter resets on liveness

### 4.2.4 What Was Rejected (Over-Engineered)
- [x] Dolt-backed bead tracking → we have SQLite, no second DB needed
- [x] Prefix-based routing across rigs → single-repo, no rigs exist
- [x] Convoy system for batch work → workstreams + worker packets already cover this
- [x] ACP proxy layer → PTY direct, no proxy needed
- [x] Rig dock/undock/park lifecycle → irrelevant for single-repo
- [x] Zombie process scanning → PTY sessions are managed by us
- [x] Heartbeat files on disk → tick-based in-memory monitoring is superior
- [x] 16 Deacon CLI subcommands → watchdog is deterministic code in binary

---

## Phase 4.7 — Gas Town Reliability Patterns ✅ **DONE**

> Extracted from Gas Town reference (decon.md, witness.md, supervise.md, convoy.md). Stripped all Dolt/beads/convoy over-engineering. Kept only what materially improves reliability.

### 4.7.1 Zombie TOCTOU Mitigation (`src/tmux/zombie.rs`)
- [x] `verify_zombie()` — grace period re-check before killing zombies (5s wait, then re-verify)
- [x] Records session creation time, waits, re-checks liveness before kill
- [x] TOCTOU guard: checks if session was replaced by another process between checks
- [x] Prevents false kills during slow agent startup (Qwen takes 3s, Claude 1.5s)
- [x] File: `src/tmux/zombie.rs` (~164 lines)

### 4.7.2 Zombie Debounce (`src/orchestrator/health.rs`)
- [x] `ZombieDebounce` struct — consecutive zombie count before restart (threshold: 3)
- [x] Wired into watchdog loop: `zombie_debounce_check()` called every tick
- [x] Resets on any output arrival (`zombie_debounce.record_alive()`)
- [x] Only triggers restart after 3 consecutive zombie detections (debounces transient gaps)
- [x] From Gas Town supervise.md:1568-1600 (mayorZombieCount pattern)

### 4.7.3 Health Probe Before Stall Escalation (`src/orchestrator/health.rs`)
- [x] `SessionHealthState` — per-session probe/response/failure counters with cooldown
- [x] `health_probe_sessions()` — nudges sessions at 75% of stall threshold before escalation
- [x] Two-tier response: health probe → verify response → escalate (Gas Town decon.md pattern)
- [x] Wired into watchdog loop: called before `handle_stalls()` every tick
- [x] `record_response()` resets consecutive failure counter on any output

### 4.7.4 Message Deduplication (`src/orchestrator/dedup.rs`)
- [x] `MessageDeduplicator` — prevents duplicate mail/directive processing after restart
- [x] Wired into mail directive handling: checks `already_processed()` before routing

---

## Phase 5 — Real Team Coordination ✅ **DONE**

> Agent-to-agent coordination now has real routing structure, bounded memory, pod visibility, and meeting artifacts. This is no longer just raw pairwise mail.

### 5.1 Coordination Fabric ✅
- [x] Coordination topology by role type → pod mapping (`build`, `platform`, `product`, `research`, `revenue`, `executive`)
- [x] Mail governance metadata on live threads: `intent`, `duplicate_key`, `thread_state`, `sender_pod`, `recipient_pod`, `routing_class`
- [x] Duplicate ask suppression per sender/recipient/subject/intent lane
- [x] Backpressure guard for too many open coordination threads per worker
- [x] CC fan-out cap to keep coordination direct instead of noisy
- [x] Thread lifecycle states: `open`, `waiting_ack`, `in_progress`, `answered`, `needs_reroute`, `coordination_failure`, `closed`

### 5.2 Scoped Memory ✅
- [x] Per-worker memory artifact written to `.sp/workers/<display_name>/memory.json`
- [x] Hidden mirror written to `.hide.sp/workers/<display_name>/memory.json`
- [x] Memory captures role, pod, owned scope, current state, current summary, active threads, recent risks, preferred counterparts
- [x] Worker prompt contract updated to read memory from `.sp` first and `.hide.sp` second

### 5.3 Pod Structure ✅
- [x] Live pod summaries generated from active runtime state
- [x] Pod membership, blocked members, and open thread counts shown in status snapshot
- [x] Pod summaries added to supervisor state cards and dashboard surface

### 5.4 Meetings ✅
- [x] Small coordination meetings generated from reroute/failure threads and recovery states
- [x] Meeting artifacts written under `.sp/control/meetings/` and mirrored under `.hide.sp/control/meetings/`
- [x] Dashboard now surfaces active meetings with participants and reason
- [x] Supervisor state card includes active meeting count
- [x] Atomic check-and-set: `!processed.insert(id)` prevents race conditions
- [x] From Gas Town witness.md:725-751 (MessageDeduplicator pattern)
- [x] 6 unit tests

### 4.7.5 Sliding Window Mass Death Detection (`src/orchestrator/health.rs`)
- [x] `MassDeathDetector` — tracks recent session deaths in 30s sliding window
- [x] Emits `tracing::error!` when 3+ sessions die within 30s
- [x] Wired into `Exited` event handler in `handle_runtime_event()`
- [x] Emit cooldown prevents spam (60s minimum between mass death events)
- [x] From Gas Town supervise.md:2389-2436 (massDeathWindow pattern)

### 4.7.6 Session Readiness Verification (`src/tmux/zombie.rs`)
- [x] `wait_for_ready()` — polls pane state until shell is ready (replaces fixed boot delays)
- [x] More reliable than fixed delays — actually verifies process liveness
- [x] From Gas Town witness.md:220-228 (WaitForCommand pattern)

### 4.7.7 What Was Rejected (Over-Engineered)
- [x] Beads/Dolt-backed health state → we have SQLite, no second DB needed
- [x] Convoy manager for work-item queue → workstreams + worker packets already cover this
- [x] Doctor health check framework (pluggable checks) → hardcoded checks simpler for Sapphire
- [x] Help escalation protocol with keyword matching → mail types already cover categorization
- [x] Mayor/polecat/refinery role concepts → Sapphire has 13 role templates, no poles
- [x] Linked pane check → tmux infrastructure, not coordination logic
- [x] PID file locking → tokio cancellation handles shutdown

---

## Phase 4.3 — Shell Script Proof of Reliability ✅ **DONE**

> Everything in `launch-supervisor-grid.sh` that works end-to-end and must be ported to Rust.

### 4.3.1 File-Based Status Reporting (Primary) ✅
- [x] Workers write `.sp/workers/<display_name>/status.json` on every state change
- [x] Watchdog reads status files directly — no ANSI, no scrollback, no regex gymnastics
- [x] `read_worker_state_from_file()` — validates JSON, requires `"state"` key, ignores malformed
- [x] tmux capture-pane with multi-strategy extraction as fallback (single-line regex, brace-depth counter, window search)
- [x] Fallback to "progressing" inference when no directive found but agent is active
- [x] `--test` mode validates file read/write, malformed JSON handling, missing files, missing keys
- [x] **Ported to Rust**: `read_worker_status_files()` in orchestrator, reads `.sp/workers/<name>/status.json` every watchdog tick

### 4.3.2 Readline Wake Nudge ✅
- [x] Send Enter to all panes after boot delay — wakes Qwen's TUI readline loop
- [x] Sleep 1.0s before pasting real prompt — prevents "Queue" indicator
- [x] `load-buffer` from file + paste + 1.0s + Enter + 0.5s between workers
- [x] **Already in Rust**: `startup_input` on all agents in `src/agent/mod.rs`

### 4.3.3 Supervisor Interaction ✅
- [x] Briefing via `load-buffer` from file (not `set-buffer` with shell variable)
- [x] Snapshots sent as human-readable text: "Snapshot #6 — 1 of 2 agents done: {json}"
- [x] Clear response instruction: "SUPERVISOR_MD: **Overall:** ... | **Blockers:** ... | **Next:** ..."
- [x] 3-strategy response capture: exact prefix → keyword search → any substantive line
- [ ] **Port to Rust**: live supervisor briefing uses structured state cards (already functional via `build_supervisor_state_card`)

### 4.3.4 Role Awareness ✅
- [x] Supervisor prompt explains 13 role types with multiple instances (Engineer-1..N)
- [x] Placeholder-based JSON example — no hardcoded role_types
- [x] Explicit: "pick 1 type and spawn all workers as that type, OR distribute across types"
- [x] **Ported to Rust**: `build_supervisor_plan_prompt_impl` updated with 15 rules, placeholder example

### 4.3.5 Grid Layout ✅
- [x] Dynamic cols×rows mapping: 2→2×1, 4→2×2, 11→11×1, 12→4×3
- [x] `even-vertical`, `even-horizontal`, `tiled` layout selection per grid shape
- [x] zsh 1-based array indexing: `pane_idx=$((idx + 1))`, `PANE_IDS[$pane_idx]`

---

## Phase 5 — Agent-to-Agent Communication & 8-Terminal Scale 🚧 **TODO**

> Phase 5 treats this as a **real enterprise team simulation** — not a toy. 8 agents talking, discussing, pushing back, committing — just like a real 8-person engineering team.

### 5.1 Scale to 8 Terminals
- [ ] Expand tmux layout from 2×2 (4 panes) to 2×4 or 4×2 (8 panes)
- [ ] Test pane splitting, prompt injection, and Ghostty attachment at 8-agent scale
- [ ] Verify no race conditions or pane ID collisions at higher scale
- [ ] Adjust boot wait time and prompt injection delays for 8 parallel boots

### 5.2 Agent-to-Agent Mail System (Terminal-to-Terminal)
- [ ] Implement `SAPPHIRE_MAIL` directive support in shell script
- [ ] Agent can send mail to another agent: `SAPPHIRE_MAIL {"to":"Security Engineer","from":"Debug Engineer","type":"dependency_request","message":"Found auth module issue, need review","priority":"high"}`
- [ ] Script parses mail directives, routes to recipient's tmux pane
- [ ] Inject mail into recipient's terminal as a visible message block
- [ ] Recipient acknowledges: `SAPPHIRE_ACK {"thread_id":"...","status":"acked"}`
- [ ] Script tracks unacked mail and escalates to supervisor after timeout

### 5.3 Real Team Communication Patterns
- [ ] **Dependency requests**: "I'm blocked on X, can you prioritize Y?"
- [ ] **Review requests**: "I changed Z, please verify it doesn't break your scope"
- [ ] **Blocker escalation**: "I found a critical issue affecting 3 agents, halting work"
- [ ] **Pushback**: "Your proposed change conflicts with my scope, let's discuss"
- [ ] **Commitment**: "I've completed X, handing off to Y for integration"
- [ ] **Architecture concern**: "The current approach will break under load, suggest redesign"
- [ ] Each pattern maps to a `MailType` and has a structured message format

### 5.4 Complex Task Simulation
- [ ] Design missions that **REQUIRE** inter-agent communication:
  - "Build a REST API with auth, rate limiting, and documentation" (Debug + Security + Docs + Integration must coordinate)
  - "Migrate from sync to async architecture without breaking tests" (all agents touch shared files)
  - "Add a plugin system with backward compatibility" (architecture discussion required)
- [ ] Tasks should force agents to: claim leases on overlapping files, request dependencies, negotiate scope, push back on unsafe changes
- [ ] Measure: how many mail exchanges, how many conflicts, how many pushbacks, final outcome quality

### 5.5 Conflict Resolution Under Tension
- [ ] Simulate real team tension: two agents claim the same file
- [ ] Script detects lease conflict → notifies both agents + supervisor
- [ ] Challenger receives: "Agent X already owns this file. Request transfer via mail or pick alternative."
- [ ] Track conflict resolution time and outcome quality
- [ ] Test: does the team self-resolve or does the supervisor need to intervene?

### 5.6 Supervisor as Team Lead
- [ ] Supervisor receives all mail CC'd for visibility
- [ ] Supervisor can broadcast directives: "Priority shift — Security issue takes precedence"
- [ ] Supervisor mediates conflicts: "Debug Engineer owns auth.rs, Security Engineer reviews only"
- [ ] Supervisor generates a real-time team health report: morale, blockers, velocity

---

## Phase 4.8 — Supervisor Repair & Continuity ✅ **DONE**

> The CEO can fail; the company cannot. Live runs now keep a repair supervisor warm and preserve team continuity.

- [x] Launch a standby repair supervisor (`supervisor-02-repair`) for every live mission
- [x] Track `active_supervisor_id` so only the acting supervisor can issue actions or final summaries
- [x] Take over automatically when the primary supervisor becomes unavailable or unhealthy
- [x] Keep the standby supervisor synchronized with state cards while passive
- [x] Notify workers once about peer continuity when supervision changes
- [x] Show active + standby supervisor in status snapshots and dashboard views
- [x] Increase supervisor restart budget to reduce CEO single-point-of-failure risk

---

## Phase 5 — Real Agent-to-Agent Communication 🚧 **IN PROGRESS**

> Real teams solve complex work by coordinating laterally, not by waiting for the CEO for every dependency. Sapphire needs the same behavior.

### 5.1 Why This Matters
- [x] Document the principle: worker-to-worker coordination is the default path for dependencies, reviews, handoffs, and unblock requests
- [x] Treat supervisor involvement as the escalation path for rulings, contradictions, or failed peer coordination
- [x] Tighten worker prompts so teammate-first coordination is part of the runtime contract, not an optional suggestion

### 5.2 What Is Already Real
- [x] Durable `SAPPHIRE_MAIL` routing with thread IDs, priorities, CC visibility, ack tracking, and SQLite persistence
- [x] Non-destructive queue delivery for normal mail and direct interrupt delivery for urgent mail
- [x] Explicit ack semantics: `acked`, `done`, `cannot_comply`
- [x] Multi-stage timeout ladder: recipient probe → reroute warning → coordination-failure escalation
- [x] `cannot_comply` now becomes a real supervisor-visible blocker instead of a silent ack variant
- [x] Worker prompts now say: teammate first, supervisor second

### 5.3 Next Mail Upgrades
- [ ] Add thread-level conversation summaries to reduce context bloat on long coordination chains
- [ ] Add stronger review / dependency request templates so workers ask for high-signal evidence, not vague help
- [ ] Track mail outcome quality: resolved, rerouted, cannot_comply, abandoned
- [ ] Add integration tests for 2-worker and 8-worker coordination flows
- [ ] Stress-test complex missions where completion is impossible without lateral coordination

### 5.4 Team-Simulation Goal
- [ ] Prove that two workers can solve a read-only analysis task by talking to each other first
- [ ] Prove that larger teams can split dependencies without collapsing into supervisor spam
- [ ] Prove that failed peer coordination escalates cleanly instead of looping forever

---

## Phase 6 — Scale to 16 Terminals & Full Stress Testing 🚧 **TODO**

> Phase 6 is the ultimate stress test: 16 agents, ultra-complex missions, everything from Phases 4–5 combined, iterating and compounding.

### 6.1 16-Terminal Orchestration
- [ ] Expand tmux layout to 4×4 grid (16 panes)
- [ ] Test pane splitting, boot sequence, and prompt injection at 16-agent scale
- [ ] Verify no deadlocks, no pane starvation, no prompt injection failures
- [ ] Ghostty: test if it can handle 16 attached sessions (may need multiple windows)

### 6.2 Ultra-Complex Mission Simulation
- [ ] Design missions that require all 16 agents to coordinate:
  - "Build a full-stack e-commerce platform: frontend, backend, auth, payments, inventory, notifications, docs, tests, CI/CD, security audit, performance optimization"
  - "Migrate a monolith to microservices: split 8 modules, add inter-service communication, maintain backward compatibility, update docs, write migration guide"
- [ ] Each agent gets a tightly scoped packet with clear dependencies on 2–3 other agents
- [ ] Mission should be impossible to complete without agent-to-agent communication

### 6.3 Full Iteration Pipeline
- [ ] Agent produces output → watchdog captures → formats → aggregates → reports
- [ ] Supervisor reviews aggregated output → issues corrections → agents iterate
- [ ] Multiple rounds of iteration until all agents reach `done_claimed` → `validated`
- [ ] Track iteration count per agent, total mission time, final quality score

### 6.4 Stress Testing Metrics
- [ ] How many mail exchanges occur? (target: 20+)
- [ ] How many lease conflicts? (target: 3–5, all self-resolved)
- [ ] How many pushbacks? (target: 5+, all resolved through discussion)
- [ ] How many supervisor interventions? (target: <5, system should self-manage)
- [ ] Total mission time? (target: <30 minutes for 16 agents)
- [ ] Final output quality? (target: all agents validated, no regressions)

### 6.5 Failure Mode Testing
- [ ] Simulate agent crashes: kill a mid-work agent, verify others adapt
- [ ] Simulate stuck agents: inject a blocker that prevents progress, verify escalation
- [ ] Simulate supervisor failure: supervisor stalls, verify watchdog takes over
- [ ] Simulate mail loss: drop a mail message, verify ack timeout triggers resend
- [ ] Document all failure modes and recovery paths

---

## Phase 7 — Port to Rust Runtime (`sp` Binary) 🚧 **IN PROGRESS**

> Everything proven in shell scripts gets ported into the real `sp` Rust codebase. Shell scripts were the test harness; Rust is the product.

### 7.1 Audit Current `src/` State ✅
- [x] Identify all modules that are broken/untested vs. working
- [x] Cross-reference shell script capabilities with Rust implementations
- [x] Document gaps: what works in shell but not in Rust
- [x] Prioritize: which gaps block real usage vs. nice-to-have

### 7.2 Port Supervisor Planning (Stdin Pipe + Markdown Fallback) ✅
- [x] `plan_with_supervisor_qwen_pipe` with stdin pipe approach
- [x] Strict JSON prompting rules (15 rules in supervisor template)
- [x] Adapter handles both marker-based and markdown-wrapped JSON
- [x] Role awareness: 13 role types, multiple instances, placeholder-based example
- [x] Word-limit enforcement in supervisor prompt template

### 7.3 Port Live Watchdog Loop ✅
- [x] Tick-based monitoring in `run_live_mission()` — already in Rust
- [x] `SAPPHIRE_STATUS` parsed from each worker's PTY output in real-time
- [x] **File-based status as PRIMARY**: `read_worker_status_files()` reads `.sp/workers/<name>/status.json` every tick
- [x] Structured aggregation layer: `WatchdogView`, `LogEntry`, `HealthView` in TUI
- [x] Live status snapshots written to `.sp/control/status.txt` on each tick

### 7.4 Port Agent-to-Agent Mail ✅
- [x] Implement `handle_mail_directive()` in orchestrator
- [x] Parse `SAPPHIRE_MAIL` from worker PTY output
- [x] Route mail to recipient via queue or direct injection depending on urgency
- [x] Track ack timeouts with staged escalation
- [x] Persist all mail to SQLite `messages` table
- [x] Support explicit `acked`, `done`, and `cannot_comply` semantics
- [x] Support scavenge claim / release flows
- [x] Supervisor visibility on coordination failure

### 7.5 Port Lease System ✅
- [x] Implement `handle_lease_directive()` in orchestrator
- [x] Parse `SAPPHIRE_LEASE` from worker output
- [x] Detect conflicts, downgrade challenger to `Contradictory`
- [x] Notify owner and supervisor of conflicts
- [x] Persist leases to SQLite `ownership_leases` table

### 7.6 Port Role Naming System ✅
- [x] `role_type` and `display_name` fields on `WorkerPacket`
- [x] Supervisor prompt template generates role names with multiple instances awareness
- [x] All UI/logging displays role names (Engineer-1, Designer-2, etc.) instead of "worker-01"
- [x] 13 role templates compiled via `include_str!`
- [x] File-based status instructions in worker prompts

### 7.7 Port tmux / Ghostty Integration ✅
- [x] Pre-split all panes before launching agents
- [x] Grid-aware layout selection
- [x] `open_ghostty_window_for_session()` — AppleScript approach
- [x] `open_ghostty_tab_for_session_applescript()` — tab support
- [x] Max 10 workers per tab, auto-create extra sessions
- [x] `create_session_for_workers`, `list_pane_ids`, `send_command`

### 7.8 Port TUI Dashboard ✅
- [x] Two-color theme: off-white grey + light bright purple
- [x] Structured data views: `WorkerView`, `HealthView`, `WatchdogView`, `LogEntry`
- [x] 4 clean sidebar tabs: Workers, Watchdog, Events, Supervisor
- [x] Watchdog tab shows all 9 stats + health + live log (last 20)
- [x] Main pane (70%): supervisor markdown with full rendering
- [x] Clean event bodies: `clean_event_body()` strips JSON noise
- [x] Markdown rendering: headings, tables, code blocks, quotes, lists

### 7.9 Integration Testing 🚧 TODO
- [ ] Create integration test harness around PTY orchestration
- [ ] Test: 4-agent mission with supervisor planning, file-based status, watchdog
- [ ] Test: 8-agent mission with conflicts, pushbacks, supervisor intervention
- [ ] Test: agent crash recovery, supervisor degradation, mail loss
- [ ] All tests must pass before declaring Phase 7 complete

### 7.10 Final Polish
- [x] Update README with new capabilities
- [x] Update AGENTS.md with full architecture documentation
- [x] Update AGENT-HANDOFF.md with complete context
- [ ] Update SP-USAGE.md with new CLI flags and commands
- [ ] Build release binary, verify end-to-end on clean machine
- [ ] Tag v1.0 release
