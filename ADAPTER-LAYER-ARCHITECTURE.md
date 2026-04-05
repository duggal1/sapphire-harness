# Adapter Layer Architecture

## Overview

The adapter layer (`src/adapter.rs`) is the translation surface between the orchestration runtime and four distinct agent CLIs. It solves a single hard problem: **normalize heterogeneous terminal output into a shared control protocol**, and **shape prompts to match each agent's behavioral profile**.

All four adapters implement the `CliAdapter` trait. Three share the `impl_standard_adapter!` macro; Qwen alone uses `PromptStyle::Compact` while the other three use `PromptStyle::Standard`.

---

## Agent Comparison Table

| Dimension | Qwen Code | OpenAI Codex | Claude Code | Forge |
|---|---|---|---|---|
| **Executable** | `qwen` | `codex` | `claude` | `forge` |
| **Default Args** | `--screen-reader` | `--no-alt-screen -m gpt-5.4-mini -c model_reasoning_effort=low` | _(none)_ | _(none)_ |
| **Submit Mode** | `CarriageReturn` (`\r\n`) | `CarriageReturn` (`\r\n`) | `CarriageReturn` (`\r\n`) | `LineFeed` (`\n`) |
| **Prompt Delay** | 1100ms | 1800ms | 3200ms | 1200ms |
| **Startup Automation** | Auto-dismisses IDE prompt ("Do you want to connect IDE?" → sends "2") | Auto-accepts directory trust ("directory?" → sends "1"); initial `\n` after 400ms | Auto-accepts folder trust ("Yes, I trust this folder" → sends `\n") | _(none — isolation via env vars)_ |
| **Environment Isolation** | `SAPPHIRE_SESSION_ROOT` only | `SAPPHIRE_SESSION_ROOT` only | `SAPPHIRE_SESSION_ROOT` only | `HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME` → `.sp/forge-home/` |
| **Prompt Style** | **Compact** (inline key=value format) | **Standard** (full template expansion) | **Standard** (full template expansion) | **Standard** (full template expansion) |
| **Protocol Nudge** | "Reply with exactly one raw line that starts with SAPPHIRE_STATUS followed by compact JSON. Do not wrap it in markdown." | "Print a single-line SAPPHIRE_STATUS JSON record now. No code fence, no prose before it." | "Output exactly one SAPPHIRE_STATUS line with compact JSON, not a markdown block." | "Emit one plain SAPPHIRE_STATUS line with compact JSON. Keep it machine-readable and on a single line." |
| **Heuristic Confidence** | Low for done claims; High for CLI failure | Medium for done claims; Medium for CLI failure | Medium for done claims; Medium for CLI failure | Medium for done claims; Medium for CLI failure |
| **Signal Detection** | Requires `KEYWORDS_QWEN_SIGNAL` (model:, success:, readfile, writefile, execute, bash) to trust heuristics | Standard keyword matching | Standard keyword matching | Standard keyword matching |

---

## Startup Automation Rules

Startup rules live in `src/agent/mod.rs` as `StartupAutomationRule` instances. Each rule is a `(name, match_pattern, response)` triplet applied by the runtime's background PTY reader thread.

### Qwen — IDE Prompt Dismiss
- **Pattern**: `"Do you want to connect IDE to Qwen Code?"`
- **Response**: `"2\n"`
- **Rationale**: Qwen's screen-reader mode prompts for IDE connection. Sending "2" selects the "no" option and proceeds directly to prompt injection.

### Codex — Directory Trust + Initial Nudge
- **Pattern**: `"directory?"` (matches the trust prompt)
- **Response**: `"1\n"` (accepts trust)
- **Startup Input**: After 400ms, sends `\n` before any prompt
- **Rationale**: Codex requires explicit directory trust on first launch. The initial `\n` clears any residual welcome screen. `--no-alt-screen` prevents alt-buffer switching that breaks output capture.

### Claude — Folder Trust Accept
- **Pattern**: `"Yes, I trust this folder"`
- **Response**: `"\n"` (accepts the default)
- **Rationale**: Claude asks for folder trust confirmation. A bare newline accepts the default "yes" answer.

### Forge — Environment Isolation Only
- **No startup rules**: Forge has no trust/setup prompts to dismiss.
- **Isolation strategy**: Sets `HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME` to `.sp/forge-home/` at launch. This sandboxes Forge's config and data without requiring interactive automation.

---

## Submit Modes

`SubmitMode` controls how prompts are terminated when injected into the PTY. Defined in `src/runtime/mod.rs`, consumed per-agent in `src/agent/mod.rs`.

| Mode | Termination | Agents |
|---|---|---|
| `CarriageReturn` | `\r\n` | Qwen, Codex, Claude |
| `LineFeed` | `\n` | Forge |

**Why this matters**: Different agent CLIs expect different line endings. Wrong termination causes the agent to not recognize the prompt boundary, leading to silent prompt drops or garbled input. The `RunningSession::send_prompt()` method applies the correct terminator based on the agent's `SubmitMode`.

**Codex special case**: Codex also receives a timed initial `\n` via `startup_input` (400ms after launch) to clear the welcome screen before the actual prompt arrives.

---

## Prompt Styles: Compact vs Standard

The `PromptStyle` enum controls how assignment prompts are constructed.

### Standard (Codex, Claude, Forge)

Uses the full `PromptLibrary::render_worker_prompt()` template, which expands:
- Worker ID, mission, role, starting angle
- Owned scope, explicit task, out-of-scope
- Definition of done (bulleted)
- Required evidence (bulleted)
- Blocker protocol, conflict warning
- Communication rules (bulleted)
- Validation standard (bulleted)
- Expected output format (bulleted)
- Sapphire Control Protocol instructions

Then appends the **status envelope contract** as a separate section:
```
Status reporting fallback:
Use the exact 5-line envelope below whenever the watchdog asks for status or validation.

STATE: progressing|blocked|done_claimed|...
SUMMARY: one short sentence
FILES: comma-separated paths or NONE
BLOCKER: one short sentence or NONE
DONE: yes or no
```

### Compact (Qwen only)

Inlining all fields into a single dense block:
```
You are worker-01.
Mission: ...
Role: ...
Scope: ...
Task: ...
Out of scope: ...
Done:
- ...
Evidence:
- ...
Blocker protocol: ...
Conflict warning: ...
Output format:
- ...
Rules:
- Stay inside your scope.
- Read AGENTS.md before real work.
- If Sapphire asks for status, reply only with the 5-line envelope below.
- If you can emit SAPPHIRE markers, do it, but the envelope is the required fallback.

STATE: progressing|blocked|done_claimed|...
SUMMARY: one short sentence
FILES: comma-separated paths or NONE
BLOCKER: one short sentence or NONE
DONE: yes or no
```

**Key difference**: Compact omits the `render_worker_prompt()` template expansion entirely. It uses raw key-value formatting and shorter instructions. This matches Qwen's tendency to echo verbose prompts back into output, which confuses heuristic detection.

### Supervisor Plan Prompt Differences

**Compact style** (Qwen supervisor):
- Returns a single JSON object between tags
- No prose before or after
- Short, concrete strings
- Explicit worker count constraint
- Schema inline in the prompt

**Standard style** (others):
- Includes framing ("You are the supervisor brain")
- More verbose rules
- Compactness hint for Qwen-style terminals
- Schema inline in the prompt

### All Prompt Style Variants

| Prompt Type | Standard | Compact |
|---|---|---|
| **Assignment** | Full template + status envelope | Inline key-values + status envelope |
| **Validation** | "Validation challenge.\nDo not continue broad explanation." | "Validation only.\nDo not continue implementation." |
| **Correction** | "Your last reply did not follow the required status format: {reason}\nDo not continue broad explanation." | "Your last reply was not usable: {reason}\nStop." |
| **Status Request** | "Status update only.\n{reason}\nReply exactly with:" | "Status update only. {reason}\nReply only with:" |
| **Supervisor Action** | "Supervisor intervention required.\n\n{rule}\n\nIssue:\n{context}\n\nReply only with: ACTION/TARGET/SUMMARY/MESSAGE" | "Supervisor action required.\n\n{rule}\n\nIssue:\n{context}\n\nReply only with: ACTION/TARGET/SUMMARY/MESSAGE" |
| **Final Summary** | "All workers are terminal. Produce the final concise synthesis.\nReply only with:\nFINAL_STATE: validated or failed\nFINAL_SUMMARY: one concise paragraph" | "Final summary only.\nReply only with:\nFINAL_STATE: validated or failed\nFINAL_SUMMARY: one concise paragraph" |

---

## Heuristic State Detection

When no explicit `SAPPHIRE_STATUS` directive is found in output, the adapter layer infers state via keyword matching. The `detect_state_impl()` function in `src/adapter.rs` implements this.

### Detection Pipeline

1. **Try status envelope extraction first** — regex match on `STATE: ...` / `SUMMARY: ...` / `FILES: ...` / `BLOCKER: ...` / `DONE: ...`
2. **If no envelope, fall back to keyword heuristics** — scan last 4000 chars of output
3. **Qwen-specific signal gating** — Qwen heuristics only fire if `KEYWORDS_QWEN_SIGNAL` is also present (prevents false positives from prompt echo)
4. **Qwen prompt echo filtering** — if output contains `KEYWORDS_PROMPT_ECHO` but not `KEYWORDS_QWEN_PROMPT_ECHO_NEG`, return `None` (agent is echoing the prompt, not acting on it)
5. **CLI failure detection** — Qwen has extended keywords including `[api error:`, `critical error`
6. **State keyword matching** — ordered checks: Validated → WeakOutput → WrongDirection → DoneClaim → Blocked → Contradiction → QwenToolSignal → Progressing

### Keyword Lists

| State | Keywords |
|---|---|
| **Validated** | `validation passed`, `validated`, `all checks passed` (Qwen: also `validated successfully`) |
| **WeakOutput** | `probably fixed`, `should work now`, `didn't run tests`, `cannot verify`, `can't verify` |
| **WrongDirection** | `rewrote the architecture`, `rewrote the whole`, `full rewrite`, `changed unrelated`, `refactored broadly`, `took over` |
| **Blocked** | `blocked`, `cannot proceed`, `can't proceed`, `waiting on`, `need clarification`, `dependency`, `unclear owner` |
| **Contradictory** | `conflict`, `overlap`, `contradiction`, `someone else changed`, `merge collision` |
| **Progressing** | `investigating`, `reproducing`, `working on`, `running tests`, `profiling`, `reviewing`, `checking` |
| **DoneClaim** | `i'm done`, `im done`, `completed the task`, `finished the task`, `task is complete`, `implemented the requested`, `done with` |
| **CLI Failure** | `invalid number of stops`, `traceback`, `panic`, `exception`, `fatal:` (Qwen: also `[api error:`, `critical error`) |
| **Qwen Tool Signal** | `model:`, `success:`, `readfile`, `writefile`, `editfile`, `starting work`, `execute`, `bash` |

### Confidence Levels by Agent

| State | Qwen | Others |
|---|---|---|
| Validated | Medium | Medium |
| WeakOutput | Medium | Medium |
| WrongDirection | Medium | Medium |
| DoneClaim | Low | Medium |
| Blocked | Medium | Medium |
| Contradictory | Medium | Medium |
| Progressing (generic) | Low | Low |
| CLI Failure | High | Medium |
| Qwen Tool Signal | Medium | N/A |

### Heuristic Window

The detection scans the **last 4000 characters** of output. For Qwen specifically, the window is further trimmed:
1. Find the last occurrence of any Qwen tool marker (`Model:`, `Success:`, `ReadFile`, `WriteFile`, `EditFile`, `Execute`)
2. Scan only from that position onward
3. Strip echoed watchdog prompts (lines starting with "Status update only.", "Supervisor action only.", "Your last reply was not usable:")

This prevents Qwen's verbose tool output and prompt echoing from triggering false state detection.

---

## Regex Parsing: Envelopes and Actions

### Status Envelope (Strict)
```
(?ms)STATE:\s*(?P<state>[^\n]+)\nSUMMARY:\s*(?P<summary>[^\n]+)\nFILES:\s*(?P<files>[^\n]+)\nBLOCKER:\s*(?P<blocker>[^\n]+)\nDONE:\s*(?P<done>[^\n]+)
```

### Status Envelope (Relaxed)
```
(?ms)STATE:\s*(?P<state>[A-Za-z_]+)\s+SUMMARY:\s*(?P<summary>.+?)\s+FILES:\s*(?P<files>.+?)\s+BLOCKER:\s*(?P<blocker>.+?)\s+DONE:\s*(?P<done>yes|no|[A-Za-z_]+)
```

### Supervisor Action
```
(?ms)ACTION:\s*(?P<action>[^\n]+)\nTARGET:\s*(?P<target>[^\n]+)\nSUMMARY:\s*(?P<summary>[^\n]+)\nMESSAGE:\s*(?P<message>[^\n]+)
```

### Final Envelope
```
(?ms)FINAL_STATE:\s*(?P<state>[^\n]+)\nFINAL_SUMMARY:\s*(?P<summary>[^\n]+)
```

### Supervisor Plan JSON
Extracted via tagged block parsing between `BEGIN_SAPPHIRE_PLAN_JSON` and `END_SAPPHIRE_PLAN_JSON`, with fallback to raw JSON object extraction (matching `{...}` containing `"mission_rewrite"`, `"worker_packets"`, `"workstreams"`).

---

## Template Dependencies

The adapter layer depends on `src/templates.rs` for:
- **`PromptLibrary`** — loads all prompt sources at compile time via `include_str!`
- **`render_worker_prompt()`** — used by Standard-style assignment prompts
- **`render_supervisor_prompt()`** — used by supervisor assignment (not per-agent, but consumes the same library)
- **`agents_instruction_source()`** — returns the embedded `agents.md-instructions.md` for AGENTS.md seeding

The adapter does **not** read prompt files at runtime. All sources are compiled into the binary. Changes to prompt files require a rebuild.

---

## Macro Architecture

`impl_standard_adapter!` generates all four adapter implementations from a single template:

```rust
impl_standard_adapter!(CodexAdapter,  AgentKind::Codex,  PromptStyle::Standard);
impl_standard_adapter!(ClaudeAdapter, AgentKind::Claude, PromptStyle::Standard);
impl_standard_adapter!(ForgeAdapter,  AgentKind::Forge,  PromptStyle::Standard);
impl_standard_adapter!(QwenAdapter,   AgentKind::Qwen,   PromptStyle::Compact);
```

**Impact**: Changes to the macro affect all four agents simultaneously. The `PromptStyle` parameter is the primary differentiation axis. Any macro refactor requires testing against all four agents.

---

## State Mapping

The `map_state_token()` function converts envelope state strings to `SessionState` enum values:

| Token | SessionState |
|---|---|
| `progressing` | `Progressing` |
| `blocked` | `Blocked` |
| `done_claimed` | `DoneClaimed` |
| `needs_validation` | `NeedsValidation` |
| `validated`, `pass` | `Validated` |
| `needs_retry`, `partial` | `NeedsRetry` |
| `wrong_direction` | `WrongDirection` |
| `failed`, `fail` | `Failed` |
| `stalled` | `Stalled` |

If the token doesn't match any known value, it falls through to `SessionState::from_directive()` for additional parsing.

---

SAPPHIRE_STATUS {"state":"done_claimed","summary":"Adapter layer architecture fully documented: agent comparison table, startup automation rules, submit modes, prompt style contrasts, heuristic state detection pipeline, regex parsing contracts, template dependencies, and macro architecture.","files":["ADAPTER-LAYER-ARCHITECTURE.md"],"commands":[],"risks":[],"overlap":"none"}
