#!/bin/zsh

# Deliberately not using set -e.
# This script supervises tmux + qwen processes and should not die on harmless non-zero exits.

REPO="/Users/harshitduggal/workspace/sapphire-agent-Factory"
WORKER_COUNT=2
MISSION="Read AGENTS.md and AGENT-HANDOFF.md in the repo root. Read both files fully. Then reply: 'I acknowledge that I have read and understood the Sapphire Agent Factory architecture and codebase.' That is all. Do not edit any files. Do not run any commands. Just read and acknowledge."

WORKER_SESSION="sapphire-workers"
LIVE_SUPERVISOR_SESSION="sapphire-live-supervisor"

PLAN_FILE="/tmp/sapphire-supervisor-plan.json"
SUPERVISOR_PROMPT_FILE="/tmp/qwen-supervisor-prompt.txt"
SUPERVISOR_STDOUT="/tmp/qwen-supervisor-stdout.txt"
SUPERVISOR_STDERR="/tmp/qwen-supervisor-stderr.txt"
SUPERVISOR_COMBINED="/tmp/qwen-supervisor-combined.txt"

PROMPT_DIR="/tmp/sapphire-worker-prompts"
AGENT_STATES_FILE="/tmp/sapphire-agent-states.json"

STATE_DIR="$REPO/.sp"
WORKERS_STATE_DIR="$STATE_DIR/workers"
WATCHDOG_LOG="$STATE_DIR/watchdog-log.jsonl"
FINAL_SUMMARY="$STATE_DIR/control/final-summary.md"

WATCHDOG_INTERVAL=5
WATCHDOG_MAX_TICKS=360

log() {
  printf '%s\n' "$*"
}

# ========================================================================
# Test mode: run with --test to verify file-based state without launching
# ========================================================================
test_file_based_state() {
  log "=== Running file-based state tests ==="
  local test_dir="/tmp/sapphire-test-$$"
  mkdir -p "$test_dir/workers/TestAgent-1"

  # Test 1: Write a valid status.json and read it back
  log "  Test 1: Write + read status.json"
  echo '{"state":"progressing","summary":"writing code","files":["src/main.rs"],"commands":[],"risks":[]}' > "$test_dir/workers/TestAgent-1/status.json"
  WORKERS_STATE_DIR="$test_dir/workers" read_worker_state_from_file "TestAgent-1" >/dev/null 2>&1
  local result
  result=$(WORKERS_STATE_DIR="$test_dir/workers" read_worker_state_from_file "TestAgent-1")
  if [ -n "$result" ]; then
    log "    PASS: Read status.json -> $result"
  else
    log "    FAIL: Could not read status.json"
  fi

  # Test 2: Malformed JSON should be silently skipped
  log "  Test 2: Malformed JSON"
  echo '{broken json' > "$test_dir/workers/TestAgent-1/status.json"
  result=$(WORKERS_STATE_DIR="$test_dir/workers" read_worker_state_from_file "TestAgent-1")
  if [ -z "$result" ]; then
    log "    PASS: Malformed JSON correctly ignored"
  else
    log "    FAIL: Malformed JSON was returned: $result"
  fi

  # Test 3: Missing file should return empty
  log "  Test 3: Missing status file"
  result=$(WORKERS_STATE_DIR="$test_dir/workers" read_worker_state_from_file "NonExistent")
  if [ -z "$result" ]; then
    log "    PASS: Missing file returns empty"
  else
    log "    FAIL: Missing file returned: $result"
  fi

  # Test 4: JSON without "state" key should be ignored
  log "  Test 4: JSON without state key"
  echo '{"summary":"no state key"}' > "$test_dir/workers/TestAgent-1/status.json"
  result=$(WORKERS_STATE_DIR="$test_dir/workers" read_worker_state_from_file "TestAgent-1")
  if [ -z "$result" ]; then
    log "    PASS: JSON without state key correctly ignored"
  else
    log "    FAIL: JSON without state key was returned: $result"
  fi

  rm -rf "$test_dir"
  log "=== Tests complete ==="
}

need_cmd() {
  local name="$1"
  if ! command -v "$name" >/dev/null 2>&1; then
    log "ERROR: Missing required command: $name"
    exit 1
  fi
}

check_dependencies() {
  need_cmd qwen
  need_cmd tmux
  need_cmd python3
}

prepare_state() {
  mkdir -p "$STATE_DIR/control"
  mkdir -p "$WORKERS_STATE_DIR"
  rm -rf "$PROMPT_DIR"
  mkdir -p "$PROMPT_DIR"
  : > "$WATCHDOG_LOG"
}

extract_plan() {
  PLAN_FILE="$PLAN_FILE" SUPERVISOR_COMBINED="$SUPERVISOR_COMBINED" python3 <<'PY'
import json
import os
import re
import sys

combined = os.environ["SUPERVISOR_COMBINED"]
plan_file = os.environ["PLAN_FILE"]

try:
    text = open(combined, "r", encoding="utf-8", errors="replace").read()
except Exception as e:
    print(f"INVALID:READ_FAILED:{e}")
    sys.exit(0)

start_marker = "BEGIN_SAPPHIRE_PLAN_JSON"
end_marker = "END_SAPPHIRE_PLAN_JSON"
plan_json = None

start = text.rfind(start_marker)
if start >= 0:
    rest = text[start + len(start_marker):]
    end = rest.find(end_marker)
    if end >= 0:
        plan_json = rest[:end].strip()

if not plan_json:
    m = re.search(r"```json\s*\n(\{.*?\})\s*\n```", text, re.DOTALL)
    if not m:
        m = re.search(r"```\s*\n(\{.*?\})\s*\n```", text, re.DOTALL)
    if m:
        plan_json = m.group(1).strip()

if not plan_json:
    print("INVALID:NO_PLAN_BLOCK")
    sys.exit(0)

try:
    plan = json.loads(plan_json)
except Exception as e:
    print(f"INVALID:BAD_JSON:{e}")
    sys.exit(0)

with open(plan_file, "w", encoding="utf-8") as f:
    json.dump(plan, f, indent=2, ensure_ascii=False)

packets = len(plan.get("worker_packets", []))
team = len(plan.get("team_activation", []))
print(f"VALID:{packets}:{team}")
PY
}

init_agent_states() {
  PLAN_FILE="$PLAN_FILE" AGENT_STATES_FILE="$AGENT_STATES_FILE" ACTUAL_WORKERS="$ACTUAL_WORKERS" python3 <<'PY'
import json
import os

plan_file = os.environ["PLAN_FILE"]
out_file = os.environ["AGENT_STATES_FILE"]
actual_workers = int(os.environ["ACTUAL_WORKERS"])

agents = []
try:
    plan = json.load(open(plan_file, "r", encoding="utf-8"))
    packets = plan.get("worker_packets", [])
    for i, pkt in enumerate(packets):
        agents.append({
            "name": pkt.get("display_name", pkt.get("worker_id", f"Engineer-{i+1}")),
            "state": "awaiting-status",
            "summary": "waiting for first SAPPHIRE_STATUS",
            "files": [],
            "commands": [],
            "risks": [],
            "overlap": "",
            "last_seen": 0,
        })
except Exception:
    pass

if not agents:
    agents = [{
        "name": f"Engineer-{i+1}",
        "state": "awaiting-status",
        "summary": "waiting for first SAPPHIRE_STATUS",
        "files": [],
        "commands": [],
        "risks": [],
        "overlap": "",
        "last_seen": 0,
    } for i in range(actual_workers)]

with open(out_file, "w", encoding="utf-8") as f:
    json.dump(agents, f, indent=2, ensure_ascii=False)
PY
}

# Read worker state from file (primary) — returns JSON or empty string
read_worker_state_from_file() {
  local display_name="$1"
  local status_file="$WORKERS_STATE_DIR/$display_name/status.json"
  if [ -f "$status_file" ]; then
    STATUS_FILE="$status_file" python3 -c '
import json, sys, os
path = os.environ["STATUS_FILE"]
try:
    obj = json.load(open(path, "r", encoding="utf-8"))
    if isinstance(obj, dict) and "state" in obj:
        print(json.dumps(obj, ensure_ascii=False, separators=(",", ":")))
except Exception:
    pass
' 2>/dev/null
  fi
}

# Read worker state from tmux pane (fallback) — returns JSON or empty string
extract_status_json_from_pane() {
  local pane="$1"
  tmux capture-pane -t "$pane" -p -S -500 2>/dev/null | python3 -c '
import sys, re, json

text = sys.stdin.read()

# Strip ANSI escape codes
text = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", text)
text = re.sub(r"\x1b\][^\x07]*\x07", "", text)
text = re.sub(r"\x1b\^[^\x1b]*\x1b", "", text)

# Strategy 1: Single-line SAPPHIRE_STATUS {...}
matches = re.findall(r"SAPPHIRE_STATUS\s+(\{[^\n]+\})", text)
for raw in reversed(matches):
    try:
        obj = json.loads(raw)
        if "state" in obj:
            print(json.dumps(obj, ensure_ascii=False, separators=(",", ":")))
            sys.exit(0)
    except Exception:
        continue

# Strategy 2: Brace-depth extraction after SAPPHIRE_STATUS prefix
pos = text.rfind("SAPPHIRE_STATUS")
if pos >= 0:
    rest = text[pos + len("SAPPHIRE_STATUS"):]
    rest = rest.lstrip()
    if rest.startswith("{"):
        depth = 0
        end = 0
        for i, ch in enumerate(rest):
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    end = i + 1
                    break
        if end > 0:
            raw = rest[:end]
            try:
                obj = json.loads(raw)
                if "state" in obj:
                    print(json.dumps(obj, ensure_ascii=False, separators=(",", ":")))
                    sys.exit(0)
            except Exception:
                pass

# Strategy 3: Find any JSON object with "state" key in 2000-char window
for m in re.finditer(r"SAPPHIRE_STATUS", text):
    ws = max(0, m.start() - 100)
    we = min(len(text), m.end() + 2000)
    window = text[ws:we]
    for jm in re.finditer(r"\{[^{}]*\}", window):
        try:
            obj = json.loads(jm.group(0))
            if "state" in obj:
                print(json.dumps(obj, ensure_ascii=False, separators=(",", ":")))
                sys.exit(0)
        except Exception:
            continue
' 2>/dev/null
}

update_agent_state() {
  local idx="$1"
  local now_ts="$2"
  local status_json="$3"

  IDX="$idx" NOW_TS="$now_ts" STATUS_JSON="$status_json" AGENT_STATES_FILE="$AGENT_STATES_FILE" python3 <<'PY'
import json
import os

idx = int(os.environ["IDX"])
now_ts = int(os.environ["NOW_TS"])
status = json.loads(os.environ["STATUS_JSON"])
path = os.environ["AGENT_STATES_FILE"]

agents = json.load(open(path, "r", encoding="utf-8"))
agent = agents[idx]

agent["state"] = status.get("state", agent.get("state", "unknown"))
agent["summary"] = status.get("summary", agent.get("summary", ""))
agent["files"] = status.get("files", [])
agent["commands"] = status.get("commands", [])
agent["risks"] = status.get("risks", [])
agent["overlap"] = status.get("overlap", "")
agent["last_seen"] = now_ts

json.dump(agents, open(path, "w", encoding="utf-8"), indent=2, ensure_ascii=False)
PY
}

mark_stale_agents() {
  local now_ts="$1"
  NOW_TS="$now_ts" AGENT_STATES_FILE="$AGENT_STATES_FILE" python3 <<'PY'
import json
import os

now_ts = int(os.environ["NOW_TS"])
path = os.environ["AGENT_STATES_FILE"]
agents = json.load(open(path, "r", encoding="utf-8"))

for agent in agents:
    last_seen = int(agent.get("last_seen", 0))
    if last_seen == 0:
        continue
    if agent.get("state") in {"done_claimed", "validated", "failed", "exited"}:
        continue
    if now_ts - last_seen >= 60:
        agent["state"] = "stale"
        if not agent.get("summary"):
            agent["summary"] = "no recent status update"

json.dump(agents, open(path, "w", encoding="utf-8"), indent=2, ensure_ascii=False)
PY
}

render_json_state_view() {
  local ts="$1"
  TS="$ts" AGENT_STATES_FILE="$AGENT_STATES_FILE" python3 <<'PY'
import json
import os

agents = json.load(open(os.environ["AGENT_STATES_FILE"], "r", encoding="utf-8"))
print(f"[{os.environ.get('TS', '')}] status")

for a in agents:
    files = a.get("files", [])
    file_count = len(files) if isinstance(files, list) else 0
    risks = a.get("risks", [])
    risk_count = len(risks) if isinstance(risks, list) else 0
    summary = str(a.get("summary", ""))[:100]
    print(f"- {a.get('name', 'unknown')} | {a.get('state', 'unknown')} | files={file_count} | risks={risk_count} | {summary}")
PY
}

append_watchdog_log() {
  local tick="$1"
  local ts="$2"
  TICK="$tick" TS="$ts" AGENT_STATES_FILE="$AGENT_STATES_FILE" WATCHDOG_LOG="$WATCHDOG_LOG" python3 <<'PY'
import json
import os

tick = int(os.environ["TICK"])
ts = os.environ["TS"]
agents = json.load(open(os.environ["AGENT_STATES_FILE"], "r", encoding="utf-8"))

with open(os.environ["WATCHDOG_LOG"], "a", encoding="utf-8") as f:
    entry = {
        "tick": tick,
        "ts": ts,
        "agents": agents,
    }
    f.write(json.dumps(entry, ensure_ascii=False) + "\n")
PY
}

terminal_count() {
  AGENT_STATES_FILE="$AGENT_STATES_FILE" python3 -c '
import json, os
agents = json.load(open(os.environ["AGENT_STATES_FILE"], "r", encoding="utf-8"))
terminal = {"done_claimed", "validated", "failed", "exited"}
print(sum(1 for a in agents if a.get("state") in terminal))
'
}

compact_snapshot_json() {
  AGENT_STATES_FILE="$AGENT_STATES_FILE" python3 -c '
import json, os
agents = json.load(open(os.environ["AGENT_STATES_FILE"], "r", encoding="utf-8"))
snapshot = {
    "agents": [
        {
            "name": a.get("name"),
            "state": a.get("state"),
            "summary": a.get("summary"),
            "files": a.get("files", []),
            "risks": a.get("risks", []),
        }
        for a in agents
    ]
}
print(json.dumps(snapshot, ensure_ascii=False, separators=(",", ":")))
'
}

start_live_supervisor() {
  log ""
  log "=== Phase 5: Starting live supervisor ==="

  tmux kill-session -t "$LIVE_SUPERVISOR_SESSION" 2>/dev/null || true
  tmux new-session -d -s "$LIVE_SUPERVISOR_SESSION" -c "$REPO" -x 220 -y 50
  tmux set -t "$LIVE_SUPERVISOR_SESSION" remain-on-exit on >/dev/null 2>&1 || true

  tmux send-keys -t "$LIVE_SUPERVISOR_SESSION" -l "qwen --approval-mode yolo"
  tmux send-keys -t "$LIVE_SUPERVISOR_SESSION" Enter

  log "  Waiting for live supervisor to boot..."
  sleep 8

  # Write briefing to a file and send via load-buffer (avoids quoting issues)
  local briefing_file="/tmp/sapphire-supervisor-briefing.txt"
  cat > "$briefing_file" <<'EOF'
You are the Sapphire live supervisor. You are watching a team of AI agents working on a mission.

Every few seconds I will send you a status snapshot showing:
- Each agent's name, current state (progressing/blocked/done_claimed/validated/failed), what they are doing, files touched, and risks.
- How many agents are done vs still working.

For each snapshot, respond with EXACTLY one line starting with:
SUPERVISOR_MD: **Overall:** brief summary | **Blockers:** any blockers or none | **Next:** what should happen next

Rules:
- Never run commands. Never edit files. Only observe and respond.
- Keep each response to one line only. No code fences. No extra paragraphs.
- If all agents are progressing with no blockers, say so.
- If any agent is stalled, blocked, or failed, flag it clearly.
- When agents finish, note what was accomplished.
EOF

  tmux load-buffer -b sapphire_briefing "$briefing_file"
  tmux paste-buffer -b sapphire_briefing -t "$LIVE_SUPERVISOR_SESSION"
  sleep 1.0
  tmux send-keys -t "$LIVE_SUPERVISOR_SESSION" Enter
  sleep 1.0
  tmux delete-buffer -b sapphire_briefing 2>/dev/null || true
  rm -f "$briefing_file"
}

push_snapshot_to_supervisor() {
  local snapshot="$1"
  local tick="$2"
  local done_count="$3"
  local total="$4"

  local snapshot_file="/tmp/sapphire-snapshot-$$.txt"
  cat > "$snapshot_file" <<SNEOF
Snapshot #$tick — $done_count of $total agents done:
$snapshot

Reply with one line:
SUPERVISOR_MD: **Overall:** ... | **Blockers:** ... | **Next:** ...
SNEOF

  tmux load-buffer -b sapphire_snapshot "$snapshot_file"
  tmux paste-buffer -b sapphire_snapshot -t "$LIVE_SUPERVISOR_SESSION"
  sleep 0.5
  tmux send-keys -t "$LIVE_SUPERVISOR_SESSION" Enter
  sleep 3.0
  tmux delete-buffer -b sapphire_snapshot 2>/dev/null || true
  rm -f "$snapshot_file"
}

latest_supervisor_markdown() {
  tmux capture-pane -t "$LIVE_SUPERVISOR_SESSION" -p -S -500 2>/dev/null | python3 -c '
import sys, re

lines = sys.stdin.read().splitlines()

# Strategy 1: Exact SUPERVISOR_MD: prefix
matches = [l.strip() for l in lines if l.strip().startswith("SUPERVISOR_MD:")]
if matches:
    print(matches[-1])
    sys.exit(0)

# Strategy 2: Find last line that contains Overall/Blockers/Next keywords
for line in reversed(lines):
    if "Overall" in line or "Blockers" in line or "blockers" in line:
        print(line.strip())
        sys.exit(0)

# Strategy 3: Find any non-empty line that is not the snapshot prompt
for line in reversed(lines):
    stripped = line.strip()
    if stripped and not stripped.startswith("Snapshot #") and not stripped.startswith("SUPERVISOR_MD:") and not stripped.startswith("Reply with"):
        # Skip very short lines (single chars, tmux artifacts)
        if len(stripped) > 20:
            print(stripped)
            sys.exit(0)
' 2>/dev/null
}

write_final_summary() {
  local supervisor_md="$1"
  SUPERVISOR_MD="$supervisor_md" AGENT_STATES_FILE="$AGENT_STATES_FILE" FINAL_SUMMARY="$FINAL_SUMMARY" MISSION="$MISSION" python3 <<'PY'
import json
import os

agents = json.load(open(os.environ["AGENT_STATES_FILE"], "r", encoding="utf-8"))
out = os.environ["FINAL_SUMMARY"]
mission = os.environ["MISSION"]
supervisor_md = os.environ.get("SUPERVISOR_MD", "").strip()

lines = []
lines.append("# Final Summary")
lines.append("")
lines.append("## Mission")
lines.append(mission)
lines.append("")

if supervisor_md:
    lines.append("## Supervisor")
    lines.append(supervisor_md.replace("SUPERVISOR_MD: ", "", 1))
    lines.append("")

lines.append("## Agent States")
for a in agents:
    files = a.get("files", [])
    risks = a.get("risks", [])
    lines.append(
        f"- **{a.get('name','unknown')}** | state=`{a.get('state','unknown')}` | files={len(files) if isinstance(files, list) else 0} | risks={len(risks) if isinstance(risks, list) else 0} | {a.get('summary','')}"
    )

with open(out, "w", encoding="utf-8") as f:
    f.write("\n".join(lines) + "\n")
PY
}

check_dependencies
prepare_state

# Test mode: run with --test to verify file-based state without launching
if [[ "$1" == "--test" ]]; then
  test_file_based_state
  exit 0
fi

log "=== Phase 1: Running supervisor planner ==="

SUPERVISOR_PROMPT="You are the supervisor brain. Convert one messy mission into the execution plan Sapphire will use.

STRICT JSON OUTPUT RULES (VIOLATE ANY = INVALID):
- Output MUST be raw JSON only. NO markdown, NO code fences, NO backticks, NO explanation.
- MUST start with BEGIN_SAPPHIRE_PLAN_JSON on its own line and end with END_SAPPHIRE_PLAN_JSON on its own line.
- NOTHING outside those two markers. Not even a single word before or after.
- All strings use double quotes only. No single quotes. No trailing commas.
- NO newlines inside string values. Use spaces only.
- All arrays are actual JSON arrays [...], never strings.
- The entire block between markers MUST parse as valid JSON on first try.
- Do NOT wrap JSON in \`\`\`json or \`\`\` or any markdown formatting.

LENGTH RULES:
- EVERY string field must be 10 words or fewer.
- Total JSON must stay under 2000 characters.
- Be dense and actionable. No filler words.

EXACT JSON EXAMPLE (follow this structure, NOT the specific roles — choose roles based on YOUR analysis of the mission):
BEGIN_SAPPHIRE_PLAN_JSON
{\"mission_rewrite\":\"<10 word mission summary>\",\"team_activation\":[{\"role_type\":\"<analyze mission — pick from 13 roles>\",\"display_name\":\"<Role-Type-N>\",\"reason\":\"<why THIS role for THIS mission>\"}],\"workstreams\":[{\"id\":\"<short id>\",\"name\":\"<workstream name>\",\"execution\":\"parallel|dependent|validation|integration\",\"owned_scope\":\"<what files/areas>\",\"success_criteria\":[\"<criterion>\"],\"depends_on\":[]}],\"risk_map\":[{\"zone\":\"<area>\",\"risk\":\"<risk type>\",\"mitigation\":\"<how to prevent>\"}],\"worker_packets\":[{\"worker_id\":\"<display name>\",\"role\":\"<Role Title>\",\"role_type\":\"<same as team_activation>\",\"display_name\":\"<same as team_activation>\",\"starting_angle\":\"<unique approach>\",\"owned_scope\":\"<files/areas>\",\"explicit_task\":\"<what to do>\",\"out_of_scope\":\"<what NOT to touch>\",\"definition_of_done\":[\"<criteria>\"],\"required_evidence\":[\"<proof>\"],\"blocker_protocol\":\"report blockers immediately\",\"conflict_warning\":\"claim files first\",\"communication_rules\":[\"mail blockers\"],\"validation_standard\":[\"pass checks\"],\"expected_output_format\":[\"summary\"]}],\"supervision_strategy\":\"<tight|normal|loose>\"}
END_SAPPHIRE_PLAN_JSON

CRITICAL: The JSON example above shows the STRUCTURE only. Do NOT copy the role_types from the example. Analyze the actual mission and activate ONLY the roles that the mission genuinely needs. A docs-only mission might need only software-engineer. A security audit needs security-engineer. A full feature needs multiple roles. YOU decide based on the mission, not the example.

Rules:
- Worker packet count must equal ${WORKER_COUNT}.
- There are exactly 13 role TYPES available. Each type can have MULTIPLE instances. For example: 5 software-engineers = Engineer-1, Engineer-2, Engineer-3, Engineer-4, Engineer-5. The numeric suffix means you can spawn as many of any role type as the mission needs.
- role_type must be one of these 13 types: software-engineer, research-engineer, validation-engineer, architecture-engineer, security-engineer, debug-and-review-engineer, testing-and-automation-engineer, designer-engineer, sales-engineer, solutions-engineer, customer-success-engineer, product-engineer, compliance-engineer
- display_name uses function-first naming with numeric identity: Engineer-N (software-engineer), Designer-N (designer-engineer), Reviewer-N (debug-and-review-engineer), Architect-N (architecture-engineer), Security-N (security-engineer), Validator-N (validation-engineer), QA-N (testing-and-automation-engineer), Researcher-N (research-engineer), Sales-N (sales-engineer), Solutions-N (solutions-engineer), CustomerSuccess-N (customer-success-engineer), Product-N (product-engineer), Compliance-N (compliance-engineer)
- Supervisor analyzes the mission and activates ONLY the role types the mission genuinely needs. You can pick 1 role type and spawn all ${WORKER_COUNT} workers as that type, OR pick 5 types and distribute workers across them. YOU decide the mix.
- A coding-heavy mission might need 8 software-engineers + 2 validation-engineers. A security audit might need 3 security-engineers + 2 software-engineers. A documentation mission might need 4 software-engineers + 1 designer-engineer.
- Each packet has materially different starting angle — even same-type workers must attack different slices.
- execution must be one of: parallel, dependent, validation, integration.

Mission:
${MISSION}"

printf '%s\n' "$SUPERVISOR_PROMPT" > "$SUPERVISOR_PROMPT_FILE"
qwen --screen-reader --approval-mode yolo < "$SUPERVISOR_PROMPT_FILE" > "$SUPERVISOR_STDOUT" 2> "$SUPERVISOR_STDERR" &
QWEN_PID=$!

PLAN_FOUND=false
ACTUAL_WORKERS="$WORKER_COUNT"
START_TIME=$(date +%s)

for attempt in $(seq 1 60); do
  sleep 5
  ELAPSED=$(( $(date +%s) - START_TIME ))

  if ! kill -0 "$QWEN_PID" 2>/dev/null; then
    wait "$QWEN_PID" 2>/dev/null || true
    cat "$SUPERVISOR_STDOUT" "$SUPERVISOR_STDERR" > "$SUPERVISOR_COMBINED"
    BYTES=$(wc -c < "$SUPERVISOR_COMBINED" 2>/dev/null || echo "0")
    log "  Planner finished after ${ELAPSED}s (${BYTES} bytes output)"
    EXTRACTION_RESULT=$(extract_plan)
    if [[ "$EXTRACTION_RESULT" == VALID:* ]]; then
      PLAN_FOUND=true
      WORKER_PACKETS=$(printf '%s' "$EXTRACTION_RESULT" | cut -d: -f2)
      TEAM_SIZE=$(printf '%s' "$EXTRACTION_RESULT" | cut -d: -f3)
      ACTUAL_WORKERS="$WORKER_PACKETS"
      log "  Valid plan extracted ($WORKER_PACKETS worker packets, $TEAM_SIZE team members)"
      break
    else
      log "ERROR: Planner output invalid: $EXTRACTION_RESULT"
      exit 1
    fi
  else
    log "  Planner still running at ${ELAPSED}s..."
  fi
done

if [ "$PLAN_FOUND" != true ]; then
  log "ERROR: Could not extract valid plan."
  exit 1
fi

log ""
log "=== Phase 2: Launching worker session ==="
tmux kill-session -t "$WORKER_SESSION" 2>/dev/null || true

# Grid formula: N -> cols x rows, then pick layout
# 2->2x1  3->3x1  4->2x2  5->5x1  6->3x2  7->7x1  8->4x2  9->3x3  10->5x2  11->11x1  12->4x3
cols=1
rows=1
case "$ACTUAL_WORKERS" in
  2)  cols=2;  rows=1 ;;
  3)  cols=3;  rows=1 ;;
  4)  cols=2;  rows=2 ;;
  5)  cols=5;  rows=1 ;;
  6)  cols=3;  rows=2 ;;
  7)  cols=7;  rows=1 ;;
  8)  cols=4;  rows=2 ;;
  9)  cols=3;  rows=3 ;;
  10) cols=5;  rows=2 ;;
  11) cols=11; rows=1 ;;
  12) cols=4;  rows=3 ;;
  *)
    # Fallback: tile evenly
    rows=$(( (ACTUAL_WORKERS + 3) / 4 ))
    if (( rows < 1 )); then rows=1; fi
    cols=$(( (ACTUAL_WORKERS + rows - 1) / rows ))
    ;;
esac

# Size the session window to fit the grid comfortably
win_w=$(( cols * 55 ))
win_h=$(( rows * 22 ))
if (( win_w < 200 )); then win_w=200; fi
if (( win_h < 40 )); then win_h=40; fi

tmux new-session -d -s "$WORKER_SESSION" -c "$REPO" -x "$win_w" -y "$win_h"
tmux set -t "$WORKER_SESSION" remain-on-exit on >/dev/null 2>&1 || true
tmux set -t "$WORKER_SESSION" window-size latest >/dev/null 2>&1 || true

for (( i=1; i<ACTUAL_WORKERS; i++ )); do
  tmux split-window -t "$WORKER_SESSION" >/dev/null 2>&1 || true
done

# Apply the optimal layout after all splits
if (( cols == 1 )); then
  tmux select-layout -t "$WORKER_SESSION" even-vertical >/dev/null 2>&1 || true
elif (( rows == 1 )); then
  tmux select-layout -t "$WORKER_SESSION" even-horizontal >/dev/null 2>&1 || true
else
  # For multi-row grids, use tiled and let tmux distribute
  tmux select-layout -t "$WORKER_SESSION" tiled >/dev/null 2>&1 || true
fi

sleep 1

PANE_IDS=($(tmux list-panes -t "$WORKER_SESSION" -F "#{pane_id}" 2>/dev/null))
for pane in "${PANE_IDS[@]}"; do
  tmux send-keys -t "$pane" -l "qwen --approval-mode yolo"
  tmux send-keys -t "$pane" Enter
done

log "  Waiting for workers to boot..."
sleep 8
PANE_IDS=($(tmux list-panes -t "$WORKER_SESSION" -F "#{pane_id}" 2>/dev/null))

# Wake each pane's readline BEFORE pasting the real prompt.
# Qwen's TUI may still be in boot animation — a single Enter wakes the readline.
for pane in "${PANE_IDS[@]}"; do
  tmux send-keys -t "$pane" Enter
done
sleep 1.0

log ""
log "=== Phase 3: Building worker prompts ==="

PLAN_FILE="$PLAN_FILE" PROMPT_DIR="$PROMPT_DIR" MISSION="$MISSION" REPO="$REPO" WORKERS_STATE_DIR="$WORKERS_STATE_DIR" python3 <<'PY'
import json
import os

plan = json.load(open(os.environ["PLAN_FILE"], "r", encoding="utf-8"))
prompt_dir = os.environ["PROMPT_DIR"]
mission = plan.get("mission_rewrite", os.environ["MISSION"])
repo = os.environ["REPO"]
workers_dir = os.environ["WORKERS_STATE_DIR"]
packets = plan.get("worker_packets", [])

for i, pkt in enumerate(packets):
    display_name = pkt.get("display_name", pkt.get("worker_id", f"Engineer-{i+1}"))
    role_type = pkt.get("role_type", "software-engineer")
    role = pkt.get("role", "Software Engineer")
    status_file = os.path.join(workers_dir, display_name, "status.json")

    lines = [
        f"You are {display_name}.",
        f"Role Type: {role_type} ({role})",
        "",
        f"Mission: {mission}",
        "",
        f"Starting Angle: {pkt.get('starting_angle', '')}",
        f"Owned Scope: {pkt.get('owned_scope', '')}",
        f"Explicit Task: {pkt.get('explicit_task', '')}",
        f"Out of Scope: {pkt.get('out_of_scope', '')}",
        "",
        "Definition of Done:",
    ]
    for x in pkt.get("definition_of_done", []):
        lines.append(f"- {x}")

    lines.extend(["", "Required Evidence:"])
    for x in pkt.get("required_evidence", []):
        lines.append(f"- {x}")

    lines.extend(["", "Communication Rules:"])
    for x in pkt.get("communication_rules", []):
        lines.append(f"- {x}")

    lines.extend(["", "Validation Standard:"])
    for x in pkt.get("validation_standard", []):
        lines.append(f"- {x}")

    lines.extend([
        "",
        f"Blocker Protocol: {pkt.get('blocker_protocol', 'Report blockers immediately.')}",
        f"Conflict Warning: {pkt.get('conflict_warning', 'Do not modify files outside your owned scope.')}",
        "",
        "Before starting work, read AGENTS.md in the repo root.",
        "",
        "=== STATUS REPORTING — STATUS FILE IS MANDATORY ===",
        "",
        "You MUST write a status file EVERY time your state changes. This is NOT optional.",
        "It is how the supervisor tracks what you are doing.",
        "",
        f"STATUS FILE PATH (always write here, always overwrite):",
        f"  {status_file}",
        "",
        "HOW TO WRITE IT:",
        "  1. Create the parent directory if it does not exist.",
        "  2. Write one valid JSON object to the file. Nothing else. No markdown fences.",
        "  3. Overwrite the entire file each time — do NOT append.",
        "",
        "EXACT JSON FORMAT (use this exact structure):",
        '  {"state":"progressing","summary":"what you are doing right now","files":["file1","file2"],"commands":["cmd1","cmd2"],"risks":["any risks"],"overlap":"none or describe conflicts"}',
        "",
        "VALID STATES:",
        "  progressing | blocked | done_claimed | validated | failed",
        "",
        "WRITE THE STATUS FILE when:",
        "  - You start working (state: progressing)",
        "  - You hit a blocker (state: blocked)",
        "  - You finish your task (state: done_claimed)",
        "  - You validate your work (state: validated)",
        "  - You fail or get stuck (state: failed)",
        "",
        "OPTIONAL — also emit a terminal line in your output as backup:",
        '  SAPPHIRE_STATUS {"state":"...","summary":"...","files":["..."],"commands":[],"risks":[],"overlap":"..."}',
        "The status FILE is the real thing. The terminal line is only a backup.",
        "",
        "Begin work now."
    ])

    with open(os.path.join(prompt_dir, f"{i}.txt"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
PY

ACTUAL_PROMPTS=$(ls "$PROMPT_DIR"/*.txt 2>/dev/null | wc -l | tr -d ' ')
if [ -z "$ACTUAL_PROMPTS" ] || [ "$ACTUAL_PROMPTS" -eq 0 ] 2>/dev/null; then
  log "ERROR: No worker prompts generated."
  exit 1
fi

log "  Injecting $ACTUAL_PROMPTS prompts..."
for idx in $(seq 0 $((ACTUAL_PROMPTS - 1))); do
  prompt_file="$PROMPT_DIR/$idx.txt"
  if [ ! -f "$prompt_file" ]; then
    continue
  fi

  pane_idx=$((idx + 1))
  if (( pane_idx > ${#PANE_IDS[@]} )); then
    log "  WARNING: No pane for worker index $idx"
    continue
  fi

  pane="${PANE_IDS[$pane_idx]}"
  if [ -z "$pane" ]; then
    log "  WARNING: Empty pane target for worker index $idx"
    continue
  fi

  worker_name=$(head -n 1 "$prompt_file")
  log "  $worker_name -> $pane"

  # Use load-buffer from file — atomic, no shell escaping issues
  tmux load-buffer -b sapphire_prompt "$prompt_file"
  tmux paste-buffer -b sapphire_prompt -t "$pane"
  # Wait for paste to fully arrive before pressing Enter
  sleep 1.0
  tmux send-keys -t "$pane" Enter
  sleep 0.5
done
tmux delete-buffer -b sapphire_prompt 2>/dev/null || true

log ""
log "=== Phase 4: Opening Ghostty ==="
if [ -d "/Applications/Ghostty.app" ] && command -v osascript >/dev/null 2>&1; then
  SESSION="$WORKER_SESSION" osascript <<'OSA'
tell application "Ghostty"
    activate
    set win to new window
    delay 0.3
    set term to focused terminal of selected tab of win
    input text ("tmux attach-session -t " & (system attribute "SESSION") & "\n") to term
end tell
OSA
  log "  Ghostty opened on worker session."
elif command -v open >/dev/null 2>&1; then
  open -na /Applications/Ghostty.app --args -e /bin/zsh -lc "tmux attach-session -t $WORKER_SESSION" >/dev/null 2>&1 || true
  log "  Ghostty opened on worker session (fallback)."
else
  log "  Ghostty not found. Attach manually: tmux attach-session -t $WORKER_SESSION"
fi

start_live_supervisor
init_agent_states

log ""
log "=== Phase 6: JSON watchdog ==="
log "  Only parsed JSON state is printed."
log "  No raw pane spam. No broken Python stderr spam."
log ""

LAST_SUPERVISOR_MD=""
for (( TICK=1; TICK<=WATCHDOG_MAX_TICKS; TICK++ )); do
  NOW_TS=$(date +%s)
  TS=$(date +"%H:%M:%S")

  PANE_IDS=($(tmux list-panes -t "$WORKER_SESSION" -F "#{pane_id}" 2>/dev/null))
  log "  Worker panes: ${PANE_IDS[*]}"

  for idx in $(seq 0 $((ACTUAL_PROMPTS - 1))); do
    pane_idx=$((idx + 1))
    if (( pane_idx > ${#PANE_IDS[@]} )); then
      continue
    fi

    pane="${PANE_IDS[$pane_idx]}"
    if [ -z "$pane" ]; then
      continue
    fi

    # Get display name for file lookup
    display_name=$(IDX="$idx" AGENT_STATES_FILE="$AGENT_STATES_FILE" python3 -c '
import json, os
idx = int(os.environ["IDX"])
agents = json.load(open(os.environ["AGENT_STATES_FILE"], "r", encoding="utf-8"))
print(agents[idx].get("name", ""))
' 2>/dev/null)

    status_json=""

    # === PRIMARY: Read from status file (file-based state) ===
    if [ -n "$display_name" ]; then
      status_json=$(read_worker_state_from_file "$display_name")
    fi

    # === FALLBACK 1: tmux capture-pane with multi-strategy extraction ===
    if [ -z "$status_json" ]; then
      status_json=$(extract_status_json_from_pane "$pane")
    fi

    # === FALLBACK 2: No directive found — mark as progressing if active ===
    if [ -z "$status_json" ]; then
      IDX="$idx" NOW_TS="$NOW_TS" AGENT_STATES_FILE="$AGENT_STATES_FILE" python3 <<'PY'
import json
import os

idx = int(os.environ["IDX"])
now_ts = int(os.environ["NOW_TS"])
path = os.environ["AGENT_STATES_FILE"]

agents = json.load(open(path, "r", encoding="utf-8"))
agent = agents[idx]

# Skip terminal states
if agent.get("state") in {"validated", "done_claimed", "failed", "exited"}:
    pass
elif agent.get("state") == "awaiting-status":
    # First tick with no directive — agent is working
    agent["state"] = "progressing"
    agent["summary"] = "working (no directive yet)"
    agent["last_seen"] = now_ts
else:
    agent["last_seen"] = now_ts

json.dump(agents, open(path, "w", encoding="utf-8"), indent=2, ensure_ascii=False)
PY
    else
      update_agent_state "$idx" "$NOW_TS" "$status_json" >/dev/null 2>&1 || true
    fi
  done

  mark_stale_agents "$NOW_TS" >/dev/null 2>&1 || true
  render_json_state_view "$TS"
  append_watchdog_log "$TICK" "$TS" >/dev/null 2>&1 || true

  DONE_COUNT=$(terminal_count 2>/dev/null || echo "0")

  if (( TICK % 3 == 0 )); then
    SNAPSHOT=$(compact_snapshot_json)
    push_snapshot_to_supervisor "$SNAPSHOT" "$TICK" "$DONE_COUNT" "$ACTUAL_PROMPTS"
    SUPERVISOR_MD=$(latest_supervisor_markdown)
    if [ -n "$SUPERVISOR_MD" ] && [ "$SUPERVISOR_MD" != "$LAST_SUPERVISOR_MD" ]; then
      log "$SUPERVISOR_MD"
      LAST_SUPERVISOR_MD="$SUPERVISOR_MD"
    fi
  fi

  if (( DONE_COUNT == ACTUAL_PROMPTS )); then
    log ""
    log "All workers reached terminal state."
    break
  fi

  sleep "$WATCHDOG_INTERVAL"
done

log ""
log "=== Phase 7: Final summary ==="
write_final_summary "$LAST_SUPERVISOR_MD"
log "Saved: $FINAL_SUMMARY"
log ""
cat "$FINAL_SUMMARY"
log ""
log "To reattach workers:"
log "  tmux attach-session -t $WORKER_SESSION"
log "To reattach live supervisor:"
log "  tmux attach-session -t $LIVE_SUPERVISOR_SESSION"
log "To tail watchdog log:"
log "  tail -f $WATCHDOG_LOG"