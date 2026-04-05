package cmd

import (
	"fmt"
	"os"
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/spf13/cobra"
	"github.com/steveyegge/gastown/internal/beads"
	"github.com/steveyegge/gastown/internal/tmux"
	"github.com/steveyegge/gastown/internal/tui/feed"
	"github.com/steveyegge/gastown/internal/workspace"
	"golang.org/x/term"
)

var (
	feedFollow   bool
	feedLimit    int
	feedSince    string
	feedMol      string
	feedType     string
	feedRig      string
	feedNoFollow bool
	feedWindow   bool
	feedPlain    bool
	feedProblems bool
)

func init() {
	rootCmd.AddCommand(feedCmd)

	feedCmd.Flags().BoolVarP(&feedFollow, "follow", "f", false, "Stream events in real-time (default when no other flags)")
	feedCmd.Flags().BoolVar(&feedNoFollow, "no-follow", false, "Show events once and exit")
	feedCmd.Flags().IntVarP(&feedLimit, "limit", "n", 100, "Maximum number of events to show")
	feedCmd.Flags().StringVar(&feedSince, "since", "", "Show events since duration (e.g., 5m, 1h, 30s)")
	feedCmd.Flags().StringVar(&feedMol, "mol", "", "Filter by molecule/issue ID prefix")
	feedCmd.Flags().StringVar(&feedType, "type", "", "Filter by event type (create, update, delete, comment)")
	feedCmd.Flags().StringVar(&feedRig, "rig", "", "Filter events by rig name")
	feedCmd.Flags().BoolVarP(&feedWindow, "window", "w", false, "Open in dedicated tmux window (creates 'feed' window)")
	feedCmd.Flags().BoolVar(&feedPlain, "plain", false, "Use plain text output (bd activity) instead of TUI")
	feedCmd.Flags().BoolVarP(&feedProblems, "problems", "p", false, "Start in problems view (shows stuck agents)")
}

var feedCmd = &cobra.Command{
	Use:     "feed",
	GroupID: GroupDiag,
	Short:   "Show real-time activity feed of gt events",
	Long: `Display a real-time feed of issue changes and agent activity.

By default, launches an interactive TUI dashboard with:
  - Agent tree (top): Shows all agents organized by role with latest activity
  - Convoy panel (middle): Shows in-progress and recently landed convoys
  - Event stream (bottom): Chronological feed you can scroll through
  - Vim-style navigation: j/k to scroll, tab to switch panels, 1/2/3 for panels, q to quit

Problems View (--problems/-p):
  A problem-first view that surfaces agents needing attention:
  - Detects stuck agents via structured beads data (hook state, timestamps)
  - Shows GUPP violations (hooked work + 30m no progress)
  - Keyboard actions: Enter=attach, n=nudge, h=handoff
  - Press 'p' to toggle between activity and problems view

The feed combines multiple event sources:
  - GT events: Agent activity like patrol, sling, handoff (from .events.jsonl)
  - Beads activity: Issue creates, updates, completions (from bd activity, when available)
  - Convoy status: In-progress and recently-landed convoys (refreshes every 10s)

Use --plain for simple text output (reads .events.jsonl directly).

Tmux Integration:
  Use --window to open the feed in a dedicated tmux window named 'feed'.
  This creates a persistent window you can cycle to with C-b n/p.

Event symbols:
  +  created/bonded    - New issue or molecule created
  →  in_progress       - Work started on an issue
  ✓  completed         - Issue closed or step completed
  ✗  failed            - Step or issue failed
  ⊘  deleted           - Issue removed
  🦉  patrol_started   - Witness began patrol cycle
  ⚡  polecat_nudged   - Worker was nudged
  🎯  sling            - Work was slung to worker
  🤝  handoff          - Session handed off

Agent state symbols (problems view):
  🔥  GUPP violation   - Hooked work + 30m no progress (critical)
  ⚠   STALLED          - Hooked work + 15m no progress
  ●   Working          - Actively producing output
  ○   Idle             - No hooked work
  💀  Zombie           - Dead/crashed session

MQ (Merge Queue) event symbols:
  ⚙  merge_started   - Refinery began processing an MR
  ✓  merged          - MR successfully merged (green)
  ✗  merge_failed    - Merge failed (conflict, tests, etc.) (red)
  ⊘  merge_skipped   - MR skipped (already merged, etc.)

Examples:
  gt feed                       # Launch TUI dashboard
  gt feed --problems            # Start in problems view
  gt feed -p                    # Short flag for problems view
  gt feed --plain               # Plain text output (bd activity)
  gt feed --window              # Open in dedicated tmux window
  gt feed --since 1h            # Events from last hour
  gt feed --rig greenplace      # Use gastown rig's beads`,
	RunE: runFeed,
}

func runFeed(cmd *cobra.Command, args []string) error {
	// Must be in a Gas Town workspace
	townRoot, err := workspace.FindFromCwdOrError()
	if err != nil {
		return fmt.Errorf("not in a Gas Town workspace (run from ~/gt or a rig directory)")
	}

	// Build feed arguments for window mode
	bdArgs := buildFeedArgs()

	// Handle --window mode: --rig is forwarded as a CLI flag via buildFeedArgs
	if feedWindow {
		workDir, err := os.Getwd()
		if err != nil {
			return fmt.Errorf("getting current directory: %w", err)
		}
		return runFeedInWindow(workDir, bdArgs)
	}

	// Use TUI by default if running in a terminal and not --plain
	useTUI := !feedPlain && term.IsTerminal(int(os.Stdout.Fd()))

	if useTUI {
		// TUI mode: resolve --rig to a beads directory for BdActivitySource
		workDir, err := os.Getwd()
		if err != nil {
			return fmt.Errorf("getting current directory: %w", err)
		}
		if feedRig != "" {
			candidates := []string{
				fmt.Sprintf("%s/%s/mayor/rig", townRoot, feedRig),
				fmt.Sprintf("%s/%s", townRoot, feedRig),
			}
			found := false
			for _, candidate := range candidates {
				if _, err := os.Stat(candidate + "/.beads"); err == nil {
					workDir = candidate
					found = true
					break
				}
			}
			if !found {
				return fmt.Errorf("rig '%s' not found or has no .beads directory", feedRig)
			}
		}
		return runFeedTUI(workDir, feedProblems)
	}

	// Plain mode: --rig is a pure event filter via PrintOptions.Rig
	return runFeedDirect(townRoot)
}

// buildFeedArgs builds the feed CLI arguments for window mode.
func buildFeedArgs() []string {
	var args []string

	// Default to follow mode unless --no-follow set
	shouldFollow := !feedNoFollow
	if feedFollow {
		shouldFollow = true
	}

	// Auto-disable follow when stdout is not a TTY (e.g. agents, pipes),
	// unless the user explicitly passed --follow. This prevents agents
	// from blocking on a streaming feed that never terminates.
	if !term.IsTerminal(int(os.Stdout.Fd())) && !feedFollow {
		shouldFollow = false
	}

	if shouldFollow {
		args = append(args, "--follow")
	}

	if feedLimit != 100 {
		args = append(args, "--limit", fmt.Sprintf("%d", feedLimit))
	}

	if feedSince != "" {
		args = append(args, "--since", feedSince)
	}

	if feedMol != "" {
		args = append(args, "--mol", feedMol)
	}

	if feedType != "" {
		args = append(args, "--type", feedType)
	}

	if feedRig != "" {
		args = append(args, "--rig", feedRig)
	}

	return args
}

// runFeedDirect prints events from .events.jsonl to stdout.
// Supports --follow for tailing, and --since/--mol/--type for filtering.
// townRoot is the resolved workspace root (incorporates --rig if set).
func runFeedDirect(townRoot string) error {
	// Determine follow behavior:
	// - Explicit --follow: always follow
	// - Explicit --no-follow: never follow
	// - Non-TTY (pipe/script): no follow unless explicitly requested
	// - Default (TTY, no flags): follow
	shouldFollow := feedFollow
	if !shouldFollow && !feedNoFollow {
		shouldFollow = term.IsTerminal(int(os.Stdout.Fd()))
	}

	opts := feed.PrintOptions{
		Limit:  feedLimit,
		Follow: shouldFollow,
		Since:  feedSince,
		Mol:    feedMol,
		Type:   feedType,
		Rig:    feedRig,
	}

	return feed.PrintGtEvents(townRoot, opts)
}

// runFeedTUI runs the interactive TUI feed.
func runFeedTUI(workDir string, problemsView bool) error {
	// Must be in a Gas Town workspace
	townRoot, err := workspace.FindFromCwdOrError()
	if err != nil {
		return fmt.Errorf("not in a Gas Town workspace: %w", err)
	}

	var sources []feed.EventSource

	// Create event source from bd activity (optional - bd may not have activity command)
	bdSource, err := feed.NewBdActivitySource(workDir)
	if err == nil {
		sources = append(sources, bdSource)
	}

	// Create MQ event source (optional - don't fail if not available)
	mqSource, err := feed.NewMQEventSourceFromWorkDir(workDir)
	if err == nil {
		sources = append(sources, mqSource)
	}

	// Create GT events source (optional - don't fail if not available)
	gtSource, err := feed.NewGtEventsSource(townRoot)
	if err == nil {
		sources = append(sources, gtSource)
	}

	if len(sources) == 0 {
		return fmt.Errorf("no event sources available (check that .events.jsonl exists in %s)", townRoot)
	}

	// Combine all sources
	multiSource := feed.NewMultiSource(sources...)
	defer func() { _ = multiSource.Close() }()

	// Create beads instance for agent health detection
	bd := beads.New(townRoot)

	// Create model and connect event source
	var m *feed.Model
	if problemsView {
		m = feed.NewModelWithProblemsView(bd)
	} else {
		m = feed.NewModel(bd)
	}
	m.SetEventChannel(multiSource.Events())
	m.SetTownRoot(townRoot)

	// Run the TUI
	p := tea.NewProgram(m, tea.WithAltScreen())
	if _, err := p.Run(); err != nil {
		return fmt.Errorf("running TUI: %w", err)
	}

	return nil
}

// runFeedInWindow opens the feed in a dedicated tmux window.
func runFeedInWindow(workDir string, bdArgs []string) error {
	// Check if we're in tmux
	if !tmux.IsInsideTmux() {
		return fmt.Errorf("--window requires running inside tmux")
	}

	// Get current session from TMUX env var
	// Format: /tmp/tmux-501/default,12345,0 -> we need the session name
	tmuxEnv := os.Getenv("TMUX")
	if tmuxEnv == "" {
		return fmt.Errorf("TMUX environment variable not set")
	}

	t := tmux.NewTmux()

	// Get current session name
	sessionName, err := getCurrentTmuxSession()
	if err != nil {
		return fmt.Errorf("getting current session: %w", err)
	}

	// Build the command to run in the window
	// Use gt feed --plain instead of bd activity (which may not exist)
	gtPath, err := os.Executable()
	if err != nil {
		gtPath = "gt"
	}
	feedWindowCmd := fmt.Sprintf("cd \"%s\" && \"%s\" feed --plain --follow", workDir, gtPath)
	if len(bdArgs) > 0 {
		var filteredArgs []string
		for _, arg := range bdArgs {
			if arg != "--follow" {
				filteredArgs = append(filteredArgs, arg)
			}
		}
		if len(filteredArgs) > 0 {
			feedWindowCmd = fmt.Sprintf("cd \"%s\" && \"%s\" feed --plain --follow %s", workDir, gtPath, strings.Join(filteredArgs, " "))
		}
	}

	// Check if 'feed' window already exists
	windowTarget := sessionName + ":feed"
	exists, err := windowExists(t, sessionName, "feed")
	if err != nil {
		return fmt.Errorf("checking for feed window: %w", err)
	}

	if exists {
		// Window exists - just switch to it
		fmt.Printf("Switching to existing feed window...\n")
		return selectWindow(t, windowTarget)
	}

	// Create new window named 'feed'
	fmt.Printf("Creating feed window in session %s...\n", sessionName)
	if err := createWindow(t, sessionName, "feed", workDir, feedWindowCmd); err != nil {
		return fmt.Errorf("creating feed window: %w", err)
	}

	// Switch to the new window
	return selectWindow(t, windowTarget)
}

// windowExists checks if a window with the given name exists in the session.
// Note: getCurrentTmuxSession is defined in handoff.go
func windowExists(_ *tmux.Tmux, session, windowName string) (bool, error) { // t unused: direct exec for simplicity
	cmd := tmux.BuildCommand("list-windows", "-t", session, "-F", "#{window_name}")
	out, err := cmd.Output()
	if err != nil {
		return false, err
	}

	for _, line := range strings.Split(string(out), "\n") {
		if strings.TrimSpace(line) == windowName {
			return true, nil
		}
	}
	return false, nil
}

// createWindow creates a new tmux window with the given name and command.
func createWindow(_ *tmux.Tmux, session, windowName, workDir, command string) error { // t unused: direct exec for simplicity
	args := []string{"new-window", "-t", session, "-n", windowName, "-c", workDir, command}
	cmd := tmux.BuildCommand(args...)
	return cmd.Run()
}

// selectWindow switches to the specified window.
func selectWindow(_ *tmux.Tmux, target string) error { // t unused: direct exec for simplicity
	cmd := tmux.BuildCommand("select-window", "-t", target)
	return cmd.Run()
}
// Package session provides polecat session lifecycle management.
package session

import (
	"context"
	"fmt"
	"os"
	"sort"
	"strings"
	"time"

	"github.com/google/uuid"
	"github.com/steveyegge/gastown/internal/config"
	"github.com/steveyegge/gastown/internal/constants"
	"github.com/steveyegge/gastown/internal/git"
	"github.com/steveyegge/gastown/internal/runtime"
	"github.com/steveyegge/gastown/internal/telemetry"
	"github.com/steveyegge/gastown/internal/tmux"
)

// SessionConfig describes how to create and start a tmux session.
// This unifies the common startup pattern that was previously duplicated
// across polecat, mayor, boot, deacon, witness, refinery, crew, and dog
// session managers. Each of those managers previously had to coordinate
// 4+ packages (config, runtime, session, tmux) manually.
//
// Usage pattern:
//
//	result, err := session.StartSession(t, session.SessionConfig{
//	    SessionID: "gt-myrig-toast",
//	    WorkDir:   "/path/to/worktree",
//	    Role:      "polecat",
//	    TownRoot:  "/path/to/town",
//	    Beacon:    session.BeaconConfig{...},
//	})
type SessionConfig struct {
	// SessionID is the tmux session name (e.g., "gt-wyvern-Toast", "hq-mayor").
	SessionID string

	// WorkDir is the working directory for the session.
	WorkDir string

	// Role is the agent role (e.g., "polecat", "mayor", "boot", "deacon").
	Role string

	// TownRoot is the root of the Gas Town workspace (e.g., ~/gt).
	TownRoot string

	// RigPath is the rig directory path for config resolution.
	// Empty for town-level agents (mayor, deacon, boot).
	RigPath string

	// RigName is the rig name for environment variables and theming.
	// Empty for town-level agents.
	RigName string

	// AgentName is the specific agent name within a rig.
	// Used for polecats, crew, and dogs. Empty for singletons.
	AgentName string

	// Command is a pre-built startup command. If non-empty, skips command building.
	// If empty, the command is built from Beacon + config.BuildAgentStartupCommand.
	Command string

	// Beacon configures the startup beacon message for session identification.
	// Ignored if Command is non-empty.
	Beacon BeaconConfig

	// Instructions are appended after the beacon in the startup prompt.
	// Used by roles like Boot and Deacon that need explicit instructions.
	// Ignored if Command is non-empty.
	Instructions string

	// AgentOverride optionally specifies a different agent alias (e.g., "opencode").
	AgentOverride string

	// RuntimeConfigDir overrides the config directory for the runtime.
	RuntimeConfigDir string

	// ExtraEnv adds additional environment variables beyond the standard AgentEnv set.
	// These are set in the tmux session environment after the standard vars.
	ExtraEnv map[string]string

	// Theme is the tmux theme to apply. Nil means no theme is applied.
	Theme *tmux.Theme

	// Post-start behavior options.

	// WaitForAgent waits for the agent command to appear in the pane.
	WaitForAgent bool

	// WaitFatal makes WaitForAgent failure fatal — kills the session and returns error.
	// If false, WaitForAgent failure is silently ignored.
	WaitFatal bool

	// AcceptBypass accepts the bypass permissions warning dialog if it appears.
	AcceptBypass bool

	// ReadyDelay sleeps for the runtime's configured readiness delay.
	ReadyDelay bool

	// AutoRespawn sets the auto-respawn hook so the session survives crashes.
	AutoRespawn bool

	// RemainOnExit sets remain-on-exit immediately after session creation.
	RemainOnExit bool

	// TrackPID tracks the pane PID for defense-in-depth orphan cleanup.
	TrackPID bool

	// VerifySurvived checks that the session is still alive after startup.
	VerifySurvived bool
}

// StartResult contains the results of session startup.
type StartResult struct {
	// RuntimeConfig is the resolved runtime config for the role.
	// Callers may need this for role-specific post-startup steps
	// (e.g., handling fallback nudges, legacy fallback).
	RuntimeConfig *config.RuntimeConfig

	// RunID is the GASTA run identifier (GT_RUN) generated for this session.
	// All telemetry events emitted within the session carry this ID, enabling
	// waterfall correlation across prompts, BD calls, mail operations, and
	// agent conversation events.
	RunID string
}

// StartSession creates a tmux session following the standard Gas Town lifecycle.
//
// The lifecycle handles:
//  1. Resolve runtime config for the role
//  2. Ensure settings/plugins exist for the agent
//  3. Build startup command (if not provided)
//  4. Create tmux session with command
//  5. Set environment variables (standard + extra)
//  6. Apply theme (if configured)
//  7. Optional post-start: wait for agent, accept bypass, ready delay,
//     auto-respawn, PID tracking, verify survived
//
// Role-specific concerns (issue validation, fallback nudges, pane-died hooks,
// crew cycle bindings, etc.) should be handled by the caller before/after
// calling StartSession.
func StartSession(t *tmux.Tmux, cfg SessionConfig) (_ *StartResult, retErr error) {
	// Generate the GASTA run ID — the root identifier for all telemetry emitted
	// by this agent session and its subprocesses (bd, mail, …).
	runID := uuid.New().String()
	ctx := telemetry.WithRunID(context.Background(), runID)

	defer func() { telemetry.RecordSessionStart(ctx, cfg.SessionID, cfg.Role, retErr) }()
	if cfg.SessionID == "" {
		return nil, fmt.Errorf("SessionID is required")
	}
	if cfg.WorkDir == "" {
		return nil, fmt.Errorf("WorkDir is required")
	}
	if cfg.Role == "" {
		return nil, fmt.Errorf("Role is required")
	}

	// 1. Resolve runtime config.
	runtimeConfig := config.ResolveRoleAgentConfig(cfg.Role, cfg.TownRoot, cfg.RigPath)

	// 2. Ensure settings/plugins exist for the agent.
	settingsDir := config.RoleSettingsDir(cfg.Role, cfg.RigPath)
	if settingsDir == "" {
		settingsDir = cfg.WorkDir
	}
	if err := runtime.EnsureSettingsForRole(settingsDir, cfg.WorkDir, cfg.Role, runtimeConfig); err != nil {
		return nil, fmt.Errorf("ensuring runtime settings: %w", err)
	}

	// 3. Build startup command if not provided.
	command := cfg.Command
	if command == "" {
		prompt := buildPrompt(cfg)
		var err error
		command, err = buildCommand(cfg, prompt)
		if err != nil {
			return nil, fmt.Errorf("building startup command: %w", err)
		}
	}

	// Prepend runtime config dir env if needed.
	if runtimeConfig.Session != nil && runtimeConfig.Session.ConfigDirEnv != "" && cfg.RuntimeConfigDir != "" {
		command = config.PrependEnv(command, map[string]string{
			runtimeConfig.Session.ConfigDirEnv: cfg.RuntimeConfigDir,
		})
	}

	// Prepend GT_RUN (GASTA run ID) and any extra env vars into the command so
	// that they are inherited by the initial shell before tmux SetEnvironment runs.
	extraWithRun := make(map[string]string, len(cfg.ExtraEnv)+1)
	for k, v := range cfg.ExtraEnv {
		extraWithRun[k] = v
	}
	extraWithRun["GT_RUN"] = runID
	command = config.PrependEnv(command, extraWithRun)

	// 4. Create tmux session with command.
	if err := t.NewSessionWithCommand(cfg.SessionID, cfg.WorkDir, command); err != nil {
		return nil, fmt.Errorf("creating session: %w", err)
	}

	// 5. Set remain-on-exit immediately if requested (before anything else can fail).
	if cfg.RemainOnExit {
		_ = t.SetRemainOnExit(cfg.SessionID, true)
	}

	// 6. Set environment variables.
	envVars := config.AgentEnv(config.AgentEnvConfig{
		Role:             cfg.Role,
		Rig:              cfg.RigName,
		AgentName:        cfg.AgentName,
		TownRoot:         cfg.TownRoot,
		RuntimeConfigDir: cfg.RuntimeConfigDir,
		Agent:            cfg.AgentOverride,
		SessionName:      cfg.SessionID,
	})
	envVars = MergeRuntimeLivenessEnv(envVars, runtimeConfig)
	for _, k := range mapKeysSorted(envVars) {
		_ = t.SetEnvironment(cfg.SessionID, k, envVars[k])
	}
	// Set GT_RUN in the session environment so respawned processes also inherit it.
	_ = t.SetEnvironment(cfg.SessionID, "GT_RUN", runID)
	for _, k := range mapKeysSorted(cfg.ExtraEnv) {
		_ = t.SetEnvironment(cfg.SessionID, k, cfg.ExtraEnv[k])
	}

	// 7. Apply theme.
	if cfg.Theme != nil {
		_ = t.ConfigureGasTownSession(cfg.SessionID, cfg.Theme, cfg.RigName, cfg.AgentName, cfg.Role)
	}

	// 8. Wait for agent to start.
	if cfg.WaitForAgent {
		if err := t.WaitForCommand(cfg.SessionID, constants.SupportedShells, constants.ClaudeStartTimeout); err != nil {
			if cfg.WaitFatal {
				_ = t.KillSessionWithProcesses(cfg.SessionID)
				return nil, fmt.Errorf("waiting for %s to start: %w", cfg.Role, err)
			}
		}
	}

	// 9. Auto-respawn hook.
	if cfg.AutoRespawn {
		if err := t.SetAutoRespawnHook(cfg.SessionID); err != nil {
			fmt.Printf("warning: failed to set auto-respawn hook for %s: %v\n", cfg.Role, err)
		}
	}

	// 10. Accept startup dialogs (workspace trust + bypass permissions).
	if cfg.AcceptBypass {
		_ = t.AcceptStartupDialogs(cfg.SessionID)
	}

	// 11. Ready delay: wait for agent to be fully ready at the prompt.
	// Uses prompt-based polling for agents with ReadyPromptPrefix,
	// falling back to ReadyDelayMs sleep for agents without prompt detection.
	if cfg.ReadyDelay {
		if err := t.WaitForRuntimeReady(cfg.SessionID, runtimeConfig, constants.ClaudeStartTimeout); err != nil {
			fmt.Fprintf(os.Stderr, "Warning: agent readiness detection timed out for %s: %v\n", cfg.SessionID, err)
		}
	}

	// 12. Verify session survived startup.
	if cfg.VerifySurvived {
		running, err := t.HasSession(cfg.SessionID)
		if err != nil {
			// Clean up session on verification error to prevent orphan
			_ = t.KillSessionWithProcesses(cfg.SessionID)
			return nil, fmt.Errorf("verifying session: %w", err)
		}
		if !running {
			return nil, fmt.Errorf("session %s died during startup (agent command may have failed)", cfg.SessionID)
		}
	}

	// 13. Record agent's pane_id for ZFC-compliant liveness checks (gt-qmsx).
	// Declared pane identity replaces process-tree inference in IsRuntimeRunning
	// and FindAgentPane. Legacy sessions without GT_PANE_ID fall back to scanning.
	if paneID, err := t.GetPaneID(cfg.SessionID); err == nil {
		_ = t.SetEnvironment(cfg.SessionID, "GT_PANE_ID", paneID)
	}

	// 14. Track PID for defense-in-depth orphan cleanup.
	if cfg.TrackPID && cfg.TownRoot != "" {
		_ = TrackSessionPID(cfg.TownRoot, cfg.SessionID, t)
	}

	// 14. Stream agent conversation events to VictoriaLogs (opt-in).
	// Reads ~/.claude/projects/<hash>/<session>.jsonl and emits agent.event logs.
	// Non-fatal: observability failures must never block agent startup.
	if os.Getenv("GT_LOG_AGENT_OUTPUT") == "true" && os.Getenv("GT_OTEL_LOGS_URL") != "" {
		if err := ActivateAgentLogging(cfg.SessionID, cfg.WorkDir, runID); err != nil {
			fmt.Fprintf(os.Stderr, "warning: agent log watcher setup failed for %s: %v\n", cfg.SessionID, err)
		}
	}

	// Record the agent instantiation event (GASTA root span).
	// Done after session creation so we only emit on success.
	RecordAgentInstantiateFromDir(ctx, runID, runtimeConfig.ResolvedAgent,
		cfg.Role, cfg.AgentName, cfg.SessionID, cfg.RigName, cfg.TownRoot, "", cfg.WorkDir)

	return &StartResult{RuntimeConfig: runtimeConfig, RunID: runID}, nil
}

// RecordAgentInstantiateFromDir resolves the git branch/commit from workDir and
// emits the agent.instantiate root telemetry event. resolvedAgent defaults to
// "claudecode" when empty. Use this instead of calling telemetry.RecordAgentInstantiate
// directly to avoid duplicating the agentType/git-lookup boilerplate.
func RecordAgentInstantiateFromDir(ctx context.Context, runID, resolvedAgent, role, agentName, sessionID, rigName, townRoot, issueID, workDir string) {
	agentType := resolvedAgent
	if agentType == "" {
		agentType = "claudecode"
	}
	branch, commit := "", ""
	if g := git.NewGit(workDir); g != nil {
		if b, err := g.CurrentBranch(); err == nil {
			branch = b
		}
		if c, err := g.Rev("HEAD"); err == nil {
			commit = c
		}
	}
	telemetry.RecordAgentInstantiate(ctx, telemetry.AgentInstantiateInfo{
		RunID:     runID,
		AgentType: agentType,
		Role:      role,
		AgentName: agentName,
		SessionID: sessionID,
		RigName:   rigName,
		TownRoot:  townRoot,
		IssueID:   issueID,
		GitBranch: branch,
		GitCommit: commit,
	})
}

// StopSession stops a tmux session with optional graceful shutdown.
//
// If graceful is true, sends Ctrl-C first and waits for the session to exit
// before force-killing. This allows the agent to clean up.
func StopSession(t *tmux.Tmux, sessionID string, graceful bool) error {
	running, err := t.HasSession(sessionID)
	if err != nil {
		return fmt.Errorf("checking session: %w", err)
	}
	if !running {
		return fmt.Errorf("session not found: %s", sessionID)
	}

	if graceful {
		_ = t.SendKeysRaw(sessionID, "C-c")
		WaitForSessionExit(t, sessionID, constants.GracefulShutdownTimeout)
	}

	// Kill any detached agent-log watcher for this session before tearing down
	// the tmux session, to avoid orphan processes accumulating over time.
	DeactivateAgentLogging(sessionID)

	if err := t.KillSessionWithProcesses(sessionID); err != nil {
		return fmt.Errorf("killing session: %w", err)
	}

	return nil
}

func mapKeysSorted(m map[string]string) []string {
	if len(m) == 0 {
		return nil
	}
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}

// MergeRuntimeLivenessEnv ensures liveness-critical env vars are present in the
// tmux session environment table, even when agent resolution came from
// workspace/default settings rather than an explicit --agent override.
//
// Call this after config.AgentEnv() to add GT_AGENT and GT_PROCESS_NAMES
// before writing env vars to the tmux session via SetEnvironment.
func MergeRuntimeLivenessEnv(envVars map[string]string, runtimeConfig *config.RuntimeConfig) map[string]string {
	if envVars == nil {
		envVars = make(map[string]string)
	}
	if runtimeConfig == nil {
		return envVars
	}

	if _, hasGTAgent := envVars["GT_AGENT"]; !hasGTAgent && runtimeConfig.ResolvedAgent != "" {
		envVars["GT_AGENT"] = runtimeConfig.ResolvedAgent
	}

	if _, hasProcessNames := envVars["GT_PROCESS_NAMES"]; !hasProcessNames {
		agentForLookup := runtimeConfig.ResolvedAgent
		commandForLookup := runtimeConfig.Command
		if existing, ok := envVars["GT_AGENT"]; ok && existing != "" {
			agentForLookup = existing
			// When GT_AGENT was set by AgentOverride (differs from the
			// workspace-resolved agent), the runtimeConfig.Command belongs
			// to the workspace agent, not the override. Pass empty command
			// so ResolveProcessNames uses the preset's own command.
			if existing != runtimeConfig.ResolvedAgent {
				commandForLookup = ""
			}
		}
		processNames := config.ResolveProcessNames(agentForLookup, commandForLookup)
		if len(processNames) > 0 {
			envVars["GT_PROCESS_NAMES"] = strings.Join(processNames, ",")
		}
	}

	return envVars
}

// KillExistingSession kills an existing session if one is found.
// Returns true if a session was killed.
//
// If checkAlive is true, only kills zombie sessions (tmux alive but agent dead).
// If the session exists and the agent is alive, returns ErrAlreadyRunning.
// If checkAlive is false, kills any existing session unconditionally.
func KillExistingSession(t *tmux.Tmux, sessionID string, checkAlive bool) (bool, error) {
	running, err := t.HasSession(sessionID)
	if err != nil {
		return false, fmt.Errorf("checking session: %w", err)
	}
	if !running {
		return false, nil
	}

	if checkAlive && t.IsAgentAlive(sessionID) {
		return false, fmt.Errorf("session already running: %s", sessionID)
	}

	if err := t.KillSessionWithProcesses(sessionID); err != nil {
		return false, fmt.Errorf("killing session %s: %w", sessionID, err)
	}

	return true, nil
}

// buildPrompt creates the startup prompt from beacon + instructions.
func buildPrompt(cfg SessionConfig) string {
	if cfg.Instructions != "" {
		return BuildStartupPrompt(cfg.Beacon, cfg.Instructions)
	}
	return FormatStartupBeacon(cfg.Beacon)
}

// buildCommand creates the startup command using the config package.
func buildCommand(cfg SessionConfig, prompt string) (string, error) {
	if cfg.AgentOverride != "" {
		return config.BuildAgentStartupCommandWithAgentOverride(
			cfg.Role, cfg.RigName, cfg.TownRoot, cfg.RigPath, prompt, cfg.AgentOverride)
	}
	return config.BuildAgentStartupCommand(
		cfg.Role, cfg.RigName, cfg.TownRoot, cfg.RigPath, prompt), nil
}

// ShutdownDelay is the standard delay after session creation.
// Some roles use this instead of the runtime's ready delay.
func ShutdownDelay() time.Duration {
	return constants.ShutdownNotifyDelay
}
