#!/bin/zsh

# Deliberately not using set -e.
# Dead simple: 4 Codex per Ghostty tab, one tmux session per tab.
# Requires Ghostty 1.3+ for AppleScript tab automation.

REPO="/Users/harshitduggal/workspace/sapphire-agent-Factory"
PER_TAB=4
TOTAL=8
GHOSTTY_APP="Ghostty"

launch_tmux_session() {
  local session="$1"
  local batch_size="$2"

  tmux kill-session -t "$session" 2>/dev/null || true
  tmux new-session -d -s "$session" -c "$REPO" -x 240 -y 60
  tmux set -t "$session" remain-on-exit on >/dev/null 2>&1 || true
  tmux set -t "$session" window-size latest >/dev/null 2>&1 || true

  for (( i=1; i<batch_size; i++ )); do
    tmux split-window -t "$session" >/dev/null 2>&1 || true
    tmux select-layout -t "$session" tiled >/dev/null 2>&1 || true
  done

  sleep 1

  local pane_ids
  pane_ids=($(tmux list-panes -t "$session" -F "#{pane_id}" 2>/dev/null))
  for pane in "${pane_ids[@]}"; do
    tmux send-keys -t "$pane" -l "codex"
    tmux send-keys -t "$pane" Enter
  done

  echo "  Session ready: $session ($batch_size panes)"
}

ghostty_new_window_and_run() {
  local session="$1"

  SESSION="$session" osascript <<'OSA'
tell application "Ghostty"
    activate
    set win to new window
    delay 0.3
    set term to focused terminal of selected tab of win
    input text ("tmux attach-session -t " & (system attribute "SESSION") & "\n") to term
end tell
OSA
}

ghostty_new_tab_and_run() {
  local session="$1"

  SESSION="$session" osascript <<'OSA'
tell application "Ghostty"
    activate

    if (count of windows) is 0 then
        set win to new window
    else
        set win to front window
    end if

    set t to new tab in win
    delay 0.3
    select tab t
    delay 0.1
    set term to focused terminal of selected tab of win
    input text ("tmux attach-session -t " & (system attribute "SESSION") & "\n") to term
end tell
OSA
}

echo "=== Launching $TOTAL Codex instances across Ghostty tabs ==="

TAB_NUM=0
for (( start=0; start<TOTAL; start+=PER_TAB )); do
  end=$(( start + PER_TAB ))
  (( end > TOTAL )) && end=$TOTAL
  batch_size=$(( end - start ))
  SESSION="codex-batch-${TAB_NUM}"

  launch_tmux_session "$SESSION" "$batch_size"

  if (( TAB_NUM == 0 )); then
    ghostty_new_window_and_run "$SESSION"
    sleep 1.5
  else
    ghostty_new_tab_and_run "$SESSION"
    sleep 1.0
  fi

  echo "  Tab $TAB_NUM attached: $SESSION"
  TAB_NUM=$((TAB_NUM + 1))
done

echo ""
echo "=== Done ==="
echo "To reattach tab 1: tmux attach-session -t codex-batch-0"
echo "To reattach tab 2: tmux attach-session -t codex-batch-1"