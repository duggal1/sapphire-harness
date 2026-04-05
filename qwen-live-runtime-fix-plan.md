# Qwen Live Runtime Fix Plan

## Goal

Make Sapphire work end-to-end with `qwen` as the primary live validator.

The control plane is already strong enough. The blocker is runtime compatibility in the spawned interactive Qwen path.

## Current truth

- Parallel launch is real.
- Watchdog, SQLite persistence, validation rows, resume/replay, agent mail, AGENTS bootstrap, and the control surface are real.
- Direct CLI health checks can work while the spawned interactive path still fails.
- The historical live Qwen failure is:

  - `ERROR Invalid number of stops (< 2)`

- Until the spawned interactive Qwen session survives startup and emits a first usable status envelope, the following are still unproven:

  - supervisor-first live planning
  - real worker execution
  - real coordination under pressure
  - real end-to-end completion

## Evidence from the current codebase

- Qwen launch is still minimal in [src/agent/mod.rs](src/agent/mod.rs):
  - program: `qwen`
  - no runtime flags
  - one startup rule for the IDE-connect prompt
  - prompt delay only

- The PTY runtime in [src/runtime/mod.rs](src/runtime/mod.rs):
  - allocates a standard PTY
  - writes prompts after a fixed delay
  - optionally sends one startup input
  - does not yet apply any Qwen-specific warmup or session-mode shaping

- The preflight gate in [src/orchestrator/mod.rs](src/orchestrator/mod.rs):
  - launches one agent
  - requires the first strict status envelope
  - aborts multi-terminal runs if that one-worker check fails

- Historical Qwen runs in `/tmp/sapphire-phase3-test4` and `/tmp/sapphire-phase3-test20` show:
  - immediate Qwen crash output
  - normalized failure persistence
  - no meaningful work progression

## Most likely root causes

### 1. Wrong Qwen session mode

Sapphire currently launches plain `qwen` with no runtime shaping.

Likely issue:
- Qwen needs explicit mode flags for stable terminal automation
- `YOLO` / approval mode is not being set
- the default interactive UI path may initialize features that conflict with the PTY/session shape Sapphire provides

### 2. Startup sequence mismatch

Sapphire currently:
- spawns Qwen
- waits a fixed delay
- injects the first prompt

Likely issue:
- Qwen may still be inside startup initialization when Sapphire writes the first prompt
- the IDE-connect prompt may not be the only startup gate
- the first write may land before the session is ready for normal input

### 3. PTY + screen / terminal capability mismatch

The historical error:
- `Invalid number of stops (< 2)`

Likely issue:
- Qwen is initializing terminal color/gradient/UI state in a way that is sensitive to terminal shape, environment, or output mode
- the PTY surface or environment may be missing something Qwen expects

### 4. Prompt formatting is still too early or too rich

Even with the compact adapter, Sapphire still assumes Qwen can:
- start cleanly
- receive the first prompt immediately
- return the required first envelope quickly

Likely issue:
- the very first preflight/status request needs to be even more minimal
- Qwen may need an explicit startup nudge before the real status prompt

### 5. Environment mismatch between direct and spawned runs

Sapphire should compare:
- direct `qwen` invocation that survives
- Sapphire-spawned `qwen` invocation that fails

Likely issue lives in one of:
- flags
- env vars
- working directory state
- PTY dimensions
- stdin timing
- output mode

## Non-goals

Do not:
- add more orchestration features
- redesign the control plane
- add more persistence or UI work
- expand platform scope

This is a runtime compatibility fix.

## Fix strategy

### Phase 1. Reproduce with one Qwen worker only

Run one-worker experiments only until the first strict status envelope succeeds.

Compare:
- direct interactive `qwen`
- direct non-interactive `qwen`
- Sapphire-spawned `qwen` preflight

Capture:
- exact stdout/stderr
- first 10 seconds of PTY output
- startup prompt timing
- whether the IDE prompt appears
- whether the error happens before or after first input

### Phase 2. Add Qwen-specific launch shaping

Test the minimum combinations of:
- `--yolo`
- `--approval-mode yolo`
- any Qwen output mode or debug mode that reduces TUI complexity
- any flag that stabilizes interactive startup

Keep only the smallest set that materially fixes startup.

### Phase 3. Add Qwen-specific startup automation

Potential minimum changes:
- one startup newline or warmup input
- longer Qwen prompt delay if required
- additional startup rules if another trust/setup prompt exists
- a stricter first preflight prompt:

```text
Status only.
Reply only with:
STATE: progressing
SUMMARY: one short sentence
FILES: NONE
BLOCKER: NONE
DONE: no
```

### Phase 4. Compare environment deltas

Explicitly compare direct vs spawned values for:
- cwd
- env
- PTY dimensions
- inherited terminal settings
- startup writes

If needed, add only the missing Qwen-specific environment shaping.

### Phase 5. Keep the preflight hard gate

Do not weaken the preflight.

The preflight must keep blocking multi-terminal runs until:
- one spawned interactive Qwen worker survives startup
- one strict status envelope is emitted successfully

## Validation order

### 1. Single-worker Qwen preflight

Must prove:
- spawned interactive Qwen survives startup
- first strict status envelope is emitted

### 2. Supervisor-only Qwen planning preflight

Must prove:
- live supervisor session survives
- produces a usable plan envelope without deterministic fallback

### 3. Four-terminal real run in isolated `/tmp`

Must prove:
- supervisor plans live
- workers run real tasks
- AGENTS bootstrap happens
- at least one coordination event happens
- validation runs
- artifacts are created

### 4. Twenty-terminal stress run in isolated `/tmp`

Must prove:
- launch survives at scale
- supervision still works
- state persists correctly
- failure modes remain observable and durable

## Minimum code changes expected

Only in:
- [src/agent/mod.rs](src/agent/mod.rs)
- [src/runtime/mod.rs](src/runtime/mod.rs)
- [src/orchestrator/mod.rs](src/orchestrator/mod.rs)
- optionally [src/adapter.rs](src/adapter.rs) if Qwen needs an even smaller first-envelope path

## Exit criteria

The Qwen live path is fixed only when all of the following are true:

- one spawned interactive Qwen preflight passes
- supervisor live planning passes without deterministic fallback
- a 4-terminal isolated task run completes materially
- a 20-terminal isolated run stays alive and supervisable
- SQLite shows real workers, real events, real validation, and at least one real coordination record

## Blunt summary

This is not an orchestration-design problem anymore.

It is a Qwen runtime compatibility problem across:
- startup mode
- PTY behavior
- first input timing
- first-envelope prompting

Fix that path first. Do not add more platform logic until one-worker Qwen preflight is stable.
