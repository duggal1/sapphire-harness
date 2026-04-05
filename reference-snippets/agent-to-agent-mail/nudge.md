// Package nudge provides non-destructive nudge delivery for Gas Town agents.
//
// The nudge queue allows messages to be delivered cooperatively: instead of
// sending text directly to a tmux session (which cancels in-flight tool calls),
// nudges are written to a queue directory and picked up by the agent's
// UserPromptSubmit hook at the next natural turn boundary.
//
// Queue location: <townRoot>/.runtime/nudge_queue/<session>/
// Each nudge is a JSON file named by timestamp for FIFO ordering.
package nudge

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/steveyegge/gastown/internal/config"
	"github.com/steveyegge/gastown/internal/constants"
)

// Priority levels for nudge delivery.
const (
	// PriorityNormal is the default — delivered at next turn boundary.
	PriorityNormal = "normal"
	// PriorityUrgent means the agent should handle this promptly.
	PriorityUrgent = "urgent"
)

// Operational limits and defaults.
// These are compiled-in fallbacks. Configurable via operational.nudge
// in settings/config.json (ZFC pattern).
const (
	// DefaultNormalTTL is the time-to-live for normal-priority nudges.
	DefaultNormalTTL = 30 * time.Minute

	// DefaultUrgentTTL is the time-to-live for urgent-priority nudges.
	DefaultUrgentTTL = 2 * time.Hour

	// MaxQueueDepth is the maximum number of pending nudges per session.
	MaxQueueDepth = 50

	// staleClaimThreshold is how long a .claimed file must be untouched
	// before Drain considers it orphaned (from a crashed drainer) and removes it.
	staleClaimThreshold = 5 * time.Minute
)

// nudgeConfig loads nudge-specific thresholds from town settings.
func nudgeConfig(townRoot string) *config.NudgeThresholds {
	return config.LoadOperationalConfig(townRoot).GetNudgeConfig()
}

// QueuedNudge represents a nudge message stored in the queue.
type QueuedNudge struct {
	Sender    string    `json:"sender"`
	Message   string    `json:"message"`
	Priority  string    `json:"priority"`
	Kind      string    `json:"kind,omitempty"`
	ThreadID  string    `json:"thread_id,omitempty"`
	Severity  string    `json:"severity,omitempty"`
	Timestamp time.Time `json:"timestamp"`
	ExpiresAt time.Time `json:"expires_at,omitempty"`
	// DeliverAfter, if non-zero, defers delivery until this time has passed.
	// Drain skips (but does not discard) the nudge until the deadline is met.
	DeliverAfter time.Time `json:"deliver_after,omitempty"`
}

// queueDir returns the nudge queue directory for a given session.
// Path: <townRoot>/.runtime/nudge_queue/<session>/
func queueDir(townRoot, session string) string {
	// Sanitize session name for filesystem safety
	safe := strings.ReplaceAll(session, "/", "_")
	return filepath.Join(townRoot, constants.DirRuntime, "nudge_queue", safe)
}

// randomSuffix returns a short random hex string to disambiguate filenames
// when multiple processes enqueue within the same nanosecond.
func randomSuffix() string {
	var b [4]byte
	_, _ = rand.Read(b[:])
	return hex.EncodeToString(b[:])
}

// Enqueue writes a nudge to the queue for the given session.
// The nudge will be picked up by the agent's hook at the next turn boundary.
// Returns an error if the queue is full (MaxQueueDepth reached).
func Enqueue(townRoot, session string, nudge QueuedNudge) error {
	dir := queueDir(townRoot, session)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return fmt.Errorf("creating nudge queue dir: %w", err)
	}

	// Check queue depth before writing to prevent runaway senders.
	maxDepth := nudgeConfig(townRoot).MaxQueueDepthV()
	pending, _ := Pending(townRoot, session)
	if pending >= maxDepth {
		return fmt.Errorf("nudge queue for %s is full (%d/%d pending)", session, pending, maxDepth)
	}

	if nudge.Timestamp.IsZero() {
		nudge.Timestamp = time.Now()
	}
	if nudge.Priority == "" {
		nudge.Priority = PriorityNormal
	}

	// Set expiry if not already specified by the caller.
	if nudge.ExpiresAt.IsZero() {
		switch nudge.Priority {
		case PriorityUrgent:
			nudge.ExpiresAt = nudge.Timestamp.Add(DefaultUrgentTTL)
		default:
			nudge.ExpiresAt = nudge.Timestamp.Add(DefaultNormalTTL)
		}
	}

	data, err := json.MarshalIndent(nudge, "", "  ")
	if err != nil {
		return fmt.Errorf("marshaling nudge: %w", err)
	}

	// Use nanosecond timestamp + random suffix for unique, ordered filenames.
	// The random suffix prevents collisions when multiple agents enqueue
	// nudges for the same session within the same nanosecond.
	filename := fmt.Sprintf("%d-%s.json", nudge.Timestamp.UnixNano(), randomSuffix())
	path := filepath.Join(dir, filename)

	if err := os.WriteFile(path, data, 0644); err != nil {
		return fmt.Errorf("writing nudge to queue: %w", err)
	}

	return nil
}

// Requeue writes previously drained nudges back to the queue for later delivery.
// Existing timestamps are preserved so FIFO ordering remains stable relative to
// one another; only expired nudges are skipped.
func Requeue(townRoot, session string, nudges []QueuedNudge) error {
	for _, n := range nudges {
		if !n.ExpiresAt.IsZero() && time.Now().After(n.ExpiresAt) {
			continue
		}
		if err := Enqueue(townRoot, session, n); err != nil {
			return err
		}
	}
	return nil
}

// Drain reads and removes all queued nudges for a session, returning them
// in FIFO order. This is called by the hook to pick up pending nudges.
//
// Uses rename-then-process to prevent concurrent Drain calls from delivering
// the same nudge twice: each file is atomically renamed to a .claimed suffix
// before reading, so only one caller can claim each nudge.
//
// Expired nudges (past ExpiresAt) are silently discarded during drain.
// Orphaned .claimed files from crashed drainers are swept if older than 5 minutes.
func Drain(townRoot, session string) ([]QueuedNudge, error) {
	dir := queueDir(townRoot, session)

	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, fmt.Errorf("reading nudge queue: %w", err)
	}

	// Requeue orphaned .claimed files from crashed drainers.
	// A .claimed file older than staleClaimThreshold is certainly orphaned —
	// normal processing completes in milliseconds. We rename it back to .json
	// so it gets picked up on this or a future Drain call, rather than deleting
	// it (which would permanently drop the nudge).
	staleThreshold := nudgeConfig(townRoot).StaleClaimThresholdD()
	now := time.Now()
	for _, entry := range entries {
		if !strings.Contains(entry.Name(), ".claimed") {
			continue
		}
		info, err := entry.Info()
		if err != nil {
			continue
		}
		if now.Sub(info.ModTime()) > staleThreshold {
			orphanPath := filepath.Join(dir, entry.Name())
			// Strip everything from ".claimed" onward to restore original .json filename
			name := entry.Name()
			claimedIdx := strings.Index(name, ".claimed")
			restoredPath := filepath.Join(dir, name[:claimedIdx])
			if err := os.Rename(orphanPath, restoredPath); err != nil {
				// Rename failed — remove as last resort to prevent infinite accumulation
				fmt.Fprintf(os.Stderr, "Warning: failed to requeue orphaned claim %s: %v\n", entry.Name(), err)
				_ = os.Remove(orphanPath)
			}
		}
	}

	// Sort by name (timestamp-based) for FIFO ordering
	sort.Slice(entries, func(i, j int) bool {
		return entries[i].Name() < entries[j].Name()
	})

	var nudges []QueuedNudge
	for _, entry := range entries {
		if entry.IsDir() || !strings.HasSuffix(entry.Name(), ".json") {
			continue
		}

		path := filepath.Join(dir, entry.Name())

		// Atomically claim the file by renaming it. If another Drain call
		// is racing us, only one rename will succeed — the loser gets
		// ENOENT and moves on. This prevents double-delivery.
		//
		// Each drainer uses a unique claim suffix to avoid destination
		// collisions. On Windows, os.Rename to a shared destination is
		// not atomic — two goroutines can both "succeed" via
		// MOVEFILE_REPLACE_EXISTING, causing data loss. Unique suffixes
		// ensure each rename has a distinct target.
		claimPath := path + ".claimed." + randomSuffix()
		if err := os.Rename(path, claimPath); err != nil {
			// Another Drain got it first, or file was already removed
			continue
		}

		data, err := os.ReadFile(claimPath)
		if err != nil {
			if os.IsNotExist(err) {
				// File vanished between rename and read — treat as lost race
				continue
			}
			// Transient read error (e.g., Windows AV/indexer holding a share
			// lock) — unclaim so the nudge can be retried on a future Drain
			// call rather than permanently lost.
			_ = os.Rename(claimPath, path) // best-effort unclaim; orphan sweep catches failures
			continue
		}

		var n QueuedNudge
		if err := json.Unmarshal(data, &n); err != nil {
			// Malformed — clean up
			if rmErr := os.Remove(claimPath); rmErr != nil {
				fmt.Fprintf(os.Stderr, "Warning: failed to remove malformed claim %s: %v\n", entry.Name(), rmErr)
			}
			continue
		}

		// Skip expired nudges — stale messages create noise, not value.
		if !n.ExpiresAt.IsZero() && now.After(n.ExpiresAt) {
			if rmErr := os.Remove(claimPath); rmErr != nil {
				fmt.Fprintf(os.Stderr, "Warning: failed to remove expired nudge %s: %v\n", entry.Name(), rmErr)
			}
			continue
		}

		// Deferred nudge: not ready yet — unclaim and leave in queue.
		if !n.DeliverAfter.IsZero() && now.Before(n.DeliverAfter) {
			if renameErr := os.Rename(claimPath, path); renameErr != nil {
				fmt.Fprintf(os.Stderr, "Warning: failed to unclaim deferred nudge %s: %v\n", entry.Name(), renameErr)
			}
			continue
		}

		nudges = append(nudges, n)

		// Remove the claimed file after successful processing
		if rmErr := os.Remove(claimPath); rmErr != nil {
			fmt.Fprintf(os.Stderr, "Warning: failed to remove processed claim %s: %v\n", entry.Name(), rmErr)
		}
	}

	return nudges, nil
}

// Pending returns the count of queued nudges for a session without draining.
// This is an approximate count — it does not check expiry or read file contents.
func Pending(townRoot, session string) (int, error) {
	dir := queueDir(townRoot, session)

	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return 0, nil
		}
		return 0, fmt.Errorf("reading nudge queue: %w", err)
	}

	count := 0
	for _, entry := range entries {
		if !entry.IsDir() && strings.HasSuffix(entry.Name(), ".json") {
			count++
		}
	}

	return count, nil
}

// QueueLen returns the number of pending nudges for a session without draining.
// Returns 0 on error — callers use this for quick checks. Missing queue
// directories are expected (no nudges yet) and silenced; other filesystem
// errors are logged to stderr so they don't go unnoticed.
func QueueLen(townRoot, session string) int {
	n, err := Pending(townRoot, session)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Warning: nudge queue check failed for %s: %v\n", session, err)
	}
	return n
}

// FormatForInjection formats queued nudges as a system-reminder block
// suitable for Claude Code hook output.
func FormatForInjection(nudges []QueuedNudge) string {
	if len(nudges) == 0 {
		return ""
	}

	var b strings.Builder
	b.WriteString("<system-reminder>\n")

	// Separate urgent from normal
	var urgent, normal []QueuedNudge
	for _, n := range nudges {
		if n.Priority == PriorityUrgent {
			urgent = append(urgent, n)
		} else {
			normal = append(normal, n)
		}
	}

	if len(urgent) > 0 {
		b.WriteString(fmt.Sprintf("QUEUED NUDGE (%d urgent):\n\n", len(urgent)))
		for _, n := range urgent {
			b.WriteString(fmt.Sprintf("  [URGENT from %s] %s\n", n.Sender, n.Message))
		}
		if len(normal) > 0 {
			b.WriteString(fmt.Sprintf("\nPlus %d non-urgent nudge(s):\n", len(normal)))
			for _, n := range normal {
				b.WriteString(fmt.Sprintf("  [from %s] %s\n", n.Sender, n.Message))
			}
		}
		b.WriteString("\nHandle urgent nudges before continuing current work.\n")
	} else {
		b.WriteString(fmt.Sprintf("QUEUED NUDGE (%d message(s)):\n\n", len(normal)))
		for _, n := range normal {
			b.WriteString(fmt.Sprintf("  [from %s] %s\n", n.Sender, n.Message))
		}
		b.WriteString("\nThis is a background notification. Continue current work unless the nudge is higher priority.\n")
	}

	b.WriteString("</system-reminder>\n")
	return b.String()
}
// poller.go provides a background nudge-queue poller for agents that lack
// turn-boundary drain hooks (e.g., Gemini, Codex). Claude Code drains its
// queue via the UserPromptSubmit hook on every turn. Other runtimes have no
// equivalent hook, so queued nudges would sit undelivered forever.
//
// The poller runs as a background goroutine launched by crew/manager.Start().
// It polls the queue every PollInterval, waits for the agent to be idle, then
// drains and injects the formatted nudges via tmux NudgeSession.
//
// Lifecycle: StartPoller() → background loop → StopPoller() (or session death).
// A PID file at <townRoot>/.runtime/nudge_poller/<session>.pid allows Stop()
// to clean up even if the original manager has been replaced.
package nudge

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/fsnotify/fsnotify"
	"github.com/steveyegge/gastown/internal/constants"
	"github.com/steveyegge/gastown/internal/util"
)

// Poller tuning defaults (overridable via flags or tests).
var (
	// DefaultPollInterval is how often the poller checks the queue.
	DefaultPollInterval = "10s"
	// DefaultIdleTimeout is how long to wait for the agent to become idle
	// before skipping this poll cycle and trying again next interval.
	DefaultIdleTimeout = "3s"
)

// pollerPidDir returns the directory for poller PID files.
func pollerPidDir(townRoot string) string {
	return filepath.Join(townRoot, constants.DirRuntime, "nudge_poller")
}

// pollerPidFile returns the PID file path for a session's poller.
func pollerPidFile(townRoot, session string) string {
	safe := strings.ReplaceAll(session, "/", "_")
	return filepath.Join(pollerPidDir(townRoot), safe+".pid")
}

// StartPoller launches a background `gt nudge-poller <session>` process.
// The process is detached (Setpgid) so it survives the caller's exit.
// Returns the PID of the launched process, or an error.
func StartPoller(townRoot, session string) (int, error) {
	pidDir := pollerPidDir(townRoot)
	if err := os.MkdirAll(pidDir, 0755); err != nil {
		return 0, fmt.Errorf("creating poller pid dir: %w", err)
	}

	// Check if a poller is already running for this session.
	if pid, alive := pollerAlive(townRoot, session); alive {
		return pid, nil // already running
	}

	// Find the gt binary.
	gtBin, err := os.Executable()
	if err != nil {
		return 0, fmt.Errorf("finding gt binary: %w", err)
	}

	cmd := buildPollerCommand(gtBin, townRoot, session)

	if err := cmd.Start(); err != nil {
		return 0, fmt.Errorf("starting nudge-poller: %w", err)
	}

	pid := cmd.Process.Pid

	// Write PID file for later cleanup.
	pidPath := pollerPidFile(townRoot, session)
	if err := os.WriteFile(pidPath, []byte(strconv.Itoa(pid)), 0644); err != nil {
		// Non-fatal — the process is running, we just can't track it.
		fmt.Fprintf(os.Stderr, "Warning: failed to write poller PID file: %v\n", err)
	}

	// Release the process so it runs independently.
	_ = cmd.Process.Release()

	return pid, nil
}

func buildPollerCommand(gtBin, townRoot, session string) *exec.Cmd {
	cmd := exec.Command(gtBin, "nudge-poller", session)
	cmd.Dir = townRoot
	cmd.Stdout = nil // discard
	cmd.Stderr = nil // discard
	util.SetDetachedProcessGroup(cmd)
	return cmd
}

// StopPoller terminates the nudge-poller for a session, if running.
func StopPoller(townRoot, session string) error {
	pidPath := pollerPidFile(townRoot, session)

	data, err := os.ReadFile(pidPath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil // no poller to stop
		}
		return fmt.Errorf("reading poller PID file: %w", err)
	}

	pid, err := strconv.Atoi(strings.TrimSpace(string(data)))
	if err != nil {
		_ = os.Remove(pidPath)
		return nil // corrupt PID file, clean up
	}

	if !pollerProcessAlive(pid) {
		// Process already dead.
		_ = os.Remove(pidPath)
		return nil
	}

	proc, err := os.FindProcess(pid)
	if err != nil {
		_ = os.Remove(pidPath)
		return nil
	}

	// Send SIGTERM for graceful shutdown.
	if err := proc.Signal(syscall.SIGTERM); err != nil {
		_ = os.Remove(pidPath)
		return fmt.Errorf("sending SIGTERM to poller (pid %d): %w", pid, err)
	}

	_ = os.Remove(pidPath)
	return nil
}

// pollerAlive checks if a poller is running for the given session.
// Returns the PID and whether the process is alive.
func pollerAlive(townRoot, session string) (int, bool) {
	pidPath := pollerPidFile(townRoot, session)

	data, err := os.ReadFile(pidPath)
	if err != nil {
		return 0, false
	}

	pid, err := strconv.Atoi(strings.TrimSpace(string(data)))
	if err != nil {
		return 0, false
	}

	if !pollerProcessAlive(pid) {
		// Stale PID file — clean up.
		_ = os.Remove(pidPath)
		return 0, false
	}

	return pid, true
}

// Watcher provides a filesystem-event-driven interface to the nudge queue.
// This is an ACP-safe alternative to polling and is preferred for long-running
// watchers like ACP Propeller.
type Watcher struct {
	townRoot string
	session  string
	dir      string
	closed   chan struct{}
	wg       sync.WaitGroup
	events   chan struct{}
}

// NewWatcher creates a new watcher for the given town root and session.
// The watcher observes nudge queue writes and signals via the Events() channel.
func NewWatcher(townRoot, session string) (*Watcher, error) {
	dir := queueDir(townRoot, session)
	// Ensure the directory exists so watch can start immediately.
	if err := os.MkdirAll(dir, 0755); err != nil {
		return nil, fmt.Errorf("creating nudge queue dir: %w", err)
	}

	w := &Watcher{
		townRoot: townRoot,
		session:  session,
		dir:      dir,
		closed:   make(chan struct{}),
		events:   make(chan struct{}, 1), // buffer one signal for coalescing
	}

	w.wg.Add(1)
	go w.watch()
	return w, nil
}

// Events returns a channel that receives a struct{} when the queue may have
// changed. Multiple changes within a short window are coalesced.
func (w *Watcher) Events() <-chan struct{} {
	return w.events
}

// Close stops the watcher and releases resources.
func (w *Watcher) Close() error {
	select {
	case <-w.closed:
		return fmt.Errorf("watcher already closed")
	default:
	}
	close(w.closed)
	w.wg.Wait()
	return nil
}

func (w *Watcher) watch() {
	defer w.wg.Done()

	// Use fsnotify directly.
	watcher, err := fsnotify.NewWatcher()
	if err != nil {
		// Log but don't block; fallback behavior is explicit in callers.
		fmt.Fprintf(os.Stderr, "nudge watcher init failed for %s: %v\n", w.dir, err)
		return
	}
	defer func() { _ = watcher.Close() }()

	// Watch the directory.
	if err := watcher.Add(w.dir); err != nil {
		fmt.Fprintf(os.Stderr, "nudge watcher failed to add dir %s: %v\n", w.dir, err)
		return
	}

	// Coalescing window.
	coalesceTimer := time.NewTicker(100 * time.Millisecond)
	defer coalesceTimer.Stop()

	pending := false
	for {
		select {
		case <-w.closed:
			return
		case event, ok := <-watcher.Events:
			if !ok {
				return
			}
			// Only care about file creation/modification in the queue dir
			if event.Op&(fsnotify.Create|fsnotify.Write) != 0 {
				// Filter: only .json files in the queue directory
				if strings.HasSuffix(event.Name, ".json") && filepath.Dir(event.Name) == w.dir {
					pending = true
				}
			}
		case err, ok := <-watcher.Errors:
			if !ok {
				return
			}
			fmt.Fprintf(os.Stderr, "nudge watcher error: %v\n", err)
		case <-coalesceTimer.C:
			if pending {
				pending = false
				select {
				case w.events <- struct{}{}:
				default:
				}
			}
		}
	}
}

// WatcherForSession returns a Watcher for a specific session or an error if
// creation fails (e.g., filesystem issues). Callers should handle cleanup.
func WatcherForSession(townRoot, session string) (*Watcher, error) {
	return NewWatcher(townRoot, session)
}
package config

import (
	"os"
	"path/filepath"
	"strings"
)

// LoadRoleDirective loads role directive content from the directive file layout.
// Resolution order:
//  1. Town-level: <townRoot>/directives/<role>.md
//  2. Rig-level:  <townRoot>/<rigName>/directives/<role>.md
//
// If both exist, they are concatenated (town first, then rig) separated by a
// newline, giving rig-level content the last word. If only one exists, that
// content is returned. Returns empty string if no directive files exist.
//
// Invalid or unreadable paths are treated as absent (no error).
func LoadRoleDirective(role, townRoot, rigName string) string {
	var parts []string

	// Town-level directive
	townPath := filepath.Join(townRoot, "directives", role+".md")
	if content, err := os.ReadFile(townPath); err == nil { //nolint:gosec // G304: path is from trusted config
		if s := strings.TrimSpace(string(content)); s != "" {
			parts = append(parts, s)
		}
	}

	// Rig-level directive (wins by appearing last)
	if rigName != "" {
		rigPath := filepath.Join(townRoot, rigName, "directives", role+".md")
		if content, err := os.ReadFile(rigPath); err == nil { //nolint:gosec // G304: path is from trusted config
			if s := strings.TrimSpace(string(content)); s != "" {
				parts = append(parts, s)
			}
		}
	}

	return strings.Join(parts, "\n")
}
