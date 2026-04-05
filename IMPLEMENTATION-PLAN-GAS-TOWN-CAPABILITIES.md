# Implementation Plan: Gas Town Capabilities → Sapphire Orchestration

> **Rule:** Zero over-engineering. Only extract the materially superior patterns. Reject all Dolt/beads/convoy/wisp/town/rig bureaucracy.
> **Principle:** Every change must provide direct material benefit to watchdog reliability, recovery speed, or operator visibility.

---

## Phase Split

| Phase | Capabilities | Goal |
|---|---|---|
| **Phase 1** | 1. Health State + Cooldown, 2. Zombie Detection, 3. Restart Tracker + Backoff, 4. Crash Loop Detection | Make the watchdog **smarter** — cooldowns, zombie kills, crash loop detection, persistent state |
| **Phase 2** | 5. Nudge Queue, 6. Auto-Respawn Hook, 7. Problems View (TUI), 8. Mass Death Detection Enhancement | Make the watchdog **faster** and the UI **sharper** |

---

# Phase 1: Watchdog Intelligence

## 1. Health State Tracker with Cooldown

### What to add to `ActiveSession`
- `intervention_cooldown_until: Option<Instant>` — after ANY watchdog action (stall prompt, redirect, validation challenge), set a cooldown. During cooldown, skip redundant interventions.
- `last_intervention_type: Option<String>` — what was the last intervention
- `intervention_count: usize` — lifetime count (escalates threshold)
- `last_response_time: Option<Duration>` — how long from intervention to first output

### Reference: Go Health State Tracking
**Read:** `reference-snippets/health/health.md` lines **300–420** (`AgentHealthState` struct + `RecordPing`/`RecordResponse`/`RecordFailure`/`RecordForceKill`/`IsInCooldown`/`CooldownRemaining`)

The critical pattern:
```go
type AgentHealthState struct {
    LastPingTime         time.Time
    LastResponseTime     time.Time
    ConsecutiveFailures  int
    LastForceKillTime    time.Time
    ForceKillCount       int
}

func (s *AgentHealthState) RecordResponse() {
    s.LastResponseTime = time.Now().UTC()
    s.ConsecutiveFailures = 0  // RESET on response
}

func (s *AgentHealthState) RecordForceKill() {
    s.LastForceKillTime = time.Now().UTC()
    s.ForceKillCount++
    s.ConsecutiveFailures = 0  // RESET after kill
}

func (s *AgentHealthState) IsInCooldown(cooldown time.Duration) bool {
    if s.LastForceKillTime.IsZero() { return false }
    return time.Since(s.LastForceKillTime) < cooldown
}
```

### Files to change
1. **`src/orchestrator/mod.rs`** — Add 4 fields to `ActiveSession`:
   ```rust
   intervention_cooldown_until: Option<Instant>,
   last_intervention_type: Option<String>,
   intervention_count: usize,
   total_interventions: usize,
   ```
2. **`src/orchestrator/mod.rs`** — In `handle_stalls()`, before escalating, check `intervention_cooldown_until`. If `Some(now)` and `now < cooldown_until`, skip. After intervention, set `Some(now + cooldown)`.
3. **`src/orchestrator/mod.rs`** — In `handle_pending_supervisor_decisions()`, same cooldown check before fallback.
4. **`src/orchestrator/mod.rs`** — In `handle_runtime_event()`, on Output/Automation: update `last_response_time`, reset `consecutive_stall_failures`, update `intervention_count` tracking.
5. **`src/orchestrator/mod.rs`** — Add constants:
   ```rust
   const DEFAULT_INTERVENTION_COOLDOWN: Duration = Duration::from_secs(30);
   const MAX_INTERVENTION_COOLDOWN: Duration = Duration::from_secs(120);
   ```
6. **`register_session()`** — Initialize new fields to defaults.

### Behavior
- After any watchdog intervention (stall prompt, redirect, validation challenge, low-confidence correction), set cooldown to `30s × intervention_count` (capped at `120s`)
- During cooldown: watchdog skips redundant interventions for that worker
- On output: reset `consecutive_stall_failures`, record response time
- Persist `total_interventions` to stall event JSON

---

## 2. Zombie Detection

### What to add
- Distinguish "tmux session exists but agent process dead" from "agent truly dead"
- TOCTOU grace period: don't kill a session that just became healthy
- Zombie status reporting in `write_status_snapshot()`

### Reference: Go Zombie Detection
**Read:** `reference-snippets/health/health.md` lines **50–95** (`Start()` method — zombie detection with grace period)

The critical pattern:
```go
running, _ := t.HasSession(sessionID)
if running {
    if t.IsAgentAlive(sessionID) {
        return ErrAlreadyRunning  // Healthy
    }
    // Zombie: tmux alive but agent dead
    createdAt, _ := t.GetSessionCreatedUnix(sessionID)
    time.Sleep(constants.ZombieKillGracePeriod)  // TOCTOU grace
    if t.IsAgentAlive(sessionID) {
        return ErrAlreadyRunning  // Became healthy during grace
    }
    if createdNow, _ := t.GetSessionCreatedUnix(sessionID); createdAt > 0 && createdNow != createdAt {
        return ErrAlreadyRunning  // Session was replaced by another process
    }
    t.KillSession(sessionID)
}
```

**Read:** `reference-snippets/Supervisor/witness.md` lines **42–60** (`IsHealthy()` and `IsRunning()`)

The critical pattern:
```go
func (m *Manager) IsRunning() (bool, error) {
    t := tmux.NewTmux()
    status := t.CheckSessionHealth(m.SessionName(), 0)
    return status == tmux.SessionHealthy, nil
}

func (m *Manager) IsHealthy(maxInactivity time.Duration) tmux.ZombieStatus {
    t := tmux.NewTmux()
    return t.CheckSessionHealth(m.SessionName(), maxInactivity)
}
```

### Files to change
1. **`src/tmux/mod.rs`** — Add `check_session_health(session: &str, max_inactivity: Duration) -> SessionHealth` method:
   ```rust
   pub enum SessionHealth {
       Healthy,           // tmux session + agent process alive + recent output
       Zombie,            // tmux session exists but agent process dead
       Hung,              // agent alive but no output for > max_inactivity
       Dead,              // no tmux session
       Starting,          // recently created, within grace period
   }
   ```
2. **`src/tmux/mod.rs`** — Implement using `tmux list-panes -F "#{pane_pid}"` to get PID, then check if PID is alive via `kill(pid, 0)` or `/proc/pid/status`.
3. **`src/orchestrator/mod.rs`** — In `handle_supervisor_health()`, use `check_session_health()` instead of just `last_output_at` elapsed. Differentiate zombie (kill + restart) from hung (send health check prompt).
4. **`src/orchestrator/mod.rs`** — In `write_status_snapshot()`, report zombie/hung/starting status separately.
5. **`src/tmux/mod.rs`** — Add `get_session_created(session: &str) -> Option<Instant>` using `tmux display-message -p "#{session_created}"`.
6. **Add TOCTOU grace constant:**
   ```rust
   const ZOMBIE_KILL_GRACE_PERIOD: Duration = Duration::from_secs(2);
   ```

---

## 3. Restart Tracker with Exponential Backoff (Persistent)

### What to add
- Persist restart attempts to SQLite (survives orchestrator restart)
- Exponential backoff with configurable params
- Crash loop detection: N restarts within M minutes → escalate

### Reference: Go Restart Tracker
**Read:** `reference-snippets/decon-background-processsor/supervisor.md` lines **85–105** (Daemon struct fields) + search for `RestartTracker` in that file

The critical pattern:
```go
// In Daemon struct:
restartTracker *RestartTracker

// Initialization with configurable params:
var rtCfg RestartTrackerConfig
if patrolConfig != nil && patrolConfig.Patrols != nil && patrolConfig.Patrols.RestartTracker != nil {
    rtCfg = *patrolConfig.Patrols.RestartTracker
}
restartTracker := NewRestartTracker(config.TownRoot, rtCfg)
if err := restartTracker.Load(); err != nil {
    logger.Printf("Warning: failed to load restart state: %v", err)
}
```

**Also read:** `reference-snippets/Supervisor/convoy.md` lines **1–35** (exponential backoff pattern in `pollStoresSnapshot`)

The critical pattern:
```go
if hadError {
    newInterval := currentInterval * 2
    if newInterval > eventPollMaxBackoff {
        newInterval = eventPollMaxBackoff
    }
    ticker.Reset(currentInterval)
} else if currentInterval != eventPollInterval {
    currentInterval = eventPollInterval  // Reset on success
}
```

### Files to change
1. **`src/store/mod.rs`** — Add `session_restarts` table:
   ```sql
   CREATE TABLE session_restarts (
       id TEXT PRIMARY KEY,
       session_id TEXT NOT NULL REFERENCES workers(id),
       mission_id TEXT NOT NULL REFERENCES sessions(id),
       restart_count INTEGER NOT NULL,
       first_restart_at TEXT NOT NULL,
       last_restart_at TEXT NOT NULL,
       backoff_seconds REAL NOT NULL,
       escalated INTEGER NOT NULL DEFAULT 0
   );
   ```
2. **`src/store/mod.rs`** — Add CRUD: `upsert_restart_attempt(session_id, mission_id)`, `load_restart_state(session_id)`, `reset_restart_tracker(session_id)`, `get_crash_loop_sessions(mission_id, threshold, window)`
3. **`src/orchestrator/mod.rs`** — Replace hardcoded `restart_backoff()` with persistent tracker. On restart:
   - Load persisted state from SQLite
   - Calculate backoff: `base × 2^(count-1)` capped at max
   - If N restarts within M minutes → mark as crash loop, escalate to supervisor, don't auto-restart
4. **`src/orchestrator/mod.rs`** — Add constants:
   ```rust
   const RESTART_BASE_SECS: u64 = 2;
   const RESTART_MAX_SECS: u64 = 300;  // 5 min
   const RESTART_CRASH_LOOP_THRESHOLD: usize = 5;
   const RESTART_CRASH_LOOP_WINDOW: Duration = Duration::from_secs(600);  // 10 min
   ```
5. **`src/orchestrator/mod.rs`** — In `handle_runtime_event()` for `Exited`, before scheduling restart:
   - Check crash loop → if detected, mark `Failed`, notify supervisor
   - Otherwise, upsert restart attempt, compute backoff, schedule
6. **`src/orchestrator/mod.rs`** — On successful output after restart, reset the tracker for that session.

---

## 4. Crash Loop Detection + Mass Death Enhancement

### What to add
- Enhance existing `mass_failure_threshold`/`mass_failure_window` with crash loop awareness
- When crash loop detected, include in supervisor notice: "X restarts in Y minutes, escalating"
- Add `crash_loop` to `WatchdogStats`

### Reference: Go Mass Death Detection
**Read:** `reference-snippets/decon-background-processsor/supervisor.md` lines **110–130** (mass death params)

The critical pattern:
```go
const (
    massDeathWindow    = 30 * time.Second
    massDeathThreshold = 3
)

type sessionDeath struct {
    sessionName string
    timestamp   time.Time
}

func trim_recent_failures(recent_failures: &mut Vec<RecentFailure>) {
    let cutoff = Instant::now() - mass_failure_window();
    recent_failures.retain(|entry| entry.recorded_at >= cutoff);
}
```

### Files to change
1. **`src/orchestrator/mod.rs`** — In existing `handle_runtime_event()` mass failure section, add crash loop context:
   - When mass failure detected, also check if any of those sessions are in crash loop state
   - Include crash loop info in supervisor notice: "3 workers died, 2 of them were in crash loops"
2. **`src/orchestrator/mod.rs`** — Add `crash_loops_detected: usize` to `WatchdogStats`
3. **`src/orchestrator/mod.rs`** — In `write_status_snapshot()`, add crash loop section:
   ```
   Crash Loops: Engineer-1 (5 restarts in 8min), Designer-2 (6 restarts in 10min)
   ```
4. **`src/model.rs`** — Add `crash_loop` to session state metadata (not a new state, just a flag in event JSON)
5. **`src/orchestrator/mod.rs`** — Add `should_escalate_to_supervisor_crash_loop()` helper that checks restart tracker + mass failure window together

---

# Phase 2: Speed + Visibility

## 5. Nudge Queue (Wait-Idle Prompt Injection)

### What to add
- Before `send_prompt()`, check if agent is mid-response (recent output in buffer)
- If mid-response: queue the prompt instead of injecting immediately
- Queued prompts fire when output goes quiet for > 3s

### Reference: Go Nudge Wait-Idle
**Read:** `reference-snippets/Supervisor/decon.md` lines **560–580** (nudge protocol mention) — but the real pattern is conceptual: wait for agent to be at a prompt before injecting.

There's no exact Go code for this in the snippets, so the pattern is:
```
1. Check if recent output in carry buffer (last 3s had activity)
2. If yes → queue prompt
3. If no → send immediately
4. On each watchdog tick: check queued prompts, fire if quiet > 3s
```

### Files to change
1. **`src/orchestrator/mod.rs`** — Add to `ActiveSession`:
   ```rust
   queued_prompts: VecDeque<String>,
   ```
2. **`src/orchestrator/mod.rs`** — Add `is_agent_mid_response(session: &ActiveSession) -> bool`:
   ```rust
   fn is_agent_mid_response(&self, session: &ActiveSession) -> bool {
       session.last_output_at.elapsed() < Duration::from_secs(3)
           && session.output_chunks > 0
   }
   ```
3. **`src/orchestrator/mod.rs`** — In watchdog tick loop, before sending any prompt via `send_prompt()`:
   - If `is_agent_mid_response(session)`, push to `queued_prompts` instead
   - After processing all events, drain queued prompts for sessions that are now quiet
4. **`src/orchestrator/mod.rs`** — In `handle_protocol_reminders()`, `handle_stalls()`, `handle_pending_supervisor_decisions()` — all prompt sends go through the queue check.
5. **`src/runtime/mod.rs`** — No changes needed. `send_prompt()` remains as-is. Queue is managed at orchestrator level.

---

## 6. Auto-Respawn Hook (tmux-level)

### What to add
- Set `remain-on-exit` on tmux panes immediately after creation
- Use tmux's `set-hook` to auto-respawn the command when process exits
- Faster recovery than 1s watchdog polling (instant tmux-level respawn)

### Reference: Go Auto-Respawn
**Read:** `reference-snippets/health/health.md` lines **120–145** (`SetAutoRespawnHook` usage)

The critical pattern:
```go
// PATCH-010: Set remain-on-exit IMMEDIATELY after session creation.
_ = t.SetRemainOnExit(sessionID, true)

// PATCH-010: Set auto-respawn hook for Deacon resilience.
// When Claude exits (for any reason), tmux will automatically respawn it.
if err := t.SetAutoRespawnHook(sessionID); err != nil {
    fmt.Printf("warning: failed to set auto-respawn hook for deacon: %v\n", err)
}
```

**Read:** `reference-snippets/real-time/md.md` lines **435–460** (`RemainOnExit`, `AutoRespawn` in `SessionConfig`)

The critical pattern:
```go
type SessionConfig struct {
    RemainOnExit bool   // Sets remain-on-exit immediately after session creation
    AutoRespawn  bool   // Sets the auto-respawn hook so the session survives crashes
}

// In StartSession():
if cfg.RemainOnExit {
    _ = t.SetRemainOnExit(cfg.SessionID, true)
}
if cfg.AutoRespawn {
    if err := t.SetAutoRespawnHook(cfg.SessionID); err != nil {
        fmt.Printf("warning: failed to set auto-respawn hook for %s: %v\n", cfg.Role, err)
    }
}
```

### Files to change
1. **`src/tmux/mod.rs`** — Add `set_auto_respawn_hook(session: &str, command: &str) -> Result<(), String>`:
   ```rust
   pub fn set_auto_respawn_hook(&self, session: &str, command: &str) -> Result<(), String> {
       // tmux set-hook -t SESSION "pane-exited" "respawn-pane -k -t #{pane_id} -c '#{pane_current_path}' -- COMMAND"
       self.run(&["set-hook", "-t", session, "pane-exited",
           &format!("respawn-pane -k -t #{{pane_id}} -c '#{{pane_current_path}}' -- {}", command)])
   }
   ```
2. **`src/tmux/mod.rs`** — Add `set_remain_on_exit(pane: &str, on: bool) -> Result<(), String>`
3. **`src/runtime/mod.rs`** — In `TmuxBackend::spawn()`, after pane creation:
   - Call `set_remain_on_exit()` on the pane
   - Call `set_auto_respawn_hook()` with the agent command
4. **`src/orchestrator/mod.rs`** — In `handle_pending_restarts()`, after spawning, set the auto-respawn hook
5. **`src/orchestrator/mod.rs`** — In `handle_runtime_event()` for `Exited`, check if tmux auto-respawn already handled it (pane still exists with new PID) before scheduling Rust-level restart. If tmux respawned, just update state instead of spawning a new session.

---

## 7. Problems View (TUI Dashboard)

### What to add
- Add a "Problems" filter to the TUI: show ONLY workers that need attention
- Criteria: stalled, blocked, contradictory, low-confidence (2+), crash loop, unacked mail > 30s
- Keyboard toggle: press `p` to switch between "all workers" and "problems only"

### Reference: Go Problems View
**Read:** `reference-snippets/real-time/md.md` lines **1–80** (feed command with `--problems` flag)

The critical pattern:
```go
feedCmd.Flags().BoolVarP(&feedProblems, "problems", "p", false, "Start in problems view (shows stuck agents)")

// In runFeed:
if problemsView {
    m = feed.NewModelWithProblemsView(bd)
} else {
    m = feed.NewModel(bd)
}
```

The TUI shows:
```
Agent state symbols (problems view):
  🔥  GUPP violation   - Hooked work + 30m no progress (critical)
  ⚠   STALLED          - Hooked work + 15m no progress
  ●   Working          - Actively producing output
  ○   Idle             - No hooked work
  💀  Zombie           - Dead/crashed session
```

### Files to change
1. **`src/tui/state.rs`** — Add `problems_only: bool` to `AppState`
2. **`src/tui/state.rs`** — Add `SidebarTab::Problems` or toggle on existing `Watchdog` tab
3. **`src/tui/data.rs`** — Add `load_problems()` method: filter workers by:
   ```rust
   fn needs_attention(session: &SessionRecord) -> bool {
       matches!(session.state, "stalled" | "blocked" | "contradictory" | "failed")
   }
   ```
4. **`src/tui/render.rs`** — Render problems view with severity icons:
   - 🔴 `Failed` / crash loop
   - 🟡 `Stalled` / blocked
   - 🟠 `Contradictory` / low-confidence
5. **`src/tui/app.rs`** — Add `p` key binding to toggle problems view
6. **`src/orchestrator/mod.rs`** — In `write_status_snapshot()`, add a `problems` section to status.txt for the tmux dashboard to also show

---

## 8. Mass Death Detection Enhancement

### What to add
- We already have `mass_failure_threshold` (3) and `mass_failure_window` (30s)
- **Enhancement:** When crash loop detection (Phase 1 #4) + mass death overlap → escalate to "CRITICAL"
- Add `critical_failure` event type when mass death + crash loops coincide
- Include crash loop context in mass death supervisor notice

### Reference: Go Mass Death + Recovery
**Read:** `reference-snippets/decon-background-processsor/supervisor.md` lines **110–130** (mass death constants)

### Files to change
1. **`src/orchestrator/mod.rs`** — In existing `handle_runtime_event()` mass failure section (around `mass_failure_threshold`/`mass_failure_window`), add:
   - Check if any of the dead sessions were in crash loop state (from restart tracker)
   - If mass death + crash loops → emit `critical_failure` event instead of just `mass_failure_detected`
   - Include restart counts in supervisor notice
2. **`src/orchestrator/mod.rs`** — Add `critical_failures: usize` to `WatchdogStats`
3. **`src/orchestrator/mod.rs`** — In `write_status_snapshot()`, add critical failure section

---

# Execution Instructions for the AI Agent

## How to Read Gas Town Code Efficiently

**DO NOT read entire Go files.** They are massive (4000+ lines) and full of Dolt/beads/convoy bureaucracy we don't want.

**DO read only the specific line windows listed above** for each capability. Those windows contain ONLY the superior pattern without the over-engineering.

**Reference file mapping:**
| Capability | File to Read | Line Range |
|---|---|---|
| Health State + Cooldown | `reference-snippets/health/health.md` | 300–420 |
| Zombie Detection | `reference-snippets/health/health.md` | 50–95 |
| Zombie Detection (IsHealthy) | `reference-snippets/Supervisor/witness.md` | 42–60 |
| Restart Tracker | `reference-snippets/decon-background-processsor/supervisor.md` | 85–105 |
| Exponential Backoff | `reference-snippets/Supervisor/convoy.md` | 1–35 |
| Mass Death | `reference-snippets/decon-background-processsor/supervisor.md` | 110–130 |
| Auto-Respawn Hook | `reference-snippets/health/health.md` | 120–145 |
| Session Lifecycle | `reference-snippets/real-time/md.md` | 435–460 |
| Problems View | `reference-snippets/real-time/md.md` | 1–80 |

## Files That Will Change

### Phase 1 Files
| File | What Changes |
|---|---|
| `src/orchestrator/mod.rs` | ActiveSession fields (+4), cooldown checks, crash loop detection, restart tracker integration, mass death enhancement |
| `src/store/mod.rs` | New `session_restarts` table + CRUD methods |
| `src/tmux/mod.rs` | `check_session_health()`, `get_session_created()`, zombie detection helpers |
| `src/model.rs` | Minor: add `crash_loop` to event metadata types |

### Phase 2 Files
| File | What Changes |
|---|---|
| `src/orchestrator/mod.rs` | Nudge queue, problems snapshot in status.txt, crash loop + mass death overlap |
| `src/tmux/mod.rs` | `set_auto_respawn_hook()`, `set_remain_on_exit()` |
| `src/runtime/mod.rs` | Auto-respawn hook setup in `TmuxBackend::spawn()` |
| `src/tui/state.rs` | `problems_only` toggle, problems view state |
| `src/tui/data.rs` | `load_problems()` method |
| `src/tui/render.rs` | Problems view rendering with severity icons |
| `src/tui/app.rs` | `p` key binding for toggle |

## Invariants to Preserve

1. **Do not break the 4 directive protocol** (SAPPHIRE_STATUS, SAPPHIRE_MAIL, SAPPHIRE_ACK, SAPPHIRE_LEASE)
2. **Do not change `SessionState` enum** — add metadata, not new states
3. **Do not add new dependencies** beyond what's already in `Cargo.toml`
4. **Preserve backward compatibility** — all existing tests must pass
5. **Do not create new tables without migration** — `session_restarts` must have graceful schema creation
6. **Keep `src/orchestrator/mod.rs` split discipline** — if adding >50 lines to a function, extract a helper
7. **All new fields on `ActiveSession` must be initialized in `register_session()`**
8. **Cooldown must NOT block terminal state transitions** — if worker is `Failed`, cooldown doesn't prevent supervisor notice
