package deacon

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/steveyegge/gastown/internal/config"
	"github.com/steveyegge/gastown/internal/constants"
	"github.com/steveyegge/gastown/internal/runtime"
	"github.com/steveyegge/gastown/internal/session"
	"github.com/steveyegge/gastown/internal/tmux"
)

// Common errors
var (
	ErrNotRunning     = errors.New("deacon not running")
	ErrAlreadyRunning = errors.New("deacon already running")
)

// tmuxOps abstracts tmux operations for testing.
type tmuxOps interface {
	HasSession(name string) (bool, error)
	IsAgentAlive(session string) bool
	KillSessionWithProcesses(name string) error
	NewSessionWithCommand(name, workDir, command string) error
	SetRemainOnExit(pane string, on bool) error
	SetEnvironment(session, key, value string) error
	GetPaneID(session string) (string, error)
	ConfigureGasTownSession(session string, theme *tmux.Theme, rig, worker, role string) error
	WaitForCommand(session string, excludeCommands []string, timeout time.Duration) error
	SetAutoRespawnHook(session string) error
	AcceptStartupDialogs(session string) error
	AcceptWorkspaceTrustDialog(session string) error
	AcceptBypassPermissionsWarning(session string) error
	SendKeysRaw(session, keys string) error
	GetSessionInfo(name string) (*tmux.SessionInfo, error)
}

// Manager handles deacon lifecycle operations.
type Manager struct {
	townRoot string
	tmux     tmuxOps
}

// NewManager creates a new deacon manager for a town.
func NewManager(townRoot string) *Manager {
	return &Manager{
		townRoot: townRoot,
		tmux:     tmux.NewTmux(),
	}
}

// SessionName returns the tmux session name for the deacon.
// This is a package-level function for convenience.
func SessionName() string {
	return session.DeaconSessionName()
}

// SessionName returns the tmux session name for the deacon.
func (m *Manager) SessionName() string {
	return SessionName()
}

// deaconDir returns the working directory for the deacon.
func (m *Manager) deaconDir() string {
	return filepath.Join(m.townRoot, "deacon")
}

// Start starts the deacon session.
// agentOverride allows specifying an alternate agent alias (e.g., for testing).
// Restarts are handled by daemon via ensureDeaconRunning on each heartbeat.
func (m *Manager) Start(agentOverride string) error {
	t := m.tmux
	sessionID := m.SessionName()

	// Check if session already exists
	running, _ := t.HasSession(sessionID)
	if running {
		// Session exists - check if agent is actually running (healthy vs zombie)
		if t.IsAgentAlive(sessionID) {
			return ErrAlreadyRunning
		}

		// Session exists but agent is dead. Kill and recreate uniformly.
		// The auto-respawn hook (SetAutoRespawnHook) handles clean exits at the
		// tmux level — Go doesn't need to distinguish dead pane vs zombie shell.
		// Use KillSessionWithProcesses to ensure all descendant processes are killed.
		if err := t.KillSessionWithProcesses(sessionID); err != nil {
			return fmt.Errorf("killing zombie session: %w", err)
		}
	}

	// Ensure deacon directory exists
	deaconDir := m.deaconDir()
	if err := os.MkdirAll(deaconDir, 0755); err != nil {
		return fmt.Errorf("creating deacon directory: %w", err)
	}

	// Ensure runtime settings exist in deaconDir where session runs.
	runtimeConfig := config.ResolveRoleAgentConfig("deacon", m.townRoot, deaconDir)
	if err := runtime.EnsureSettingsForRole(deaconDir, deaconDir, "deacon", runtimeConfig); err != nil {
		return fmt.Errorf("ensuring runtime settings: %w", err)
	}

	initialPrompt := session.BuildStartupPrompt(session.BeaconConfig{
		Recipient: "deacon",
		Sender:    "daemon",
		Topic:     "patrol",
	}, "I am Deacon. Start patrol: run gt deacon heartbeat, then check gt hook. If no hook, create mol-deacon-patrol wisp and execute it.")
	startupCmd, err := config.BuildStartupCommandFromConfig(config.AgentEnvConfig{
		Role:        "deacon",
		TownRoot:    m.townRoot,
		Prompt:      initialPrompt,
		Topic:       "patrol",
		SessionName: sessionID,
	}, "", initialPrompt, agentOverride)
	if err != nil {
		return fmt.Errorf("building startup command: %w", err)
	}

	// Create session with command directly to avoid send-keys race condition.
	// See: https://github.com/anthropics/gastown/issues/280
	if err := t.NewSessionWithCommand(sessionID, deaconDir, startupCmd); err != nil {
		return fmt.Errorf("creating tmux session: %w", err)
	}

	// PATCH-010: Set remain-on-exit IMMEDIATELY after session creation.
	// This ensures the pane stays if Claude exits before hooks are fully set.
	// The pane will show "[Exited]" status but remain available for respawn.
	_ = t.SetRemainOnExit(sessionID, true)

	// Set environment variables (non-fatal: session works without these)
	// Use centralized AgentEnv for consistency across all role startup paths
	envVars := config.AgentEnv(config.AgentEnvConfig{
		Role:        "deacon",
		TownRoot:    m.townRoot,
		Agent:       agentOverride,
		SessionName: sessionID,
	})
	envVars = session.MergeRuntimeLivenessEnv(envVars, runtimeConfig)
	for k, v := range envVars {
		_ = t.SetEnvironment(sessionID, k, v)
	}

	// Record agent's pane_id for ZFC-compliant liveness checks (gt-qmsx).
	if paneID, err := t.GetPaneID(sessionID); err == nil {
		_ = t.SetEnvironment(sessionID, "GT_PANE_ID", paneID)
	}

	// Apply Deacon theming (non-fatal: theming failure doesn't affect operation)
	theme := tmux.ResolveSessionTheme(m.townRoot, "", "deacon")
	_ = t.ConfigureGasTownSession(sessionID, theme, "", "Deacon", "health-check")

	// Wait for Claude to start - fatal if Claude fails to launch
	if err := t.WaitForCommand(sessionID, constants.SupportedShells, constants.ClaudeStartTimeout); err != nil {
		// Kill the zombie session before returning error
		_ = t.KillSessionWithProcesses(sessionID)
		return fmt.Errorf("waiting for deacon to start: %w", err)
	}

	// Track PID for defense-in-depth orphan cleanup (non-fatal)
	if realTmux, ok := t.(*tmux.Tmux); ok {
		_ = session.TrackSessionPID(m.townRoot, sessionID, realTmux)
	}

	// PATCH-010: Set auto-respawn hook for Deacon resilience.
	// When Claude exits (for any reason), tmux will automatically respawn it.
	// This prevents the crash loop where daemon repeatedly restarts Deacon.
	// Note: SetAutoRespawnHook calls SetRemainOnExit again (harmless, already set above).
	if err := t.SetAutoRespawnHook(sessionID); err != nil {
		// Non-fatal: Deacon still works, just won't auto-respawn on crash
		// Daemon will still restart it, but with a delay
		fmt.Printf("warning: failed to set auto-respawn hook for deacon: %v\n", err)
	}

	// Accept startup dialogs (workspace trust + bypass permissions) if they appear.
	_ = t.AcceptStartupDialogs(sessionID)

	time.Sleep(constants.ShutdownNotifyDelay)

	return nil
}

// Stop stops the deacon session.
func (m *Manager) Stop() error {
	t := m.tmux
	sessionID := m.SessionName()

	// Check if session exists
	running, err := t.HasSession(sessionID)
	if err != nil {
		return fmt.Errorf("checking session: %w", err)
	}
	if !running {
		return ErrNotRunning
	}

	// Try graceful shutdown first (best-effort interrupt)
	_ = t.SendKeysRaw(sessionID, "C-c")
	time.Sleep(100 * time.Millisecond)

	// Kill the session.
	// Use KillSessionWithProcesses to ensure all descendant processes are killed.
	// This prevents orphan bash processes from Claude's Bash tool surviving session termination.
	if err := t.KillSessionWithProcesses(sessionID); err != nil {
		return fmt.Errorf("killing session: %w", err)
	}

	return nil
}

// IsRunning checks if the deacon session is active.
func (m *Manager) IsRunning() (bool, error) {
	return m.tmux.HasSession(m.SessionName())
}

// Status returns information about the deacon session.
func (m *Manager) Status() (*tmux.SessionInfo, error) {
	t := m.tmux
	sessionID := m.SessionName()

	running, err := t.HasSession(sessionID)
	if err != nil {
		return nil, fmt.Errorf("checking session: %w", err)
	}
	if !running {
		return nil, ErrNotRunning
	}

	return t.GetSessionInfo(sessionID)
}
// Package deacon provides the Deacon agent infrastructure.
// The Deacon is a Claude agent that monitors Mayor and Witnesses,
// handles lifecycle requests, and keeps Gas Town running.
package deacon

import (
	"encoding/json"
	"os"
	"path/filepath"
	"time"
)

// Heartbeat age thresholds — these are compiled-in defaults.
// Configurable via operational.deacon.heartbeat_stale_threshold and
// operational.deacon.heartbeat_very_stale_threshold in settings/config.json.
const (
	// HeartbeatStaleThreshold is the age at which a heartbeat is considered stale.
	HeartbeatStaleThreshold = 5 * time.Minute

	// HeartbeatVeryStaleThreshold is the age at which a heartbeat is considered
	// very stale, meaning the Deacon should be poked or restarted.
	// Must be greater than patrol backoff-max (15m) to avoid false positives
	// during legitimate await-signal sleep.
	HeartbeatVeryStaleThreshold = 20 * time.Minute
)

// Heartbeat represents the Deacon's heartbeat file contents.
// Written by the Deacon on each wake cycle.
// Read by the Go daemon to decide whether to poke.
type Heartbeat struct {
	// Timestamp is when the heartbeat was written.
	Timestamp time.Time `json:"timestamp"`

	// Cycle is the current wake cycle number.
	Cycle int64 `json:"cycle"`

	// LastAction describes what the Deacon did in this cycle.
	LastAction string `json:"last_action,omitempty"`

	// HealthyAgents is the count of healthy agents observed.
	HealthyAgents int `json:"healthy_agents"`

	// UnhealthyAgents is the count of unhealthy agents observed.
	UnhealthyAgents int `json:"unhealthy_agents"`
}

// HeartbeatFile returns the path to the Deacon heartbeat file.
func HeartbeatFile(townRoot string) string {
	return filepath.Join(townRoot, "deacon", "heartbeat.json")
}

// WriteHeartbeat writes a new heartbeat to disk.
// Called by the Deacon at the start of each wake cycle.
func WriteHeartbeat(townRoot string, hb *Heartbeat) error {
	hbFile := HeartbeatFile(townRoot)

	// Ensure deacon directory exists
	if err := os.MkdirAll(filepath.Dir(hbFile), 0755); err != nil {
		return err
	}

	// Set timestamp if not already set
	if hb.Timestamp.IsZero() {
		hb.Timestamp = time.Now().UTC()
	}

	data, err := json.MarshalIndent(hb, "", "  ")
	if err != nil {
		return err
	}

	if err := os.WriteFile(hbFile, data, 0600); err != nil {
		return err
	}

	// Also touch .deacon-heartbeat for backward compatibility with shell scripts
	// that check this file's mtime for liveness detection (stuck-agent-dog).
	// These scripts predate heartbeat.json and check mtime, not file contents.
	legacyFile := filepath.Join(filepath.Dir(hbFile), ".deacon-heartbeat")
	_ = os.WriteFile(legacyFile, []byte(""), 0644) //nolint:gosec // G306: world-readable liveness file is intentional

	return nil
}

// ReadHeartbeat reads the Deacon heartbeat from disk.
// Returns nil if the file doesn't exist or can't be read.
func ReadHeartbeat(townRoot string) *Heartbeat {
	hbFile := HeartbeatFile(townRoot)

	data, err := os.ReadFile(hbFile) //nolint:gosec // G304: path is constructed from trusted townRoot
	if err != nil {
		return nil
	}

	var hb Heartbeat
	if err := json.Unmarshal(data, &hb); err != nil {
		return nil
	}

	return &hb
}

// Age returns how old the heartbeat is.
// Returns a very large duration if the heartbeat is nil.
func (hb *Heartbeat) Age() time.Duration {
	if hb == nil {
		return 24 * time.Hour * 365 // Very stale
	}
	return time.Since(hb.Timestamp)
}

// IsFresh returns true if the heartbeat is less than 5 minutes old.
// A fresh heartbeat means the Deacon is actively working or recently finished.
func (hb *Heartbeat) IsFresh() bool {
	return hb != nil && hb.Age() < HeartbeatStaleThreshold
}

// IsStale returns true if the heartbeat is 5-20 minutes old.
// A stale heartbeat may indicate the Deacon is doing a long operation.
func (hb *Heartbeat) IsStale() bool {
	if hb == nil {
		return false
	}
	age := hb.Age()
	return age >= HeartbeatStaleThreshold && age < HeartbeatVeryStaleThreshold
}

// IsVeryStale returns true if the heartbeat is more than 20 minutes old.
// A very stale heartbeat means the Deacon should be poked.
func (hb *Heartbeat) IsVeryStale() bool {
	return hb == nil || hb.Age() >= HeartbeatVeryStaleThreshold
}

// Touch writes a minimal heartbeat with just the timestamp.
// This is a convenience function for simple heartbeat updates.
func Touch(townRoot string) error {
	// Read existing heartbeat to increment cycle
	existing := ReadHeartbeat(townRoot)
	cycle := int64(1)
	if existing != nil {
		cycle = existing.Cycle + 1
	}

	return WriteHeartbeat(townRoot, &Heartbeat{
		Timestamp: time.Now().UTC(),
		Cycle:     cycle,
	})
}

// TouchWithAction writes a heartbeat with an action description.
func TouchWithAction(townRoot, action string, healthy, unhealthy int) error {
	existing := ReadHeartbeat(townRoot)
	cycle := int64(1)
	if existing != nil {
		cycle = existing.Cycle + 1
	}

	return WriteHeartbeat(townRoot, &Heartbeat{
		Timestamp:       time.Now().UTC(),
		Cycle:           cycle,
		LastAction:      action,
		HealthyAgents:   healthy,
		UnhealthyAgents: unhealthy,
	})
}
// Package deacon provides the Deacon agent infrastructure.
package deacon

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/steveyegge/gastown/internal/config"
)

// Default parameters for stuck-session detection.
// These are fallbacks when no config override exists.
// Configurable via operational.deacon in settings/config.json.
const (
	DefaultPingTimeout         = 30 * time.Second // How long to wait for response
	DefaultConsecutiveFailures = 3                // Failures before force-kill
	DefaultCooldown            = 5 * time.Minute  // Minimum time between force-kills
)

// StuckConfig holds configurable parameters for stuck-session detection.
type StuckConfig struct {
	PingTimeout         time.Duration `json:"ping_timeout"`
	ConsecutiveFailures int           `json:"consecutive_failures"`
	Cooldown            time.Duration `json:"cooldown"`
}

// DefaultStuckConfig returns the default stuck detection config.
func DefaultStuckConfig() *StuckConfig {
	return &StuckConfig{
		PingTimeout:         DefaultPingTimeout,
		ConsecutiveFailures: DefaultConsecutiveFailures,
		Cooldown:            DefaultCooldown,
	}
}

// LoadStuckConfig loads stuck detection config from town settings, falling
// back to compiled-in defaults. The townRoot parameter is used to locate
// the settings/config.json file.
func LoadStuckConfig(townRoot string) *StuckConfig {
	opCfg := config.LoadOperationalConfig(townRoot)
	deaconCfg := opCfg.GetDeaconConfig()
	return &StuckConfig{
		PingTimeout:         deaconCfg.PingTimeoutD(),
		ConsecutiveFailures: deaconCfg.ConsecutiveFailuresV(),
		Cooldown:            deaconCfg.CooldownD(),
	}
}

// AgentHealthState tracks the health check state for a single agent.
type AgentHealthState struct {
	// AgentID is the identifier (e.g., "gastown/polecats/max" or "deacon")
	AgentID string `json:"agent_id"`

	// LastPingTime is when we last sent a HEALTH_CHECK nudge
	LastPingTime time.Time `json:"last_ping_time,omitempty"`

	// LastResponseTime is when the agent last updated their activity
	LastResponseTime time.Time `json:"last_response_time,omitempty"`

	// ConsecutiveFailures counts how many health checks failed in a row
	ConsecutiveFailures int `json:"consecutive_failures"`

	// LastForceKillTime is when we last force-killed this agent
	LastForceKillTime time.Time `json:"last_force_kill_time,omitempty"`

	// ForceKillCount is total number of force-kills for this agent
	ForceKillCount int `json:"force_kill_count"`
}

// HealthCheckState holds health check state for all monitored agents.
type HealthCheckState struct {
	// Agents maps agent ID to their health state
	Agents map[string]*AgentHealthState `json:"agents"`

	// LastUpdated is when this state was last written
	LastUpdated time.Time `json:"last_updated"`
}

// HealthCheckStateFile returns the path to the health check state file.
func HealthCheckStateFile(townRoot string) string {
	return filepath.Join(townRoot, "deacon", "health-check-state.json")
}

// LoadHealthCheckState loads the health check state from disk.
// Returns empty state if file doesn't exist.
func LoadHealthCheckState(townRoot string) (*HealthCheckState, error) {
	stateFile := HealthCheckStateFile(townRoot)

	data, err := os.ReadFile(stateFile) //nolint:gosec // G304: path is constructed from trusted townRoot
	if err != nil {
		if os.IsNotExist(err) {
			// Return empty state
			return &HealthCheckState{
				Agents: make(map[string]*AgentHealthState),
			}, nil
		}
		return nil, fmt.Errorf("reading health check state: %w", err)
	}

	var state HealthCheckState
	if err := json.Unmarshal(data, &state); err != nil {
		return nil, fmt.Errorf("parsing health check state: %w", err)
	}

	if state.Agents == nil {
		state.Agents = make(map[string]*AgentHealthState)
	}

	return &state, nil
}

// SaveHealthCheckState saves the health check state to disk.
func SaveHealthCheckState(townRoot string, state *HealthCheckState) error {
	stateFile := HealthCheckStateFile(townRoot)

	// Ensure directory exists
	if err := os.MkdirAll(filepath.Dir(stateFile), 0755); err != nil {
		return fmt.Errorf("creating deacon directory: %w", err)
	}

	state.LastUpdated = time.Now().UTC()

	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return fmt.Errorf("marshaling health check state: %w", err)
	}

	return os.WriteFile(stateFile, data, 0600)
}

// GetAgentState returns the health state for an agent, creating if needed.
func (s *HealthCheckState) GetAgentState(agentID string) *AgentHealthState {
	if s.Agents == nil {
		s.Agents = make(map[string]*AgentHealthState)
	}

	state, ok := s.Agents[agentID]
	if !ok {
		state = &AgentHealthState{AgentID: agentID}
		s.Agents[agentID] = state
	}
	return state
}

// HealthCheckResult represents the outcome of a health check.
type HealthCheckResult struct {
	AgentID             string        `json:"agent_id"`
	Responded           bool          `json:"responded"`
	ResponseTime        time.Duration `json:"response_time,omitempty"`
	ConsecutiveFailures int           `json:"consecutive_failures"`
	ShouldForceKill     bool          `json:"should_force_kill"`
	InCooldown          bool          `json:"in_cooldown"`
	CooldownRemaining   time.Duration `json:"cooldown_remaining,omitempty"`
}

// Common errors for stuck-session detection.
var (
	ErrAgentInCooldown  = errors.New("agent is in cooldown period after recent force-kill")
	ErrAgentNotFound    = errors.New("agent not found or session doesn't exist")
	ErrAgentResponsive  = errors.New("agent is responsive, no action needed")
)

// RecordPing records that a health check ping was sent to an agent.
func (s *AgentHealthState) RecordPing() {
	s.LastPingTime = time.Now().UTC()
}

// RecordResponse records that an agent responded to a health check.
// This resets the consecutive failure counter.
func (s *AgentHealthState) RecordResponse() {
	s.LastResponseTime = time.Now().UTC()
	s.ConsecutiveFailures = 0
}

// RecordFailure records that an agent failed to respond to a health check.
func (s *AgentHealthState) RecordFailure() {
	s.ConsecutiveFailures++
}

// RecordForceKill records that an agent was force-killed.
func (s *AgentHealthState) RecordForceKill() {
	s.LastForceKillTime = time.Now().UTC()
	s.ForceKillCount++
	s.ConsecutiveFailures = 0 // Reset after kill
}

// IsInCooldown returns true if the agent was recently force-killed.
func (s *AgentHealthState) IsInCooldown(cooldown time.Duration) bool {
	if s.LastForceKillTime.IsZero() {
		return false
	}
	return time.Since(s.LastForceKillTime) < cooldown
}

// CooldownRemaining returns how long until cooldown expires.
func (s *AgentHealthState) CooldownRemaining(cooldown time.Duration) time.Duration {
	if s.LastForceKillTime.IsZero() {
		return 0
	}
	remaining := cooldown - time.Since(s.LastForceKillTime)
	if remaining < 0 {
		return 0
	}
	return remaining
}

// ShouldForceKill returns true if the agent has exceeded the failure threshold.
func (s *AgentHealthState) ShouldForceKill(threshold int) bool {
	return s.ConsecutiveFailures >= threshold
}
// Package deacon provides the Deacon agent infrastructure.
package deacon

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/steveyegge/gastown/internal/beads"
	"github.com/steveyegge/gastown/internal/util"
)

// Default parameters for re-dispatch rate-limiting.
// Configurable via operational.deacon.max_redispatches and
// operational.deacon.redispatch_cooldown in settings/config.json.
const (
	// DefaultMaxRedispatches is the number of times a bead can be re-dispatched
	// before escalating to Mayor instead of re-slinging.
	DefaultMaxRedispatches = 3

	// DefaultRedispatchCooldown is the minimum time between re-dispatches of
	// the same bead. Prevents thrashing when a bead keeps killing polecats.
	DefaultRedispatchCooldown = 5 * time.Minute
)

// RedispatchState tracks re-dispatch attempts for recovered beads.
// Persisted to deacon/redispatch-state.json.
type RedispatchState struct {
	// Beads maps bead ID to their re-dispatch tracking state.
	Beads map[string]*BeadRedispatchState `json:"beads"`

	// LastUpdated is when this state was last written.
	LastUpdated time.Time `json:"last_updated"`
}

// BeadRedispatchState tracks the re-dispatch history for a single bead.
type BeadRedispatchState struct {
	// BeadID is the bead identifier.
	BeadID string `json:"bead_id"`

	// AttemptCount is total number of re-dispatch attempts for this bead.
	AttemptCount int `json:"attempt_count"`

	// LastAttemptTime is when the last re-dispatch was attempted.
	LastAttemptTime time.Time `json:"last_attempt_time,omitempty"`

	// LastRig is the rig where the last re-dispatch was sent.
	LastRig string `json:"last_rig,omitempty"`

	// Escalated is true if this bead has been escalated to Mayor.
	Escalated bool `json:"escalated,omitempty"`

	// EscalatedAt is when the bead was escalated.
	EscalatedAt time.Time `json:"escalated_at,omitempty"`
}

// RedispatchResult describes the outcome of a re-dispatch attempt.
type RedispatchResult struct {
	BeadID     string `json:"bead_id"`
	Action     string `json:"action"` // "redispatched", "cooldown", "escalated", "error"
	TargetRig  string `json:"target_rig,omitempty"`
	Attempts   int    `json:"attempts"`
	Message    string `json:"message,omitempty"`
	Error      error  `json:"error,omitempty"`
}

// RedispatchStateFile returns the path to the re-dispatch state file.
func RedispatchStateFile(townRoot string) string {
	return filepath.Join(townRoot, "deacon", "redispatch-state.json")
}

// LoadRedispatchState loads the re-dispatch state from disk.
// Returns empty state if file doesn't exist.
func LoadRedispatchState(townRoot string) (*RedispatchState, error) {
	stateFile := RedispatchStateFile(townRoot)

	data, err := os.ReadFile(stateFile) //nolint:gosec // G304: path is constructed from trusted townRoot
	if err != nil {
		if os.IsNotExist(err) {
			return &RedispatchState{
				Beads: make(map[string]*BeadRedispatchState),
			}, nil
		}
		return nil, fmt.Errorf("reading redispatch state: %w", err)
	}

	var state RedispatchState
	if err := json.Unmarshal(data, &state); err != nil {
		return nil, fmt.Errorf("parsing redispatch state: %w", err)
	}

	if state.Beads == nil {
		state.Beads = make(map[string]*BeadRedispatchState)
	}

	return &state, nil
}

// SaveRedispatchState saves the re-dispatch state to disk.
func SaveRedispatchState(townRoot string, state *RedispatchState) error {
	stateFile := RedispatchStateFile(townRoot)

	// Ensure directory exists
	if err := os.MkdirAll(filepath.Dir(stateFile), 0755); err != nil {
		return fmt.Errorf("creating deacon directory: %w", err)
	}

	state.LastUpdated = time.Now().UTC()

	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return fmt.Errorf("marshaling redispatch state: %w", err)
	}

	return os.WriteFile(stateFile, data, 0600)
}

// GetBeadState returns the re-dispatch state for a bead, creating if needed.
func (s *RedispatchState) GetBeadState(beadID string) *BeadRedispatchState {
	if s.Beads == nil {
		s.Beads = make(map[string]*BeadRedispatchState)
	}

	state, ok := s.Beads[beadID]
	if !ok {
		state = &BeadRedispatchState{BeadID: beadID}
		s.Beads[beadID] = state
	}
	return state
}

// IsInCooldown returns true if the bead was recently re-dispatched.
func (s *BeadRedispatchState) IsInCooldown(cooldown time.Duration) bool {
	if s.LastAttemptTime.IsZero() {
		return false
	}
	return time.Since(s.LastAttemptTime) < cooldown
}

// CooldownRemaining returns how long until cooldown expires.
func (s *BeadRedispatchState) CooldownRemaining(cooldown time.Duration) time.Duration {
	if s.LastAttemptTime.IsZero() {
		return 0
	}
	remaining := cooldown - time.Since(s.LastAttemptTime)
	if remaining < 0 {
		return 0
	}
	return remaining
}

// ShouldEscalate returns true if the bead has exceeded the max re-dispatch attempts.
func (s *BeadRedispatchState) ShouldEscalate(maxAttempts int) bool {
	return s.AttemptCount >= maxAttempts
}

// RecordAttempt records a re-dispatch attempt for the bead.
func (s *BeadRedispatchState) RecordAttempt(rig string) {
	s.AttemptCount++
	s.LastAttemptTime = time.Now().UTC()
	s.LastRig = rig
}

// RecordEscalation records that the bead was escalated to Mayor.
func (s *BeadRedispatchState) RecordEscalation() {
	s.Escalated = true
	s.EscalatedAt = time.Now().UTC()
}

// Redispatch handles a RECOVERED_BEAD message by re-slinging the bead to an
// available polecat, or escalating to Mayor if the bead has failed too many times.
//
// Parameters:
//   - townRoot: the Gas Town workspace root
//   - beadID: the recovered bead to re-dispatch
//   - sourceRig: the rig from which the bead was recovered (empty = auto-detect from prefix)
//   - maxAttempts: max re-dispatches before escalating (0 = use default)
//   - cooldown: min time between re-dispatches (0 = use default)
func Redispatch(townRoot, beadID, sourceRig string, maxAttempts int, cooldown time.Duration) *RedispatchResult {
	result := &RedispatchResult{BeadID: beadID}

	if maxAttempts <= 0 {
		maxAttempts = DefaultMaxRedispatches
	}
	if cooldown <= 0 {
		cooldown = DefaultRedispatchCooldown
	}

	// Load state
	state, err := LoadRedispatchState(townRoot)
	if err != nil {
		result.Action = "error"
		result.Error = fmt.Errorf("loading redispatch state: %w", err)
		return result
	}

	beadState := state.GetBeadState(beadID)
	result.Attempts = beadState.AttemptCount

	// Check if already escalated
	if beadState.Escalated {
		result.Action = "already-escalated"
		result.Message = fmt.Sprintf("bead already escalated to Mayor at %s", beadState.EscalatedAt.Format(time.RFC3339))
		return result
	}

	// Check cooldown
	if beadState.IsInCooldown(cooldown) {
		remaining := beadState.CooldownRemaining(cooldown)
		result.Action = "cooldown"
		result.Message = fmt.Sprintf("in cooldown (remaining: %s)", remaining.Round(time.Second))
		return result
	}

	// Check if we should escalate instead of re-dispatching
	if beadState.ShouldEscalate(maxAttempts) {
		result.Action = "escalated"
		result.Attempts = beadState.AttemptCount

		// Escalate to Mayor
		err := escalateToMayor(townRoot, beadID, beadState)
		if err != nil {
			result.Error = fmt.Errorf("escalating to mayor: %w", err)
			result.Message = fmt.Sprintf("failed to escalate after %d attempts: %v", beadState.AttemptCount, err)
		} else {
			beadState.RecordEscalation()
			result.Message = fmt.Sprintf("escalated to Mayor after %d failed re-dispatches", beadState.AttemptCount)
		}

		// Save state regardless of escalation success
		if saveErr := SaveRedispatchState(townRoot, state); saveErr != nil {
			// Log but don't fail - escalation mail was already sent
			result.Message += fmt.Sprintf(" (warning: state save failed: %v)", saveErr)
		}

		return result
	}

	// Determine target rig
	targetRig := sourceRig
	if targetRig == "" {
		targetRig = resolveRigFromBead(townRoot, beadID)
	}
	if targetRig == "" {
		result.Action = "error"
		result.Error = fmt.Errorf("cannot determine target rig for bead %s", beadID)
		return result
	}
	result.TargetRig = targetRig

	// Verify bead is still open (not already claimed or closed).
	// Only proceed when status is explicitly "open". Empty status (query
	// failure) is treated as "not open" to avoid re-dispatching closed
	// beads when bd show fails. (gt-sy8)
	beadStatus := getBeadStatusForRedispatch(townRoot, beadID)
	if beadStatus != "open" {
		result.Action = "skipped"
		if beadStatus == "" {
			result.Message = "could not determine bead status (treating as not open)"
		} else {
			result.Message = fmt.Sprintf("bead status is %q (expected open)", beadStatus)
		}
		return result
	}

	// Re-dispatch via gt sling
	err = slingBead(townRoot, beadID, targetRig)
	if err != nil {
		result.Action = "error"
		result.Error = fmt.Errorf("slinging bead to %s: %w", targetRig, err)

		// Record the failed attempt
		beadState.RecordAttempt(targetRig)
		_ = SaveRedispatchState(townRoot, state)

		return result
	}

	// Record successful dispatch
	beadState.RecordAttempt(targetRig)
	result.Action = "redispatched"
	result.Attempts = beadState.AttemptCount
	result.Message = fmt.Sprintf("re-dispatched to %s (attempt %d/%d)", targetRig, beadState.AttemptCount, maxAttempts)

	// Save state
	if saveErr := SaveRedispatchState(townRoot, state); saveErr != nil {
		result.Message += fmt.Sprintf(" (warning: state save failed: %v)", saveErr)
	}

	return result
}

// PruneRedispatchState removes entries for beads that are no longer open.
// Call periodically to prevent unbounded state growth.
func PruneRedispatchState(townRoot string) (int, error) {
	state, err := LoadRedispatchState(townRoot)
	if err != nil {
		return 0, err
	}

	pruned := 0
	for beadID := range state.Beads {
		status := getBeadStatusForRedispatch(townRoot, beadID)
		// Remove entries for beads that are closed, or that we can't find
		if status == "closed" || status == "" {
			delete(state.Beads, beadID)
			pruned++
		}
	}

	if pruned > 0 {
		if err := SaveRedispatchState(townRoot, state); err != nil {
			return pruned, err
		}
	}

	return pruned, nil
}

// resolveRigFromBead determines the rig that owns a bead based on its prefix.
func resolveRigFromBead(townRoot, beadID string) string {
	prefix := beads.ExtractPrefix(beadID)
	if prefix == "" {
		return ""
	}
	return beads.GetRigNameForPrefix(townRoot, prefix)
}

// getBeadStatusForRedispatch returns the current status of a bead.
func getBeadStatusForRedispatch(townRoot, beadID string) string {
	cmd := exec.Command("bd", "show", beadID, "--json")
	cmd.Dir = townRoot
	util.SetDetachedProcessGroup(cmd)

	output, err := cmd.Output()
	if err != nil {
		return ""
	}

	var issues []struct {
		Status string `json:"status"`
	}
	if err := json.Unmarshal(output, &issues); err != nil || len(issues) == 0 {
		return ""
	}
	return issues[0].Status
}

// slingBead dispatches a bead to a rig via gt sling.
func slingBead(townRoot, beadID, rig string) error {
	cmd := exec.Command("gt", "sling", beadID, rig, "--force", "--no-convoy")
	cmd.Dir = townRoot
	util.SetDetachedProcessGroup(cmd)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// escalateToMayor sends an escalation mail to the Mayor about a repeatedly-failing bead.
func escalateToMayor(townRoot, beadID string, beadState *BeadRedispatchState) error {
	subject := fmt.Sprintf("REDISPATCH_FAILED: %s (%d attempts)", beadID, beadState.AttemptCount)
	body := fmt.Sprintf(`Bead %s has been recovered and re-dispatched %d times but keeps failing.

Bead: %s
Attempts: %d
Last Rig: %s
Last Attempt: %s

This bead may have a systemic issue (e.g., causes polecat crashes).
Please investigate and either:
1. Fix the underlying issue and re-sling manually
2. Close/deprioritize the bead if it's not actionable
3. Increase the re-dispatch limit if the failures were transient`,
		beadID,
		beadState.AttemptCount,
		beadID,
		beadState.AttemptCount,
		beadState.LastRig,
		beadState.LastAttemptTime.Format(time.RFC3339),
	)

	cmd := exec.Command("gt", "mail", "send", "mayor/", "-s", subject, "-m", body)
	cmd.Dir = townRoot
	util.SetDetachedProcessGroup(cmd)
	return cmd.Run()
}

// ParseRecoveredBeadSubject extracts the bead ID from a RECOVERED_BEAD mail subject.
// Expected format: "RECOVERED_BEAD <bead-id>"
func ParseRecoveredBeadSubject(subject string) (beadID string, ok bool) {
	const prefix = "RECOVERED_BEAD "
	if !strings.HasPrefix(subject, prefix) {
		return "", false
	}
	beadID = strings.TrimSpace(strings.TrimPrefix(subject, prefix))
	if beadID == "" {
		return "", false
	}
	return beadID, true
}

// ParseRecoveredBeadBody extracts the source rig from a RECOVERED_BEAD mail body.
// Looks for "Polecat: <rig>/<name>" line.
func ParseRecoveredBeadBody(body string) (rig string) {
	for _, line := range strings.Split(body, "\n") {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, "Polecat:") {
			polecatAddr := strings.TrimSpace(strings.TrimPrefix(line, "Polecat:"))
			parts := strings.SplitN(polecatAddr, "/", 2)
			if len(parts) >= 1 {
				return parts[0]
			}
		}
	}
	return ""
}
package deacon

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/steveyegge/gastown/internal/constants"
	"github.com/steveyegge/gastown/internal/util"
)

// Default parameters for feed-stranded rate limiting.
// Configurable via operational.deacon.max_feeds_per_cycle and
// operational.deacon.feed_cooldown in settings/config.json.
const (
	// DefaultMaxFeedsPerCycle is the maximum number of convoys to feed in one invocation.
	// Prevents spawning too many dogs at once.
	DefaultMaxFeedsPerCycle = 3

	// DefaultFeedCooldown is the minimum time between feeding the same convoy.
	// Prevents re-dispatching a dog before the previous one finishes.
	DefaultFeedCooldown = 10 * time.Minute
)

// FeedStrandedState tracks feeding attempts per convoy.
// Persisted to deacon/feed-stranded-state.json.
type FeedStrandedState struct {
	// Convoys maps convoy ID to their feed tracking state.
	Convoys map[string]*ConvoyFeedState `json:"convoys"`

	// LastUpdated is when this state was last written.
	LastUpdated time.Time `json:"last_updated"`
}

// ConvoyFeedState tracks the feed history for a single convoy.
type ConvoyFeedState struct {
	// ConvoyID is the convoy identifier.
	ConvoyID string `json:"convoy_id"`

	// FeedCount is total number of feed dispatches for this convoy.
	FeedCount int `json:"feed_count"`

	// LastFeedTime is when the last feed was dispatched.
	LastFeedTime time.Time `json:"last_feed_time,omitempty"`
}

// StrandedConvoy holds info about a stranded convoy from `gt convoy stranded --json`.
type StrandedConvoy struct {
	ID           string   `json:"id"`
	Title        string   `json:"title"`
	TrackedCount int      `json:"tracked_count"`
	ReadyCount   int      `json:"ready_count"`
	ReadyIssues  []string `json:"ready_issues"`
}

// FeedResult describes the outcome of a feed-stranded invocation.
type FeedResult struct {
	// Fed is the number of convoys dispatched to dogs for feeding.
	Fed int `json:"fed"`

	// Closed is the number of empty convoys auto-closed.
	Closed int `json:"closed"`

	// Skipped is the number of convoys skipped (cooldown).
	Skipped int `json:"skipped"`

	// NeedsAttention is the number of convoys with tracked issues but no ready
	// issues. These require agent judgment — Go surfaces the raw data but does
	// not classify or act on them.
	NeedsAttention int `json:"needs_attention"`

	// Errors is the number of convoys that failed to process.
	Errors int `json:"errors"`

	// Details has per-convoy results.
	Details []FeedConvoyResult `json:"details"`
}

// FeedConvoyResult describes the outcome for a single convoy.
type FeedConvoyResult struct {
	ConvoyID     string `json:"convoy_id"`
	Action       string `json:"action"` // "fed", "closed", "cooldown", "error", "limit", "needs_attention"
	Message      string `json:"message"`
	TrackedCount int    `json:"tracked_count,omitempty"` // Raw data for agent inspection
	ReadyCount   int    `json:"ready_count,omitempty"`   // Raw data for agent inspection
}

// FeedStrandedStateFile returns the path to the feed-stranded state file.
func FeedStrandedStateFile(townRoot string) string {
	return filepath.Join(townRoot, "deacon", "feed-stranded-state.json")
}

// LoadFeedStrandedState loads the feed-stranded state from disk.
// Returns empty state if file doesn't exist.
func LoadFeedStrandedState(townRoot string) (*FeedStrandedState, error) {
	stateFile := FeedStrandedStateFile(townRoot)

	data, err := os.ReadFile(stateFile) //nolint:gosec // G304: path is constructed from trusted townRoot
	if err != nil {
		if os.IsNotExist(err) {
			return &FeedStrandedState{
				Convoys: make(map[string]*ConvoyFeedState),
			}, nil
		}
		return nil, fmt.Errorf("reading feed-stranded state: %w", err)
	}

	var state FeedStrandedState
	if err := json.Unmarshal(data, &state); err != nil {
		return nil, fmt.Errorf("parsing feed-stranded state: %w", err)
	}

	if state.Convoys == nil {
		state.Convoys = make(map[string]*ConvoyFeedState)
	}

	return &state, nil
}

// SaveFeedStrandedState saves the feed-stranded state to disk.
func SaveFeedStrandedState(townRoot string, state *FeedStrandedState) error {
	stateFile := FeedStrandedStateFile(townRoot)

	// Ensure directory exists
	if err := os.MkdirAll(filepath.Dir(stateFile), 0755); err != nil {
		return fmt.Errorf("creating deacon directory: %w", err)
	}

	state.LastUpdated = time.Now().UTC()

	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return fmt.Errorf("marshaling feed-stranded state: %w", err)
	}

	return os.WriteFile(stateFile, data, 0600)
}

// GetConvoyState returns the feed state for a convoy, creating if needed.
func (s *FeedStrandedState) GetConvoyState(convoyID string) *ConvoyFeedState {
	if s.Convoys == nil {
		s.Convoys = make(map[string]*ConvoyFeedState)
	}

	state, ok := s.Convoys[convoyID]
	if !ok {
		state = &ConvoyFeedState{ConvoyID: convoyID}
		s.Convoys[convoyID] = state
	}
	return state
}

// IsInCooldown returns true if the convoy was recently fed.
func (s *ConvoyFeedState) IsInCooldown(cooldown time.Duration) bool {
	if s.LastFeedTime.IsZero() {
		return false
	}
	return time.Since(s.LastFeedTime) < cooldown
}

// CooldownRemaining returns how long until cooldown expires.
func (s *ConvoyFeedState) CooldownRemaining(cooldown time.Duration) time.Duration {
	if s.LastFeedTime.IsZero() {
		return 0
	}
	remaining := cooldown - time.Since(s.LastFeedTime)
	if remaining < 0 {
		return 0
	}
	return remaining
}

// RecordFeed records a feed dispatch for the convoy.
func (s *ConvoyFeedState) RecordFeed() {
	s.FeedCount++
	s.LastFeedTime = time.Now().UTC()
}

// FindStrandedConvoys runs `gt convoy stranded --json` and parses the output.
func FindStrandedConvoys(townRoot string) ([]StrandedConvoy, error) {
	cmd := exec.Command("gt", "convoy", "stranded", "--json")
	cmd.Dir = townRoot
	util.SetDetachedProcessGroup(cmd)

	output, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("running gt convoy stranded: %w", err)
	}

	var stranded []StrandedConvoy
	if err := json.Unmarshal(output, &stranded); err != nil {
		return nil, fmt.Errorf("parsing stranded convoys: %w", err)
	}

	return stranded, nil
}

// FeedStranded detects stranded convoys and takes mechanical actions where safe.
// Empty convoys (0 tracked) are auto-closed. Feedable convoys get a dog dispatched.
// Convoys with tracked-but-not-ready issues are surfaced as "needs_attention" with
// raw data (tracked_count, ready_count) for the deacon agent to inspect and decide.
// Rate limits by maxPerCycle and per-convoy cooldown.
func FeedStranded(townRoot string, maxPerCycle int, cooldown time.Duration) *FeedResult {
	result := &FeedResult{}

	if maxPerCycle <= 0 {
		maxPerCycle = DefaultMaxFeedsPerCycle
	}
	if cooldown <= 0 {
		cooldown = DefaultFeedCooldown
	}

	// Find stranded convoys
	stranded, err := FindStrandedConvoys(townRoot)
	if err != nil {
		result.Errors++
		result.Details = append(result.Details, FeedConvoyResult{
			Action:  "error",
			Message: fmt.Sprintf("failed to find stranded convoys: %v", err),
		})
		return result
	}

	if len(stranded) == 0 {
		return result
	}

	// Load state for cooldown tracking
	state, err := LoadFeedStrandedState(townRoot)
	if err != nil {
		result.Errors++
		result.Details = append(result.Details, FeedConvoyResult{
			Action:  "error",
			Message: fmt.Sprintf("failed to load feed state: %v", err),
		})
		return result
	}

	fedCount := 0

	for _, convoy := range stranded {
		// Handle convoys with no ready issues.
		if convoy.ReadyCount == 0 {
			// Convoy has tracked issues but none are ready — surface raw data
			// for the deacon agent to inspect. Go does not classify WHY issues
			// aren't ready (dependency resolution, external block, etc.).
			if convoy.TrackedCount > 0 {
				result.NeedsAttention++
				result.Details = append(result.Details, FeedConvoyResult{
					ConvoyID:     convoy.ID,
					Action:       "needs_attention",
					Message:      fmt.Sprintf("%d tracked issues, 0 ready — requires agent review", convoy.TrackedCount),
					TrackedCount: convoy.TrackedCount,
					ReadyCount:   0,
				})
				continue
			}

			// Truly empty convoy (0 tracked issues) — auto-close
			if err := closeEmptyConvoy(townRoot, convoy.ID); err != nil {
				result.Errors++
				result.Details = append(result.Details, FeedConvoyResult{
					ConvoyID: convoy.ID,
					Action:   "error",
					Message:  fmt.Sprintf("failed to auto-close empty convoy: %v", err),
				})
			} else {
				result.Closed++
				result.Details = append(result.Details, FeedConvoyResult{
					ConvoyID: convoy.ID,
					Action:   "closed",
					Message:  "auto-closed empty convoy (0 tracked issues)",
				})
			}
			continue
		}

		// Rate limit: check per-cycle cap
		if fedCount >= maxPerCycle {
			result.Details = append(result.Details, FeedConvoyResult{
				ConvoyID: convoy.ID,
				Action:   "limit",
				Message:  fmt.Sprintf("skipped: per-cycle limit reached (%d/%d)", fedCount, maxPerCycle),
			})
			continue
		}

		// Rate limit: check per-convoy cooldown
		convoyState := state.GetConvoyState(convoy.ID)
		if convoyState.IsInCooldown(cooldown) {
			remaining := convoyState.CooldownRemaining(cooldown)
			result.Skipped++
			result.Details = append(result.Details, FeedConvoyResult{
				ConvoyID: convoy.ID,
				Action:   "cooldown",
				Message:  fmt.Sprintf("in cooldown (remaining: %s)", remaining.Round(time.Second)),
			})
			continue
		}

		// Dispatch dog to feed the convoy
		if err := dispatchFeedDog(townRoot, convoy.ID); err != nil {
			result.Errors++
			result.Details = append(result.Details, FeedConvoyResult{
				ConvoyID: convoy.ID,
				Action:   "error",
				Message:  fmt.Sprintf("failed to dispatch feed dog: %v", err),
			})
			continue
		}

		convoyState.RecordFeed()
		fedCount++
		result.Fed++
		result.Details = append(result.Details, FeedConvoyResult{
			ConvoyID: convoy.ID,
			Action:   "fed",
			Message:  fmt.Sprintf("dispatched dog to feed (%d ready issues)", convoy.ReadyCount),
		})
	}

	// Save state
	if err := SaveFeedStrandedState(townRoot, state); err != nil {
		result.Details = append(result.Details, FeedConvoyResult{
			Action:  "error",
			Message: fmt.Sprintf("warning: failed to save feed state: %v", err),
		})
	}

	return result
}

// closeEmptyConvoy runs `gt convoy check <id>` to auto-close an empty convoy.
func closeEmptyConvoy(townRoot, convoyID string) error {
	cmd := exec.Command("gt", "convoy", "check", convoyID)
	cmd.Dir = townRoot
	util.SetDetachedProcessGroup(cmd)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// dispatchFeedDog dispatches a dog to feed a stranded convoy via gt sling.
func dispatchFeedDog(townRoot, convoyID string) error {
	cmd := exec.Command("gt", "sling", constants.MolConvoyFeed, "deacon/dogs",
		"--var", fmt.Sprintf("convoy=%s", convoyID))
	cmd.Dir = townRoot
	util.SetDetachedProcessGroup(cmd)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// PruneFeedStrandedState removes entries for convoys that are no longer open.
// Call periodically to prevent unbounded state growth.
func PruneFeedStrandedState(townRoot string) (int, error) {
	state, err := LoadFeedStrandedState(townRoot)
	if err != nil {
		return 0, err
	}

	pruned := 0
	for convoyID := range state.Convoys {
		status := getConvoyStatus(townRoot, convoyID)
		if status == "closed" || status == "" {
			delete(state.Convoys, convoyID)
			pruned++
		}
	}

	if pruned > 0 {
		if err := SaveFeedStrandedState(townRoot, state); err != nil {
			return pruned, err
		}
	}

	return pruned, nil
}

// getConvoyStatus returns the current status of a convoy bead.
func getConvoyStatus(townRoot, convoyID string) string {
	cmd := exec.Command("bd", "show", convoyID, "--json")
	cmd.Dir = townRoot
	util.SetDetachedProcessGroup(cmd)

	output, err := cmd.Output()
	if err != nil {
		return ""
	}

	var issues []struct {
		Status string `json:"status"`
	}
	if err := json.Unmarshal(output, &issues); err != nil || len(issues) == 0 {
		return ""
	}
	return issues[0].Status
}
