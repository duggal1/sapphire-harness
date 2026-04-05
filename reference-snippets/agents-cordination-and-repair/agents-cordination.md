package cmd

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/spf13/cobra"
	"github.com/steveyegge/gastown/internal/session"
	"github.com/steveyegge/gastown/internal/style"
	"github.com/steveyegge/gastown/internal/tmux"
	"github.com/steveyegge/gastown/internal/witness"
	"github.com/steveyegge/gastown/internal/workspace"
)

// Witness command flags
var (
	witnessForeground    bool
	witnessStatusJSON    bool
	witnessAgentOverride string
	witnessEnvOverrides  []string
)

var witnessCmd = &cobra.Command{
	Use:     "witness",
	GroupID: GroupAgents,
	Short:   "Manage the Witness (per-rig polecat health monitor)",
	RunE:    requireSubcommand,
	Long: `Manage the Witness - the per-rig polecat health monitor.

The Witness patrols a single rig, watching over its polecats:
  - Detects stalled polecats (crashed or stuck mid-work)
  - Nudges unresponsive sessions back to life
  - Cleans up zombie polecats (finished but failed to exit)
  - Nukes sandboxes when polecats complete via 'gt done'

The Witness does NOT force session cycles or interrupt working polecats.
Polecats manage their own sessions (via gt handoff). The Witness handles
failures and edge cases only.

One Witness per rig. The Deacon monitors all Witnesses.

Role shortcuts: "witness" in mail/nudge addresses resolves to this rig's Witness.`,
}

var witnessStartCmd = &cobra.Command{
	Use:     "start <rig>",
	Aliases: []string{"spawn"},
	Short:   "Start the witness",
	Long: `Start the Witness for a rig.

Launches the monitoring agent which watches for stuck polecats and orphaned
sandboxes, taking action to keep work flowing.

Self-Cleaning Model: Polecats nuke themselves after work. The Witness handles
crash recovery (restart with hooked work) and orphan cleanup (nuke abandoned
sandboxes). There is no "idle" state - polecats either have work or don't exist.

Examples:
  gt witness start greenplace
  gt witness start greenplace --agent codex
  gt witness start greenplace --env ANTHROPIC_MODEL=claude-3-haiku
  gt witness start greenplace --foreground`,
	Args: cobra.ExactArgs(1),
	RunE: runWitnessStart,
}

var witnessStopCmd = &cobra.Command{
	Use:   "stop <rig>",
	Short: "Stop the witness",
	Long: `Stop a running Witness.

Gracefully stops the witness monitoring agent.`,
	Args: cobra.ExactArgs(1),
	RunE: runWitnessStop,
}

var witnessStatusCmd = &cobra.Command{
	Use:   "status <rig>",
	Short: "Show witness status",
	Long: `Show the status of a rig's Witness.

Displays running state, monitored polecats, and statistics.`,
	Args: cobra.ExactArgs(1),
	RunE: runWitnessStatus,
}

var witnessAttachCmd = &cobra.Command{
	Use:     "attach [rig]",
	Aliases: []string{"at"},
	Short:   "Attach to witness session",
	Long: `Attach to the Witness tmux session for a rig.

Attaches the current terminal to the witness's tmux session.
Detach with Ctrl-B D.

If the witness is not running, this will start it first.
If rig is not specified, infers it from the current directory.

Examples:
  gt witness attach greenplace
  gt witness attach          # infer rig from cwd`,
	Args: cobra.MaximumNArgs(1),
	RunE: runWitnessAttach,
}

var witnessRestartCmd = &cobra.Command{
	Use:   "restart <rig>",
	Short: "Restart the witness",
	Long: `Restart the Witness for a rig.

Stops the current session (if running) and starts a fresh one.

Examples:
  gt witness restart greenplace
  gt witness restart greenplace --agent codex
  gt witness restart greenplace --env ANTHROPIC_MODEL=claude-3-haiku`,
	Args: cobra.ExactArgs(1),
	RunE: runWitnessRestart,
}

func init() {
	// Start flags
	witnessStartCmd.Flags().BoolVar(&witnessForeground, "foreground", false, "Run in foreground (default: background)")
	witnessStartCmd.Flags().StringVar(&witnessAgentOverride, "agent", "", "Agent alias to run the Witness with (overrides town default)")
	witnessStartCmd.Flags().StringArrayVar(&witnessEnvOverrides, "env", nil, "Environment variable override (KEY=VALUE, can be repeated)")

	// Status flags
	witnessStatusCmd.Flags().BoolVar(&witnessStatusJSON, "json", false, "Output as JSON")

	// Restart flags
	witnessRestartCmd.Flags().StringVar(&witnessAgentOverride, "agent", "", "Agent alias to run the Witness with (overrides town default)")
	witnessRestartCmd.Flags().StringArrayVar(&witnessEnvOverrides, "env", nil, "Environment variable override (KEY=VALUE, can be repeated)")

	// Add subcommands
	witnessCmd.AddCommand(witnessStartCmd)
	witnessCmd.AddCommand(witnessStopCmd)
	witnessCmd.AddCommand(witnessRestartCmd)
	witnessCmd.AddCommand(witnessStatusCmd)
	witnessCmd.AddCommand(witnessAttachCmd)

	rootCmd.AddCommand(witnessCmd)
}

// getWitnessManager creates a witness manager for a rig.
func getWitnessManager(rigName string) (*witness.Manager, error) {
	_, r, err := getRig(rigName)
	if err != nil {
		return nil, err
	}

	mgr := witness.NewManager(r)
	return mgr, nil
}

func runWitnessStart(cmd *cobra.Command, args []string) error {
	rigName := args[0]

	if err := checkRigNotParkedOrDocked(rigName); err != nil {
		return err
	}

	mgr, err := getWitnessManager(rigName)
	if err != nil {
		return err
	}

	fmt.Printf("Starting witness for %s...\n", rigName)

	if err := mgr.Start(witnessForeground, witnessAgentOverride, witnessEnvOverrides); err != nil {
		if err == witness.ErrAlreadyRunning {
			fmt.Printf("%s Witness is already running\n", style.Dim.Render("⚠"))
			fmt.Printf("  %s\n", style.Dim.Render("Use 'gt witness attach' to connect"))
			return nil
		}
		return fmt.Errorf("starting witness: %w", err)
	}

	if witnessForeground {
		fmt.Printf("%s Note: Foreground mode no longer runs patrol loop\n", style.Dim.Render("⚠"))
		fmt.Printf("  %s\n", style.Dim.Render("Patrol logic is now handled by mol-witness-patrol molecule"))
		return nil
	}

	fmt.Printf("%s Witness started for %s\n", style.Bold.Render("✓"), rigName)
	fmt.Printf("  %s\n", style.Dim.Render("Use 'gt witness attach' to connect"))
	fmt.Printf("  %s\n", style.Dim.Render("Use 'gt witness status' to check progress"))
	return nil
}

func runWitnessStop(cmd *cobra.Command, args []string) error {
	rigName := args[0]

	mgr, err := getWitnessManager(rigName)
	if err != nil {
		return err
	}

	// Kill tmux session if it exists.
	// Use KillSessionWithProcesses to ensure all descendant processes are killed.
	t := tmux.NewTmux()
	sessionName := witnessSessionName(rigName)
	running, _ := t.HasSession(sessionName)
	if running {
		if err := t.KillSessionWithProcesses(sessionName); err != nil {
			style.PrintWarning("failed to kill session: %v", err)
		}
	}

	// Update state file
	if err := mgr.Stop(); err != nil {
		if err == witness.ErrNotRunning && !running {
			fmt.Printf("%s Witness is not running\n", style.Dim.Render("⚠"))
			return nil
		}
		// Even if manager.Stop fails, if we killed the session it's stopped
		if !running {
			return fmt.Errorf("stopping witness: %w", err)
		}
	}

	fmt.Printf("%s Witness stopped for %s\n", style.Bold.Render("✓"), rigName)
	return nil
}

// WitnessStatusOutput is the JSON output format for witness status.
type WitnessStatusOutput struct {
	Running           bool     `json:"running"`
	RigName           string   `json:"rig_name"`
	Session           string   `json:"session,omitempty"`
	MonitoredPolecats []string `json:"monitored_polecats,omitempty"`
}

func runWitnessStatus(cmd *cobra.Command, args []string) error {
	rigName := args[0]

	// Get rig for polecat info
	_, r, err := getRig(rigName)
	if err != nil {
		return err
	}

	mgr := witness.NewManager(r)

	// ZFC: tmux is source of truth for running state
	running, _ := mgr.IsRunning()
	sessionInfo, _ := mgr.Status() // may be nil if not running

	// Polecats come from rig config, not state file
	polecats := r.Polecats

	// JSON output
	if witnessStatusJSON {
		output := WitnessStatusOutput{
			Running:           running,
			RigName:           rigName,
			MonitoredPolecats: polecats,
		}
		if sessionInfo != nil {
			output.Session = sessionInfo.Name
		}
		enc := json.NewEncoder(os.Stdout)
		enc.SetIndent("", "  ")
		return enc.Encode(output)
	}

	// Human-readable output
	fmt.Printf("%s Witness: %s\n\n", style.Bold.Render(AgentTypeIcons[AgentWitness]), rigName)

	if running {
		fmt.Printf("  State: %s\n", style.Bold.Render("● running"))
		if sessionInfo != nil {
			fmt.Printf("  Session: %s\n", sessionInfo.Name)
		}
	} else {
		fmt.Printf("  State: %s\n", style.Dim.Render("○ stopped"))
	}

	// Show monitored polecats
	fmt.Printf("\n  %s\n", style.Bold.Render("Monitored Polecats:"))
	if len(polecats) == 0 {
		fmt.Printf("    %s\n", style.Dim.Render("(none)"))
	} else {
		for _, p := range polecats {
			fmt.Printf("    • %s\n", p)
		}
	}

	return nil
}

// witnessSessionName returns the tmux session name for a rig's witness.
func witnessSessionName(rigName string) string {
	return session.WitnessSessionName(session.PrefixFor(rigName))
}

func runWitnessAttach(cmd *cobra.Command, args []string) error {
	rigName := ""
	if len(args) > 0 {
		rigName = args[0]
	}

	// Infer rig from cwd if not provided
	if rigName == "" {
		townRoot, err := workspace.FindFromCwdOrError()
		if err != nil {
			return fmt.Errorf("not in a Gas Town workspace: %w", err)
		}
		rigName, err = inferRigFromCwd(townRoot)
		if err != nil {
			return fmt.Errorf("could not determine rig: %w\nUsage: gt witness attach <rig>", err)
		}
	}

	// Verify rig exists and get manager
	mgr, err := getWitnessManager(rigName)
	if err != nil {
		return err
	}

	sessionName := witnessSessionName(rigName)

	// Ensure session exists (creates if needed)
	if err := mgr.Start(false, "", nil); err != nil && err != witness.ErrAlreadyRunning {
		return err
	} else if err == nil {
		fmt.Printf("Started witness session for %s\n", rigName)
	}

	// Attach to the session (socket-aware: uses the town's tmux socket).
	return attachToTmuxSession(sessionName)
}

func runWitnessRestart(cmd *cobra.Command, args []string) error {
	rigName := args[0]

	if err := checkRigNotParkedOrDocked(rigName); err != nil {
		return err
	}

	mgr, err := getWitnessManager(rigName)
	if err != nil {
		return err
	}

	fmt.Printf("Restarting witness for %s...\n", rigName)

	// Stop existing session (non-fatal: may not be running)
	_ = mgr.Stop()

	// Start fresh
	if err := mgr.Start(false, witnessAgentOverride, witnessEnvOverrides); err != nil {
		return fmt.Errorf("starting witness: %w", err)
	}

	fmt.Printf("%s Witness restarted for %s\n", style.Bold.Render("✓"), rigName)
	fmt.Printf("  %s\n", style.Dim.Render("Use 'gt witness attach' to connect"))
	return nil
}
package cmd

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/spf13/cobra"
	"github.com/steveyegge/gastown/internal/beads"
	"github.com/steveyegge/gastown/internal/git"
	"github.com/steveyegge/gastown/internal/polecat"
	"github.com/steveyegge/gastown/internal/rig"
	"github.com/steveyegge/gastown/internal/style"
	"github.com/steveyegge/gastown/internal/tmux"
	"github.com/steveyegge/gastown/internal/util"
)

// Polecat command flags
var (
	polecatListJSON  bool
	polecatListAll   bool
	polecatForce     bool
	polecatRemoveAll bool
)

var polecatCmd = &cobra.Command{
	Use:     "polecat",
	Aliases: []string{"polecats"},
	GroupID: GroupAgents,
	Short:   "Manage polecats (persistent identity, ephemeral sessions)",
	RunE:    requireSubcommand,
	Long: `Manage polecat lifecycle in rigs.

Polecats have PERSISTENT IDENTITY but EPHEMERAL SESSIONS. Each polecat has
a permanent agent bead and CV chain that accumulates work history across
assignments. Sessions and sandboxes are ephemeral — spawned for specific
tasks, cleaned up on completion — but the identity persists.

A polecat is either:
  - Working: Actively doing assigned work
  - Stalled: Session crashed mid-work (needs Witness intervention)
  - Zombie: Finished but gt done failed (needs cleanup)
  - Nuked: Session ended, identity persists (ready for next assignment)

Self-cleaning model: When work completes, the polecat runs 'gt done',
which pushes the branch, submits to the merge queue, and exits. The
Witness then nukes the sandbox. The polecat's identity (agent bead)
persists with agent_state=nuked, preserving work history.

Session vs sandbox: The Claude session cycles frequently (handoffs,
compaction). The git worktree (sandbox) persists until nuke. Work
survives session restarts.

Cats build features. Dogs clean up messes.`,
}

var polecatListCmd = &cobra.Command{
	Use:   "list [rig]",
	Short: "List polecats in a rig",
	Long: `List polecats in a rig or all rigs.

In the transient model, polecats exist only while working. The list shows
all polecats with their states:
  - working: Actively working on an issue
  - done: Completed work, waiting for cleanup
  - stuck: Needs assistance

Examples:
  gt polecat list greenplace
  gt polecat list --all
  gt polecat list greenplace --json`,
	RunE: runPolecatList,
}

var polecatAddCmd = &cobra.Command{
	Use:        "add <rig> <name>",
	Short:      "Add a new polecat to a rig (DEPRECATED)",
	Deprecated: "use 'gt polecat identity add' instead. This command will be removed in v1.0.",
	Long: `Add a new polecat to a rig.

DEPRECATED: Use 'gt polecat identity add' instead. This command will be removed in v1.0.

Creates a polecat directory, clones the rig repo, creates a work branch,
and initializes state.

Example:
  gt polecat identity add greenplace Toast  # Preferred
  gt polecat add greenplace Toast           # Deprecated`,
	Args: cobra.ExactArgs(2),
	RunE: runPolecatAdd,
}

var polecatRemoveCmd = &cobra.Command{
	Use:   "remove <rig>/<polecat>... | <rig> --all",
	Short: "Remove polecats from a rig",
	Long: `Remove one or more polecats from a rig.

Fails if session is running (stop first).
Warns if uncommitted changes exist.
Use --force to bypass checks.

Examples:
  gt polecat remove greenplace/Toast
  gt polecat remove greenplace/Toast greenplace/Furiosa
  gt polecat remove greenplace --all
  gt polecat remove greenplace --all --force`,
	Args: cobra.MinimumNArgs(1),
	RunE: runPolecatRemove,
}

var polecatStatusCmd = &cobra.Command{
	Use:   "status <rig>/<polecat>",
	Short: "Show detailed status for a polecat",
	Long: `Show detailed status for a polecat.

Displays comprehensive information including:
  - Current lifecycle state (working, done, stuck, idle)
  - Assigned issue (if any)
  - Session status (running/stopped, attached/detached)
  - Session creation time
  - Last activity time

NOTE: The argument is <rig>/<polecat> — a single argument with a slash
separator, NOT two separate arguments. For example: greenplace/Toast

Examples:
  gt polecat status greenplace/Toast
  gt polecat status greenplace/Toast --json`,
	Args: cobra.ExactArgs(1),
	RunE: runPolecatStatus,
}

var (
	polecatStatusJSON        bool
	polecatGitStateJSON      bool
	polecatGCDryRun          bool
	polecatNukeAll           bool
	polecatNukeDryRun        bool
	polecatNukeForce         bool
	polecatCheckRecoveryJSON bool
	polecatPoolInitDryRun    bool
	polecatPoolInitSize      int
)

var polecatGCCmd = &cobra.Command{
	Use:   "gc <rig>",
	Short: "Garbage collect stale polecat branches",
	Long: `Garbage collect stale polecat branches in a rig.

Polecats use unique timestamped branches (polecat/<name>-<timestamp>) to
prevent drift issues. Over time, these branches accumulate when stale
polecats are repaired.

This command removes orphaned branches:
  - Branches for polecats that no longer exist
  - Old timestamped branches (keeps only the current one per polecat)

Examples:
  gt polecat gc greenplace
  gt polecat gc greenplace --dry-run`,
	Args: cobra.ExactArgs(1),
	RunE: runPolecatGC,
}

var polecatNukeCmd = &cobra.Command{
	Use:   "nuke <rig>/<polecat>... | <rig> --all",
	Short: "Completely destroy a polecat (session, worktree, branch, agent bead)",
	Long: `Completely destroy a polecat and all its artifacts.

This is the nuclear option for post-merge cleanup. It:
  1. Kills the Claude session (if running)
  2. Deletes the git worktree (bypassing all safety checks)
  3. Deletes the polecat branch
  4. Closes the agent bead (if exists)

SAFETY CHECKS: The command refuses to nuke a polecat if:
  - Worktree has unpushed/uncommitted changes
  - Polecat has an open merge request (MR bead)
  - Polecat has work on its hook

Use --force to bypass safety checks (LOSES WORK).
Use --dry-run to see what would happen and safety check status.

Examples:
  gt polecat nuke greenplace/Toast
  gt polecat nuke greenplace/Toast greenplace/Furiosa
  gt polecat nuke greenplace --all
  gt polecat nuke greenplace --all --dry-run
  gt polecat nuke greenplace/Toast --force  # bypass safety checks`,
	Args: cobra.MinimumNArgs(1),
	RunE: runPolecatNuke,
}

var polecatGitStateCmd = &cobra.Command{
	Use:   "git-state <rig>/<polecat>",
	Short: "Show git state for pre-kill verification",
	Long: `Show git state for a polecat's worktree.

Used by the Witness for pre-kill verification to ensure no work is lost.
Returns whether the worktree is clean (safe to kill) or dirty (needs cleanup).

Checks:
  - Working tree: uncommitted changes
  - Unpushed commits: commits ahead of origin/main
  - Stashes: stashed changes

Examples:
  gt polecat git-state greenplace/Toast
  gt polecat git-state greenplace/Toast --json`,
	Args: cobra.ExactArgs(1),
	RunE: runPolecatGitState,
}

var polecatCheckRecoveryCmd = &cobra.Command{
	Use:   "check-recovery <rig>/<polecat>",
	Short: "Check if polecat needs recovery vs safe to nuke",
	Long: `Check recovery status of a polecat based on cleanup_status and merge queue state.

Used by the Witness to determine appropriate cleanup action:
  - SAFE_TO_NUKE: cleanup_status is 'clean' AND work submitted to merge queue
  - NEEDS_MQ_SUBMIT: git is clean but work was never submitted to the merge queue
  - NEEDS_RECOVERY: cleanup_status indicates unpushed/uncommitted work

This prevents accidental data loss when cleaning up dormant polecats.
The Witness should escalate NEEDS_RECOVERY and NEEDS_MQ_SUBMIT cases to the Mayor.

Examples:
  gt polecat check-recovery greenplace/Toast
  gt polecat check-recovery greenplace/Toast --json`,
	Args: cobra.ExactArgs(1),
	RunE: runPolecatCheckRecovery,
}

var (
	polecatStaleJSON      bool
	polecatStaleThreshold int
	polecatStaleCleanup   bool
	polecatStaleDryRun    bool
	polecatPruneDryRun    bool
	polecatPruneRemote    bool
)

var polecatStaleCmd = &cobra.Command{
	Use:   "stale <rig>",
	Short: "Detect stale polecats that may need cleanup",
	Long: `Detect stale polecats in a rig that are candidates for cleanup.

A polecat is considered stale if:
  - No active tmux session
  - Way behind main (>threshold commits) OR no agent bead
  - Has no uncommitted work that could be lost

The default threshold is 20 commits behind main.

Use --cleanup to automatically nuke stale polecats that are safe to remove.
Use --dry-run with --cleanup to see what would be cleaned.

Examples:
  gt polecat stale greenplace
  gt polecat stale greenplace --threshold 50
  gt polecat stale greenplace --json
  gt polecat stale greenplace --cleanup
  gt polecat stale greenplace --cleanup --dry-run`,
	Args: cobra.ExactArgs(1),
	RunE: runPolecatStale,
}

var polecatPruneCmd = &cobra.Command{
	Use:   "prune <rig>",
	Short: "Prune stale polecat branches (local and remote)",
	Long: `Prune stale polecat branches in a rig.

Finds and deletes polecat branches that are no longer needed:
  - Branches fully merged to main
  - Branches whose remote tracking branch was deleted (post-merge cleanup)
  - Branches for polecats that no longer exist (orphaned)

Uses safe deletion (git branch -d) — only removes fully merged branches.
Also cleans up remote polecat branches that are fully merged.

Use --dry-run to preview what would be pruned.
Use --remote to also prune remote polecat branches on origin.

Examples:
  gt polecat prune greenplace
  gt polecat prune greenplace --dry-run
  gt polecat prune greenplace --remote`,
	Args: cobra.ExactArgs(1),
	RunE: runPolecatPrune,
}

var polecatPoolInitCmd = &cobra.Command{
	Use:   "pool-init <rig>",
	Short: "Initialize a persistent polecat pool for a rig",
	Long: `Initialize a persistent polecat pool for a rig.

Creates N polecats with identities and worktrees in IDLE state,
ready for immediate work assignment via gt sling.

Pool size is determined by (in priority order):
  1. --size flag
  2. polecat_pool_size in rig config.json
  3. Default: 4

Polecat names come from:
  1. polecat_names in rig config.json (if specified)
  2. The rig's name pool theme (default: mad-max)

Existing polecats are preserved — only new ones are created
to reach the target pool size.

Examples:
  gt polecat pool-init gastown
  gt polecat pool-init gastown --size 6
  gt polecat pool-init gastown --dry-run`,
	Args: cobra.ExactArgs(1),
	RunE: runPolecatPoolInit,
}

func init() {
	// List flags
	polecatListCmd.Flags().BoolVar(&polecatListJSON, "json", false, "Output as JSON")
	polecatListCmd.Flags().BoolVar(&polecatListAll, "all", false, "List polecats in all rigs")

	// Remove flags
	polecatRemoveCmd.Flags().BoolVarP(&polecatForce, "force", "f", false, "Force removal, bypassing checks")
	polecatRemoveCmd.Flags().BoolVar(&polecatRemoveAll, "all", false, "Remove all polecats in the rig")

	// Status flags
	polecatStatusCmd.Flags().BoolVar(&polecatStatusJSON, "json", false, "Output as JSON")

	// Git-state flags
	polecatGitStateCmd.Flags().BoolVar(&polecatGitStateJSON, "json", false, "Output as JSON")

	// GC flags
	polecatGCCmd.Flags().BoolVar(&polecatGCDryRun, "dry-run", false, "Show what would be deleted without deleting")

	// Nuke flags
	polecatNukeCmd.Flags().BoolVar(&polecatNukeAll, "all", false, "Nuke all polecats in the rig")
	polecatNukeCmd.Flags().BoolVar(&polecatNukeDryRun, "dry-run", false, "Show what would be nuked without doing it")
	polecatNukeCmd.Flags().BoolVarP(&polecatNukeForce, "force", "f", false, "Force nuke, bypassing all safety checks (LOSES WORK)")

	// Check-recovery flags
	polecatCheckRecoveryCmd.Flags().BoolVar(&polecatCheckRecoveryJSON, "json", false, "Output as JSON")

	// Stale flags
	polecatStaleCmd.Flags().BoolVar(&polecatStaleJSON, "json", false, "Output as JSON")
	polecatStaleCmd.Flags().IntVar(&polecatStaleThreshold, "threshold", 20, "Commits behind main to consider stale")
	polecatStaleCmd.Flags().BoolVar(&polecatStaleCleanup, "cleanup", false, "Automatically nuke stale polecats")
	polecatStaleCmd.Flags().BoolVar(&polecatStaleDryRun, "dry-run", false, "Show what would be cleaned without doing it")

	// Prune flags
	polecatPruneCmd.Flags().BoolVar(&polecatPruneDryRun, "dry-run", false, "Show what would be pruned without doing it")
	polecatPruneCmd.Flags().BoolVar(&polecatPruneRemote, "remote", false, "Also prune remote polecat branches on origin")

	// Pool-init flags
	polecatPoolInitCmd.Flags().BoolVar(&polecatPoolInitDryRun, "dry-run", false, "Show what would be created without doing it")
	polecatPoolInitCmd.Flags().IntVar(&polecatPoolInitSize, "size", 0, "Pool size (overrides rig config)")

	// Add subcommands
	polecatCmd.AddCommand(polecatListCmd)
	polecatCmd.AddCommand(polecatAddCmd)
	polecatCmd.AddCommand(polecatRemoveCmd)
	polecatCmd.AddCommand(polecatStatusCmd)
	polecatCmd.AddCommand(polecatGitStateCmd)
	polecatCmd.AddCommand(polecatCheckRecoveryCmd)
	polecatCmd.AddCommand(polecatGCCmd)
	polecatCmd.AddCommand(polecatNukeCmd)
	polecatCmd.AddCommand(polecatStaleCmd)
	polecatCmd.AddCommand(polecatPruneCmd)
	polecatCmd.AddCommand(polecatPoolInitCmd)

	rootCmd.AddCommand(polecatCmd)
}

// PolecatListItem represents a polecat in list output.
type PolecatListItem struct {
	Rig            string        `json:"rig"`
	Name           string        `json:"name"`
	State          polecat.State `json:"state"`
	Issue          string        `json:"issue,omitempty"`
	SessionRunning bool          `json:"session_running"`
	Zombie         bool          `json:"zombie,omitempty"`
	SessionName    string        `json:"session_name,omitempty"`
}

// effectivePolecatState returns the observable state used by polecat list output.
// Session liveness is ground truth for working/idle/done transitions. Zombie entries
// are never auto-rewritten.
func effectivePolecatState(item PolecatListItem) polecat.State {
	state := item.State
	// A running session overrides both "done" and "idle" — the polecat is working.
	// "idle" can be stale when a polecat is reused (cross-rig beads, stale heartbeat,
	// or beads query timing), and "done" can be stale when gt done didn't complete.
	if item.SessionRunning && (state == polecat.StateDone || state == polecat.StateIdle) {
		return polecat.StateWorking
	}
	if !item.SessionRunning && !item.Zombie && state == polecat.StateWorking {
		return polecat.StateDone
	}
	return state
}

// getPolecatManager creates a polecat manager for the given rig.
func getPolecatManager(rigName string) (*polecat.Manager, *rig.Rig, error) {
	_, r, err := getRig(rigName)
	if err != nil {
		return nil, nil, err
	}

	polecatGit := git.NewGit(r.Path)
	t := tmux.NewTmux()
	mgr := polecat.NewManager(r, polecatGit, t)

	return mgr, r, nil
}

func runPolecatList(cmd *cobra.Command, args []string) error {
	var rigs []*rig.Rig

	if polecatListAll {
		// List all rigs
		allRigs, err := getAllRigs()
		if err != nil {
			return err
		}
		rigs = allRigs
	} else {
		// Need a rig name
		if len(args) < 1 {
			return fmt.Errorf("rig name required (or use --all)")
		}
		_, r, err := getPolecatManager(args[0])
		if err != nil {
			return err
		}
		rigs = []*rig.Rig{r}
	}

	// Collect polecats from all rigs
	t := tmux.NewTmux()
	allPolecats := make([]PolecatListItem, 0)

	for _, r := range rigs {
		polecatGit := git.NewGit(r.Path)
		mgr := polecat.NewManager(r, polecatGit, t)
		polecatMgr := polecat.NewSessionManager(t, r)

		polecats, err := mgr.List()
		if err != nil {
			fmt.Fprintf(os.Stderr, "warning: failed to list polecats in %s: %v\n", r.Name, err)
			continue
		}

		// Track known polecat names from filesystem for zombie detection
		knownNames := make(map[string]bool)
		for _, p := range polecats {
			running, _ := polecatMgr.IsRunning(p.Name)
			allPolecats = append(allPolecats, PolecatListItem{
				Rig:            r.Name,
				Name:           p.Name,
				State:          p.State,
				Issue:          p.Issue,
				SessionRunning: running,
			})
			knownNames[p.Name] = true
		}

		// Discover zombie tmux sessions: sessions without matching worktree directories.
		// These occur when a worktree is deleted but the tmux session persists
		// (incomplete nuke or session naming mismatch).
		zombieSessions, _ := findRigPolecatSessions(r.Name)
		for _, sessionName := range zombieSessions {
			_, polecatName, ok := parsePolecatSessionName(sessionName)
			if !ok {
				continue
			}
			if !knownNames[polecatName] {
				allPolecats = append(allPolecats, PolecatListItem{
					Rig:            r.Name,
					Name:           polecatName,
					State:          polecat.StateZombie,
					SessionRunning: true,
					Zombie:         true,
					SessionName:    sessionName,
				})
			}
		}
	}

	// Output
	for i := range allPolecats {
		allPolecats[i].State = effectivePolecatState(allPolecats[i])
	}

	if polecatListJSON {
		enc := json.NewEncoder(os.Stdout)
		enc.SetIndent("", "  ")
		return enc.Encode(allPolecats)
	}

	if len(allPolecats) == 0 {
		fmt.Println("No polecats found.")
		return nil
	}

	fmt.Printf("%s\n\n", style.Bold.Render("Polecats"))
	for _, p := range allPolecats {
		// Session indicator
		sessionStatus := style.Dim.Render("○")
		if p.SessionRunning {
			sessionStatus = style.Success.Render("●")
		}

		// State color
		stateStr := string(p.State)
		switch p.State {
		case polecat.StateWorking:
			stateStr = style.Info.Render(stateStr)
		case polecat.StateStuck:
			stateStr = style.Warning.Render(stateStr)
		case polecat.StateDone:
			stateStr = style.Success.Render(stateStr)
		case polecat.StateZombie:
			stateStr = style.Error.Render(stateStr)
		default:
			stateStr = style.Dim.Render(stateStr)
		}

		fmt.Printf("  %s %s/%s  %s\n", sessionStatus, p.Rig, p.Name, stateStr)
		if p.Issue != "" {
			fmt.Printf("    %s\n", style.Dim.Render(p.Issue))
		}
		if p.Zombie && p.SessionName != "" {
			fmt.Printf("    %s\n", style.Dim.Render("session: "+p.SessionName+" (no worktree)"))
		}
	}

	return nil
}

func runPolecatAdd(cmd *cobra.Command, args []string) error {
	// Emit deprecation warning
	fmt.Fprintf(os.Stderr, "%s 'gt polecat add' is deprecated. Use 'gt polecat identity add' instead.\n",
		style.Warning.Render("Warning:"))
	fmt.Fprintf(os.Stderr, "         This command will be removed in v1.0.\n\n")

	rigName := args[0]
	polecatName := args[1]

	mgr, _, err := getPolecatManager(rigName)
	if err != nil {
		return err
	}

	fmt.Printf("Adding polecat %s to rig %s...\n", polecatName, rigName)

	p, err := mgr.Add(polecatName)
	if err != nil {
		return fmt.Errorf("adding polecat: %w", err)
	}

	fmt.Printf("%s Polecat %s added.\n", style.SuccessPrefix, p.Name)
	fmt.Printf("  %s\n", style.Dim.Render(p.ClonePath))
	fmt.Printf("  Branch: %s\n", style.Dim.Render(p.Branch))

	return nil
}

func runPolecatRemove(cmd *cobra.Command, args []string) error {
	targets, err := resolvePolecatTargets(args, polecatRemoveAll)
	if err != nil {
		return err
	}

	if len(targets) == 0 {
		fmt.Println("No polecats to remove.")
		return nil
	}

	// Remove each polecat
	t := tmux.NewTmux()
	var removeErrors []string
	removed := 0

	for _, p := range targets {
		// Check if session is running
		if !polecatForce {
			polecatMgr := polecat.NewSessionManager(t, p.r)
			running, _ := polecatMgr.IsRunning(p.polecatName)
			if running {
				removeErrors = append(removeErrors, fmt.Sprintf("%s/%s: session is running (stop first or use --force)", p.rigName, p.polecatName))
				continue
			}
		}

		fmt.Printf("Removing polecat %s/%s...\n", p.rigName, p.polecatName)

		if err := p.mgr.Remove(p.polecatName, polecatForce); err != nil {
			if errors.Is(err, polecat.ErrHasChanges) {
				removeErrors = append(removeErrors, fmt.Sprintf("%s/%s: has uncommitted changes (use --force)", p.rigName, p.polecatName))
			} else {
				removeErrors = append(removeErrors, fmt.Sprintf("%s/%s: %v", p.rigName, p.polecatName, err))
			}
			continue
		}

		fmt.Printf("  %s removed\n", style.Success.Render("✓"))
		removed++
	}

	// Report results
	if len(removeErrors) > 0 {
		fmt.Printf("\n%s Some removals failed:\n", style.Warning.Render("Warning:"))
		for _, e := range removeErrors {
			fmt.Printf("  - %s\n", e)
		}
	}

	if removed > 0 {
		fmt.Printf("\n%s Removed %d polecat(s).\n", style.SuccessPrefix, removed)
	}

	if len(removeErrors) > 0 {
		return fmt.Errorf("%d removal(s) failed", len(removeErrors))
	}

	return nil
}

// PolecatStatus represents detailed polecat status for JSON output.
type PolecatStatus struct {
	Rig            string        `json:"rig"`
	Name           string        `json:"name"`
	State          polecat.State `json:"state"`
	Issue          string        `json:"issue,omitempty"`
	ClonePath      string        `json:"clone_path"`
	Branch         string        `json:"branch"`
	SessionRunning bool          `json:"session_running"`
	SessionID      string        `json:"session_id,omitempty"`
	Attached       bool          `json:"attached,omitempty"`
	Windows        int           `json:"windows,omitempty"`
	CreatedAt      string        `json:"created_at,omitempty"`
	LastActivity   string        `json:"last_activity,omitempty"`
}

func runPolecatStatus(cmd *cobra.Command, args []string) error {
	rigName, polecatName, err := parseAddress(args[0])
	if err != nil {
		return err
	}

	mgr, r, err := getPolecatManager(rigName)
	if err != nil {
		return err
	}

	// Get polecat info
	p, err := mgr.Get(polecatName)
	if err != nil {
		return fmt.Errorf("polecat '%s' not found in rig '%s'", polecatName, rigName)
	}

	// Get session info
	t := tmux.NewTmux()
	polecatMgr := polecat.NewSessionManager(t, r)
	sessInfo, err := polecatMgr.Status(polecatName)
	if err != nil {
		// Non-fatal - continue without session info
		sessInfo = &polecat.SessionInfo{
			Polecat: polecatName,
			Running: false,
		}
	}

	// JSON output
	if polecatStatusJSON {
		status := PolecatStatus{
			Rig:            rigName,
			Name:           polecatName,
			State:          p.State,
			Issue:          p.Issue,
			ClonePath:      p.ClonePath,
			Branch:         p.Branch,
			SessionRunning: sessInfo.Running,
			SessionID:      sessInfo.SessionID,
			Attached:       sessInfo.Attached,
			Windows:        sessInfo.Windows,
		}
		if !sessInfo.Created.IsZero() {
			status.CreatedAt = sessInfo.Created.Format("2006-01-02 15:04:05")
		}
		if !sessInfo.LastActivity.IsZero() {
			status.LastActivity = sessInfo.LastActivity.Format("2006-01-02 15:04:05")
		}
		enc := json.NewEncoder(os.Stdout)
		enc.SetIndent("", "  ")
		return enc.Encode(status)
	}

	// Human-readable output
	fmt.Printf("%s\n\n", style.Bold.Render(fmt.Sprintf("Polecat: %s/%s", rigName, polecatName)))

	// State with color
	stateStr := string(p.State)
	switch p.State {
	case polecat.StateWorking:
		stateStr = style.Info.Render(stateStr)
	case polecat.StateStuck:
		stateStr = style.Warning.Render(stateStr)
	case polecat.StateDone:
		stateStr = style.Success.Render(stateStr)
	default:
		stateStr = style.Dim.Render(stateStr)
	}
	fmt.Printf("  State:         %s\n", stateStr)

	// Issue
	if p.Issue != "" {
		fmt.Printf("  Issue:         %s\n", p.Issue)
	} else {
		fmt.Printf("  Issue:         %s\n", style.Dim.Render("(none)"))
	}

	// Clone path and branch
	fmt.Printf("  Clone:         %s\n", style.Dim.Render(p.ClonePath))
	fmt.Printf("  Branch:        %s\n", style.Dim.Render(p.Branch))

	// Session info
	fmt.Println()
	fmt.Printf("%s\n", style.Bold.Render("Session"))

	if sessInfo.Running {
		fmt.Printf("  Status:        %s\n", style.Success.Render("running"))
		fmt.Printf("  Session ID:    %s\n", style.Dim.Render(sessInfo.SessionID))

		if sessInfo.Attached {
			fmt.Printf("  Attached:      %s\n", style.Info.Render("yes"))
		} else {
			fmt.Printf("  Attached:      %s\n", style.Dim.Render("no"))
		}

		if sessInfo.Windows > 0 {
			fmt.Printf("  Windows:       %d\n", sessInfo.Windows)
		}

		if !sessInfo.Created.IsZero() {
			fmt.Printf("  Created:       %s\n", sessInfo.Created.Format("2006-01-02 15:04:05"))
		}

		if !sessInfo.LastActivity.IsZero() {
			// Show relative time for activity
			ago := formatActivityTime(sessInfo.LastActivity)
			fmt.Printf("  Last Activity: %s (%s)\n",
				sessInfo.LastActivity.Format("15:04:05"),
				style.Dim.Render(ago))
		}
	} else {
		fmt.Printf("  Status:        %s\n", style.Dim.Render("not running"))
	}

	return nil
}

// formatActivityTime returns a human-readable relative time string.
func formatActivityTime(t time.Time) string {
	d := time.Since(t)
	switch {
	case d < time.Minute:
		return fmt.Sprintf("%d seconds ago", int(d.Seconds()))
	case d < time.Hour:
		return fmt.Sprintf("%d minutes ago", int(d.Minutes()))
	case d < 24*time.Hour:
		return fmt.Sprintf("%d hours ago", int(d.Hours()))
	default:
		return fmt.Sprintf("%d days ago", int(d.Hours()/24))
	}
}

// GitState represents the git state of a polecat's worktree.
type GitState struct {
	Clean            bool     `json:"clean"`
	UncommittedFiles []string `json:"uncommitted_files"`
	UnpushedCommits  int      `json:"unpushed_commits"`
	StashCount       int      `json:"stash_count"`
}

func runPolecatGitState(cmd *cobra.Command, args []string) error {
	rigName, polecatName, err := parseAddress(args[0])
	if err != nil {
		return err
	}

	mgr, r, err := getPolecatManager(rigName)
	if err != nil {
		return err
	}

	// Verify polecat exists
	p, err := mgr.Get(polecatName)
	if err != nil {
		return fmt.Errorf("polecat '%s' not found in rig '%s'", polecatName, rigName)
	}

	// Get git state from the polecat's worktree
	state, err := getGitState(p.ClonePath)
	if err != nil {
		return fmt.Errorf("getting git state: %w", err)
	}

	// JSON output
	if polecatGitStateJSON {
		enc := json.NewEncoder(os.Stdout)
		enc.SetIndent("", "  ")
		return enc.Encode(state)
	}

	// Human-readable output
	fmt.Printf("%s\n\n", style.Bold.Render(fmt.Sprintf("Git State: %s/%s", r.Name, polecatName)))

	// Working tree status
	if len(state.UncommittedFiles) == 0 {
		fmt.Printf("  Working Tree:  %s\n", style.Success.Render("clean"))
	} else {
		fmt.Printf("  Working Tree:  %s\n", style.Warning.Render("dirty"))
		fmt.Printf("  Uncommitted:   %s\n", style.Warning.Render(fmt.Sprintf("%d files", len(state.UncommittedFiles))))
		for _, f := range state.UncommittedFiles {
			fmt.Printf("                 %s\n", style.Dim.Render(f))
		}
	}

	// Unpushed commits
	if state.UnpushedCommits == 0 {
		fmt.Printf("  Unpushed:      %s\n", style.Success.Render("0 commits"))
	} else {
		fmt.Printf("  Unpushed:      %s\n", style.Warning.Render(fmt.Sprintf("%d commits ahead", state.UnpushedCommits)))
	}

	// Stashes
	if state.StashCount == 0 {
		fmt.Printf("  Stashes:       %s\n", style.Dim.Render("0"))
	} else {
		fmt.Printf("  Stashes:       %s\n", style.Warning.Render(fmt.Sprintf("%d", state.StashCount)))
	}

	// Verdict
	fmt.Println()
	if state.Clean {
		fmt.Printf("  Verdict:       %s\n", style.Success.Render("CLEAN (safe to kill)"))
	} else {
		fmt.Printf("  Verdict:       %s\n", style.Error.Render("DIRTY (needs cleanup)"))
	}

	return nil
}

// getGitState checks the git state of a worktree.
func getGitState(worktreePath string) (*GitState, error) {
	state := &GitState{
		Clean:            true,
		UncommittedFiles: []string{},
	}

	// Check for uncommitted changes (git status --porcelain)
	statusCmd := exec.Command("git", "status", "--porcelain")
	statusCmd.Dir = worktreePath
	output, err := statusCmd.Output()
	if err != nil {
		return nil, fmt.Errorf("git status: %w", err)
	}
	if len(output) > 0 {
		lines := splitLines(string(output))
		for _, line := range lines {
			if line != "" {
				// Extract filename (skip the status prefix)
				if len(line) > 3 {
					state.UncommittedFiles = append(state.UncommittedFiles, line[3:])
				} else {
					state.UncommittedFiles = append(state.UncommittedFiles, line)
				}
			}
		}
		state.Clean = false
	}

	// Check for unpushed commits (git log origin/main..HEAD)
	// We check commits first, then verify if content differs.
	// After squash merge, commits may differ but content may be identical.
	mainRef := "origin/main"
	logCmd := exec.Command("git", "log", mainRef+"..HEAD", "--oneline")
	logCmd.Dir = worktreePath
	output, err = logCmd.Output()
	if err != nil {
		// origin/main might not exist - try origin/master
		mainRef = "origin/master"
		logCmd = exec.Command("git", "log", mainRef+"..HEAD", "--oneline")
		logCmd.Dir = worktreePath
		output, _ = logCmd.Output() // non-fatal: might be a new repo without remote tracking
	}
	if len(output) > 0 {
		lines := splitLines(string(output))
		count := 0
		for _, line := range lines {
			if line != "" {
				count++
			}
		}
		if count > 0 {
			// Commits exist that aren't on main. But after squash merge,
			// the content may actually be on main with different commit SHAs.
			// Check if there's any actual diff between HEAD and main.
			diffCmd := exec.Command("git", "diff", mainRef, "HEAD", "--quiet")
			diffCmd.Dir = worktreePath
			diffErr := diffCmd.Run()
			if diffErr == nil {
				// Exit code 0 means no diff - content IS on main (squash merged)
				// Don't count these as unpushed
				state.UnpushedCommits = 0
			} else {
				// Exit code 1 means there's a diff - truly unpushed work
				state.UnpushedCommits = count
				state.Clean = false
			}
		}
	}

	// Check for stashes using Git.StashCount() which filters by current branch.
	// Without branch filtering, worktrees see repo-wide stashes and produce
	// false "NEEDS_RECOVERY" verdicts for worktrees with zero stashes of their own.
	worktreeGit := git.NewGit(worktreePath)
	if stashCount, stashErr := worktreeGit.StashCount(); stashErr == nil {
		state.StashCount = stashCount
		if stashCount > 0 {
			state.Clean = false
		}
	}

	return state, nil
}

// RecoveryStatus represents whether a polecat needs recovery or is safe to nuke.
type RecoveryStatus struct {
	Rig           string                `json:"rig"`
	Polecat       string                `json:"polecat"`
	CleanupStatus polecat.CleanupStatus `json:"cleanup_status"`
	NeedsRecovery bool                  `json:"needs_recovery"`
	Verdict       string                `json:"verdict"` // SAFE_TO_NUKE, NEEDS_RECOVERY, or NEEDS_MQ_SUBMIT
	Branch        string                `json:"branch,omitempty"`
	Issue         string                `json:"issue,omitempty"`
	MQStatus      string                `json:"mq_status,omitempty"` // "submitted", "not_submitted", "unknown"
}

func runPolecatCheckRecovery(cmd *cobra.Command, args []string) error {
	rigName, polecatName, err := parseAddress(args[0])
	if err != nil {
		return err
	}

	mgr, r, err := getPolecatManager(rigName)
	if err != nil {
		return err
	}

	// Verify polecat exists and get info
	p, err := mgr.Get(polecatName)
	if err != nil {
		return fmt.Errorf("polecat '%s' not found in rig '%s'", polecatName, rigName)
	}

	// Get cleanup_status from agent bead
	// We need to read it directly from beads since manager doesn't expose it
	rigPath := r.Path
	bd := beads.New(rigPath)
	agentBeadID := polecatBeadIDForRig(r, rigName, polecatName)
	_, fields, err := bd.GetAgentBead(agentBeadID)

	status := RecoveryStatus{
		Rig:     rigName,
		Polecat: polecatName,
		Branch:  p.Branch,
		Issue:   p.Issue,
	}

	if err != nil || fields == nil {
		// No agent bead or no cleanup_status - fall back to git check
		// This handles polecats that haven't self-reported yet
		gitState, gitErr := getGitState(p.ClonePath)
		if gitErr != nil {
			status.CleanupStatus = polecat.CleanupUnknown
			status.NeedsRecovery = true
			status.Verdict = "NEEDS_RECOVERY"
		} else if gitState.Clean {
			status.CleanupStatus = polecat.CleanupClean
			status.NeedsRecovery = false
			status.Verdict = "SAFE_TO_NUKE"
		} else if gitState.UnpushedCommits > 0 {
			status.CleanupStatus = polecat.CleanupUnpushed
			status.NeedsRecovery = true
			status.Verdict = "NEEDS_RECOVERY"
		} else if gitState.StashCount > 0 {
			status.CleanupStatus = polecat.CleanupStash
			status.NeedsRecovery = true
			status.Verdict = "NEEDS_RECOVERY"
		} else {
			status.CleanupStatus = polecat.CleanupUncommitted
			status.NeedsRecovery = true
			status.Verdict = "NEEDS_RECOVERY"
		}
	} else {
		// Use cleanup_status from agent bead
		status.CleanupStatus = polecat.CleanupStatus(fields.CleanupStatus)
		if status.CleanupStatus.IsSafe() && fields.ActiveMR == "" {
			status.NeedsRecovery = false
			status.Verdict = "SAFE_TO_NUKE"
		} else {
			// RequiresRecovery covers uncommitted, stash, unpushed
			// Unknown/empty also treated conservatively
			status.NeedsRecovery = true
			status.Verdict = "NEEDS_RECOVERY"
		}
	}

	// MQ check: if verdict is SAFE_TO_NUKE and polecat has a branch,
	// verify the work was actually submitted to the merge queue.
	// Without this check, polecats that crashed between push and MQ submission
	// would be nuked with orphaned branches on the remote. See #1035.
	if status.Verdict == "SAFE_TO_NUKE" && status.Branch != "" {
		mqBd := beads.New(r.Path)
		mr, mrErr := mqBd.FindMRForBranchAny(status.Branch)
		if mrErr != nil {
			// Can't verify MQ — be conservative
			status.MQStatus = "unknown"
		} else if mr != nil {
			status.MQStatus = "submitted"
		} else {
			// Work was pushed but never entered the merge queue
			status.MQStatus = "not_submitted"
			status.NeedsRecovery = true
			status.Verdict = "NEEDS_MQ_SUBMIT"
		}
	}

	// JSON output
	if polecatCheckRecoveryJSON {
		enc := json.NewEncoder(os.Stdout)
		enc.SetIndent("", "  ")
		return enc.Encode(status)
	}

	// Human-readable output
	fmt.Printf("%s\n\n", style.Bold.Render(fmt.Sprintf("Recovery Status: %s/%s", rigName, polecatName)))
	fmt.Printf("  Cleanup Status:  %s\n", status.CleanupStatus)
	if status.Branch != "" {
		fmt.Printf("  Branch:          %s\n", status.Branch)
	}
	if status.Issue != "" {
		fmt.Printf("  Issue:           %s\n", status.Issue)
	}
	fmt.Println()

	switch status.Verdict {
	case "NEEDS_MQ_SUBMIT":
		fmt.Printf("  Verdict:         %s\n", style.Warning.Render("NEEDS_MQ_SUBMIT"))
		fmt.Printf("  MQ Status:       %s\n", status.MQStatus)
		fmt.Println()
		fmt.Printf("  %s Work is pushed but was never submitted to the merge queue.\n", style.Warning.Render("⚠"))
		fmt.Println("  Submit to MQ before cleanup, or the branch will be orphaned.")
	case "NEEDS_RECOVERY":
		fmt.Printf("  Verdict:         %s\n", style.Error.Render("NEEDS_RECOVERY"))
		fmt.Println()
		fmt.Printf("  %s This polecat has unpushed/uncommitted work.\n", style.Warning.Render("⚠"))
		fmt.Println("  Escalate to Mayor for recovery before cleanup.")
	default:
		fmt.Printf("  Verdict:         %s\n", style.Success.Render("SAFE_TO_NUKE"))
		if status.MQStatus != "" {
			fmt.Printf("  MQ Status:       %s\n", status.MQStatus)
		}
		fmt.Println()
		fmt.Printf("  %s Safe to nuke - no work at risk.\n", style.Success.Render("✓"))
	}

	return nil
}

func runPolecatGC(cmd *cobra.Command, args []string) error {
	rigName := args[0]

	mgr, r, err := getPolecatManager(rigName)
	if err != nil {
		return err
	}

	fmt.Printf("Garbage collecting stale polecat branches in %s...\n\n", r.Name)

	if polecatGCDryRun {
		// Dry run - list branches that would be deleted
		repoGit := git.NewGit(r.Path)

		// List all polecat branches
		branches, err := repoGit.ListBranches("polecat/*")
		if err != nil {
			return fmt.Errorf("listing branches: %w", err)
		}

		if len(branches) == 0 {
			fmt.Println("No polecat branches found.")
			return nil
		}

		// Get current branches
		polecats, err := mgr.List()
		if err != nil {
			return fmt.Errorf("listing polecats: %w", err)
		}

		currentBranches := make(map[string]bool)
		for _, p := range polecats {
			currentBranches[p.Branch] = true
		}

		// Show what would be deleted
		toDelete := 0
		for _, branch := range branches {
			if !currentBranches[branch] {
				fmt.Printf("  Would delete: %s\n", style.Dim.Render(branch))
				toDelete++
			} else {
				fmt.Printf("  Keep (in use): %s\n", style.Success.Render(branch))
			}
		}

		fmt.Printf("\nWould delete %d branch(es), keep %d\n", toDelete, len(branches)-toDelete)
		return nil
	}

	// Actually clean up
	deleted, err := mgr.CleanupStaleBranches()
	if err != nil {
		return fmt.Errorf("cleanup failed: %w", err)
	}

	if deleted == 0 {
		fmt.Println("No stale branches to clean up.")
	} else {
		fmt.Printf("%s Deleted %d stale branch(es).\n", style.SuccessPrefix, deleted)
	}

	return nil
}

// splitLines splits a string into non-empty lines.
func splitLines(s string) []string {
	var lines []string
	for _, line := range strings.Split(s, "\n") {
		if line != "" {
			lines = append(lines, line)
		}
	}
	return lines
}

func runPolecatNuke(cmd *cobra.Command, args []string) error {
	targets, err := resolvePolecatTargets(args, polecatNukeAll)
	if err != nil {
		return err
	}

	if len(targets) == 0 {
		fmt.Println("No polecats to nuke.")
		return nil
	}

	// Safety checks: refuse to nuke polecats with active work unless --force is set
	if !polecatNukeForce && !polecatNukeDryRun {
		var blocked []*SafetyCheckResult
		for _, p := range targets {
			result := checkPolecatSafety(p)
			if result.Blocked {
				blocked = append(blocked, result)
			}
		}

		if len(blocked) > 0 {
			displaySafetyCheckBlocked(blocked)
			return fmt.Errorf("blocked: %d polecat(s) have active work", len(blocked))
		}
	}

	// Nuke each polecat
	var nukeErrors []string
	nuked := 0

	for _, p := range targets {
		if polecatNukeDryRun {
			fmt.Printf("Would nuke %s/%s:\n", p.rigName, p.polecatName)
			fmt.Printf("  - Kill session: gt-%s-%s\n", p.rigName, p.polecatName)
			fmt.Printf("  - Delete worktree: %s/polecats/%s\n", p.r.Path, p.polecatName)
			fmt.Printf("  - Delete branch (if exists)\n")
			fmt.Printf("  - Close agent bead: %s\n", polecatBeadIDForRig(p.r, p.rigName, p.polecatName))

			displayDryRunSafetyCheck(p)
			fmt.Println()
			continue
		}

		if polecatNukeForce {
			fmt.Printf("%s Nuking %s/%s (--force)...\n", style.Warning.Render("⚠"), p.rigName, p.polecatName)
		} else {
			fmt.Printf("Nuking %s/%s...\n", p.rigName, p.polecatName)
		}

		if err := nukePolecatFull(p.polecatName, p.rigName, p.mgr, p.r); err != nil {
			nukeErrors = append(nukeErrors, fmt.Sprintf("%s/%s: %v", p.rigName, p.polecatName, err))
			continue
		}

		nuked++
	}

	// Report results
	if polecatNukeDryRun {
		fmt.Printf("\n%s Would nuke %d polecat(s).\n", style.Info.Render("ℹ"), len(targets))
		return nil
	}

	if len(nukeErrors) > 0 {
		fmt.Printf("\n%s Some nukes failed:\n", style.Warning.Render("Warning:"))
		for _, e := range nukeErrors {
			fmt.Printf("  - %s\n", e)
		}
	}

	if nuked > 0 {
		fmt.Printf("\n%s Nuked %d polecat(s).\n", style.SuccessPrefix, nuked)
	}

	// Final cleanup: Kill any orphaned Claude processes that escaped the session termination.
	// This catches processes that called setsid() or were reparented during session shutdown.
	if !polecatNukeDryRun {
		cleanupOrphanedProcesses()
	}

	if len(nukeErrors) > 0 {
		return fmt.Errorf("%d nuke(s) failed", len(nukeErrors))
	}

	return nil
}

// nukePolecatFull performs the complete cleanup sequence for a single polecat:
// 1. Kill tmux session
// 2. Delete worktree (via RemoveWithOptions with nuclear=true)
// 3. Delete git branch
// 4. Close agent bead
// This is the canonical cleanup path used by both `polecat nuke` and `polecat stale --cleanup`.
func nukePolecatFull(polecatName, rigName string, mgr *polecat.Manager, r *rig.Rig) error {
	t := tmux.NewTmux()

	// Step 1: Kill tmux session unconditionally to prevent ghost sessions
	// when IsRunning fails to detect the session.
	sessMgr := polecat.NewSessionManager(t, r)
	if err := sessMgr.Stop(polecatName, true); err != nil {
		if !errors.Is(err, polecat.ErrSessionNotFound) {
			fmt.Printf("  %s session kill failed: %v\n", style.Warning.Render("⚠"), err)
		}
	} else {
		fmt.Printf("  %s killed session\n", style.Success.Render("✓"))
	}

	// Step 2: Get polecat info before deletion (for branch name + hooked work bead)
	polecatInfo, getErr := mgr.Get(polecatName)
	var branchToDelete string
	if getErr == nil && polecatInfo != nil {
		branchToDelete = polecatInfo.Branch
	}

	// Step 2.5: Burn any molecule attached to the polecat's hooked work bead.
	// Without this, nuked polecats leave orphan molecule refs that block re-sling.
	// The stale attached_molecule in the work bead's description causes sling to
	// fail with "bead already has N attached molecule(s)" on re-dispatch (gt-npzy).
	if getErr == nil && polecatInfo != nil && polecatInfo.Issue != "" {
		nukeCleanupMolecules(polecatInfo.Issue, r)
	}

	// Step 2.75: Best-effort push before nuke (gt-4vr guardrail).
	// Try to preserve any unpushed commits on the branch. If push fails,
	// proceed — --force already means "I accept data loss".
	if branchToDelete != "" {
		var pushGit *git.Git
		// Try worktree first (may still exist), then bare repo fallback.
		// Use ClonePath from the polecat record — the worktree lives at
		// <rig>/polecats/<name>/<rigName>/, not <rig>/polecats/<name>/.
		if polecatInfo != nil && polecatInfo.ClonePath != "" {
			if _, statErr := os.Stat(polecatInfo.ClonePath); statErr == nil {
				pushGit = git.NewGit(polecatInfo.ClonePath)
			}
		}
		if pushGit == nil {
			bareRepoPath := filepath.Join(r.Path, ".repo.git")
			if info, statErr := os.Stat(bareRepoPath); statErr == nil && info.IsDir() {
				pushGit = git.NewGitWithDir(bareRepoPath, "")
			}
		}
		if pushGit != nil {
			refspec := branchToDelete + ":" + branchToDelete
			if err := pushGit.Push("origin", refspec, false); err != nil {
				fmt.Printf("  %s best-effort push failed (proceeding): %v\n", style.Dim.Render("○"), err)
			} else {
				fmt.Printf("  %s pushed branch %s before nuke\n", style.Success.Render("✓"), branchToDelete)
			}
		}
	}

	// Step 3: Delete worktree (nuclear=true to bypass safety checks for stale polecats)
	if err := mgr.RemoveWithOptions(polecatName, true, true, false); err != nil {
		if errors.Is(err, polecat.ErrPolecatNotFound) {
			fmt.Printf("  %s worktree already gone\n", style.Dim.Render("○"))
		} else {
			return fmt.Errorf("worktree removal failed: %w", err)
		}
	} else {
		fmt.Printf("  %s deleted worktree\n", style.Success.Render("✓"))
	}

	// Step 4: Delete local branch (if we know it)
	// Local branch can always be deleted (worktree is already gone).
	// Remote branch is never deleted during nuke — the refinery owns
	// remote branch cleanup after successful merge (gt mq post-merge).
	// This prevents the race where nuke deletes the branch before the
	// refinery has a chance to merge it. (gt-v5ku)
	if branchToDelete != "" {
		repoGit := getRepoGitForRig(r.Path)
		if err := repoGit.DeleteBranch(branchToDelete, true); err != nil {
			fmt.Printf("  %s branch delete: %v\n", style.Dim.Render("○"), err)
		} else {
			fmt.Printf("  %s deleted local branch %s\n", style.Success.Render("✓"), branchToDelete)
		}
		fmt.Printf("  %s remote branch preserved for refinery merge\n", style.Dim.Render("○"))
	}

	// Step 5: Reset agent bead for reuse (if exists)
	// Uses ResetAgentBeadForReuse instead of bd close because agent beads are
	// ephemeral (wisps table). Shelling out to `bd close` operates on the issues
	// table and silently fails to affect wisps, leaving stale rows that block
	// re-sling with "duplicate primary key" errors. (gt--irj)
	// ResetAgentBeadForReuse keeps the bead open with agent_state="nuked" so
	// CreateOrReopenAgentBead can simply update it on re-spawn without needing
	// a close/reopen cycle.
	agentBeadID := polecatBeadIDForRig(r, rigName, polecatName)
	bd := beads.New(r.Path)
	if err := bd.ResetAgentBeadForReuse(agentBeadID, "nuked"); err != nil {
		// Bead may not exist (first spawn failed, or test environment)
		fmt.Printf("  %s agent bead not found or already cleaned\n", style.Dim.Render("○"))
	} else {
		fmt.Printf("  %s closed agent bead %s\n", style.Success.Render("✓"), agentBeadID)
	}

	// Step 6: Purge closed ephemeral beads (wisps) accumulated during sessions.
	// Without this, closed wisps from mol-polecat-work steps, mol-witness-patrol
	// cycles, etc. accumulate across sessions and pollute bd ready/list (hq-6161m).
	purgeClosedEphemeralBeads(bd)

	return nil
}

// nukeCleanupMolecules burns any molecule attached to a work bead during polecat nuke.
// This prevents stale attached_molecule references from blocking re-dispatch (gt-npzy).
// Best-effort: failures are logged but don't abort the nuke.
func nukeCleanupMolecules(workBeadID string, r *rig.Rig) {
	// Use mayor/rig as workDir so ResolveBeadsDir finds the Dolt-backed
	// .beads/ directory, not the gitignored rig-root .beads/. Without this,
	// detach/close operations route to the wrong database and the stale
	// molecule attachment persists on the work bead. (gt--1up)
	bd := beads.New(filepath.Join(r.Path, "mayor", "rig"))

	// Fetch the work bead to check for attached molecules
	issue, err := bd.Show(workBeadID)
	if err != nil {
		fmt.Printf("  %s molecule cleanup: could not fetch work bead %s: %v\n",
			style.Dim.Render("○"), workBeadID, err)
		return
	}

	attachment := beads.ParseAttachmentFields(issue)
	if attachment == nil || attachment.AttachedMolecule == "" {
		return // No molecule attached — nothing to clean up
	}

	moleculeID := attachment.AttachedMolecule

	// Force-close descendant steps before detaching (prevents orphaned step beads).
	// Uses force variant since nuke is destructive — must succeed even for beads in
	// invalid states. Best-effort — log but proceed in nuke path.
	if _, err := forceCloseDescendants(bd, moleculeID); err != nil {
		style.PrintWarning("nuke: could not close descendants of %s: %v", moleculeID, err)
	}

	// Detach the molecule with audit trail
	if _, detachErr := bd.DetachMoleculeWithAudit(workBeadID, beads.DetachOptions{
		Operation: "burn",
		Reason:    "polecat nuked: cleaning stale molecule",
	}); detachErr != nil {
		fmt.Printf("  %s molecule detach failed for %s: %v\n",
			style.Warning.Render("⚠"), moleculeID, detachErr)
		return
	}

	// Remove dependency bond so collectExistingMolecules won't find the
	// closed molecule and block re-dispatch. Without this, the bond persists
	// and every sling attempt fails with "bead has existing molecule(s)".
	if err := bd.RemoveDependency(workBeadID, moleculeID); err != nil {
		fmt.Printf("  %s molecule bond removal failed for %s → %s: %v\n",
			style.Warning.Render("⚠"), workBeadID, moleculeID, err)
		// Non-fatal: detach already cleared the description pointer.
	}

	// Force-close the orphaned wisp root so it doesn't linger
	if closeErr := bd.ForceCloseWithReason("burned: polecat nuked", moleculeID); closeErr != nil {
		fmt.Printf("  %s molecule root close failed for %s: %v\n",
			style.Warning.Render("⚠"), moleculeID, closeErr)
	} else {
		fmt.Printf("  %s burned stale molecule %s from work bead %s\n",
			style.Success.Render("✓"), moleculeID, workBeadID)
	}
}

// cleanupOrphanedProcesses kills Claude processes that survived session termination.
// Uses aggressive zombie detection via tmux session verification.
func cleanupOrphanedProcesses() {
	results, err := util.CleanupZombieClaudeProcesses()
	if err != nil {
		// Non-fatal: log and continue
		fmt.Printf("  %s orphan cleanup check failed: %v\n", style.Dim.Render("○"), err)
		return
	}

	if len(results) == 0 {
		return
	}

	// Report what was cleaned up
	var killed, escalated int
	for _, r := range results {
		switch r.Signal {
		case "SIGTERM", "SIGKILL":
			killed++
		case "UNKILLABLE":
			escalated++
		}
	}

	if killed > 0 {
		fmt.Printf("  %s cleaned up %d orphaned process(es)\n", style.Success.Render("✓"), killed)
	}
	if escalated > 0 {
		fmt.Printf("  %s %d process(es) survived SIGKILL (unkillable)\n", style.Warning.Render("⚠"), escalated)
	}
}

func runPolecatStale(cmd *cobra.Command, args []string) error {
	rigName := args[0]
	mgr, r, err := getPolecatManager(rigName)
	if err != nil {
		return err
	}

	fmt.Printf("Detecting stale polecats in %s (threshold: %d commits behind main)...\n\n", r.Name, polecatStaleThreshold)

	staleInfos, err := mgr.DetectStalePolecats(polecatStaleThreshold)
	if err != nil {
		return fmt.Errorf("detecting stale polecats: %w", err)
	}

	if len(staleInfos) == 0 {
		fmt.Println("No polecats found.")
		return nil
	}

	// JSON output
	if polecatStaleJSON {
		return json.NewEncoder(os.Stdout).Encode(staleInfos)
	}

	// Summary counts
	var staleCount, safeCount int
	for _, info := range staleInfos {
		if info.IsStale {
			staleCount++
		} else {
			safeCount++
		}
	}

	// Display results
	for _, info := range staleInfos {
		statusIcon := style.Success.Render("●")
		statusText := "active"
		if info.IsStale {
			statusIcon = style.Warning.Render("○")
			statusText = "stale"
		}

		fmt.Printf("%s %s (%s)\n", statusIcon, style.Bold.Render(info.Name), statusText)

		// Session status
		if info.HasActiveSession {
			fmt.Printf("    Session: %s\n", style.Success.Render("running"))
		} else {
			fmt.Printf("    Session: %s\n", style.Dim.Render("stopped"))
		}

		// Commits behind
		if info.CommitsBehind > 0 {
			behindStyle := style.Dim
			if info.CommitsBehind >= polecatStaleThreshold {
				behindStyle = style.Warning
			}
			fmt.Printf("    Behind main: %s\n", behindStyle.Render(fmt.Sprintf("%d commits", info.CommitsBehind)))
		}

		// Agent state
		if info.AgentState != "" {
			fmt.Printf("    Agent state: %s\n", info.AgentState)
		} else {
			fmt.Printf("    Agent state: %s\n", style.Dim.Render("no bead"))
		}

		// Uncommitted work
		if info.HasUncommittedWork {
			fmt.Printf("    Uncommitted: %s\n", style.Error.Render("yes"))
		}

		// Reason
		fmt.Printf("    Reason: %s\n", info.Reason)
		fmt.Println()
	}

	// Summary
	fmt.Printf("Summary: %d stale, %d active\n", staleCount, safeCount)

	// Cleanup if requested
	if polecatStaleCleanup && staleCount > 0 {
		fmt.Println()
		if polecatStaleDryRun {
			fmt.Printf("Would clean up %d stale polecat(s):\n", staleCount)
			for _, info := range staleInfos {
				if info.IsStale {
					fmt.Printf("  - %s: %s\n", info.Name, info.Reason)
				}
			}
		} else {
			fmt.Printf("Cleaning up %d stale polecat(s)...\n", staleCount)
			nuked := 0
			for _, info := range staleInfos {
				if !info.IsStale {
					continue
				}
				fmt.Printf("Nuking %s...\n", info.Name)
				if err := nukePolecatFull(info.Name, rigName, mgr, r); err != nil {
					fmt.Printf("  %s (%v)\n", style.Error.Render("failed"), err)
				} else {
					nuked++
				}
			}
			fmt.Printf("\n%s Nuked %d stale polecat(s).\n", style.SuccessPrefix, nuked)

			// Clean up any orphaned processes that survived session termination
			cleanupOrphanedProcesses()
		}
	}

	return nil
}

func runPolecatPrune(cmd *cobra.Command, args []string) error {
	rigName := args[0]

	_, r, err := getPolecatManager(rigName)
	if err != nil {
		return err
	}

	// Use the mayor/rig clone (or bare repo) for branch operations
	var repoGit *git.Git
	bareRepoPath := filepath.Join(r.Path, ".repo.git")
	if info, statErr := os.Stat(bareRepoPath); statErr == nil && info.IsDir() {
		repoGit = git.NewGitWithDir(bareRepoPath, "")
	} else {
		repoGit = git.NewGit(filepath.Join(r.Path, "mayor", "rig"))
	}

	fmt.Printf("Pruning stale polecat branches in %s...\n", r.Name)

	// First, prune stale remote-tracking refs so we detect deleted remote branches
	if err := repoGit.FetchPrune("origin"); err != nil {
		fmt.Printf("  %s fetch --prune: %v (continuing anyway)\n", style.Warning.Render("⚠"), err)
	}

	// Prune local branches that are merged or have no remote
	pruned, err := repoGit.PruneStaleBranches("polecat/*", polecatPruneDryRun)
	if err != nil {
		return fmt.Errorf("pruning local branches: %w", err)
	}

	if len(pruned) == 0 {
		fmt.Println("No stale local polecat branches found.")
	} else {
		verb := "Pruned"
		if polecatPruneDryRun {
			verb = "Would prune"
		}
		for _, b := range pruned {
			fmt.Printf("  %s %s (%s)\n", style.Success.Render("✓"), b.Name, b.Reason)
		}
		fmt.Printf("\n%s %d local branch(es).\n", verb, len(pruned))
	}

	// Optionally prune remote polecat branches
	if polecatPruneRemote {
		fmt.Println()
		fmt.Println("Pruning remote polecat branches...")

		defaultBranch := repoGit.RemoteDefaultBranch()
		remoteRefs, lsErr := repoGit.ListPushRemoteRefs("origin", "refs/heads/polecat/")
		if lsErr != nil {
			return fmt.Errorf("listing remote refs: %w", lsErr)
		}

		remotePruned := 0
		for _, ref := range remoteRefs {
			branch := strings.TrimPrefix(ref, "refs/heads/")
			// Check if merged to main
			merged, mergeErr := repoGit.IsAncestor(branch, "origin/"+defaultBranch)
			if mergeErr != nil {
				continue
			}
			if !merged {
				continue
			}

			if polecatPruneDryRun {
				fmt.Printf("  Would delete remote: %s\n", style.Dim.Render(branch))
			} else {
				if delErr := repoGit.DeleteRemoteBranch("origin", branch); delErr != nil {
					fmt.Printf("  %s remote %s: %v\n", style.Warning.Render("⚠"), branch, delErr)
				} else {
					fmt.Printf("  %s deleted remote %s\n", style.Success.Render("✓"), branch)
				}
			}
			remotePruned++
		}

		if remotePruned == 0 {
			fmt.Println("No stale remote polecat branches found.")
		} else {
			verb := "Pruned"
			if polecatPruneDryRun {
				verb = "Would prune"
			}
			fmt.Printf("\n%s %d remote branch(es).\n", verb, remotePruned)
		}
	}

	return nil
}

// runPolecatPoolInit creates a persistent polecat pool for a rig.
// Creates N polecats with identities and worktrees in IDLE state.
// Existing polecats are preserved — only new ones are created.
func runPolecatPoolInit(cmd *cobra.Command, args []string) error {
	rigName := args[0]

	mgr, r, err := getPolecatManager(rigName)
	if err != nil {
		return err
	}

	// Determine pool size: flag > rig config > default
	poolSize := 4 // default
	rigCfg, cfgErr := rig.LoadRigConfig(r.Path)
	if cfgErr == nil && rigCfg.PolecatPoolSize > 0 {
		poolSize = rigCfg.PolecatPoolSize
	}
	if polecatPoolInitSize > 0 {
		poolSize = polecatPoolInitSize
	}

	// Determine names: rig config > name pool theme
	var fixedNames []string
	if cfgErr == nil && len(rigCfg.PolecatNames) > 0 {
		fixedNames = rigCfg.PolecatNames
	}

	// List existing polecats to avoid recreating them
	existing, err := mgr.List()
	if err != nil {
		return fmt.Errorf("listing existing polecats: %w", err)
	}
	existingNames := make(map[string]bool)
	for _, p := range existing {
		existingNames[p.Name] = true
	}

	fmt.Printf("Initializing persistent polecat pool for %s (target size: %d)\n", rigName, poolSize)
	if len(existing) > 0 {
		fmt.Printf("  Existing polecats: %d\n", len(existing))
	}

	// Build the list of names to create
	var namesToCreate []string
	if len(fixedNames) > 0 {
		// Use configured names, skip ones that already exist
		for _, name := range fixedNames {
			if len(namesToCreate)+len(existingNames) >= poolSize {
				break
			}
			if !existingNames[name] {
				namesToCreate = append(namesToCreate, name)
			}
		}
	} else {
		// Use name pool allocation for new names
		namePool := mgr.GetNamePool()
		namePool.Reconcile(existingNamesList(existing))
		for len(namesToCreate)+len(existingNames) < poolSize {
			name, allocErr := namePool.Allocate()
			if allocErr != nil {
				return fmt.Errorf("allocating polecat name: %w", allocErr)
			}
			if !existingNames[name] {
				namesToCreate = append(namesToCreate, name)
			}
		}
	}

	if len(namesToCreate) == 0 {
		fmt.Printf("\n%s Pool already at target size (%d polecats).\n", style.Bold.Render("✓"), len(existing))
		return nil
	}

	if polecatPoolInitDryRun {
		fmt.Printf("\nWould create %d polecat(s):\n", len(namesToCreate))
		for _, name := range namesToCreate {
			fmt.Printf("  %s %s\n", style.Dim.Render("→"), name)
		}
		return nil
	}

	// Create each polecat
	fmt.Printf("\nCreating %d polecat(s)...\n", len(namesToCreate))
	created := 0
	for _, name := range namesToCreate {
		fmt.Printf("  %s Creating %s...", style.Dim.Render("→"), name)
		p, addErr := mgr.Add(name)
		if addErr != nil {
			fmt.Printf(" %s %v\n", style.Warning.Render("FAILED"), addErr)
			continue
		}
		// Set agent state to idle (polecat was created without work)
		if stateErr := mgr.SetAgentState(name, "idle"); stateErr != nil {
			fmt.Printf(" %s (created but couldn't set idle state: %v)\n", style.Warning.Render("⚠"), stateErr)
		} else {
			fmt.Printf(" %s (%s)\n", style.Success.Render("✓"), style.Dim.Render(p.ClonePath))
		}
		created++
	}

	fmt.Printf("\n%s Pool initialized: %d created, %d total (target: %d)\n",
		style.Bold.Render("✓"), created, created+len(existing), poolSize)

	return nil
}

// existingNamesList extracts polecat names from a slice of Polecat pointers.
func existingNamesList(polecats []*polecat.Polecat) []string {
	names := make([]string, len(polecats))
	for i, p := range polecats {
		names[i] = p.Name
	}
	return names
}
package cmd

import (
	"fmt"
	"strings"

	"github.com/steveyegge/gastown/internal/constants"
	"github.com/steveyegge/gastown/internal/session"
)

// cyclePolecatSession switches to the next or previous polecat session in the same rig.
// direction: 1 for next, -1 for previous
// sessionOverride: if non-empty, use this instead of detecting current session
func cyclePolecatSession(direction int, sessionOverride string) error {
	currentSession, err := resolveCurrentSession(sessionOverride)
	if err != nil {
		return fmt.Errorf("not in a tmux session: %w", err)
	}
	if currentSession == "" {
		return fmt.Errorf("not in a tmux session")
	}

	rigName, _, ok := parsePolecatSessionName(currentSession)
	if !ok {
		return nil
	}

	sessions, err := findRigPolecatSessions(rigName)
	if err != nil {
		return fmt.Errorf("listing sessions: %w", err)
	}

	return cycleInGroup(direction, currentSession, sessions)
}

// parsePolecatSessionName extracts rig and polecat name from a tmux session name.
// Format: gt-<rig>-<name> where name is NOT crew-*, witness, refinery, mayor, or deacon.
// Returns empty strings and false if the format doesn't match.
//
// Delegates to session.ParseSessionName for consistent parsing of hyphenated
// rig names (e.g., gt-my-rig-Toast correctly yields rig="my-rig", name="Toast").
func parsePolecatSessionName(sessionName string) (rigName, polecatName string, ok bool) { //nolint:unparam // polecatName kept for API consistency
	identity, err := session.ParseSessionName(sessionName)
	if err != nil {
		return "", "", false
	}
	if identity.Role != session.RolePolecat {
		return "", "", false
	}
	if identity.Rig == "" || identity.Name == "" {
		return "", "", false
	}
	// Exclude names that are reserved for other session types.
	// Mayor/deacon use hq- prefix in practice, but gt-<rig>-mayor/deacon
	// patterns should still be excluded defensively.
	switch identity.Name {
	case constants.RoleMayor, constants.RoleDeacon:
		return "", "", false
	}
	return identity.Rig, identity.Name, true
}

// findRigPolecatSessions returns all polecat sessions for a given rig.
// Finds sessions matching gt-<rig>-<name> pattern, excluding crew, witness,
// and refinery sessions.
func findRigPolecatSessions(rigName string) ([]string, error) { //nolint:unparam // error return kept for future use
	allSessions, err := listTmuxSessions()
	if err != nil {
		return nil, nil
	}

	prefix := session.PrefixFor(rigName) + "-"
	var sessions []string

	for _, s := range allSessions {
		if !strings.HasPrefix(s, prefix) {
			continue
		}
		if _, _, ok := parsePolecatSessionName(s); ok {
			sessions = append(sessions, s)
		}
	}

	return sessions, nil
}
// Package cmd provides polecat spawning utilities for gt sling.
package cmd

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/steveyegge/gastown/internal/beads"
	"github.com/steveyegge/gastown/internal/config"
	"github.com/steveyegge/gastown/internal/constants"
	"github.com/steveyegge/gastown/internal/events"
	"github.com/steveyegge/gastown/internal/git"
	"github.com/steveyegge/gastown/internal/polecat"
	"github.com/steveyegge/gastown/internal/rig"
	"github.com/steveyegge/gastown/internal/style"
	"github.com/steveyegge/gastown/internal/tmux"
	"github.com/steveyegge/gastown/internal/witness"
	"github.com/steveyegge/gastown/internal/workspace"
)

// SpawnedPolecatInfo contains info about a spawned polecat session.
type SpawnedPolecatInfo struct {
	RigName     string // Rig name (e.g., "gastown")
	PolecatName string // Polecat name (e.g., "Toast")
	ClonePath   string // Path to polecat's git worktree
	SessionName string // Tmux session name (e.g., "gt-gastown-p-Toast")
	Pane        string // Tmux pane ID (empty until StartSession is called)
	BaseBranch  string // Effective base branch (e.g., "main", "integration/epic-id")
	Branch      string // Git branch name (for cleanup on rollback)

	// Internal fields for deferred session start
	account string
	agent   string
}

// AgentID returns the agent identifier (e.g., "gastown/polecats/Toast")
func (s *SpawnedPolecatInfo) AgentID() string {
	return fmt.Sprintf("%s/polecats/%s", s.RigName, s.PolecatName)
}

// SessionStarted returns true if the tmux session has been started.
func (s *SpawnedPolecatInfo) SessionStarted() bool {
	return s.Pane != ""
}

// SlingSpawnOptions contains options for spawning a polecat via sling.
type SlingSpawnOptions struct {
	Force      bool   // Force spawn even if polecat has uncommitted work
	Account    string // Claude Code account handle to use
	Create     bool   // Create polecat if it doesn't exist (currently always true for sling)
	HookBead   string // Bead ID to set as hook_bead at spawn time (atomic assignment)
	Agent      string // Agent override for this spawn (e.g., "gemini", "codex", "claude-haiku")
	BaseBranch string // Override base branch for polecat worktree (e.g., "develop", "release/v2")
}

// SpawnPolecatForSling creates a fresh polecat and optionally starts its session.
// This is used by gt sling when the target is a rig name.
// The caller (sling) handles hook attachment and nudging.
func SpawnPolecatForSling(rigName string, opts SlingSpawnOptions) (*SpawnedPolecatInfo, error) {
	// Find workspace
	townRoot, err := workspace.FindFromCwdOrError()
	if err != nil {
		return nil, fmt.Errorf("not in a Gas Town workspace: %w", err)
	}

	// Load rig config
	rigsConfigPath := filepath.Join(townRoot, "mayor", "rigs.json")
	rigsConfig, err := config.LoadRigsConfig(rigsConfigPath)
	if err != nil {
		rigsConfig = &config.RigsConfig{Rigs: make(map[string]config.RigEntry)}
	}

	g := git.NewGit(townRoot)
	rigMgr := rig.NewManager(townRoot, rigsConfig, g)
	r, err := rigMgr.GetRig(rigName)
	if err != nil {
		return nil, fmt.Errorf("rig '%s' not found", rigName)
	}

	// Get polecat manager (with tmux for session-aware allocation)
	polecatGit := git.NewGit(r.Path)
	t := tmux.NewTmux()
	polecatMgr := polecat.NewManager(r, polecatGit, t)

	// Pre-spawn Dolt health check (gt-94llt7): verify Dolt is reachable before
	// allocating a polecat. Prevents orphaned polecats when Dolt is down.
	if err := polecatMgr.CheckDoltHealth(); err != nil {
		return nil, fmt.Errorf("pre-spawn health check failed: %w", err)
	}

	// Pre-spawn admission control (gt-1obzke): verify Dolt server has connection
	// capacity before spawning. Prevents connection storms during mass sling.
	if err := polecatMgr.CheckDoltServerCapacity(); err != nil {
		return nil, fmt.Errorf("admission control: %w", err)
	}

	// Polecat count cap (clown show #22): refuse to spawn if there are already
	// too many working polecats. This is a last-resort safety net for the direct-dispatch
	// path. For configurable capacity gating, use scheduler.max_polecats in town settings
	// (see internal/scheduler/capacity/).
	// Uses countWorkingPolecats to exclude idle polecats (completed work, no hook bead)
	// that are available for re-sling under the persistent polecat model.
	const defaultMaxActivePolecats = 25
	workingCount := countWorkingPolecats()
	if workingCount >= defaultMaxActivePolecats {
		return nil, fmt.Errorf("polecat cap reached: %d working polecats (max %d). "+
			"This is a safety limit to prevent spawn storms. "+
			"Investigate why polecats are accumulating before spawning more",
			workingCount, defaultMaxActivePolecats)
	}

	// Per-bead respawn circuit breaker (clown show #22):
	// Track how many times this bead has been slung. Block after N attempts
	// to prevent witness→deacon→sling feedback loops.
	if opts.HookBead != "" && !opts.Force {
		if witness.ShouldBlockRespawn(townRoot, opts.HookBead) {
			maxRespawns := config.LoadOperationalConfig(townRoot).GetWitnessConfig().MaxBeadRespawnsV()
			return nil, fmt.Errorf("respawn limit reached for %s (%d attempts). "+
				"This bead keeps failing — investigate before re-dispatching.\n"+
				"Override: gt sling %s %s --force\n"+
				"Reset:    gt sling respawn-reset %s",
				opts.HookBead, maxRespawns,
				opts.HookBead, rigName, opts.HookBead)
		}
		witness.RecordBeadRespawn(townRoot, opts.HookBead)
	}

	// Per-rig directory cap: prevent unbounded worktree accumulation even when
	// polecats die quickly (tmux session count stays low).
	const maxPolecatDirsPerRig = 30
	rigPolecatDir := filepath.Join(townRoot, rigName, "polecats")
	if entries, err := os.ReadDir(rigPolecatDir); err == nil {
		dirCount := 0
		for _, e := range entries {
			if e.IsDir() && !strings.HasPrefix(e.Name(), ".") {
				dirCount++
			}
		}
		if dirCount >= maxPolecatDirsPerRig {
			return nil, fmt.Errorf("rig %s has %d polecat directories (max %d). "+
				"Nuke idle polecats first: gt polecat nuke %s/<name> --force",
				rigName, dirCount, maxPolecatDirsPerRig, rigName)
		}
	}

	// Persistent polecat model (gt-4ac): try to reuse an idle polecat first.
	// Idle polecats have completed their work but kept their sandbox (worktree).
	// Reusing avoids the overhead of creating a new worktree.
	idlePolecat, findErr := polecatMgr.FindIdlePolecat()
	if findErr == nil && idlePolecat != nil {
		polecatName := idlePolecat.Name
		fmt.Printf("Reusing idle polecat: %s\n", polecatName)

		// Determine base branch
		baseBranch := opts.BaseBranch
		if baseBranch == "" && opts.HookBead != "" {
			settingsPath := filepath.Join(r.Path, "settings", "config.json")
			polecatIntegrationEnabled := true
			if settings, err := config.LoadRigSettings(settingsPath); err == nil && settings.MergeQueue != nil {
				polecatIntegrationEnabled = settings.MergeQueue.IsPolecatIntegrationEnabled()
			}
			if polecatIntegrationEnabled {
				repoGit, repoErr := getRigGit(r.Path)
				if repoErr == nil {
					bd := beads.New(r.Path)
					detected, detectErr := beads.DetectIntegrationBranch(bd, repoGit, opts.HookBead)
					if detectErr == nil && detected != "" {
						baseBranch = "origin/" + detected
						fmt.Printf("  Auto-detected integration branch: %s\n", detected)
					}
				}
			}
		}
		if baseBranch != "" && !strings.HasPrefix(baseBranch, "origin/") {
			baseBranch = "origin/" + baseBranch
		}

		// Reuse the idle polecat with branch-only operations (no worktree add/remove).
		// Phase 3 of persistent-polecat-pool: eliminates ~5s worktree creation overhead.
		// Falls back to full worktree repair if branch-only reuse fails.
		addOpts := polecat.AddOptions{
			HookBead:   opts.HookBead,
			BaseBranch: baseBranch,
		}
		reuseOK := false
		if _, err := polecatMgr.ReuseIdlePolecat(polecatName, addOpts); err != nil {
			// Branch-only reuse failed — try full worktree repair as fallback
			fmt.Printf("  Branch-only reuse failed for idle polecat %s: %v, trying full repair...\n", polecatName, err)
			if _, err := polecatMgr.RepairWorktreeWithOptions(polecatName, true, addOpts); err != nil {
				fmt.Printf("  Full repair also failed for %s: %v, allocating new...\n", polecatName, err)
			} else {
				reuseOK = true
			}
		} else {
			reuseOK = true
		}

		if reuseOK {
			polecatObj, err := polecatMgr.Get(polecatName)
			if err != nil {
				return nil, fmt.Errorf("getting idle polecat after reuse: %w", err)
			}
			if err := verifyWorktreeExists(polecatObj.ClonePath); err != nil {
				return nil, fmt.Errorf("worktree verification failed for reused %s: %w", polecatName, err)
			}

			polecatSessMgr := polecat.NewSessionManager(t, r)
			sessionName := polecatSessMgr.SessionName(polecatName)

			fmt.Printf("%s Polecat %s reused (idle → working, session start deferred)\n", style.Bold.Render("✓"), polecatName)
			_ = events.LogFeed(events.TypeSpawn, "gt", events.SpawnPayload(rigName, polecatName))

			effectiveBranch := strings.TrimPrefix(baseBranch, "origin/")
			if effectiveBranch == "" {
				effectiveBranch = r.DefaultBranch()
			}

			return &SpawnedPolecatInfo{
				RigName:     rigName,
				PolecatName: polecatName,
				ClonePath:   polecatObj.ClonePath,
				SessionName: sessionName,
				Pane:        "",
				BaseBranch:  effectiveBranch,
				Branch:      polecatObj.Branch,
				account:     opts.Account,
				agent:       opts.Agent,
			}, nil
		}
	}

	// Determine base branch for polecat worktree
	baseBranch := opts.BaseBranch
	if baseBranch == "" && opts.HookBead != "" {
		// Auto-detect: check if the hooked bead's parent epic has an integration branch
		settingsPath := filepath.Join(r.Path, "settings", "config.json")
		polecatIntegrationEnabled := true
		if settings, err := config.LoadRigSettings(settingsPath); err == nil && settings.MergeQueue != nil {
			polecatIntegrationEnabled = settings.MergeQueue.IsPolecatIntegrationEnabled()
		}
		if polecatIntegrationEnabled {
			repoGit, repoErr := getRigGit(r.Path)
			if repoErr == nil {
				bd := beads.New(r.Path)
				detected, detectErr := beads.DetectIntegrationBranch(bd, repoGit, opts.HookBead)
				if detectErr == nil && detected != "" {
					baseBranch = "origin/" + detected
					fmt.Printf("  Auto-detected integration branch: %s\n", detected)
				}
			}
		}
	}
	if baseBranch != "" && !strings.HasPrefix(baseBranch, "origin/") {
		baseBranch = "origin/" + baseBranch
	}

	// Build add options with hook_bead set atomically at spawn time
	addOpts := polecat.AddOptions{
		HookBead:   opts.HookBead,
		BaseBranch: baseBranch,
	}

	// No idle polecat available — allocate and create atomically (GH#2215).
	// AllocateAndAdd holds the pool lock through directory creation, preventing
	// concurrent processes from allocating the same name.
	polecatName, _, err := polecatMgr.AllocateAndAdd(addOpts)
	if err != nil {
		return nil, fmt.Errorf("allocating and creating polecat: %w", err)
	}
	fmt.Printf("Created polecat: %s\n", polecatName)

	// Get polecat object for path info
	polecatObj, err := polecatMgr.Get(polecatName)
	if err != nil {
		return nil, fmt.Errorf("getting polecat after creation: %w", err)
	}

	// Verify worktree was actually created (fixes #1070)
	// The identity bead may exist but worktree creation can fail silently
	if err := verifyWorktreeExists(polecatObj.ClonePath); err != nil {
		// Clean up the partial state before returning error
		_ = polecatMgr.Remove(polecatName, true) // force=true to clean up partial state
		return nil, fmt.Errorf("worktree verification failed for %s: %w\nHint: try 'gt polecat nuke %s/%s --force' to clean up",
			polecatName, err, rigName, polecatName)
	}

	// Get session manager for session name (session start is deferred)
	polecatSessMgr := polecat.NewSessionManager(t, r)
	sessionName := polecatSessMgr.SessionName(polecatName)

	fmt.Printf("%s Polecat %s spawned (session start deferred)\n", style.Bold.Render("✓"), polecatName)

	// Log spawn event to activity feed
	_ = events.LogFeed(events.TypeSpawn, "gt", events.SpawnPayload(rigName, polecatName))

	// Compute effective base branch (strip origin/ prefix since formula prepends it)
	effectiveBranch := strings.TrimPrefix(baseBranch, "origin/")
	if effectiveBranch == "" {
		effectiveBranch = r.DefaultBranch()
	}

	return &SpawnedPolecatInfo{
		RigName:     rigName,
		PolecatName: polecatName,
		ClonePath:   polecatObj.ClonePath,
		SessionName: sessionName,
		Pane:        "", // Empty until StartSession is called
		BaseBranch:  effectiveBranch,
		Branch:      polecatObj.Branch,
		account:     opts.Account,
		agent:       opts.Agent,
	}, nil
}

// StartSession starts the tmux session for a spawned polecat.
// This is called after the molecule/bead is attached, so the polecat
// sees its work when gt prime runs on session start.
// Returns the pane ID after session start.
func (s *SpawnedPolecatInfo) StartSession() (string, error) {
	if s.SessionStarted() {
		return s.Pane, nil
	}

	townRoot, err := workspace.FindFromCwdOrError()
	if err != nil {
		return "", fmt.Errorf("not in a Gas Town workspace: %w", err)
	}

	// Load rig config
	rigsConfigPath := filepath.Join(townRoot, "mayor", "rigs.json")
	rigsConfig, err := config.LoadRigsConfig(rigsConfigPath)
	if err != nil {
		rigsConfig = &config.RigsConfig{Rigs: make(map[string]config.RigEntry)}
	}

	g := git.NewGit(townRoot)
	rigMgr := rig.NewManager(townRoot, rigsConfig, g)
	r, err := rigMgr.GetRig(s.RigName)
	if err != nil {
		return "", fmt.Errorf("rig '%s' not found", s.RigName)
	}

	// Resolve account
	accountsPath := constants.MayorAccountsPath(townRoot)
	claudeConfigDir, _, err := config.ResolveAccountConfigDir(accountsPath, s.account)
	if err != nil {
		return "", fmt.Errorf("resolving account: %w", err)
	}

	// Start session
	t := tmux.NewTmux()
	polecatSessMgr := polecat.NewSessionManager(t, r)

	fmt.Printf("Starting session for %s/%s...\n", s.RigName, s.PolecatName)
	startOpts := polecat.SessionStartOptions{
		RuntimeConfigDir: claudeConfigDir,
		Agent:            s.agent,
	}
	if err := polecatSessMgr.Start(s.PolecatName, startOpts); err != nil {
		return "", fmt.Errorf("starting session: %w", err)
	}

	// Wait for runtime to be fully ready before returning.
	// When an agent override is specified (e.g., --agent codex), resolve the runtime
	// config from the override so WaitForRuntimeReady uses the correct readiness
	// strategy (delay-based for Codex vs prompt-polling for Claude). Without this,
	// ResolveRoleAgentConfig returns the default agent (Claude) and polls for "❯ "
	// in a Codex session, always timing out after 30 seconds (gt-1j3m).
	spawnTownRoot := filepath.Dir(r.Path)
	var runtimeConfig *config.RuntimeConfig
	if s.agent != "" {
		rc, _, err := config.ResolveAgentConfigWithOverride(spawnTownRoot, r.Path, s.agent)
		if err != nil {
			style.PrintWarning("resolving agent config for %s: %v (using default)", s.agent, err)
			runtimeConfig = config.ResolveRoleAgentConfig("polecat", spawnTownRoot, r.Path)
		} else {
			runtimeConfig = rc
		}
	} else {
		runtimeConfig = config.ResolveRoleAgentConfig("polecat", spawnTownRoot, r.Path)
	}
	if err := t.WaitForRuntimeReady(s.SessionName, runtimeConfig, 30*time.Second); err != nil {
		style.PrintWarning("runtime may not be fully ready: %v", err)
	}

	// Update agent state with retry logic (gt-94llt7: fail-safe Dolt writes).
	// Note: warn-only, not fail-hard. The tmux session is already started above,
	// so returning an error here would leave an orphaned session with no cleanup path.
	// The polecat can still function without the agent state update — it only affects
	// monitoring visibility, not correctness. Compare with createAgentBeadWithRetry
	// which fails hard because a polecat without an agent bead is untrackable.
	polecatGit := git.NewGit(r.Path)
	polecatMgr := polecat.NewManager(r, polecatGit, t)
	if err := polecatMgr.SetAgentStateWithRetry(s.PolecatName, "working"); err != nil {
		style.PrintWarning("could not update agent state after retries: %v", err)
	}

	// Update issue status from hooked to in_progress.
	// Also warn-only for the same reason: session is already running.
	if err := polecatMgr.SetState(s.PolecatName, polecat.StateWorking); err != nil {
		style.PrintWarning("could not update issue status to in_progress: %v", err)
	}

	// Get pane — if this fails, the session may have died during startup.
	// Kill the dead session to prevent "session already running" on next attempt (gt-jn40ft).
	pane, err := getSessionPane(s.SessionName)
	if err != nil {
		// Session likely died — clean up the tmux session so it doesn't block re-sling
		_ = t.KillSession(s.SessionName)
		return "", fmt.Errorf("getting pane for %s (session likely died during startup): %w", s.SessionName, err)
	}

	s.Pane = pane
	return pane, nil
}

// IsRigName checks if a target string is a rig name (not a role or path).
// Returns the rig name and true if it's a valid rig.
func IsRigName(target string) (string, bool) {
	// If it contains a slash, it's a path format (rig/role or rig/crew/name)
	if strings.Contains(target, "/") {
		return "", false
	}

	// Check known non-rig role names
	switch strings.ToLower(target) {
	case constants.RoleMayor, "may", constants.RoleDeacon, "dea", constants.RoleCrew, constants.RoleWitness, "wit", constants.RoleRefinery, "ref":
		return "", false
	}

	// Try to load as a rig
	townRoot, err := workspace.FindFromCwdOrError()
	if err != nil {
		return "", false
	}

	rigsConfigPath := filepath.Join(townRoot, "mayor", "rigs.json")
	rigsConfig, err := config.LoadRigsConfig(rigsConfigPath)
	if err != nil {
		return "", false
	}

	g := git.NewGit(townRoot)
	rigMgr := rig.NewManager(townRoot, rigsConfig, g)
	_, err = rigMgr.GetRig(target)
	if err != nil {
		return "", false
	}

	return target, true
}

// verifyWorktreeExists checks that a git worktree was actually created at the given path
// and that it is a functional git repository. Returns an error if the worktree is missing,
// has a broken .git reference, or fails basic git validation. (GH#2056)
func verifyWorktreeExists(clonePath string) error {
	// Check if directory exists
	info, err := os.Stat(clonePath)
	if err != nil {
		if os.IsNotExist(err) {
			return fmt.Errorf("worktree directory does not exist: %s", clonePath)
		}
		return fmt.Errorf("checking worktree directory: %w", err)
	}
	if !info.IsDir() {
		return fmt.Errorf("worktree path is not a directory: %s", clonePath)
	}

	// Check for .git file (worktrees have a .git file, not a .git directory)
	gitPath := filepath.Join(clonePath, ".git")
	if _, err := os.Stat(gitPath); err != nil {
		if os.IsNotExist(err) {
			return fmt.Errorf("worktree missing .git file (not a valid git worktree): %s", clonePath)
		}
		return fmt.Errorf("checking .git: %w", err)
	}

	// For worktree .git files, verify the gitdir reference points to a valid path.
	// A broken reference (e.g., from os.Rename instead of git worktree move) causes
	// "fatal: not a git repository" for every git operation.
	gitContent, err := os.ReadFile(gitPath)
	if err == nil {
		content := strings.TrimSpace(string(gitContent))
		if strings.HasPrefix(content, "gitdir: ") {
			gitdirPath := strings.TrimPrefix(content, "gitdir: ")
			if !filepath.IsAbs(gitdirPath) {
				gitdirPath = filepath.Join(clonePath, gitdirPath)
			}
			if _, err := os.Stat(gitdirPath); err != nil {
				return fmt.Errorf("worktree .git references nonexistent gitdir %s: %w", gitdirPath, err)
			}
		}
	}

	// Final validation: run git rev-parse to confirm the worktree is functional
	cmd := exec.Command("git", "-C", clonePath, "rev-parse", "--git-dir")
	if output, err := cmd.CombinedOutput(); err != nil {
		return fmt.Errorf("worktree at %s is not a valid git repository: %s", clonePath, strings.TrimSpace(string(output)))
	}

	return nil
}
