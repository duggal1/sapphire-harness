# Agent Handoff — Sapphire Agent Factory (sp)

> Date: 2026-04-05
> Purpose: Complete context for any agent picking up this codebase.
> Every bug found, every fix applied, every working/non-working approach documented.

---

## What This Project Is

`sp` — A Rust CLI that orchestrates multiple coding agents (Qwen, Claude, Codex, Forge) in parallel. It:
1. Plans a mission (decompose into workstreams)
2. Activates role-based agents based on task analysis (coding-only → Software Engineers; multi-domain → Software + Design + Security etc.)
3. Opens a tmux session with one pane per agent in interactive mode
4. Opens a visible Ghostty window attached to that tmux session
5. Watches agents for status directives, handles stalls, mail, leases, conflicts
6. Persists everything to `.sp/sapphire.sqlite3`
7. **Engineers talk like engineers** — nudge queue (non-destructive delivery), scavenge claim/release, 5 clean message types, engineering-semantic rendering

---

## Architecture Evolution

### Phase 1 — Basic Terminal Launcher ✅ DONE
Proven working via `launch-supervisor-grid.sh`: tmux splitting, interactive agent boot, prompt injection, Ghostty attachment.

### Phase 2 — Supervisor Dispatch ✅ DONE
JSON plan extraction from supervisor → per-agent prompt decomposition. Supervisor plans, decomposes, and dispatches work autonomously.

### Phase 3 — Full Orchestration ✅ DONE
End-to-end: 4 Qwen agents launched in interactive mode via tmux 2×2 grid. All 4 emitted `SAPPHIRE_STATUS {"state":"done_claimed"}` independently. Builds pass, 77 tests pass. Protocol contract honored.

### Phase 4 — Function-First Team Roles + Stall Escalation ✅ DONE
Replaced generic `worker-01` IDs with enterprise engineering team roles. Supervisor activates only relevant role types based on task analysis. 13 roles available. Each agent has a full job description template. **Added escalation ladder**: consecutive stall failures (1st: corrective prompt, 2nd: redirect with narrowed scope, 3rd+: force Failed). Supervisor prompt now includes Propulsion Principle (auto-execute on restart) and Solo Artist Trap (dispatch, don't implement). All 79 tests pass.

### Phase 4.2 — Supervisor Behavioral Rules + Escalation Ladder ✅ DONE
Extracted three superior capabilities from Gas Town reference (stripping all over-engineering):
1. **Propulsion Principle** — Supervisor auto-executes pending work on restart, never waits for human approval. Documented failure mode: "supervisor restarts → announces → waits for human → workers idle → mission stalls."
2. **Solo Artist Trap** — Supervisor has explicit decision tree: coordination work → do it yourself; implementation → dispatch to worker. Anti-pattern called out: reading code to "understand the issue" and fixing it burns context needed for team supervision.
3. **Escalation Ladder** — `consecutive_stall_failures` counter on `ActiveSession` (reset on any output/automation event). Three tiers:
   - **1st**: Corrective status prompt + supervisor decision queued
   - **2nd**: Redirect with narrowed scope sent immediately + supervisor notice
   - **3rd+**: Watchdog forces `Failed` state, supervisor decides respawn or reassign

Files changed: `supervisor-templates/prompt.md` (2 new sections + failure conditions), `src/orchestrator/mod.rs` (escalation ladder + 2 new ActiveSession fields + 2 new tests), test count 77 → 79.

### Phase 4.3 — 3-Layer Prompt Injection Reliability ✅ DONE
All 4 agents (Qwen, Claude, Codex, Forge) now have 3 independent prompt delivery layers — each alone would be sufficient, together they're bulletproof:
1. **PTY `send_prompt()`** (existing): Full role template + assignment written to PTY master fd after per-agent boot delay.
2. **Startup `\n` nudge** (new): Timed Enter key wakes the TUI readline loop before the full assignment arrives. Qwen 500ms, Codex 400ms, Forge 400ms, Claude 600ms.
3. **Durable file reference** (new): Full assignment saved to `.sp/prompts/<name>.md`; agents told "if anything is unclear, re-read that file."

Files changed: `src/agent/mod.rs` (startup_input added for Qwen/Claude/Forge, Qwen boot delay 1100ms → 3000ms), `src/orchestrator/mod.rs` (durable file reference in `render_worker_prompt_with_agents`). All 79 tests pass.

**Why Qwen boot delay went from 1100ms → 3000ms:** Qwen CLI is a fork of Gemini CLI. Its TUI takes longer to boot than Claude. 1100ms was too short — the TUI might not be reading stdin when the assignment lands. The `\n` nudge at 500ms confirms the TUI is alive before the real assignment arrives at 3000ms.

### Phase 5 — Agent-to-Agent Communication ✅ **DONE (Supercharged)**
Engineering-team mail system wired into Rust orchestrator:
- **5 clean message types**: `task` (action required), `reply` (threaded), `notification` (FYI), `escalation` (blocker → auto-CCs supervisor), `scavenge` (first-to-claim work)
- **Legacy normalization**: 10 old types (dependency_request, review_request, blocker, etc.) map to the 5 clean types
- **Nudge queue**: filesystem-based non-destructive delivery — mail writes to `<state_dir>/nudge_queue/<session_id>/` as JSON files, drained at next natural turn boundary instead of injecting directly into PTY (which cancels in-flight tool calls)
- **Claim/release**: `attempt_scavenge_claim()` atomically claims via SQLite JSON patch (first wins); `release_scavenge()` releases back to pool
- **Engineering-semantic rendering**: each type gets distinct headers (`[SAPPHIRE TASK — ACTION REQUIRED]`, `⚠ [SAPPHIRE ESCALATION — BLOCKER]`, etc.)
- **Validation**: subject ≤120 chars, body ≤8KB, self-mail rejection
- **Idempotent ack**: double-ack is a no-op
- **Auto-archive**: resolved mail older than threshold auto-archives (non-pinned only)
- **Delivery modes**: `interrupt` (urgent/critical, direct PTY injection) vs `queue` (normal, filesystem queue with TTL: 30 min normal / 2 hr urgent)

### Phase 6 — Scale to 16 🚧 TODO
4×4 grid, ultra-complex missions, full iteration pipeline, stress testing metrics, failure mode testing.

### Phase 7 — Port to Rust Runtime 🚧 **IN PROGRESS**
Mail routing ✅, lease conflict detection ✅, nudge queue ✅, scavenge claim/release ✅. Remaining: integration test harness, prompt updates for mail protocol.

---

## Bugs Found & Fixed

### BUG 1: Supervisor planning always timed out (THE CORE BUG)

**Symptom:** `sp qwen 2 "audit this repo"` sat for 60s then failed: "supervisor planning timed out"

**Root cause (4 nested bugs):**

1. **Qwen `-i` flag is broken.** `embed_initial_prompt_if_supported` passed prompts via `-i` (interactive mode). Qwen's `-i` boots its TUI but NEVER auto-executes the pre-loaded prompt. Zero output ever arrives.

2. **Qwen `-p` in PTY is flaky.** Switching to `-p` (non-interactive) via `portable-pty` worked sometimes, not others. Short prompts (<600 chars) worked ~50% of time. Long prompts (>800 chars) failed 100% — produced only the boot warning (180 bytes), then silent for 60s.

3. **The WORKING approach:** `echo "prompt" | qwen --screen-reader --approval-mode yolo` (stdin pipe). Consistently reliable regardless of prompt length. Takes 30-60s but always returns output.

4. **Missing `--approval-mode yolo`:** Even the working stdin approach needs `--approval-mode yolo` or Qwen blocks on tool-approval prompts.

5. **Qwen puts strings where arrays expected:** When Qwen produces JSON plans, it sometimes writes `"definition_of_done": "Structured findings..."` (a string) instead of `["item1", "item2"]` (an array). Deserialization failed silently.

6. **Qwen invents workstream execution types:** Used `"execution":"baseline"` instead of valid `parallel|dependent|validation|integration`. `parse_execution` returned `Err`, plan rejected.

**Fix (3 files):**

#### `src/orchestrator/mod.rs` — New `plan_with_supervisor_qwen_pipe`:
```rust
async fn plan_with_supervisor_qwen_pipe(
    &self,
    _mission_id: Uuid,
    config: &LaunchConfig,
    planning_prompt: &str,
) -> Result<PlanOutcome> {
    use crate::agent::AgentKind;
    let state_dir = config.state_dir.clone();
    let repo = config.repo.clone();
    let prompt = planning_prompt.to_owned();
    let worker_count = config.worker_count;

    let result = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("qwen");
        cmd.arg("--screen-reader");
        cmd.arg("--approval-mode");
        cmd.arg("yolo");
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.current_dir(&repo);
        cmd.env("SAPPHIRE_SESSION_ROOT", state_dir.to_string_lossy().as_ref());

        let mut child = cmd.spawn().context("failed to spawn qwen for planning")?;
        {
            let mut stdin = child.stdin.take().context("failed to get stdin")?;
            std::io::Write::write_all(&mut stdin, prompt.as_bytes())
                .context("failed to write planning prompt")?;
            drop(stdin);
        }

        let output = child.wait_with_output().context("qwen planning failed")?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let combined = format!("{}\n{}", stdout, stderr);
        Ok::<_, anyhow::Error>((combined, stdout.len(), stderr.len()))
    }).await??;

    let (combined, _stdout_len, _stderr_len) = result;

    let qwen_adapter = crate::adapter::adapter_for(AgentKind::Qwen);
    if let Some(plan) = qwen_adapter.extract_supervisor_plan(&combined) {
        if plan.worker_packets.is_empty()
            || plan.worker_packets.len() != productive_worker_slots(worker_count)
            || plan.workstreams.is_empty()
        {
            anyhow::bail!("invalid plan: expected {} worker packets, got {}",
                productive_worker_slots(worker_count), plan.worker_packets.len());
        }
        return Ok(PlanOutcome { plan, source: "supervisor" });
    }

    anyhow::bail!("qwen planning produced no valid plan. Last output: {}", truncate(&combined, 600))
}
```

#### `src/orchestrator/mod.rs` — `plan_with_supervisor` dispatch:
```rust
let planning_prompt = adapter.build_supervisor_plan_prompt(&config.mission, productive_worker_slots(config.worker_count));

// Qwen: use stdin pipe (reliable). PTY with -p is flaky.
if config.supervisor_agent == crate::agent::AgentKind::Qwen {
    return self.plan_with_supervisor_qwen_pipe(mission_id, config, &planning_prompt).await;
}

// Other agents: use PTY-based planning (unchanged)
let (planning_spec, prompt_embedded) = embed_initial_prompt_if_supported(...);
```

#### `src/adapter.rs` — Lenient `SupervisorWorkerPacket`:
```rust
#[derive(Debug, Deserialize)]
struct SupervisorWorkerPacket {
    worker_id: String,
    role: String,
    #[serde(default)] starting_angle: String,
    owned_scope: String,
    explicit_task: String,
    out_of_scope: String,
    #[serde(deserialize_with = "deserialize_vec_or_string")]
    definition_of_done: Vec<String>,
    #[serde(deserialize_with = "deserialize_vec_or_string")]
    required_evidence: Vec<String>,
    blocker_protocol: String,
    conflict_warning: String,
    #[serde(deserialize_with = "deserialize_vec_or_string", default)]
    communication_rules: Vec<String>,
    #[serde(deserialize_with = "deserialize_vec_or_string", default)]
    validation_standard: Vec<String>,
    #[serde(deserialize_with = "deserialize_vec_or_string", default)]
    expected_output_format: Vec<String>,
}

fn deserialize_vec_or_string<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where D: serde::Deserializer<'de> {
    // Accepts array ["a","b"] OR string "a, b, c" and splits on newlines/commas/semicolons
}
```

#### `src/adapter.rs` — Lenient `parse_execution`:
```rust
fn parse_execution(value: &str) -> Result<WorkstreamExecution> {
    match value.trim().to_ascii_lowercase().as_str() {
        "parallel" => Ok(WorkstreamExecution::Parallel),
        "dependent" => Ok(WorkstreamExecution::Dependent),
        "validation" => Ok(WorkstreamExecution::Validation),
        "integration" => Ok(WorkstreamExecution::Integration),
        _ => Ok(WorkstreamExecution::Parallel), // Qwen invents "baseline", "review", etc.
    }
}
```

### BUG 2: Duplicate Ghostty windows opened

**Symptom:** Running `sp` opened TWO Ghostty windows — one from orchestrator, one from `main.rs`.

**Root cause:** Both `run_live_mission` (orchestrator) AND `wait_for_tmux_and_open_external_window` (main.rs) called `open_external_terminal_for_session`.

**Fix (`src/main.rs`):** Removed `wait_for_tmux_and_open_external_window` entirely. The orchestrator handles terminal opening. Changed both the `Run` and `Resume` branches to just set `let _ = tmux_ready;` instead of calling the duplicate opener.

### BUG 3: Ghostty tab AppleScript blocked (macOS error 1002)

**Symptom:** Opening Ghostty tab via `osascript` keystroke `Cmd+T` fails with "osascript is not allowed to send keystrokes (1002)".

**Fix:** Use `open -a /Applications/Ghostty.app` (reuse existing app) instead of `open -na` (new instance) for subsequent batches. macOS groups windows of the same app naturally. No AppleScript needed.

### BUG 4: `embed_initial_prompt_if_supported` always returned `(spec, false)`

**Symptom:** Code was a no-op for all agents.

**Fix:** Simplified to a pass-through. Qwen planning now uses dedicated `plan_with_supervisor_qwen_pipe`. Workers always use PTY injection.

### BUG 5: `ensure_tmux_surface` used dashboard command approach

**Symptom:** tmux session created with `respawn-pane -k` replacing the shell with a dashboard loop. Panes then created via `split_window_with_command` sometimes had race conditions with "can't find window: 0" errors.

**Fix:** New approach — create session with explicit dimensions, split ALL panes first, let agents reuse pre-created panes. Proven working with the `launch-claude-grid.sh` script.

### BUG 6: Qwen supervisor produces markdown-wrapped JSON instead of raw markers

**Symptom:** Qwen outputs JSON wrapped in \`\`\`json code fences without `BEGIN_SAPPHIRE_PLAN_JSON` / `END_SAPPHIRE_PLAN_JSON` markers, causing extraction to return `NO_BEGIN_MARKER`.

**Root cause:** Qwen's default behavior wraps JSON in markdown formatting despite instructions.

**Fix (extraction fallback):** Added regex fallback in `extract_supervisor_plan_impl` to detect \`\`\`json blocks when markers are missing:
```rust
// Fallback: look for markdown JSON block
m = re.search(r'```json\s*\n(\{.*?\})\s*\n```', text, re.DOTALL)
if not m:
    m = re.search(r'```\s*\n(\{.*?\})\s*\n```', text, re.DOTALL)
```
Also hardened the supervisor prompt with extremely strict JSON rules: no markdown, no code fences, no prose, arrays must be arrays, execution must be valid enum.

### BUG 7: `worker-01` naming is cheap and ambiguous; fake personas are worse

**Symptom:** Agents identified as `worker-01`, `worker-02` — no professional identity, no role semantics. Supervisor JSON lacked role structure.

**Fix — Function-first engineering team roles (6 files changed):**

#### `src/model.rs` — New fields on `WorkerPacket`:
```rust
pub struct WorkerPacket {
    pub worker_id: String,
    pub role_type: String,       // stable machine key: "software-engineer"
    pub display_name: String,    // runtime label: "Engineer-1"
    pub role: String,            // backward compat: "Software Engineer"
    // ... rest unchanged
}
```

#### `src/adapter.rs` — `SupervisorWorkerPacket` now parses `role_type` + `display_name`:
```rust
struct SupervisorWorkerPacket {
    worker_id: String,
    #[serde(default)] role: String,
    #[serde(default)] role_type: String,      // NEW
    #[serde(default)] display_name: String,   // NEW
    // ... rest unchanged
}

impl SupervisorWorkerPacket {
    fn into_packet(self, ordinal: usize) -> WorkerPacket {
        // Derive role_type: prefer explicit, fall back to inferring from role
        let role_type = if !self.role_type.is_empty() {
            self.role_type.clone()
        } else if !self.role.is_empty() {
            infer_role_type(&self.role)  // "Software Engineer" -> "software-engineer"
        } else {
            "software-engineer".to_owned()
        };

        // Derive display_name: prefer explicit, otherwise generate from role_type
        let display_name = if !self.display_name.is_empty() {
            self.display_name.clone()
        } else {
            generate_display_name(&role_type, ordinal)  // "software-engineer", 1 -> "Engineer-1"
        };
        // ...
    }
}
```

#### Role inference and display name generation:
- `infer_role_type()`: Maps human role names to stable keys (13 roles)
- `role_type_to_title()`: Maps stable keys back to human titles
- `generate_display_name()`: `software-engineer` → `Engineer-1` | `designer-engineer` → `Designer-2` | `debug-and-review-engineer` → `Reviewer-1` | etc.

#### `src/templates.rs` — 13 role templates loaded at build time:
```rust
role!("software-engineer", "/src/internal/agents/templetes/roles/job-roles/software-engineer.md");
role!("designer-engineer", "/src/internal/agents/templetes/roles/job-roles/designer-engineer.md");
// ... all 13 roles

pub fn render_worker_prompt(&self, mission: &str, packet: &WorkerPacket) -> String {
    let role_template = self.role_template(&packet.role_type)
        .unwrap_or(fallback_template);
    // Prepends full role template + agent-specific assignment
}
```

#### Supervisor prompt hardened with role schema:
```
role_type must be one of: software-engineer, research-engineer, validation-engineer, 
architecture-engineer, security-engineer, debug-and-review-engineer, 
testing-and-automation-engineer, designer-engineer, sales-engineer, 
solutions-engineer, customer-success-engineer, product-engineer, compliance-engineer

display_name must be function-first with numeric identity: Engineer-1, Designer-2, Reviewer-1

supervisor analyzes the mission and chooses ONLY relevant role_types. 
Do NOT hardcode all 13 roles.
```

#### `src/orchestrator/mod.rs` — AGENTS steward packet updated:
```rust
WorkerPacket {
    role_type: "software-engineer".to_owned(),
    display_name: "Steward".to_owned(),
    // ...
}
```

---

---

## Working Tmux + Ghostty Launch Logic

This is the PROVEN pattern. DO NOT change this approach without testing end-to-end.

### Shell script proof (`launch-claude-grid.sh`):
```bash
#!/bin/zsh
SESSION="claude-batch-0"
REPO="/Users/harshitduggal/workspace/sapphire-agent-Factory"
PANES_PER_TAB=10
TOTAL=20
CURRENT_PROMPT=0
TAB_NUM=0

for (( start=0; start<TOTAL; start+=PANES_PER_TAB )); do
    end=$(( start + PANES_PER_TAB ))
    (( end > TOTAL )) && end=$TOTAL
    batch_size=$(( end - start ))
    SESSION="claude-batch-${TAB_NUM}"

    tmux kill-session -t "$SESSION" 2>/dev/null || true
    tmux new-session -d -s "$SESSION" -c "$REPO" -x 240 -y 60
    tmux set -t "$SESSION" remain-on-exit on
    tmux set -t "$SESSION" window-size latest

    # Split into panes
    for (( i=1; i<batch_size; i++ )); do
        tmux split-window -t "$SESSION"
        tmux select-layout -t "$SESSION" tiled
    done

    sleep 1

    # Launch claude in each pane
    PANE_IDS=($(tmux list-panes -t "$SESSION" -F "#{pane_id}"))
    for pane in "${PANE_IDS[@]}"; do
        tmux send-keys -t "$pane" -l "claude"
        tmux send-keys -t "$pane" "Enter"
    done

    sleep 8  # Wait for ALL to boot

    # Re-read pane IDs (layout may shift)
    PANE_IDS=($(tmux list-panes -t "$SESSION" -F "#{pane_id}"))

    # Inject unique prompts one at a time
    for (( i=0; i<batch_size; i++ )); do
        prompt="${PROMPTS[$((CURRENT_PROMPT + i))]}"
        tmux send-keys -t "${PANE_IDS[$i]}" -l "$prompt"
        tmux send-keys -t "${PANE_IDS[$i]}" "Enter"
        sleep 0.3  # Critical: delay between injections
    done

    CURRENT_PROMPT=$((CURRENT_PROMPT + batch_size))

    # Open Ghostty
    if (( TAB_NUM == 0 )); then
        open -na /Applications/Ghostty.app --args -e /bin/zsh -lc "tmux attach-session -t $SESSION"
    else
        open -a /Applications/Ghostty.app --args -e /bin/zsh -lc "tmux attach-session -t $SESSION"
    fi

    TAB_NUM=$((TAB_NUM + 1))
done
```

### Key rules (learned the hard way):
1. **Use `PANE_IDS=($(tmux list-panes ...))` array**, not `while read` (subshell resets variables)
2. **Re-read pane IDs** after boot delay (layout may shift)
3. **0.3s delay between prompt injections** (last 2 panes failed without this)
4. **8s boot wait** before injecting prompts
5. **`open -na` for first window, `open -a` for subsequent** (reuses app = natural tabbing)
6. **NEVER use AppleScript keystroke for Ghostty tabs** (macOS blocks it with error 1002)

---

## Code Changes Summary

### `src/tmux/mod.rs` — New methods:
```rust
pub fn create_session_for_workers(&self, name: &str, work_dir: &str, cols: usize, rows: usize) -> Result<(), String>
pub fn list_pane_ids(&self, session: &str) -> Vec<String>
pub fn send_command(&self, pane: &str, text: &str) -> Result<(), String>
```

### `src/agent/mod.rs`:
- Qwen: boot delay 1100ms → **3000ms**, added `startup_input` (`\n` at 500ms)
- Claude: added `startup_input` (`\n` at 600ms)
- Forge: added `startup_input` (`\n` at 400ms)
- Codex: `startup_input` kept at 400ms (unchanged), moved from hardcoded `if` to match arm tuple (cleaner)
- All agents now share the same 6-tuple match arm: `(program, args, delay, startup_input, rules, submit_mode)`

### `src/orchestrator/mod.rs` (Phase 4.3):
- `render_worker_prompt_with_agents()` → added durable file reference: "Your full assignment was also saved to `.sp/prompts/<your-display-name>.md`. If any part of your prompt is unclear, re-read that file."
- `write_prompt_file()` already persisted the prompt — this just tells the agent it exists
- `ensure_tmux_surface` → uses `create_session_for_workers` + pre-splits all panes
- 800ms delay after supervisor spawn
- 400ms delay between worker spawns
- 2s delay before Ghostty window open
- `plan_with_supervisor_qwen_pipe` → stdin pipe approach
- `embed_initial_prompt_if_supported` → simplified pass-through

### `src/runtime/mod.rs`:
- `TmuxBackend::spawn` → reuses pre-created panes when available

### `src/adapter.rs`:
- `SupervisorWorkerPacket` → lenient deserialization
- `deserialize_vec_or_string` → accepts string or array
- `parse_execution` → defaults unknown to Parallel

### `src/main.rs`:
- Removed `wait_for_tmux_and_open_external_window`
- Removed `Instant` import (no longer used)

### `src/internal/ui/shimmer/shimmer.rs`:
- Blue → bright purple RGB constants

### `src/internal/ui/theme/theme-main.rs`:
- Blue palette → bright purple

### `supervisor-templates/prompt.md` (Phase 4.2):
- Added **Propulsion Principle** section — supervisor auto-executes on restart, checks persisted state, acts before summarizing
- Added **Solo Artist Trap** section — decision tree (coordination → do it, implementation → dispatch, trivial → fix directly), anti-pattern: reading code to "understand" then fixing burns context
- Added two new failure conditions: "doing implementation work yourself while workers sit idle" and "restarting and waiting for human approval instead of checking state and acting"

### `src/orchestrator/mod.rs` (Phase 4.2):
- `ActiveSession` gains `consecutive_stall_failures: usize` (reset on output/automation) and `last_confirmed_alive: Instant` (liveness timestamp)
- `handle_stalls()` rewritten with **escalation ladder**: 1st → corrective prompt, 2nd → redirect with narrowed scope + supervisor notice, 3rd+ → force `Failed` + supervisor decides respawn/reassign
- `handle_runtime_event()` resets `consecutive_stall_failures` and updates `last_confirmed_alive` on any Output or Automation event
- 2 new tests: `escalation_ladder_third_consecutive_stall_fails_worker`, `consecutive_stall_failures_reset_on_output`

---

## Code Quality Cleanup (54 → 0 Warnings)

### Removed dead scaffolding (never wired in):
- **`src/mail/`** — entire mail subsystem (types, priority, validation) was skeleton code never connected to the runtime loop. Deleted.
- **`src/tmux/dashboard.rs`** — never called from anywhere. Status reporting goes through `write_status_snapshot()` → `.sp/control/status.txt` directly.

### Simplified dead branches (never executed):
- **`PromptStyle::Compact`** — all 4 adapters used `Standard`. The Compact match arms existed in the code but were never reached. Removed the entire enum and all match arms.
- **`ProcessLaunchSpec.display_name`** — set to the same value as `surface_label`, never read anywhere. Removed from struct.

### Moved to test-only (was already only consumed by tests):
- `restart_backoff()`, `GridCell`, `worker_cells()`, `BufferManager::as_str()`/`clear()` — zero production callers, marked `#[cfg(test)]`.

### Suppressed with `#[allow(dead_code)]` (compiler false positives on cross-module usage):
- `send_keys`, `capture_pane`, `set_option` — used by `runtime/tmux_session.rs`
- `is_alive`, `is_zombie`, `terminal_program` — called from within cfg-gated macOS blocks
- `LaunchConfig`, `GridLayout`, `PendingMail`, various store methods — used across modules

### Cleaned code:
- `src/orchestrator/mod.rs` — removed `shell_quote`, `render_routed_mail`, `restart_max_secs`, `zombie_kill_grace_period`
- `src/store/mod.rs` — removed `search_messages`, `list_thread_messages`, `list_unread_messages`, `archive_old_messages`, `message_stats`, `row_to_mail`, fixed unnecessary `mut`
- `src/runtime/mod.rs` — removed `trim_recent_utf8`
- `src/templates.rs` — removed `known_role_types`
- `src/tui/widgets.rs` — removed `status_dot`

**Result:** 54 → 0 warnings, same binary behavior, no capability lost.

---

## Module Extraction: `src/orchestrator/mail.rs`

The mail subsystem (~1000 lines) was extracted from `mod.rs` into its own module to prevent the orchestrator from becoming unmanageable. This includes:
- Message type normalization (legacy → 5 clean types)
- Mail validation (subject ≤120, body ≤8KB)
- Engineering-semantic rendering (distinct headers per type)
- Nudge queue (filesystem-based non-destructive delivery)
- Scavenge claim/release
- Auto-archive + idempotent ack

The orchestrator's `handle_mail_directive`, `handle_ack_directive`, and `handle_lease_directive` now delegate to the mail module with thin wrappers.

---

## Test Status
- **59 tests pass** (79 original - 20 removed with mail subsystem cleanup = 59, all pass)
- `launch-supervisor-grid.sh` works: 4 Qwen agents in interactive mode, all emit `SAPPHIRE_STATUS {"state":"done_claimed"}` independently
- Supervisor planning works for Qwen (stdin pipe + markdown fallback + concrete JSON example)
- Role template loading works: 13 roles compiled at build time via `include_str!`
- Escalation ladder verified: 3rd consecutive stall forces `Failed`, counter resets on output
- Backward compatible: old JSON without `role_type`/`display_name` infers from `role` field; old `worker-01` mail addresses map to `engineer-01`
- Zero warnings on `cargo build`

---

## Known Issues Not Yet Fixed
- No integration test harness around PTY orchestration
- `src/telemetry/` directory is empty
- Supervisor planning for Qwen takes 30-60s (inherent to LLM, not a bug)
- Ghostty opens separate windows instead of tabs (AppleScript blocked by macOS)
- Nudge queue drain is polled every watchdog tick; could be event-driven with fsnotify in future
- Worker prompts don't yet document the new mail protocol (claim/release, message types)

---

## Shell Script Reliability Proofs (Phase 4.3)

> Everything in `launch-supervisor-grid.sh` that works end-to-end. This is the source of truth for what must be ported to Rust.

### BUG 8: `tmux capture-pane` misses status directives entirely

**Symptom:** Agents are actively working (visible in tmux panes), watchdog reports `awaiting-status` forever.

**Root cause:** `SAPPHIRE_STATUS` directives scroll off the visible viewport or are buried in ANSI-rich TUI output. Regex on captured viewport is fundamentally fragile.

**Fix — File-based status as PRIMARY, tmux as FALLBACK:**
- Workers write `.sp/workers/<display_name>/status.json` on every state change
- Watchdog reads file first (atomic, no ANSI, no scrollback loss)
- Falls back to `tmux capture-pane` with 3-strategy extraction (single-line regex, brace-depth counter, window search)
- Falls back to "progressing" inference when no directive found but agent is active

### BUG 9: Prompt gets "Queued" in Qwen's TUI instead of submitting

**Symptom:** Entire prompt appears as queued input text, never submitted.

**Root cause:** `Enter` hits before Qwen's TUI readline is active. No startup nudge.

**Fix:** Send `Enter` to all panes after 8s boot wait, sleep 1.0s, THEN paste prompt, THEN send `Enter`.

### BUG 10: zsh 1-based array indexing skips first pane

**Symptom:** `PANE_IDS[$idx]` with `idx=0` returns empty, all prompts shifted by one.

**Fix:** `pane_idx=$((idx + 1))`, `PANE_IDS[$pane_idx]`.

### BUG 11: `local` keyword outside function in zsh

**Symptom:** `local cols=1 rows=1` at top level is a syntax error in zsh.

**Fix:** Use global scope variables (`cols=1`, `rows=1`).

### BUG 12: `set-buffer` with shell variable content is fragile for large prompts

**Fix:** Use `tmux load-buffer -b name "$file"` (read from file) or read file content into variable first.

### BUG 13: `render_json_state_view()` had broken Python quoting

**Symptom:** `python3 -c '...' TS="$ts"` with escaped quotes inside f-string → `SyntaxError`.

**Fix:** Use heredoc `python3 <<'PY'` instead of `-c`.

### BUG 14: Live supervisor ignores `SNAPSHOT_JSON` prefix

**Symptom:** Supervisor terminal shows JSON but never responds with `SUPERVISOR_MD:`.

**Root cause:** `SNAPSHOT_JSON` is meaningless to Qwen — it's not a recognized instruction.

**Fix:** Send human-readable snapshot: `"Snapshot #6 — 1 of 2 agents done: {json}\n\nReply with one line: SUPERVISOR_MD: ..."`. 3-strategy response capture: exact prefix → keyword search → any substantive line.

### BUG 15: Supervisor prompt hardcodes `software-engineer` + `designer-engineer` in JSON example

**Symptom:** Supervisor copies the example roles regardless of mission.

**Fix:** Placeholder-based JSON example (`<pick based on mission>`), explicit instruction: "pick 1 type and spawn all workers as that type, OR distribute across types."

### BUG 16: Ghostty opens new windows instead of tabs

**Fix:** Proven AppleScript approach:
```applescript
tell application "Ghostty"
    activate
    set win to new window
    set term to focused terminal of selected tab of win
    input text "tmux attach-session -t <session>\n" to term
end tell
```

### Shell script capabilities added:
- `--test` mode validates file-based state without launching agents
- Grid-aware layout: 2→2×1, 4→2×2, 11→11×1, 12→4×3
- `load-buffer` from file for atomic prompt injection
- Mandatory file-based status instructions in worker prompts
- 13 role types with multiple instances awareness in supervisor prompt

---

## TUI / Frontend Rewrite (Phase 4.4)

### What was removed:
- **Excess colors** — blue, cyan, cyan_green, teal removed from palette
- **JSON display** — raw JSON blobs stripped from TUI display entirely
- **Noisy event bodies** — `clean_event_body()` strips JSON, protocol lines, raw output

### What was added:
- **Two-color theme** — off-white grey `(220,224,232)` + light bright purple `(190,170,255)`
- **Structured data views** — `WorkerView`, `HealthView`, `WatchdogView`, `LogEntry`, `SupervisorView`
- **`WatchdogView`** — all 9 stats parsed from control status file:
  - `runtime_events`, `directives_parsed`, `mail_routed`, `validation_challenges`
  - `stall_interventions`, `lease_conflicts`, `protocol_reminders`
  - `supervisor_health_events`, `supervisor_fallbacks`, `supervisor_mode` (Healthy/Recovering/Degraded)
- **Live log** — clean `{ts, source, message}` entries, shown in main pane (last 8) and sidebar Watchdog tab (last 20)
- **4 clean sidebar tabs**:
  - **Workers**: name, role, summary, files, validation status
  - **Watchdog**: all 9 watchdog stats + health counters + live log
  - **Events**: timestamped event log
  - **Supervisor**: name, state, summary, pending decisions
- **Main pane (70%)**: supervisor markdown rendered with full markdown support (headings, tables, code blocks, quotes, lists)
- **Sidebar (30%)**: structured data, no raw output

### Files changed:
- `src/tui/data.rs` — complete rewrite: structured views, watchdog parsing, clean event bodies
- `src/tui/render.rs` — complete rewrite: 70/30 layout, 4 tab contents, clean rendering
- `src/internal/ui/shimmer/shimmer.rs` — 2 colors only (off-white grey + light bright purple)
- `src/internal/ui/theme/theme-main.rs` — reduced palette (removed `cyan`, `cyan_green`, `blue` → added `accent`)
- `src/internal/ui/theme/styles.rs` — updated all surface styles to use 2-color palette
- `src/internal/ui/theme/fallback/markdown/theme.rs` — updated code lang color

---

## Rust Port of Shell Proofs

### `read_worker_status_files()` — File-based status as PRIMARY
**`src/orchestrator/mod.rs`**: New method called every watchdog tick, before `write_status_snapshot`.
- Reads `.sp/workers/<display_name>/status.json` for each worker
- Validates JSON, requires `"state"` key, ignores malformed
- Skips terminal states (won't overwrite completed state)
- Updates `session.state`, `last_summary`, `last_files`, `last_risks`, `last_confirmed_alive`
- Falls back to PTY directive parsing (unchanged) as secondary source

### `workers_state_dir` added to `ControlSurface`
- `.sp/workers/` directory created in bootstrap
- Both launch and resume paths include it

### `ActiveSession` gains new fields:
- `last_files: Vec<String>` — last known files from status file or directive
- `last_risks: Vec<String>` — last known risks

### Supervisor planning prompt role awareness
**`src/adapter.rs`**: Updated `build_supervisor_plan_prompt_impl`:
- Rule 11: "13 role TYPES, each can have MULTIPLE instances"
- Rule 14: "Coding-heavy mission might need all software-engineers. Security audit needs security-engineers. YOU decide."
- Placeholder-based JSON example (no hardcoded role_types)
- 15 rules total, all explicit about role selection

### `control_protocol()` includes file-based status instruction
**`src/templates.rs`**: Worker prompts now include:
- "STATUS FILE IS MANDATORY: On every state change, write your status to .sp/workers/<your-display-name>/status.json"
- "Exact file JSON format: {...}"
- "The file is the PRIMARY status source — the SAPPHIRE_STATUS terminal line is only a backup"

### Ghostty AppleScript in Rust tmux module
**`src/tmux/mod.rs`**: Three new methods:
- `open_ghostty_window_for_session()` — public, uses proven AppleScript (`new window` + `focused terminal`)
- `open_ghostty_tab_for_session_applescript()` — public, opens new tab in existing Ghostty window
- `open_ghostty_window_fallback()` — private `open -na` fallback

### Max 10 per tab batching
**`src/orchestrator/mod.rs`**: `MAX_WORKERS_PER_TAB = 10`
- First session opens as Ghostty window
- If workers > 10, creates extra tmux sessions (`{name}-tab1`, `{name}-tab2`) and opens each as Ghostty tab

### Test Status
- **79 tests pass** (all existing + 2 escalation tests)
- `launch-supervisor-grid.sh --test` passes 4 file-based state tests
- Builds pass (`cargo build`, `cargo build --release`)
- Clean zsh syntax (`zsh -n`)
