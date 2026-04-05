// Package config provides configuration types and serialization for Gas Town.
package config

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
)

// AgentPreset identifies a supported LLM agent runtime.
// These presets provide sensible defaults that can be overridden in config.
type AgentPreset string

// Supported agent presets (built-in, E2E tested).
const (
	// AgentClaude is Claude Code (default).
	AgentClaude AgentPreset = "claude"
	// AgentGemini is Gemini CLI.
	AgentGemini AgentPreset = "gemini"
	// AgentCodex is OpenAI Codex.
	AgentCodex AgentPreset = "codex"
	// AgentCursor is Cursor Agent.
	AgentCursor AgentPreset = "cursor"
	// AgentAuggie is Auggie CLI.
	AgentAuggie AgentPreset = "auggie"
	// AgentAmp is Sourcegraph AMP.
	AgentAmp AgentPreset = "amp"
	// AgentOpenCode is OpenCode multi-model CLI.
	AgentOpenCode AgentPreset = "opencode"
	// AgentCopilot is GitHub Copilot CLI.
	AgentCopilot AgentPreset = "copilot"
	// AgentPi is Pi Coding Agent (extension-based lifecycle).
	AgentPi AgentPreset = "pi"
	// AgentOmp is Oh My Pi (OMP) — Pi fork with hook-based lifecycle.
	// Inspired by github.com/ProbabilityEngineer/pi-mono gastown integration.
	AgentOmp AgentPreset = "omp"
)

// AgentPresetInfo contains the configuration details for an agent preset.
// This is the single source of truth for all agent-specific behavior.
// Adding a new agent = adding a builtinPresets entry + optional hook installer.
// No provider-string switch statements should exist outside this registry.
type AgentPresetInfo struct {
	// Name is the preset identifier (e.g., "claude", "gemini", "codex", "cursor", "auggie", "amp", "copilot").
	Name AgentPreset `json:"name"`

	// Command is the CLI binary to invoke.
	Command string `json:"command"`

	// Args are the default command-line arguments for autonomous mode.
	Args []string `json:"args"`

	// Env are environment variables to set when starting the agent.
	// These are merged with the standard GT_* variables.
	// Used for agent-specific configuration like OPENCODE_PERMISSION.
	Env map[string]string `json:"env,omitempty"`

	// ProcessNames are the process names to look for when detecting if the agent is running.
	// Used by tmux.IsAgentRunning to check pane_current_command.
	// E.g., ["node"] for Claude, ["cursor-agent"] for Cursor.
	ProcessNames []string `json:"process_names,omitempty"`

	// SessionIDEnv is the environment variable for session ID.
	// Used for resuming sessions across restarts.
	SessionIDEnv string `json:"session_id_env,omitempty"`

	// ResumeFlag is the flag/subcommand for resuming a specific session.
	// For claude/gemini: "--resume"
	// For codex: "resume" (subcommand)
	ResumeFlag string `json:"resume_flag,omitempty"`

	// ContinueFlag is the flag for auto-resuming the most recent session.
	// For claude: "--continue" (--resume without args opens interactive picker)
	// If empty, --resume without a session ID is rejected with a clear error.
	ContinueFlag string `json:"continue_flag,omitempty"`

	// ResumeStyle indicates how to invoke resume:
	// "flag" - pass as --resume <id> argument
	// "subcommand" - pass as 'codex resume <id>'
	ResumeStyle string `json:"resume_style,omitempty"`

	// SupportsHooks indicates if the agent supports hooks system.
	SupportsHooks bool `json:"supports_hooks,omitempty"`

	// SupportsForkSession indicates if --fork-session is available.
	// Used by the seance command for session forking.
	SupportsForkSession bool `json:"supports_fork_session,omitempty"`

	// NonInteractive contains settings for non-interactive mode.
	NonInteractive *NonInteractiveConfig `json:"non_interactive,omitempty"`

	// --- Runtime default fields (replaces scattered default*() switch statements) ---

	// PromptMode controls how the initial prompt is delivered: "arg" or "none".
	// Defaults to "arg" if empty.
	PromptMode string `json:"prompt_mode,omitempty"`

	// ConfigDirEnv is the env var for the agent's config directory (e.g., "CLAUDE_CONFIG_DIR").
	ConfigDirEnv string `json:"config_dir_env,omitempty"`

	// ConfigDir is the top-level config directory (e.g., ".claude", ".opencode").
	// Used for slash command provisioning. Empty means no command provisioning.
	ConfigDir string `json:"config_dir,omitempty"`

	// HooksProvider is the hooks framework provider type (e.g., "claude", "opencode").
	// Empty or "none" means no hooks support.
	HooksProvider string `json:"hooks_provider,omitempty"`

	// HooksDir is the directory for hooks/settings (e.g., ".claude", ".opencode/plugins").
	HooksDir string `json:"hooks_dir,omitempty"`

	// HooksSettingsFile is the settings/plugin filename (e.g., "settings.json", "gastown.js").
	HooksSettingsFile string `json:"hooks_settings_file,omitempty"`

	// HooksInformational indicates hooks are instructions-only (not executable lifecycle hooks).
	// For these providers, Gas Town sends startup fallback commands via nudge.
	HooksInformational bool `json:"hooks_informational,omitempty"`

	// HooksUseSettingsDir indicates the agent supports a separate settings directory
	// (e.g., Claude's --settings flag). When true, hook templates are installed in
	// settingsDir; when false, they're installed in workDir.
	HooksUseSettingsDir bool `json:"hooks_use_settings_dir,omitempty"`

	// ReadyPromptPrefix is the prompt prefix for tmux readiness detection (e.g., "❯ ").
	// Empty means delay-based detection only.
	ReadyPromptPrefix string `json:"ready_prompt_prefix,omitempty"`

	// ReadyDelayMs is the delay-based readiness fallback in milliseconds.
	ReadyDelayMs int `json:"ready_delay_ms,omitempty"`

	// InstructionsFile is the instructions file for this agent (e.g., "CLAUDE.md", "AGENTS.md").
	// Defaults to "AGENTS.md" if empty.
	InstructionsFile string `json:"instructions_file,omitempty"`

	// EmitsPermissionWarning indicates the agent shows a bypass-permissions warning on startup
	// that needs to be acknowledged via tmux.
	EmitsPermissionWarning bool `json:"emits_permission_warning,omitempty"`

	// HasTurnBoundaryDrain indicates the agent's hooks system drains the nudge
	// queue on every turn boundary (like Claude's UserPromptSubmit hook). When
	// false, a background nudge-poller process is started to periodically drain
	// the queue and inject via tmux.
	HasTurnBoundaryDrain bool `json:"has_turn_boundary_drain,omitempty"`

	// EscapeCancelsRequest indicates that sending an Escape keystroke to this
	// agent cancels its in-flight generation. NudgeSession normally sends
	// Escape (step 5) to exit vim INSERT mode — harmless for bash/Claude Code,
	// but destructive for agents like Gemini CLI where Escape aborts the
	// active request. When true, NudgeSessionWithOpts skips the Escape
	// keystroke and the 600ms readline timeout that follows it.
	EscapeCancelsRequest bool `json:"escape_cancels_request,omitempty"`

	// ACP is the configuration for ACP (Agent Communication Protocol) support.
	// nil means the agent does not support ACP.
	ACP *ACPConfig `json:"acp,omitempty"`
}

// ACPConfig contains configuration for ACP (Agent Communication Protocol) support.
type ACPConfig struct {
	// Mode specifies how ACP is invoked:
	// - "subcommand" (default): Agent has ACP as a subcommand (e.g., "opencode acp")
	// - "native": Agent is a native ACP binary (e.g., "claude-agent-acp")
	// - "flag": Agent uses a flag to enable ACP (e.g., "gemini --acp")
	// If empty, defaults to "subcommand" if Command is set, otherwise "native".
	Mode string `json:"mode,omitempty"`

	// Command is the subcommand for ACP (e.g., "acp").
	// Used when Mode is "subcommand".
	Command string `json:"command,omitempty"`

	// Args are additional arguments for the ACP command.
	// For "subcommand" mode, appended after the subcommand.
	// For "flag" mode, these are the flags to enable ACP.
	// For "native" mode, ignored (binary is already ACP).
	Args []string `json:"args,omitempty"`
}

// ACP mode constants.
const (
	ACPModeSubcommand = "subcommand" // Agent has ACP as subcommand (e.g., "opencode acp")
	ACPModeNative     = "native"     // Agent is native ACP binary (e.g., "claude-agent-acp")
	ACPModeFlag       = "flag"       // Agent uses flag to enable ACP (e.g., "gemini --acp")
)

// NonInteractiveConfig contains settings for running agents non-interactively.
type NonInteractiveConfig struct {
	// Subcommand is the subcommand for non-interactive execution (e.g., "exec" for codex).
	Subcommand string `json:"subcommand,omitempty"`

	// PromptFlag is the flag for passing prompts (e.g., "-p" for gemini).
	PromptFlag string `json:"prompt_flag,omitempty"`

	// OutputFlag is the flag for structured output (e.g., "--json", "--output-format json").
	OutputFlag string `json:"output_flag,omitempty"`
}

// AgentRegistry contains all known agent presets.
// Can be loaded from JSON config or use built-in defaults.
type AgentRegistry struct {
	// Version is the schema version for the registry.
	Version int `json:"version"`

	// Agents maps agent names to their configurations.
	Agents map[string]*AgentPresetInfo `json:"agents"`
}

// CurrentAgentRegistryVersion is the current schema version.
const CurrentAgentRegistryVersion = 1

// builtinPresets contains the default presets for supported agents.
// Each preset is the single source of truth for its agent's behavior.
var builtinPresets = map[AgentPreset]*AgentPresetInfo{
	AgentClaude: {
		Name:                AgentClaude,
		Command:             "claude",
		Args:                []string{"--dangerously-skip-permissions"},
		ProcessNames:        []string{"node", "claude"}, // Claude runs as Node.js
		SessionIDEnv:        "CLAUDE_SESSION_ID",
		ResumeFlag:          "--resume",
		ContinueFlag:        "--continue",
		ResumeStyle:         "flag",
		SupportsHooks:       true,
		SupportsForkSession: true,
		NonInteractive:      nil, // Claude is native non-interactive
		// Runtime defaults
		PromptMode:             "arg",
		ConfigDirEnv:           "CLAUDE_CONFIG_DIR",
		ConfigDir:              ".claude",
		HooksProvider:          "claude",
		HooksDir:               ".claude",
		HooksSettingsFile:      "settings.json",
		HooksUseSettingsDir:    true,
		ReadyPromptPrefix:      "❯ ",
		ReadyDelayMs:           10000,
		InstructionsFile:       "CLAUDE.md",
		EmitsPermissionWarning: true,
		HasTurnBoundaryDrain:   true,
	},
	AgentGemini: {
		Name:                AgentGemini,
		Command:             "gemini",
		Args:                []string{"--approval-mode", "yolo"},
		ProcessNames:        []string{"gemini"}, // Gemini CLI binary
		SessionIDEnv:        "GEMINI_SESSION_ID",
		ResumeFlag:          "--resume",
		ResumeStyle:         "flag",
		SupportsHooks:       true,
		SupportsForkSession: false,
		NonInteractive: &NonInteractiveConfig{
			PromptFlag: "-p",
			OutputFlag: "--output-format json",
		},
		// Runtime defaults
		PromptMode:           "arg",
		ConfigDir:            ".gemini",
		HooksProvider:        "gemini",
		HooksDir:             ".gemini",
		HooksSettingsFile:    "settings.json",
		ReadyDelayMs:         5000,
		InstructionsFile:     "AGENTS.md",
		EscapeCancelsRequest: true, // Gemini CLI uses Escape to abort active generation
	},
	AgentCodex: {
		Name:                AgentCodex,
		Command:             "codex",
		Args:                []string{"--dangerously-bypass-approvals-and-sandbox"},
		ProcessNames:        []string{"codex"}, // Codex CLI binary
		SessionIDEnv:        "",                // Codex captures from JSONL output
		ResumeFlag:          "resume",
		ResumeStyle:         "subcommand",
		SupportsHooks:       false, // Use env/files instead
		SupportsForkSession: false,
		NonInteractive: &NonInteractiveConfig{
			Subcommand: "exec",
			OutputFlag: "--json",
		},
		// Runtime defaults
		PromptMode:        "none",
		ReadyPromptPrefix: "› ",
		ReadyDelayMs:      3000,
		InstructionsFile:  "AGENTS.md",
	},
	AgentCursor: {
		Name:                AgentCursor,
		Command:             "cursor-agent",
		Args:                []string{"-f"}, // Force mode (YOLO equivalent), -p requires prompt
		ProcessNames:        []string{"cursor-agent"},
		SessionIDEnv:        "", // Uses --resume with chatId directly
		ResumeFlag:          "--resume",
		ResumeStyle:         "flag",
		SupportsHooks:       true,
		SupportsForkSession: false,
		NonInteractive: &NonInteractiveConfig{
			PromptFlag: "-p",
			OutputFlag: "--output-format json",
		},
		// Runtime defaults
		PromptMode:        "arg",
		ConfigDir:         ".cursor",
		HooksProvider:     "cursor",
		HooksDir:          ".cursor",
		HooksSettingsFile: "hooks.json",
		InstructionsFile:  "AGENTS.md",
	},
	AgentAuggie: {
		Name:                AgentAuggie,
		Command:             "auggie",
		Args:                []string{"--allow-indexing"},
		ProcessNames:        []string{"auggie"},
		SessionIDEnv:        "",
		ResumeFlag:          "--resume",
		ResumeStyle:         "flag",
		SupportsHooks:       false,
		SupportsForkSession: false,
		// Runtime defaults
		PromptMode:       "arg",
		InstructionsFile: "AGENTS.md",
	},
	AgentAmp: {
		Name:                AgentAmp,
		Command:             "amp",
		Args:                []string{"--dangerously-allow-all", "--no-ide"},
		ProcessNames:        []string{"amp"},
		SessionIDEnv:        "",
		ResumeFlag:          "threads continue",
		ResumeStyle:         "subcommand", // 'amp threads continue <threadId>'
		SupportsHooks:       false,
		SupportsForkSession: false,
		// Runtime defaults
		PromptMode:       "arg",
		InstructionsFile: "AGENTS.md",
	},
	AgentOpenCode: {
		Name:    AgentOpenCode,
		Command: "opencode",
		Args:    []string{}, // No CLI flags needed, YOLO via OPENCODE_PERMISSION env
		Env: map[string]string{
			// Auto-approve all tool calls (equivalent to --dangerously-skip-permissions)
			"OPENCODE_PERMISSION": `{"*":"allow"}`,
		},
		ProcessNames:        []string{"opencode", "node", "bun"}, // Runs as Node.js or Bun
		SessionIDEnv:        "",                                  // OpenCode manages sessions internally
		ResumeFlag:          "",                                  // No resume support yet
		ResumeStyle:         "",
		SupportsHooks:       true, // Uses .opencode/plugins/gastown.js
		SupportsForkSession: false,
		NonInteractive: &NonInteractiveConfig{
			Subcommand: "run",
			OutputFlag: "--format json",
		},
		// Runtime defaults
		PromptMode:        "arg",
		ConfigDir:         ".opencode",
		HooksProvider:     "opencode",
		HooksDir:          ".opencode/plugins",
		HooksSettingsFile: "gastown.js",
		ReadyDelayMs:      8000,
		InstructionsFile:  "AGENTS.md",
		// ACP support
		ACP: &ACPConfig{
			Command: "acp",
		},
	},
	AgentCopilot: {
		Name:                AgentCopilot,
		Command:             "copilot",
		Args:                []string{"--yolo"},
		ProcessNames:        []string{"copilot"}, // Copilot CLI binary (Node.js but reports as "copilot")
		SessionIDEnv:        "",                  // Session IDs stored on disk (~/.copilot/session-state/), not in env
		ResumeFlag:          "--resume",
		ContinueFlag:        "--continue", // GA: resumes most recent session without picker
		ResumeStyle:         "flag",
		SupportsHooks:       true, // Copilot CLI supports .github/hooks/*.json lifecycle hooks
		SupportsForkSession: false,
		NonInteractive: &NonInteractiveConfig{
			PromptFlag: "-p",
		},
		// Runtime defaults
		PromptMode:         "arg",
		ConfigDirEnv:       "COPILOT_HOME", // GA: overrides ~/.copilot/ config directory
		ConfigDir:          ".copilot",
		HooksProvider:      "copilot",
		HooksDir:           ".github/hooks",
		HooksSettingsFile:  "gastown.json",
		HooksInformational: false,
		ReadyPromptPrefix:  "",   // GA: no ❯ prompt; Copilot uses hint text, not a detectable prefix
		ReadyDelayMs:       5000, // Delay-based readiness detection (no prompt prefix)
		InstructionsFile:   "AGENTS.md",
	},
	AgentPi: {
		Name:                AgentPi,
		Command:             "pi",
		Args:                []string{"-e", ".pi/extensions/gastown-hooks.js"},
		ProcessNames:        []string{"pi", "node", "bun"}, // Pi runs as Node.js
		SessionIDEnv:        "PI_SESSION_ID",
		ResumeFlag:          "", // No resume support yet
		ResumeStyle:         "",
		SupportsHooks:       true, // Uses .pi/extensions/gastown-hooks.js
		HooksProvider:       "pi",
		HooksDir:            ".pi/extensions",
		HooksSettingsFile:   "gastown-hooks.js",
		SupportsForkSession: false,
		NonInteractive: &NonInteractiveConfig{
			PromptFlag: "-p",
			OutputFlag: "--no-session",
		},
		// Pi's Node.js TUI takes several seconds to initialize before it can
		// receive tmux input. Without a readiness delay, the startup nudge
		// arrives before the TUI is ready and gets dropped silently.
		PromptMode:   "arg",
		ReadyDelayMs: 8000,
	},
	AgentOmp: {
		Name:                AgentOmp,
		Command:             "omp",
		Args:                []string{"--hook", ".omp/hooks/gastown-hook.ts"},
		ProcessNames:        []string{"omp", "node", "bun"},
		SessionIDEnv:        "OMP_SESSION_ID",
		SupportsHooks:       true,
		HooksProvider:       "omp",
		HooksDir:            ".omp/hooks",
		HooksSettingsFile:   "gastown-hook.ts",
		SupportsForkSession: false,
		NonInteractive: &NonInteractiveConfig{
			PromptFlag: "--prompt",
		},
	},
}

// Registry state with proper synchronization.
var (
	// registryMu protects all registry state.
	registryMu sync.RWMutex
	// globalRegistry is the merged registry of built-in and user-defined agents.
	globalRegistry *AgentRegistry
	// loadedPaths tracks which config files have been loaded to avoid redundant reads.
	loadedPaths = make(map[string]bool)
	// registryInitialized tracks if builtins have been copied.
	registryInitialized bool
)

// initRegistry initializes the global registry with built-in presets.
// Caller must hold registryMu write lock.
func initRegistryLocked() {
	if registryInitialized {
		return
	}
	globalRegistry = &AgentRegistry{
		Version: CurrentAgentRegistryVersion,
		Agents:  make(map[string]*AgentPresetInfo),
	}
	// Copy built-in presets
	for name, preset := range builtinPresets {
		globalRegistry.Agents[string(name)] = preset
	}
	registryInitialized = true
}

// loadAgentRegistryFromPath loads agent definitions from a JSON file and merges with built-ins.
// Caller must hold registryMu write lock.
func loadAgentRegistryFromPathLocked(path string) error {
	initRegistryLocked()

	if loadedPaths[path] {
		return nil
	}

	data, err := os.ReadFile(path) //nolint:gosec // G304: path is from config
	if err != nil {
		if os.IsNotExist(err) {
			// Don't cache non-existent paths — the file may be created later
			// and we need to pick it up on the next load call.
			return nil
		}
		return err
	}

	var userRegistry AgentRegistry
	if err := json.Unmarshal(data, &userRegistry); err != nil {
		return err
	}

	for name, preset := range userRegistry.Agents {
		preset.Name = AgentPreset(name)
		globalRegistry.Agents[name] = preset
	}

	loadedPaths[path] = true
	return nil
}

// LoadAgentRegistry loads agent definitions from a JSON file and merges with built-ins.
// User-defined agents override built-in presets with the same name.
// This function caches loaded paths to avoid redundant file reads.
func LoadAgentRegistry(path string) error {
	registryMu.Lock()
	defer registryMu.Unlock()
	return loadAgentRegistryFromPathLocked(path)
}

// DefaultAgentRegistryPath returns the default path for agent registry.
// Located alongside other town settings.
func DefaultAgentRegistryPath(townRoot string) string {
	return filepath.Join(townRoot, "settings", "agents.json")
}

// DefaultRigAgentRegistryPath returns the default path for rig-level agent registry.
// Located in <rig>/settings/agents.json.
func DefaultRigAgentRegistryPath(rigPath string) string {
	return filepath.Join(rigPath, "settings", "agents.json")
}

// RigAgentRegistryPath returns the path for rig-level agent registry.
// Alias for DefaultRigAgentRegistryPath for consistency with other path functions.
func RigAgentRegistryPath(rigPath string) string {
	return DefaultRigAgentRegistryPath(rigPath)
}

// LoadRigAgentRegistry loads agent definitions from a rig-level JSON file and merges with built-ins.
// This function works similarly to LoadAgentRegistry but for rig-level configurations.
func LoadRigAgentRegistry(path string) error {
	registryMu.Lock()
	defer registryMu.Unlock()
	return loadAgentRegistryFromPathLocked(path)
}

// GetAgentPreset returns the preset info for a given agent name.
// Returns nil if the preset is not found.
func GetAgentPreset(name AgentPreset) *AgentPresetInfo {
	registryMu.Lock()
	initRegistryLocked()
	defer registryMu.Unlock()
	return globalRegistry.Agents[string(name)]
}

// GetAgentPresetByName returns the preset info by string name.
// Returns nil if not found, allowing caller to fall back to defaults.
func GetAgentPresetByName(name string) *AgentPresetInfo {
	registryMu.Lock()
	initRegistryLocked()
	defer registryMu.Unlock()
	return globalRegistry.Agents[name]
}

// ListAgentPresets returns all known agent preset names.
func ListAgentPresets() []string {
	registryMu.Lock()
	initRegistryLocked()
	defer registryMu.Unlock()
	names := make([]string, 0, len(globalRegistry.Agents))
	for name := range globalRegistry.Agents {
		names = append(names, name)
	}
	return names
}

// DefaultAgentPreset returns the default agent preset (Claude).
func DefaultAgentPreset() AgentPreset {
	return AgentClaude
}

// RuntimeConfigFromPreset creates a RuntimeConfig from an agent preset.
// This provides the basic Command/Args/Env; additional fields from AgentPresetInfo
// can be accessed separately for extended functionality.
func RuntimeConfigFromPreset(preset AgentPreset) *RuntimeConfig {
	info := GetAgentPreset(preset)
	if info == nil {
		// Fall back to Claude defaults
		return DefaultRuntimeConfig()
	}

	// Copy Env map to avoid mutation
	var envCopy map[string]string
	if len(info.Env) > 0 {
		envCopy = make(map[string]string, len(info.Env))
		for k, v := range info.Env {
			envCopy[k] = v
		}
	}

	rc := &RuntimeConfig{
		Provider: string(info.Name),
		Command:  info.Command,
		Args:     append([]string(nil), info.Args...), // Copy to avoid mutation
		Env:      envCopy,
	}

	// Resolve command path for claude preset (handles alias installations)
	// Uses resolveClaudePath() from types.go which finds ~/.claude/local/claude
	if preset == AgentClaude && rc.Command == "claude" {
		rc.Command = resolveClaudePath()
	}

	return normalizeRuntimeConfig(rc)
}

// BuildResumeCommand builds a command to resume an agent session.
// Returns the full command string including any YOLO/autonomous flags.
// If sessionID is empty or the agent doesn't support resume, returns empty string.
func BuildResumeCommand(agentName, sessionID string) string {
	if sessionID == "" {
		return ""
	}

	info := GetAgentPresetByName(agentName)
	if info == nil || info.ResumeFlag == "" {
		return ""
	}

	// Build base command with args
	args := append([]string(nil), info.Args...)

	// Add resume based on style
	switch info.ResumeStyle {
	case "subcommand":
		// e.g., "codex resume <session_id> --dangerously-bypass-approvals-and-sandbox"
		return info.Command + " " + info.ResumeFlag + " " + sessionID + " " + strings.Join(args, " ")
	case "flag":
		fallthrough
	default:
		// e.g., "claude --dangerously-skip-permissions --resume <session_id>"
		args = append(args, info.ResumeFlag, sessionID)
		return info.Command + " " + strings.Join(args, " ")
	}
}

// SupportsSessionResume checks if an agent supports session resumption.
func SupportsSessionResume(agentName string) bool {
	info := GetAgentPresetByName(agentName)
	return info != nil && info.ResumeFlag != ""
}

// GetSessionIDEnvVar returns the environment variable name for storing session IDs
// for a given agent. Returns empty string if the agent doesn't use env vars for this.
func GetSessionIDEnvVar(agentName string) string {
	info := GetAgentPresetByName(agentName)
	if info == nil {
		return ""
	}
	return info.SessionIDEnv
}

// GetProcessNames returns the process names used to detect if an agent is running.
// Used by tmux.IsAgentRunning to check pane_current_command.
// Returns ["node"] for Claude (default) if agent is not found or has no ProcessNames.
func GetProcessNames(agentName string) []string {
	info := GetAgentPresetByName(agentName)
	if info == nil || len(info.ProcessNames) == 0 {
		// Default to Claude's process names for backwards compatibility
		return []string{"node", "claude"}
	}
	return info.ProcessNames
}

// ResolveProcessNames determines the correct process names for liveness detection
// given an agent name and the actual command binary. This handles custom agents
// that shadow built-in preset names (e.g., a custom "codex" agent that runs
// "opencode" instead of the built-in "codex" binary).
//
// Resolution order:
//  1. If agentName matches a built-in preset AND the preset's Command matches
//     the actual command → use the preset's ProcessNames (no mismatch).
//  2. Otherwise, find a built-in preset whose Command matches the actual command
//     and use its ProcessNames (custom agent using a known launcher).
//  3. Fallback: [command] (fully custom binary).
func ResolveProcessNames(agentName, command string) []string {
	registryMu.Lock()
	initRegistryLocked()
	defer registryMu.Unlock()

	// Normalize command to basename for comparison. Commands may be
	// path-resolved (e.g., "/home/user/.claude/local/claude" from
	// resolveClaudePath), but built-in presets store bare names ("claude").
	// Process matching (processMatchesNames, pgrep) also uses basenames.
	cmdBase := command
	if command != "" {
		cmdBase = filepath.Base(command)
	}
	unwrappedCmdBase := strings.TrimPrefix(cmdBase, "gt-")

	// Check if agentName matches a built-in/registered preset with matching command.
	// Compare against both the raw command and basename to handle registry entries
	// that store absolute-path commands (e.g., "/opt/bin/my-tool").
	if info, ok := globalRegistry.Agents[agentName]; ok && len(info.ProcessNames) > 0 {
		if info.Command == command ||
			info.Command == cmdBase ||
			filepath.Base(info.Command) == cmdBase ||
			(info.Command == unwrappedCmdBase && strings.HasPrefix(cmdBase, "gt-")) ||
			cmdBase == "" {
			return info.ProcessNames
		}
	}

	// Agent name doesn't match or command differs — look up by command
	if cmdBase != "" {
		for _, info := range globalRegistry.Agents {
			if len(info.ProcessNames) == 0 {
				continue
			}
			if info.Command == command ||
				filepath.Base(info.Command) == cmdBase ||
				(strings.HasPrefix(cmdBase, "gt-") && filepath.Base(info.Command) == unwrappedCmdBase) {
				return info.ProcessNames
			}
		}
		// Unknown command — use the binary basename itself
		return []string{cmdBase}
	}

	// No command provided, agent not in registry — Claude defaults
	return []string{"node", "claude"}
}

// MergeWithPreset applies preset defaults to a RuntimeConfig.
// User-specified values take precedence over preset defaults.
// Returns a new RuntimeConfig without modifying the original.
func (rc *RuntimeConfig) MergeWithPreset(preset AgentPreset) *RuntimeConfig {
	if rc == nil {
		return RuntimeConfigFromPreset(preset)
	}

	info := GetAgentPreset(preset)
	if info == nil {
		return rc
	}

	result := &RuntimeConfig{
		Command:       rc.Command,
		Args:          append([]string(nil), rc.Args...),
		InitialPrompt: rc.InitialPrompt,
	}

	// Apply preset defaults only if not overridden
	if result.Command == "" {
		result.Command = info.Command
	}
	if len(result.Args) == 0 {
		result.Args = append([]string(nil), info.Args...)
	}

	return result
}

// IsKnownPreset checks if a string is a known agent preset name.
func IsKnownPreset(name string) bool {
	registryMu.Lock()
	initRegistryLocked()
	defer registryMu.Unlock()
	_, ok := globalRegistry.Agents[name]
	return ok
}

// SaveAgentRegistry writes the agent registry to a file.
func SaveAgentRegistry(path string, registry *AgentRegistry) error {
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return err
	}

	data, err := json.MarshalIndent(registry, "", "  ")
	if err != nil {
		return err
	}

	return os.WriteFile(path, data, 0644) //nolint:gosec // G306: config file
}

// NewExampleAgentRegistry creates an example registry with comments.
func NewExampleAgentRegistry() *AgentRegistry {
	return &AgentRegistry{
		Version: CurrentAgentRegistryVersion,
		Agents: map[string]*AgentPresetInfo{
			// Include one example custom agent
			"my-custom-agent": {
				Name:         "my-custom-agent",
				Command:      "my-agent-cli",
				Args:         []string{"--autonomous", "--no-confirm"},
				SessionIDEnv: "MY_AGENT_SESSION_ID",
				ResumeFlag:   "--resume",
				ResumeStyle:  "flag",
				NonInteractive: &NonInteractiveConfig{
					PromptFlag: "-m",
					OutputFlag: "--json",
				},
			},
		},
	}
}

// ResetRegistryForTesting clears all registry state.
// This is intended for use in tests only to ensure test isolation.
func ResetRegistryForTesting() {
	registryMu.Lock()
	defer registryMu.Unlock()
	globalRegistry = nil
	loadedPaths = make(map[string]bool)
	registryInitialized = false
}

// RegisterAgentForTesting adds a custom agent preset to the registry.
// The registry is initialized first if needed. Intended for test use only.
func RegisterAgentForTesting(name string, info AgentPresetInfo) {
	registryMu.Lock()
	initRegistryLocked()
	defer registryMu.Unlock()
	globalRegistry.Agents[name] = &info
}

// ResolveACPConfig determines the correct ACP configuration for an agent
// given its name and the actual command binary. This handles custom agents
// that use an ACP-compatible launcher like "opencode".
func ResolveACPConfig(agentName, command string) *ACPConfig {
	registryMu.Lock()
	initRegistryLocked()
	defer registryMu.Unlock()

	// 1. Check if agentName matches a registered preset with ACP config.
	if info, ok := globalRegistry.Agents[agentName]; ok && info.ACP != nil && info.ACP.Command != "" {
		return info.ACP
	}

	// 2. Otherwise, find a registered preset whose Command matches and has ACP.
	if command != "" {
		cmdBase := filepath.Base(command)
		for _, info := range globalRegistry.Agents {
			if (info.Command == command || filepath.Base(info.Command) == cmdBase) && info.ACP != nil && info.ACP.Command != "" {
				return info.ACP
			}
		}
	}

	return nil
}

// SupportsACP checks if an agent supports ACP (Agent Communication Protocol).
// Returns true if the agent has ACP configured.
func SupportsACP(agentName string) bool {
	info := GetAgentPresetByName(agentName)
	if info == nil {
		return false
	}
	if info.ACP != nil && info.ACP.Command != "" {
		return true
	}
	// Fallback: check if the command itself supports ACP
	return ResolveACPConfig(agentName, info.Command) != nil
}

// GetACPConfig returns the ACP configuration for an agent.
// Returns nil if the agent doesn't support ACP.
func GetACPConfig(agentName string) *ACPConfig {
	info := GetAgentPresetByName(agentName)
	if info == nil {
		return nil
	}
	return ResolveACPConfig(agentName, info.Command)
}

// GetACPCommand returns the ACP subcommand for an agent.
// Returns empty string if the agent doesn't support ACP.
func GetACPCommand(agentName string) string {
	config := GetACPConfig(agentName)
	if config == nil {
		return ""
	}
	return config.Command
}

// GetACPArgs returns the ACP arguments for an agent.
// Returns nil if the agent doesn't support ACP.
func GetACPArgs(agentName string) []string {
	config := GetACPConfig(agentName)
	if config == nil {
		return nil
	}
	return config.Args
}

// RuntimeConfigSupportsACP checks if a RuntimeConfig supports ACP.
// This is used for custom agents defined in config.json that may have
// their own ACP configuration or inherit from a preset.
// Returns true if the RuntimeConfig has ACP configured.
//
// ACP can be configured in three ways:
//  1. Native mode: { "command": "claude-agent-acp", "acp": { "mode": "native" } }
//     The binary is already an ACP adapter, no transformation needed.
//  2. Subcommand pattern: { "command": "opencode", "acp": { "command": "acp" } }
//     Results in: opencode acp
//  3. Flag pattern: { "command": "gemini", "acp": { "args": ["--acp"] } }
//     Results in: gemini --acp
func RuntimeConfigSupportsACP(rc *RuntimeConfig) bool {
	if rc == nil {
		return false
	}
	// If the RuntimeConfig has explicit ACP config, use it
	if rc.ACP != nil {
		// Native mode: the binary is already an ACP adapter
		if rc.ACP.Mode == ACPModeNative {
			return true
		}
		// Subcommand mode: has a subcommand to invoke ACP
		if rc.ACP.Command != "" {
			return true
		}
		// Flag mode: has flags to enable ACP
		if len(rc.ACP.Args) > 0 {
			return true
		}
	}
	// Fallback: check if the command matches a preset with ACP support
	if rc.Command != "" {
		return ResolveACPConfig("", rc.Command) != nil
	}
	return false
}

// GetACPConfigFromRuntime returns the ACP configuration from a RuntimeConfig.
// This is used for custom agents defined in config.json that may have
// their own ACP configuration or inherit from a preset.
// Returns nil if the RuntimeConfig doesn't support ACP.
//
// ACP can be configured in three ways:
//  1. Native mode: { "command": "claude-agent-acp", "acp": { "mode": "native" } }
//     The binary is already an ACP adapter, no transformation needed.
//  2. Subcommand pattern: { "command": "opencode", "acp": { "command": "acp" } }
//     Results in: opencode acp
//  3. Flag pattern: { "command": "gemini", "acp": { "args": ["--experimental-acp"] } }
//     Results in: gemini --experimental-acp
func GetACPConfigFromRuntime(rc *RuntimeConfig) *ACPConfig {
	if rc == nil {
		return nil
	}
	// If the RuntimeConfig has explicit ACP config, use it
	if rc.ACP != nil {
		// Native mode: the binary is already an ACP adapter
		if rc.ACP.Mode == ACPModeNative {
			return rc.ACP
		}
		// Subcommand mode: has a subcommand to invoke ACP
		if rc.ACP.Command != "" {
			return rc.ACP
		}
		// Flag mode: has flags to enable ACP
		if len(rc.ACP.Args) > 0 {
			return rc.ACP
		}
	}
	// Fallback: check if the command matches a preset with ACP support
	if rc.Command != "" {
		return ResolveACPConfig("", rc.Command)
	}
	return nil
}
// Package config provides configuration types and serialization for Gas Town.
package config

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/steveyegge/gastown/internal/scheduler/capacity"
)

// TownConfig represents the main town identity (mayor/town.json).
type TownConfig struct {
	Type       string    `json:"type"`                  // "town"
	Version    int       `json:"version"`               // schema version
	Name       string    `json:"name"`                  // town identifier (internal)
	Owner      string    `json:"owner,omitempty"`       // owner email (entity identity)
	PublicName string    `json:"public_name,omitempty"` // public display name
	CreatedAt  time.Time `json:"created_at"`
}

// MayorConfig represents town-level behavioral configuration (mayor/config.json).
// This is separate from TownConfig (identity) to keep configuration concerns distinct.
type MayorConfig struct {
	Type            string           `json:"type"`                        // "mayor-config"
	Version         int              `json:"version"`                     // schema version
	Theme           *TownThemeConfig `json:"theme,omitempty"`             // global theme settings
	Daemon          *DaemonConfig    `json:"daemon,omitempty"`            // daemon settings
	Deacon          *DeaconConfig    `json:"deacon,omitempty"`            // deacon settings
	DefaultCrewName string           `json:"default_crew_name,omitempty"` // default crew name for new rigs
}

// CurrentTownSettingsVersion is the current schema version for TownSettings.
const CurrentTownSettingsVersion = 1

// TownSettings represents town-level behavioral configuration (settings/config.json).
// This contains agent configuration that applies to all rigs unless overridden.
type TownSettings struct {
	Type    string `json:"type"`    // "town-settings"
	Version int    `json:"version"` // schema version

	// CLITheme controls CLI output color scheme.
	// Values: "dark", "light", "auto" (default).
	// "auto" lets the terminal emulator's background color guide the choice.
	// Can be overridden by GT_THEME environment variable.
	CLITheme string `json:"cli_theme,omitempty"`

	// DefaultAgent is the name of the agent preset to use by default.
	// Can be a built-in preset ("claude", "gemini", "codex", "cursor", "auggie", "amp", "opencode", "copilot")
	// or a custom agent name defined in settings/agents.json.
	// Default: "claude"
	DefaultAgent string `json:"default_agent,omitempty"`

	// Agents defines custom agent configurations or overrides.
	// Keys are agent names that can be referenced by DefaultAgent or rig settings.
	// Values override or extend the built-in presets.
	// Example: {"gemini": {"command": "/custom/path/to/gemini"}}
	Agents map[string]*RuntimeConfig `json:"agents,omitempty"`

	// RoleAgents maps role names to agent aliases for per-role model selection.
	// Keys are role names: "mayor", "deacon", "witness", "refinery", "polecat", "crew".
	// Values are agent names (built-in presets or custom agents defined in Agents).
	// This allows cost optimization by using different models for different roles.
	// Example: {"mayor": "claude-opus", "witness": "claude-haiku", "polecat": "claude-sonnet"}
	RoleAgents map[string]string `json:"role_agents,omitempty"`

	// CrewAgents maps individual crew worker names to agent aliases at the town level.
	// This allows town-wide per-crew agent assignment without modifying each rig's config.
	// Resolution: --agent flag > rig WorkerAgents > town CrewAgents > role agents > defaults.
	// Example: {"bob": "codex", "alice": "claude"}
	CrewAgents map[string]string `json:"crew_agents,omitempty"`

	// AgentEmailDomain is the domain used for agent git identity emails.
	// Agent addresses like "gastown/crew/jack" become "gastown.crew.jack@{domain}".
	// Default: "gastown.local"
	AgentEmailDomain string `json:"agent_email_domain,omitempty"`

	// WebTimeouts configures command execution timeouts for the web dashboard.
	WebTimeouts *WebTimeoutsConfig `json:"web_timeouts,omitempty"`

	// WorkerStatus configures activity-age thresholds for worker status classification.
	WorkerStatus *WorkerStatusConfig `json:"worker_status,omitempty"`

	// FeedCurator configures event deduplication and aggregation windows.
	FeedCurator *FeedCuratorConfig `json:"feed_curator,omitempty"`

	// Convoy configures convoy behavior settings.
	Convoy *ConvoyConfig `json:"convoy,omitempty"`

	// CostTier tracks which cost tier preset was applied (informational).
	// Actual model assignments live in RoleAgents and Agents.
	// Values: "standard", "economy", "budget", or empty for custom configs.
	CostTier string `json:"cost_tier,omitempty"`

	// Scheduler configures the capacity scheduler for polecat dispatch.
	Scheduler *capacity.SchedulerConfig `json:"scheduler,omitempty"`

	// Operational configures operational thresholds (timeouts, retries, intervals).
	// These were previously hardcoded as Go constants throughout the codebase.
	// All values are optional — omitted values use compiled-in defaults.
	Operational *OperationalConfig `json:"operational,omitempty"`

	// DisabledPatrols lists patrol names to disable at the town level.
	// This provides a simple way to turn off individual daemon patrol dogs
	// without editing mayor/daemon.json. Patrol names match the keys used
	// in daemon.json patrols section (e.g., "deacon", "witness", "refinery",
	// "doctor_dog", "compactor_dog", "checkpoint_dog", "wisp_reaper",
	// "dolt_remotes", "dolt_backup", "jsonl_git_backup", "scheduled_maintenance",
	// "main_branch_test", "handler").
	// Example: ["doctor_dog", "compactor_dog"]
	DisabledPatrols []string `json:"disabled_patrols,omitempty"`
}

// NewTownSettings creates a new TownSettings with defaults.
func NewTownSettings() *TownSettings {
	return &TownSettings{
		Type:         "town-settings",
		Version:      CurrentTownSettingsVersion,
		DefaultAgent: "claude",
		Agents:       make(map[string]*RuntimeConfig),
		RoleAgents:   make(map[string]string),
	}
}

// WebTimeoutsConfig configures command execution timeouts for the web dashboard.
type WebTimeoutsConfig struct {
	// CmdTimeout is the timeout for bd (beads) commands. Default: "15s".
	CmdTimeout string `json:"cmd_timeout,omitempty"`
	// GhCmdTimeout is the timeout for GitHub API commands. Default: "10s".
	GhCmdTimeout string `json:"gh_cmd_timeout,omitempty"`
	// TmuxCmdTimeout is the timeout for tmux queries. Default: "2s".
	TmuxCmdTimeout string `json:"tmux_cmd_timeout,omitempty"`
	// FetchTimeout is the maximum time for all dashboard data fetches. Default: "8s".
	FetchTimeout string `json:"fetch_timeout,omitempty"`
	// DefaultRunTimeout is the default timeout for /api/run commands. Default: "30s".
	DefaultRunTimeout string `json:"default_run_timeout,omitempty"`
	// MaxRunTimeout is the maximum allowed timeout for /api/run commands. Default: "60s".
	MaxRunTimeout string `json:"max_run_timeout,omitempty"`
}

// DefaultWebTimeoutsConfig returns a WebTimeoutsConfig with sensible defaults.
func DefaultWebTimeoutsConfig() *WebTimeoutsConfig {
	return &WebTimeoutsConfig{
		CmdTimeout:        "15s",
		GhCmdTimeout:      "10s",
		TmuxCmdTimeout:    "2s",
		FetchTimeout:      "8s",
		DefaultRunTimeout: "30s",
		MaxRunTimeout:     "60s",
	}
}

// WorkerStatusConfig configures activity-age thresholds for worker status classification.
type WorkerStatusConfig struct {
	// StaleThreshold is the activity age after which a worker is considered "stale".
	// Default: "5m".
	StaleThreshold string `json:"stale_threshold,omitempty"`
	// StuckThreshold is the activity age after which a worker is considered "stuck".
	// Default: "30m".
	StuckThreshold string `json:"stuck_threshold,omitempty"`
	// HeartbeatFreshThreshold is the max age for a Deacon heartbeat to be considered fresh.
	// Default: "5m".
	HeartbeatFreshThreshold string `json:"heartbeat_fresh_threshold,omitempty"`
	// MayorActiveThreshold is the max session inactivity for the Mayor to be considered active.
	// Default: "5m".
	MayorActiveThreshold string `json:"mayor_active_threshold,omitempty"`
}

// DefaultWorkerStatusConfig returns a WorkerStatusConfig with sensible defaults.
func DefaultWorkerStatusConfig() *WorkerStatusConfig {
	return &WorkerStatusConfig{
		StaleThreshold:          "5m",
		StuckThreshold:          "30m",
		HeartbeatFreshThreshold: "5m",
		MayorActiveThreshold:    "5m",
	}
}

// FeedCuratorConfig configures event deduplication and aggregation windows.
type FeedCuratorConfig struct {
	// DoneDedupeWindow is the time window for deduplicating repeated done events.
	// Default: "10s".
	DoneDedupeWindow string `json:"done_dedupe_window,omitempty"`
	// SlingAggregateWindow is the time window for aggregating sling events.
	// Default: "30s".
	SlingAggregateWindow string `json:"sling_aggregate_window,omitempty"`
	// MinAggregateCount is the minimum number of events to trigger aggregation.
	// Default: 3.
	MinAggregateCount int `json:"min_aggregate_count,omitempty"`
}

// DefaultFeedCuratorConfig returns a FeedCuratorConfig with sensible defaults.
func DefaultFeedCuratorConfig() *FeedCuratorConfig {
	return &FeedCuratorConfig{
		DoneDedupeWindow:     "10s",
		SlingAggregateWindow: "30s",
		MinAggregateCount:    3,
	}
}

// OperationalConfig groups operational thresholds that were previously hardcoded
// as Go constants. All fields are optional — omitted values use compiled-in defaults.
// This enables per-town tuning without code changes (ZFC: Zero Fixed Constants).
type OperationalConfig struct {
	// Session configures session management thresholds.
	Session *SessionThresholds `json:"session,omitempty"`

	// Nudge configures nudge delivery thresholds.
	Nudge *NudgeThresholds `json:"nudge,omitempty"`

	// Daemon configures daemon lifecycle thresholds.
	Daemon *DaemonThresholds `json:"daemon,omitempty"`

	// Deacon configures deacon health-check thresholds.
	Deacon *DeaconThresholds `json:"deacon,omitempty"`

	// Polecat configures polecat session thresholds.
	Polecat *PolecatThresholds `json:"polecat,omitempty"`

	// Dolt configures Dolt server operation thresholds.
	Dolt *DoltThresholds `json:"dolt,omitempty"`

	// Mail configures mail system thresholds.
	Mail *MailThresholds `json:"mail,omitempty"`

	// Web configures web API thresholds.
	Web *WebThresholds `json:"web,omitempty"`

	// Witness configures witness patrol thresholds.
	Witness *WitnessThresholds `json:"witness,omitempty"`
}

// SessionThresholds configures session management timeouts.
type SessionThresholds struct {
	// ClaudeStartTimeout is how long to wait for Claude to start (default "60s").
	ClaudeStartTimeout string `json:"claude_start_timeout,omitempty"`

	// ShellReadyTimeout is how long to wait for shell prompt after command (default "5s").
	ShellReadyTimeout string `json:"shell_ready_timeout,omitempty"`

	// GracefulShutdownTimeout is wait after Ctrl-C before force-kill (default "3s").
	GracefulShutdownTimeout string `json:"graceful_shutdown_timeout,omitempty"`

	// BdCommandTimeout is timeout for bd CLI command execution (default "30s").
	BdCommandTimeout string `json:"bd_command_timeout,omitempty"`

	// BdSubprocessTimeout is timeout for bd subprocess calls in TUI (default "5s").
	BdSubprocessTimeout string `json:"bd_subprocess_timeout,omitempty"`

	// GUPPViolationTimeout is how long an agent can have hooked work
	// without progressing before GUPP violation (default "30m").
	GUPPViolationTimeout string `json:"gupp_violation_timeout,omitempty"`

	// HungSessionThreshold is how long a tmux session can be inactive
	// before considered hung (default "30m").
	HungSessionThreshold string `json:"hung_session_threshold,omitempty"`

	// StartupNudgeVerifyDelay is wait after startup nudge before checking (default "5s").
	StartupNudgeVerifyDelay string `json:"startup_nudge_verify_delay,omitempty"`

	// StartupNudgeMaxRetries is max retries for startup nudge (default 3).
	StartupNudgeMaxRetries *int `json:"startup_nudge_max_retries,omitempty"`
}

// NudgeThresholds configures nudge queue and delivery timeouts.
type NudgeThresholds struct {
	// ReadyTimeout is how long NudgeSession waits for pane to accept input (default "10s").
	ReadyTimeout string `json:"ready_timeout,omitempty"`

	// RetryInterval is base interval between send-keys retry attempts (default "500ms").
	RetryInterval string `json:"retry_interval,omitempty"`

	// LockTimeout is how long to hold the nudge lock (default "30s").
	LockTimeout string `json:"lock_timeout,omitempty"`

	// NormalTTL is time-to-live for normal-priority nudges (default "30m").
	NormalTTL string `json:"normal_ttl,omitempty"`

	// UrgentTTL is time-to-live for urgent-priority nudges (default "2h").
	UrgentTTL string `json:"urgent_ttl,omitempty"`

	// MaxQueueDepth is max pending nudges per session (default 50).
	MaxQueueDepth *int `json:"max_queue_depth,omitempty"`

	// StaleClaimThreshold is how long a .claimed file must be untouched
	// before treated as orphan (default "5m").
	StaleClaimThreshold string `json:"stale_claim_threshold,omitempty"`
}

// DaemonThresholds configures daemon lifecycle and patrol thresholds.
type DaemonThresholds struct {
	// MassDeathWindow is time window for detecting mass session death (default "30s").
	MassDeathWindow string `json:"mass_death_window,omitempty"`

	// MassDeathThreshold is session deaths within window to trigger alert (default 3).
	MassDeathThreshold *int `json:"mass_death_threshold,omitempty"`

	// DogIdleSessionTimeout is how long a dog can be idle with tmux before kill (default "1h").
	DogIdleSessionTimeout string `json:"dog_idle_session_timeout,omitempty"`

	// DogIdleRemoveTimeout is how long a dog can be idle before removal (default "4h").
	DogIdleRemoveTimeout string `json:"dog_idle_remove_timeout,omitempty"`

	// PolecatIdleSessionTimeout is how long a polecat can be idle before its session
	// is killed to prevent API slot burn (default "15m"). Polecats are ephemeral workers;
	// unlike dogs, they should not persist when idle.
	PolecatIdleSessionTimeout string `json:"polecat_idle_session_timeout,omitempty"`

	// PolecatSelfTerminate controls whether polecats kill their own session after
	// gt done completes (default false). When true, polecats terminate 3 seconds
	// after work submission instead of transitioning to IDLE. This gives fresh
	// context windows per task, reduces token waste, and eliminates stale state
	// issues at scale. Worktree reuse is preserved — ReuseIdlePolecat creates
	// a fresh branch on the existing worktree.
	PolecatSelfTerminate *bool `json:"polecat_self_terminate,omitempty"`

	// StaleWorkingTimeout is how long a dog in state=working with no activity
	// before considered stuck (default "2h").
	StaleWorkingTimeout string `json:"stale_working_timeout,omitempty"`

	// MaxDogPoolSize is target dog pool size (default 4).
	MaxDogPoolSize *int `json:"max_dog_pool_size,omitempty"`

	// MaxLifecycleMessageAge is max age of lifecycle mail before discard (default "6h").
	MaxLifecycleMessageAge string `json:"max_lifecycle_message_age,omitempty"`

	// SyncFailureEscalationThreshold is consecutive git pull failures before
	// logging escalates from WARN to ERROR (default 3).
	SyncFailureEscalationThreshold *int `json:"sync_failure_escalation_threshold,omitempty"`

	// DoctorMolCooldown is min interval between mol-dog-doctor molecules (default "5m").
	DoctorMolCooldown string `json:"doctor_mol_cooldown,omitempty"`

	// RecoveryHeartbeatInterval is the fixed interval for recovery-focused daemon heartbeat (default "3m").
	RecoveryHeartbeatInterval string `json:"recovery_heartbeat_interval,omitempty"`

	// BootSpawnCooldown prevents Boot from spawning on every daemon heartbeat (default "2m").
	BootSpawnCooldown string `json:"boot_spawn_cooldown,omitempty"`

	// DeaconGracePeriod is time to wait after starting Deacon before checking heartbeat (default "5m").
	DeaconGracePeriod string `json:"deacon_grace_period,omitempty"`

	// PressureCPUThreshold is the per-core load average above which new
	// non-infrastructure spawns are deferred. Disabled by default (0).
	// Recommended starting value: 3.0 (only trips under severe load).
	PressureCPUThreshold *float64 `json:"pressure_cpu_threshold,omitempty"`

	// PressureMemThresholdGB is the minimum available memory (in GB) below
	// which new non-infrastructure spawns are deferred. Disabled by default (0).
	// Recommended starting value: 0.5 (only trips when swapping).
	PressureMemThresholdGB *float64 `json:"pressure_mem_threshold_gb,omitempty"`

	// PressureMaxSessions is the maximum number of concurrent agent tmux
	// sessions before new non-infrastructure spawns are deferred. Disabled by default (0 = unlimited).
	PressureMaxSessions *int `json:"pressure_max_sessions,omitempty"`
}

// DeaconThresholds configures deacon health-check and dispatch thresholds.
type DeaconThresholds struct {
	// PingTimeout is how long to wait for HEALTH_CHECK nudge response (default "30s").
	PingTimeout string `json:"ping_timeout,omitempty"`

	// ConsecutiveFailures is health check failures before force-kill (default 3).
	ConsecutiveFailures *int `json:"consecutive_failures,omitempty"`

	// Cooldown is minimum time between force-kills of same agent (default "5m").
	Cooldown string `json:"cooldown,omitempty"`

	// HeartbeatStaleThreshold is age at which deacon heartbeat is stale (default "5m").
	HeartbeatStaleThreshold string `json:"heartbeat_stale_threshold,omitempty"`

	// HeartbeatVeryStaleThreshold is age at which heartbeat is very stale (default "15m").
	HeartbeatVeryStaleThreshold string `json:"heartbeat_very_stale_threshold,omitempty"`

	// MaxRedispatches is max times a bead can be re-dispatched before escalating (default 3).
	MaxRedispatches *int `json:"max_redispatches,omitempty"`

	// RedispatchCooldown is min time between re-dispatches of same bead (default "5m").
	RedispatchCooldown string `json:"redispatch_cooldown,omitempty"`

	// MaxFeedsPerCycle is max stranded convoys to feed per invocation (default 3).
	MaxFeedsPerCycle *int `json:"max_feeds_per_cycle,omitempty"`

	// FeedCooldown is min time between feeding same convoy (default "10m").
	FeedCooldown string `json:"feed_cooldown,omitempty"`
}

// PolecatThresholds configures polecat session and retry thresholds.
type PolecatThresholds struct {
	// HeartbeatStaleThreshold is age at which polecat heartbeat is stale (default "3m").
	HeartbeatStaleThreshold string `json:"heartbeat_stale_threshold,omitempty"`

	// DoltMaxRetries is max retries for Dolt operations (default 10).
	DoltMaxRetries *int `json:"dolt_max_retries,omitempty"`

	// DoltBaseBackoff is base backoff for Dolt retry loop (default "500ms").
	DoltBaseBackoff string `json:"dolt_base_backoff,omitempty"`

	// DoltBackoffMax is cap for Dolt retry backoff (default "30s").
	DoltBackoffMax string `json:"dolt_backoff_max,omitempty"`

	// PendingMaxAge is max age for .pending reservation marker (default "5m").
	PendingMaxAge string `json:"pending_max_age,omitempty"`

	// NamepoolSize is number of name slots in pool (default 50).
	NamepoolSize *int `json:"namepool_size,omitempty"`
}

// DoltThresholds configures Dolt server operation thresholds.
type DoltThresholds struct {
	// HealthCheckInterval is how often Dolt health check fires (default "30s").
	HealthCheckInterval string `json:"health_check_interval,omitempty"`

	// CmdTimeout is timeout for individual dolt CLI commands (default "15s").
	CmdTimeout string `json:"cmd_timeout,omitempty"`

	// MaxConnections is max concurrent connections (default 1000).
	MaxConnections *int `json:"max_connections,omitempty"`

	// SlowQueryThreshold is duration above which a query is flagged slow (default "1s").
	SlowQueryThreshold string `json:"slow_query_threshold,omitempty"`
}

// MailThresholds configures mail system thresholds.
type MailThresholds struct {
	// IdleNotifyTimeout is how long to wait for idle notify (default "3s").
	IdleNotifyTimeout string `json:"idle_notify_timeout,omitempty"`

	// BdReadTimeout is timeout for bd read operations (default "60s").
	BdReadTimeout string `json:"bd_read_timeout,omitempty"`

	// BdWriteTimeout is timeout for bd write operations (default "60s").
	BdWriteTimeout string `json:"bd_write_timeout,omitempty"`

	// MaxConcurrentAckOps is max concurrent mail acknowledge operations (default 8).
	MaxConcurrentAckOps *int `json:"max_concurrent_ack_ops,omitempty"`

	// ReplyReminderDelay is how long after mail delivery to nudge the recipient
	// to reply via gt mail send rather than in chat (default "30s").
	// Set to "0s" to disable reply reminders entirely.
	ReplyReminderDelay string `json:"reply_reminder_delay,omitempty"`
}

// WebThresholds configures web API thresholds.
type WebThresholds struct {
	// MaxConcurrentCommands is max concurrent gt subprocesses via web API (default 12).
	MaxConcurrentCommands *int `json:"max_concurrent_commands,omitempty"`

	// MaxSubjectLen is max subject length for mail API (default 500).
	MaxSubjectLen *int `json:"max_subject_len,omitempty"`

	// MaxBodyLen is max body length for mail API (default 100000).
	MaxBodyLen *int `json:"max_body_len,omitempty"`
}

// WitnessThresholds configures witness patrol detection thresholds.
type WitnessThresholds struct {
	// StartupStallThreshold is the minimum session age before a session with no
	// recent activity is considered stalled at startup (default "90s").
	StartupStallThreshold string `json:"startup_stall_threshold,omitempty"`

	// StartupActivityGrace is the max time since last activity before a session
	// old enough to be past startup is considered stalled (default "60s").
	StartupActivityGrace string `json:"startup_activity_grace,omitempty"`

	// MaxBeadRespawns is the threshold above which a bead respawn is blocked
	// and escalated to mayor instead of re-dispatched (default 3).
	MaxBeadRespawns *int `json:"max_bead_respawns,omitempty"`

	// DoneIntentStuckTimeout is how long a done-intent can be active before the
	// session is considered stuck and restarted (default "60s").
	DoneIntentStuckTimeout string `json:"done_intent_stuck_timeout,omitempty"`

	// DoneIntentRecentGrace is how recently a done-intent must have been created
	// to be considered still in progress (default "30s").
	DoneIntentRecentGrace string `json:"done_intent_recent_grace,omitempty"`
}

// DefaultOperationalConfig returns an OperationalConfig with all defaults.
func DefaultOperationalConfig() *OperationalConfig {
	return &OperationalConfig{}
}

// ConvoyConfig configures convoy behavior settings.
type ConvoyConfig struct {
	// NotifyOnComplete controls whether convoy completion pushes a notification
	// into the active Mayor session (in addition to mail). Opt-in; default false.
	NotifyOnComplete bool `json:"notify_on_complete,omitempty"`
}

// ParseDurationOrDefault parses a Go duration string, returning fallback on error or empty input.
func ParseDurationOrDefault(s string, fallback time.Duration) time.Duration {
	if s == "" {
		return fallback
	}
	d, err := time.ParseDuration(s)
	if err != nil {
		return fallback
	}
	return d
}

// DaemonConfig represents daemon process settings.
type DaemonConfig struct {
	HeartbeatInterval string `json:"heartbeat_interval,omitempty"` // e.g., "30s"
	PollInterval      string `json:"poll_interval,omitempty"`      // e.g., "10s"
}

// DaemonPatrolConfig represents the daemon patrol configuration (mayor/daemon.json).
// This configures how patrols are triggered and managed.
type DaemonPatrolConfig struct {
	Type      string                  `json:"type"`                // "daemon-patrol-config"
	Version   int                     `json:"version"`             // schema version
	Heartbeat *HeartbeatConfig        `json:"heartbeat,omitempty"` // heartbeat settings
	Patrols   map[string]PatrolConfig `json:"patrols,omitempty"`   // named patrol configurations
}

// HeartbeatConfig represents heartbeat settings for daemon.
type HeartbeatConfig struct {
	Enabled  bool   `json:"enabled"`            // whether heartbeat is enabled
	Interval string `json:"interval,omitempty"` // e.g., "3m"
}

// PatrolConfig represents a single patrol configuration.
type PatrolConfig struct {
	Enabled  bool     `json:"enabled"`            // whether this patrol is enabled
	Interval string   `json:"interval,omitempty"` // e.g., "5m"
	Agent    string   `json:"agent,omitempty"`    // agent that runs this patrol
	Rigs     []string `json:"rigs,omitempty"`     // rigs this patrol manages (empty = all)
}

// CurrentDaemonPatrolConfigVersion is the current schema version for DaemonPatrolConfig.
const CurrentDaemonPatrolConfigVersion = 1

// DaemonPatrolConfigFileName is the filename for daemon patrol configuration.
const DaemonPatrolConfigFileName = "daemon.json"

// NewDaemonPatrolConfig creates a new DaemonPatrolConfig with sensible defaults.
func NewDaemonPatrolConfig() *DaemonPatrolConfig {
	return &DaemonPatrolConfig{
		Type:    "daemon-patrol-config",
		Version: CurrentDaemonPatrolConfigVersion,
		Heartbeat: &HeartbeatConfig{
			Enabled:  true,
			Interval: "3m",
		},
		Patrols: map[string]PatrolConfig{
			"deacon": {
				Enabled:  true,
				Interval: "5m",
				Agent:    "deacon",
			},
			"witness": {
				Enabled:  true,
				Interval: "5m",
				Agent:    "witness",
			},
			"refinery": {
				Enabled:  true,
				Interval: "5m",
				Agent:    "refinery",
			},
		},
	}
}

// DeaconConfig represents deacon process settings.
type DeaconConfig struct {
	PatrolInterval string `json:"patrol_interval,omitempty"` // e.g., "5m"
}

// CurrentMayorConfigVersion is the current schema version for MayorConfig.
const CurrentMayorConfigVersion = 1

// DefaultCrewName is the default name for crew workspaces when not overridden.
const DefaultCrewName = "max"

// RigsConfig represents the rigs registry (mayor/rigs.json).
type RigsConfig struct {
	Version int                 `json:"version"`
	Rigs    map[string]RigEntry `json:"rigs"`
}

// RigEntry represents a single rig in the registry.
type RigEntry struct {
	GitURL      string       `json:"git_url"`
	PushURL     string       `json:"push_url,omitempty"`
	UpstreamURL string       `json:"upstream_url,omitempty"` // optional upstream URL (for fork workflows)
	LocalRepo   string       `json:"local_repo,omitempty"`
	AddedAt     time.Time    `json:"added_at"`
	BeadsConfig *BeadsConfig `json:"beads,omitempty"`
}

// BeadsConfig represents beads configuration for a rig.
type BeadsConfig struct {
	Repo   string `json:"repo"`   // "local" | path | git-url
	Prefix string `json:"prefix"` // issue prefix
}

// CurrentTownVersion is the current schema version for TownConfig.
// Version 2: Added Owner and PublicName fields for federation identity.
const CurrentTownVersion = 2

// CurrentRigsVersion is the current schema version for RigsConfig.
const CurrentRigsVersion = 1

// CurrentRigConfigVersion is the current schema version for RigConfig.
const CurrentRigConfigVersion = 1

// CurrentRigSettingsVersion is the current schema version for RigSettings.
const CurrentRigSettingsVersion = 1

// RigConfig represents per-rig identity (rig/config.json).
// This contains only identity - behavioral config is in settings/config.json.
type RigConfig struct {
	Type        string       `json:"type"`                   // "rig"
	Version     int          `json:"version"`                // schema version
	Name        string       `json:"name"`                   // rig name
	GitURL      string       `json:"git_url"`                // git repository URL
	PushURL     string       `json:"push_url,omitempty"`     // optional push URL (fork for read-only upstreams)
	UpstreamURL string       `json:"upstream_url,omitempty"` // optional upstream URL (for fork workflows)
	LocalRepo   string       `json:"local_repo,omitempty"`
	CreatedAt   time.Time    `json:"created_at"` // when the rig was created
	Beads       *BeadsConfig `json:"beads,omitempty"`
}

// WorkflowConfig represents workflow settings for a rig.
type WorkflowConfig struct {
	// DefaultFormula is the formula to use when `gt formula run` is called without arguments.
	// If empty, no default is set and a formula name must be provided.
	DefaultFormula string `json:"default_formula,omitempty"`
}

// RigSettings represents per-rig behavioral configuration (settings/config.json).
type RigSettings struct {
	Type       string            `json:"type"`                  // "rig-settings"
	Version    int               `json:"version"`               // schema version
	MergeQueue *MergeQueueConfig `json:"merge_queue,omitempty"` // merge queue settings
	Theme      *ThemeConfig      `json:"theme,omitempty"`       // tmux theme settings
	Namepool   *NamepoolConfig   `json:"namepool,omitempty"`    // polecat name pool settings
	Crew       *CrewConfig       `json:"crew,omitempty"`        // crew startup settings
	Workflow   *WorkflowConfig   `json:"workflow,omitempty"`    // workflow settings
	Runtime    *RuntimeConfig    `json:"runtime,omitempty"`     // LLM runtime settings (deprecated: use Agent)

	// Agent selects which agent preset to use for this rig.
	// Can be a built-in preset ("claude", "gemini", "codex", "cursor", "auggie", "amp", "opencode", "copilot")
	// or a custom agent defined in settings/agents.json.
	// If empty, uses the town's default_agent setting.
	// Takes precedence over Runtime if both are set.
	Agent string `json:"agent,omitempty"`

	// Agents defines custom agent configurations or overrides for this rig.
	// Similar to TownSettings.Agents but applies to this rig only.
	// Allows per-rig custom agents for polecats and crew members.
	Agents map[string]*RuntimeConfig `json:"agents,omitempty"`

	// RoleAgents maps role names to agent aliases for per-role model selection.
	// Keys are role names: "witness", "refinery", "polecat", "crew".
	// Values are agent names (built-in presets or custom agents).
	// Overrides TownSettings.RoleAgents for this specific rig.
	// Example: {"witness": "claude-haiku", "polecat": "claude-sonnet"}
	RoleAgents map[string]string `json:"role_agents,omitempty"`

	// WorkerAgents maps individual crew worker names to agent aliases.
	// Allows per-worker agent selection, overriding RoleAgents["crew"].
	// Takes precedence over RoleAgents["crew"] but is overridden by explicit --agent flags.
	// Example: {"denali": "codex", "glacier": "gemini"}
	WorkerAgents map[string]string `json:"worker_agents,omitempty"`
}

// CrewConfig represents crew workspace settings for a rig.
type CrewConfig struct {
	// Startup is a natural language instruction for which crew to start on boot.
	// Interpreted by AI during startup. Examples:
	//   "max"                    - start only max
	//   "joe and max"            - start joe and max
	//   "all"                    - start all crew members
	//   "pick one"               - start any one crew member
	//   "none"                   - don't auto-start any crew
	//   "max, but not emma"      - start max, skip emma
	// If empty, defaults to starting no crew automatically.
	Startup string `json:"startup,omitempty"`
}

// RuntimeConfig represents LLM runtime configuration for agent sessions.
// This allows switching between different LLM backends (claude, aider, etc.)
// without modifying startup code.
type RuntimeConfig struct {
	// Provider selects runtime-specific defaults and integration behavior.
	// Known values: "claude", "codex", "generic". Default: "claude".
	Provider string `json:"provider,omitempty"`

	// Command is the CLI command to invoke (e.g., "claude", "aider").
	// Default: "claude"
	Command string `json:"command,omitempty"`

	// Args are additional command-line arguments.
	// Default: ["--dangerously-skip-permissions"] for built-in agents.
	// Empty array [] means no args (not "use defaults").
	Args []string `json:"args"`

	// Env are environment variables to set when starting the agent.
	// These are merged with the standard GT_* variables.
	// Used for agent-specific configuration like OPENCODE_PERMISSION.
	Env map[string]string `json:"env,omitempty"`

	// InitialPrompt is an optional first message to send after startup.
	// For claude, this is passed as the prompt argument.
	// Empty by default (hooks handle context).
	InitialPrompt string `json:"initial_prompt,omitempty"`

	// PromptMode controls how prompts are passed to the runtime.
	// Supported values: "arg" (append prompt arg), "none" (ignore prompt).
	// Default: "arg" for claude/generic, "none" for codex.
	PromptMode string `json:"prompt_mode,omitempty"`

	// Session config controls environment integration for runtime session IDs.
	Session *RuntimeSessionConfig `json:"session,omitempty"`

	// Hooks config controls runtime hook installation (if supported).
	Hooks *RuntimeHooksConfig `json:"hooks,omitempty"`

	// Tmux config controls process detection and readiness heuristics.
	Tmux *RuntimeTmuxConfig `json:"tmux,omitempty"`

	// Instructions controls the per-workspace instruction file name.
	Instructions *RuntimeInstructionsConfig `json:"instructions,omitempty"`

	// ACP configures ACP (Agent Communication Protocol) support.
	// When set, the agent can run in ACP mode. If nil, ACP support is
	// determined by matching the Command to a known preset with ACP config.
	ACP *ACPConfig `json:"acp,omitempty"`

	// ExecWrapper is a command prefix inserted between environment variables
	// and the agent binary in the startup command. Used for sandboxed execution.
	// Example: ["exitbox", "run", "--profile=gastown-polecat", "--"]
	// Produces: exec env VAR=val ... exitbox run --profile=gastown-polecat -- claude ...
	ExecWrapper []string `json:"exec_wrapper,omitempty"`

	// ResolvedAgent is the agent name that was resolved during config lookup.
	// Set by ResolveRoleAgentConfig / resolveAgentConfigInternal so that
	// BuildStartupCommand can export GT_AGENT for process detection.
	// Not serialized — this is a runtime-only field.
	ResolvedAgent string `json:"-"`
}

// RuntimeSessionConfig configures how Gas Town discovers runtime session IDs.
type RuntimeSessionConfig struct {
	// SessionIDEnv is the environment variable set by the runtime to identify a session.
	// Default: "CLAUDE_SESSION_ID" for claude, empty for codex/generic.
	SessionIDEnv string `json:"session_id_env,omitempty"`

	// ConfigDirEnv is the environment variable that selects a runtime account/config dir.
	// Default: "CLAUDE_CONFIG_DIR" for claude, empty for codex/generic.
	ConfigDirEnv string `json:"config_dir_env,omitempty"`
}

// RuntimeHooksConfig configures runtime hook installation.
type RuntimeHooksConfig struct {
	// Provider controls which hook templates to install: "claude", "opencode", "copilot", or "none".
	Provider string `json:"provider,omitempty"`

	// Dir is the settings directory (e.g., ".claude").
	Dir string `json:"dir,omitempty"`

	// SettingsFile is the settings file name (e.g., "settings.json").
	SettingsFile string `json:"settings_file,omitempty"`

	// Informational indicates the hooks provider installs instructions files only,
	// not executable lifecycle hooks. When true, Gas Town sends startup fallback
	// commands (gt prime) via nudge since hooks won't run automatically.
	// Defaults to false (backwards compatible with claude/opencode which have real hooks).
	Informational bool `json:"informational,omitempty"`
}

// RuntimeTmuxConfig controls tmux heuristics for detecting runtime readiness.
type RuntimeTmuxConfig struct {
	// ProcessNames are tmux pane commands that indicate the runtime is running.
	ProcessNames []string `json:"process_names,omitempty"`

	// ReadyPromptPrefix is the prompt prefix to detect readiness (e.g., "> ").
	ReadyPromptPrefix string `json:"ready_prompt_prefix,omitempty"`

	// ReadyDelayMs is a fixed delay used when prompt detection is unavailable.
	ReadyDelayMs int `json:"ready_delay_ms,omitempty"`
}

// RuntimeInstructionsConfig controls the name of the role instruction file.
type RuntimeInstructionsConfig struct {
	// File is the instruction filename (e.g., "CLAUDE.md", "AGENTS.md").
	File string `json:"file,omitempty"`
}

// DefaultRuntimeConfig returns a RuntimeConfig with sensible defaults.
func DefaultRuntimeConfig() *RuntimeConfig {
	return normalizeRuntimeConfig(&RuntimeConfig{Provider: "claude"})
}

// BuildCommand returns the full command line string.
// For use with tmux SendKeys and respawn-pane, where the string is
// interpreted by the user's shell. Args containing shell-special
// characters (e.g., brackets in "sonnet[1m]") are quoted to prevent
// glob expansion.
func (rc *RuntimeConfig) BuildCommand() string {
	resolved := normalizeRuntimeConfig(rc)

	cmd := resolved.Command
	args := resolved.Args

	// Combine command and args, quoting any that contain shell metacharacters
	if len(args) > 0 {
		quoted := make([]string, len(args))
		for i, a := range args {
			quoted[i] = ShellQuote(a)
		}
		return cmd + " " + strings.Join(quoted, " ")
	}
	return cmd
}

// BuildCommandWithPrompt returns the full command line with an initial prompt.
// If the config has an InitialPrompt, it's appended as a quoted argument.
// If prompt is provided, it overrides the config's InitialPrompt.
// For opencode, uses --prompt flag; for other agents, uses positional argument.
func (rc *RuntimeConfig) BuildCommandWithPrompt(prompt string) string {
	resolved := normalizeRuntimeConfig(rc)
	base := resolved.BuildCommand()

	// Use provided prompt or fall back to config
	p := prompt
	if p == "" {
		p = resolved.InitialPrompt
	}

	if p == "" || resolved.PromptMode == "none" {
		return base
	}

	// OpenCode requires --prompt flag for initial prompt in interactive mode.
	// Positional argument causes opencode to exit immediately.
	if resolved.Command == "opencode" {
		return base + " --prompt " + quoteForShell(p)
	}

	// Copilot  requires -i flag for initial prompt in interactive mode.
	if resolved.Command == "copilot" {
		return base + " -i " + quoteForShell(p)
	}

	// Gemini requires -i (--prompt-interactive) to auto-execute the prompt
	// while staying in interactive mode. Positional args populate the input
	// field but don't execute, and -p runs headless (exits after completion).
	if resolved.Command == "gemini" {
		return base + " -i " + quoteForShell(p)
	}

	// Quote the prompt for shell safety (positional arg for claude and others)
	return base + " " + quoteForShell(p)
}

// BuildArgsWithPrompt returns the runtime command and args suitable for exec.
func (rc *RuntimeConfig) BuildArgsWithPrompt(prompt string) []string {
	resolved := normalizeRuntimeConfig(rc)
	args := append([]string{resolved.Command}, resolved.Args...)

	p := prompt
	if p == "" {
		p = resolved.InitialPrompt
	}

	if p != "" && resolved.PromptMode != "none" {
		switch resolved.Command {
		case "opencode":
			args = append(args, "--prompt", p)
		case "copilot", "gemini":
			args = append(args, "-i", p)
		default:
			args = append(args, p)
		}
	}

	return args
}

func normalizeRuntimeConfig(rc *RuntimeConfig) *RuntimeConfig {
	if rc == nil {
		rc = &RuntimeConfig{}
	}

	// Shallow copy to avoid mutating the input
	copy := *rc
	rc = &copy

	// Deep copy nested structs to avoid shared references
	if rc.Session != nil {
		s := *rc.Session
		rc.Session = &s
	}
	if rc.Hooks != nil {
		h := *rc.Hooks
		rc.Hooks = &h
	}
	if rc.Tmux != nil {
		t := *rc.Tmux
		rc.Tmux = &t
	}
	if rc.Instructions != nil {
		i := *rc.Instructions
		rc.Instructions = &i
	}

	if rc.Provider == "" {
		rc.Provider = "claude"
	}

	if rc.Command == "" {
		rc.Command = defaultRuntimeCommand(rc.Provider)
	}

	if rc.Args == nil {
		rc.Args = defaultRuntimeArgs(rc.Provider)
	}

	if rc.PromptMode == "" {
		rc.PromptMode = defaultPromptMode(rc.Provider)
	}

	if rc.Session == nil {
		rc.Session = &RuntimeSessionConfig{}
	}

	if rc.Session.SessionIDEnv == "" {
		rc.Session.SessionIDEnv = defaultSessionIDEnv(rc.Provider)
	}

	if rc.Session.ConfigDirEnv == "" {
		rc.Session.ConfigDirEnv = defaultConfigDirEnv(rc.Provider)
	}

	if rc.Hooks == nil {
		rc.Hooks = &RuntimeHooksConfig{}
	}

	if rc.Hooks.Provider == "" {
		rc.Hooks.Provider = defaultHooksProvider(rc.Provider)
	}

	if rc.Hooks.Dir == "" {
		rc.Hooks.Dir = defaultHooksDir(rc.Provider)
	}

	if rc.Hooks.SettingsFile == "" {
		rc.Hooks.SettingsFile = defaultHooksFile(rc.Provider)
	}

	// Set informational flag for providers whose "hooks" are instructions files,
	// not executable lifecycle hooks. This tells startup fallback logic to send
	// gt prime via nudge since hooks won't run automatically.
	if !rc.Hooks.Informational {
		rc.Hooks.Informational = defaultHooksInformational(rc.Provider)
	}

	if rc.Tmux == nil {
		rc.Tmux = &RuntimeTmuxConfig{}
	}

	if rc.Tmux.ProcessNames == nil {
		rc.Tmux.ProcessNames = defaultProcessNames(rc.Provider, rc.Command)
	}

	if rc.Tmux.ReadyPromptPrefix == "" {
		rc.Tmux.ReadyPromptPrefix = defaultReadyPromptPrefix(rc.Provider)
	}

	if rc.Tmux.ReadyDelayMs == 0 {
		rc.Tmux.ReadyDelayMs = defaultReadyDelayMs(rc.Provider)
	}

	if rc.Instructions == nil {
		rc.Instructions = &RuntimeInstructionsConfig{}
	}

	if rc.Instructions.File == "" {
		rc.Instructions.File = defaultInstructionsFile(rc.Provider)
	}

	return rc
}

func defaultRuntimeCommand(provider string) string {
	if provider == "generic" {
		return ""
	}
	if preset := GetAgentPresetByName(provider); preset != nil {
		cmd := preset.Command
		// Resolve claude path for Claude preset (handles alias installations)
		if preset.Name == AgentClaude && cmd == "claude" {
			return resolveClaudePath()
		}
		return cmd
	}
	return resolveClaudePath() // fallback for unknown providers
}

// resolveClaudePath finds the claude binary, checking PATH first then common installation locations.
// This handles the case where claude is installed as an alias (not in PATH) which doesn't work
// in non-interactive shells spawned by tmux.
func resolveClaudePath() string {
	// First, try to find claude in PATH
	if path, err := exec.LookPath("claude"); err == nil {
		return path
	}

	// Check common Claude Code installation locations
	home, err := os.UserHomeDir()
	if err != nil {
		return "claude" // Fall back to bare command
	}

	// Standard Claude Code installation path
	claudePath := filepath.Join(home, ".claude", "local", "claude")
	if _, err := os.Stat(claudePath); err == nil {
		return claudePath
	}

	// Fall back to bare command (might work if PATH is set differently in tmux)
	return "claude"
}

func defaultRuntimeArgs(provider string) []string {
	if preset := GetAgentPresetByName(provider); preset != nil && preset.Args != nil {
		return append([]string(nil), preset.Args...) // copy to avoid mutation
	}
	return nil
}

func defaultPromptMode(provider string) string {
	if preset := GetAgentPresetByName(provider); preset != nil && preset.PromptMode != "" {
		return preset.PromptMode
	}
	return "arg"
}

func defaultSessionIDEnv(provider string) string {
	if preset := GetAgentPresetByName(provider); preset != nil {
		return preset.SessionIDEnv
	}
	return ""
}

func defaultConfigDirEnv(provider string) string {
	if preset := GetAgentPresetByName(provider); preset != nil {
		return preset.ConfigDirEnv
	}
	return ""
}

func defaultHooksProvider(provider string) string {
	if preset := GetAgentPresetByName(provider); preset != nil && preset.HooksProvider != "" {
		return preset.HooksProvider
	}
	return "none"
}

func defaultHooksDir(provider string) string {
	if preset := GetAgentPresetByName(provider); preset != nil {
		return preset.HooksDir
	}
	return ""
}

func defaultHooksFile(provider string) string {
	if preset := GetAgentPresetByName(provider); preset != nil {
		return preset.HooksSettingsFile
	}
	return ""
}

// defaultHooksInformational returns true for providers whose hooks are instructions
// files only (not executable lifecycle hooks). For these providers, Gas Town sends
// startup fallback commands (gt prime) via nudge since hooks won't auto-run.
func defaultHooksInformational(provider string) bool {
	if preset := GetAgentPresetByName(provider); preset != nil {
		return preset.HooksInformational
	}
	return false
}

func defaultProcessNames(provider, command string) []string {
	if preset := GetAgentPresetByName(provider); preset != nil && len(preset.ProcessNames) > 0 {
		return append([]string(nil), preset.ProcessNames...) // copy to avoid mutation
	}
	if command != "" {
		return []string{filepath.Base(command)}
	}
	return nil
}

func defaultReadyPromptPrefix(provider string) string {
	if preset := GetAgentPresetByName(provider); preset != nil {
		return preset.ReadyPromptPrefix
	}
	return ""
}

func defaultReadyDelayMs(provider string) int {
	if preset := GetAgentPresetByName(provider); preset != nil {
		return preset.ReadyDelayMs
	}
	return 0
}

func defaultInstructionsFile(provider string) string {
	if preset := GetAgentPresetByName(provider); preset != nil && preset.InstructionsFile != "" {
		return preset.InstructionsFile
	}
	return "AGENTS.md"
}

// quoteForShell quotes a string for safe shell usage.
func quoteForShell(s string) string {
	if runtime.GOOS == "windows" {
		// PowerShell: use single quotes (no interpolation). Double embedded single quotes.
		return "'" + strings.ReplaceAll(s, "'", "''") + "'"
	}
	// POSIX shell: wrap in double quotes, escaping special characters.
	escaped := strings.ReplaceAll(s, `\`, `\\`)
	escaped = strings.ReplaceAll(escaped, `"`, `\"`)
	escaped = strings.ReplaceAll(escaped, "`", "\\`")
	escaped = strings.ReplaceAll(escaped, "$", `\$`)
	return `"` + escaped + `"`
}

// ThemeConfig represents tmux theme settings for a rig.
type ThemeConfig struct {
	// Disabled skips tmux status/window theming for this rig.
	Disabled bool `json:"disabled,omitempty"`

	// Name picks from the default palette (e.g., "ocean", "forest").
	// If empty, a theme is auto-assigned based on rig name.
	Name string `json:"name,omitempty"`

	// Custom overrides the palette with specific colors.
	Custom *CustomTheme `json:"custom,omitempty"`

	// RoleThemes overrides themes for specific roles in this rig.
	// Keys: "witness", "refinery", "crew", "polecat".
	// A value of "none" disables tmux theming for that role.
	RoleThemes map[string]string `json:"role_themes,omitempty"`

	// WindowTint controls window background (window-style) coloring for this rig.
	// If nil, falls back to town-level window tint config.
	WindowTint *WindowTint `json:"window_tint,omitempty"`
}

// CustomTheme allows specifying exact colors for the status bar.
type CustomTheme struct {
	BG string `json:"bg"` // Background color (hex or tmux color name)
	FG string `json:"fg"` // Foreground color (hex or tmux color name)
}

// TownThemeConfig represents global theme settings (mayor/config.json).
type TownThemeConfig struct {
	// Disabled skips tmux status/window theming for all sessions unless a rig
	// theme overrides it.
	Disabled bool `json:"disabled,omitempty"`

	// Name picks from the default palette when no role-specific override exists.
	Name string `json:"name,omitempty"`

	// Custom overrides the palette with specific colors when no role-specific
	// override exists.
	Custom *CustomTheme `json:"custom,omitempty"`

	// RoleDefaults sets default themes for roles across all rigs.
	// Keys: "mayor", "deacon", "witness", "refinery", "crew", "polecat".
	// A value of "none" disables tmux theming for that role.
	RoleDefaults map[string]string `json:"role_defaults,omitempty"`

	// WindowTint controls window background (window-style) coloring globally.
	// Per-rig WindowTint in ThemeConfig takes precedence over this.
	WindowTint *WindowTint `json:"window_tint,omitempty"`
}

// WindowTint controls window background (window-style) coloring.
// Mirrors status bar theme customization: palette name, custom colors, per-role overrides.
// When Enabled is nil or true, window backgrounds are tinted.
// When Enabled is false, window backgrounds use terminal defaults.
type WindowTint struct {
	// Enabled controls whether window tinting is active.
	// nil or true = enabled, false = disabled (window uses terminal default).
	Enabled *bool `json:"enabled,omitempty"`

	// Name picks a palette theme for the window background.
	// If empty, falls back to the session's status bar theme colors.
	Name string `json:"name,omitempty"`

	// Custom overrides the palette with specific window background colors.
	Custom *CustomTheme `json:"custom,omitempty"`

	// RoleTints overrides window tint themes for specific roles.
	// Keys: "witness", "refinery", "crew", "polecat"
	RoleTints map[string]string `json:"role_tints,omitempty"`
}

// BuiltinRoleThemes returns the default themes for each role.
// These are used when no explicit configuration is provided.
func BuiltinRoleThemes() map[string]string {
	return map[string]string{
		"witness":  "rust", // Red/rust - watchful, alert
		"refinery": "plum", // Purple - processing, refining
		// crew and polecat use rig theme by default (no override)
	}
}

// MergeQueueConfig represents merge queue settings for a rig.
type MergeQueueConfig struct {
	// Enabled controls whether the merge queue is active.
	Enabled bool `json:"enabled"`

	// IntegrationBranchPolecatEnabled controls whether polecats auto-source
	// their worktrees from integration branches when the parent epic has one.
	// Nil defaults to true.
	IntegrationBranchPolecatEnabled *bool `json:"integration_branch_polecat_enabled,omitempty"`

	// IntegrationBranchRefineryEnabled controls whether mq submit and gt done
	// auto-detect integration branches as MR targets.
	// Nil defaults to true.
	IntegrationBranchRefineryEnabled *bool `json:"integration_branch_refinery_enabled,omitempty"`

	// IntegrationBranchTemplate is the pattern for integration branch names.
	// Supports variables: {epic}, {prefix}, {user}
	// - {epic}: Full epic ID (e.g., "RA-123")
	// - {prefix}: Epic prefix before first hyphen (e.g., "RA")
	// - {user}: Git user.name (e.g., "klauern")
	// Default: "integration/{epic}"
	IntegrationBranchTemplate string `json:"integration_branch_template,omitempty"`

	// IntegrationBranchAutoLand controls whether the refinery should automatically
	// land integration branches when all children of the epic are closed.
	// Nil defaults to false (manual landing required).
	IntegrationBranchAutoLand *bool `json:"integration_branch_auto_land,omitempty"`

	// MergeStrategy controls how the refinery lands approved work: "direct" (default)
	// merges directly to the base branch, "pr" creates a GitHub pull request.
	MergeStrategy string `json:"merge_strategy,omitempty"`

	// RequireReview controls whether the refinery requires at least one approving
	// GitHub review before merging a PR. Only meaningful when merge_strategy="pr".
	// Nil defaults to false (no review required).
	RequireReview *bool `json:"require_review,omitempty"`

	// OnConflict specifies conflict resolution strategy: "assign_back" or "auto_rebase".
	OnConflict string `json:"on_conflict"`

	// RunTests controls whether to run tests before merging.
	// Nil defaults to true (tests are run).
	RunTests *bool `json:"run_tests,omitempty"`

	// TestCommand is the command to run for tests.
	TestCommand string `json:"test_command,omitempty"`

	// LintCommand is the command to run for linting (used by formulas).
	LintCommand string `json:"lint_command,omitempty"`

	// BuildCommand is the command to run for building (used by formulas).
	BuildCommand string `json:"build_command,omitempty"`

	// SetupCommand is the command to run for project setup (e.g., pnpm install).
	SetupCommand string `json:"setup_command,omitempty"`

	// TypecheckCommand is the command to run for type checking (e.g., tsc --noEmit).
	TypecheckCommand string `json:"typecheck_command,omitempty"`

	// DeleteMergedBranches controls whether to delete branches after merging.
	// Nil defaults to true (merged branches are deleted).
	DeleteMergedBranches *bool `json:"delete_merged_branches,omitempty"`

	// RetryFlakyTests is the number of times to retry flaky tests.
	RetryFlakyTests int `json:"retry_flaky_tests"`

	// PollInterval is how often to poll for new merge requests (e.g., "30s").
	PollInterval string `json:"poll_interval"`

	// MaxConcurrent is the maximum number of concurrent merges.
	MaxConcurrent int `json:"max_concurrent"`

	// StaleClaimTimeout is how long a claimed MR can go without updates before
	// being considered abandoned and eligible for re-claim (e.g., "30m").
	StaleClaimTimeout string `json:"stale_claim_timeout,omitempty"`

	// JudgmentEnabled controls whether the refinery performs quality review
	// before merging. When true, the refinery patrol's quality-review step
	// evaluates the diff for correctness, security, and code quality.
	// Nil defaults to false (no quality review).
	JudgmentEnabled *bool `json:"judgment_enabled,omitempty"`

	// ReviewDepth controls the thoroughness of quality review when judgment
	// is enabled. Valid values: "quick", "standard", "deep".
	// Nil defaults to "standard".
	ReviewDepth string `json:"review_depth,omitempty"`
}

// OnConflict strategy constants.
const (
	OnConflictAssignBack = "assign_back"
	OnConflictAutoRebase = "auto_rebase"
)

// IsPolecatIntegrationEnabled returns whether polecat integration branch
// sourcing is enabled. Nil-safe, defaults to true.
func (c *MergeQueueConfig) IsPolecatIntegrationEnabled() bool {
	if c.IntegrationBranchPolecatEnabled == nil {
		return true
	}
	return *c.IntegrationBranchPolecatEnabled
}

// IsRefineryIntegrationEnabled returns whether refinery/submit integration
// branch auto-detection is enabled. Nil-safe, defaults to true.
func (c *MergeQueueConfig) IsRefineryIntegrationEnabled() bool {
	if c.IntegrationBranchRefineryEnabled == nil {
		return true
	}
	return *c.IntegrationBranchRefineryEnabled
}

// IsIntegrationBranchAutoLandEnabled returns whether the refinery should
// auto-land integration branches when all epic children are closed.
// Nil-safe, defaults to false (manual landing required).
func (c *MergeQueueConfig) IsIntegrationBranchAutoLandEnabled() bool {
	if c.IntegrationBranchAutoLand == nil {
		return false
	}
	return *c.IntegrationBranchAutoLand
}

// IsRunTestsEnabled returns whether tests should run before merging.
// Nil-safe, defaults to true.
func (c *MergeQueueConfig) IsRunTestsEnabled() bool {
	if c.RunTests == nil {
		return true
	}
	return *c.RunTests
}

// IsDeleteMergedBranchesEnabled returns whether merged branches should be deleted.
// Nil-safe, defaults to true.
func (c *MergeQueueConfig) IsDeleteMergedBranchesEnabled() bool {
	if c.DeleteMergedBranches == nil {
		return true
	}
	return *c.DeleteMergedBranches
}

// IsJudgmentEnabled returns whether quality review is enabled for merges.
// Nil-safe, defaults to false.
func (c *MergeQueueConfig) IsJudgmentEnabled() bool {
	if c.JudgmentEnabled == nil {
		return false
	}
	return *c.JudgmentEnabled
}

// IsRequireReviewEnabled returns whether PR reviews are required before merging.
// Nil-safe, defaults to false.
func (c *MergeQueueConfig) IsRequireReviewEnabled() bool {
	if c.RequireReview == nil {
		return false
	}
	return *c.RequireReview
}

// GetReviewDepth returns the configured review depth.
// Nil-safe, defaults to "standard".
func (c *MergeQueueConfig) GetReviewDepth() string {
	if c.ReviewDepth == "" {
		return "standard"
	}
	return c.ReviewDepth
}

// boolPtr returns a pointer to a bool value.
func boolPtr(b bool) *bool {
	return &b
}

// DefaultMergeQueueConfig returns a MergeQueueConfig with sensible defaults.
func DefaultMergeQueueConfig() *MergeQueueConfig {
	return &MergeQueueConfig{
		Enabled:                          true,
		IntegrationBranchPolecatEnabled:  boolPtr(true),
		IntegrationBranchRefineryEnabled: boolPtr(true),
		OnConflict:                       OnConflictAssignBack,
		RunTests:                         boolPtr(true),
		TestCommand:                      "",
		DeleteMergedBranches:             boolPtr(true),
		RetryFlakyTests:                  1,
		PollInterval:                     "30s",
		MaxConcurrent:                    1,
		StaleClaimTimeout:                "30m",
	}
}

// NamepoolConfig represents namepool settings for themed polecat names.
type NamepoolConfig struct {
	// Style picks from a built-in theme (e.g., "mad-max", "minerals", "wasteland").
	// If empty, defaults to "mad-max".
	Style string `json:"style,omitempty"`

	// Names is a custom list of names to use instead of a built-in theme.
	// If provided, overrides the Style setting.
	Names []string `json:"names,omitempty"`

	// MaxBeforeNumbering is when to start appending numbers.
	// Default is 50. After this many polecats, names become name-01, name-02, etc.
	MaxBeforeNumbering int `json:"max_before_numbering,omitempty"`
}

// DefaultNamepoolConfig returns a NamepoolConfig with sensible defaults.
func DefaultNamepoolConfig() *NamepoolConfig {
	return &NamepoolConfig{
		Style:              "mad-max",
		MaxBeforeNumbering: 50,
	}
}

// AccountsConfig represents Claude Code account configuration (mayor/accounts.json).
// This enables Gas Town to manage multiple Claude Code accounts with easy switching.
type AccountsConfig struct {
	Version  int                `json:"version"`  // schema version
	Accounts map[string]Account `json:"accounts"` // handle -> account details
	Default  string             `json:"default"`  // default account handle
}

// Account represents a single Claude Code account.
type Account struct {
	Email       string `json:"email"`                 // account email
	Description string `json:"description,omitempty"` // human description
	ConfigDir   string `json:"config_dir"`            // path to CLAUDE_CONFIG_DIR
}

// CurrentAccountsVersion is the current schema version for AccountsConfig.
const CurrentAccountsVersion = 1

// DefaultAccountsConfigDir returns the default base directory for account configs.
func DefaultAccountsConfigDir() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("cannot determine home directory: %w", err)
	}
	return home + "/.claude-accounts", nil
}

// QuotaState represents the quota management state (mayor/quota.json).
// Tracks which accounts are rate-limited and when they were last rotated.
type QuotaState struct {
	Version  int                          `json:"version"`  // schema version
	Accounts map[string]AccountQuotaState `json:"accounts"` // handle -> quota state

	// ActiveSwaps tracks keychain swap mappings from quota rotation.
	// Key: target config dir (where the swapped token was written)
	// Value: source account handle (whose token was swapped in)
	//
	// When a session is rotated, its config dir's keychain entry gets
	// overwritten with the source account's token. If the source account
	// later re-authenticates, the fresh token goes to the source's own
	// keychain entry — not the target's. SyncSwappedTokens uses this map
	// to propagate fresh tokens to all target keychain entries.
	ActiveSwaps map[string]string `json:"active_swaps,omitempty"` // targetConfigDir -> sourceAccountHandle
}

// AccountQuotaStatus is the rate-limit status of an account.
type AccountQuotaStatus string

const (
	// QuotaStatusAvailable means the account is not rate-limited.
	QuotaStatusAvailable AccountQuotaStatus = "available"

	// QuotaStatusLimited means the account has been detected as rate-limited.
	QuotaStatusLimited AccountQuotaStatus = "limited"

	// QuotaStatusCooldown means the account was limited and is in cooldown.
	QuotaStatusCooldown AccountQuotaStatus = "cooldown"
)

// AccountQuotaState tracks the quota status of a single account.
type AccountQuotaState struct {
	Status    AccountQuotaStatus `json:"status"`               // current status
	LimitedAt string             `json:"limited_at,omitempty"` // RFC3339 when limit was detected
	ResetsAt  string             `json:"resets_at,omitempty"`  // Human-readable reset time from provider (e.g. "7pm (America/Los_Angeles)")
	LastUsed  string             `json:"last_used,omitempty"`  // RFC3339 when account was last assigned to a session
}

// CurrentQuotaVersion is the current schema version for QuotaState.
const CurrentQuotaVersion = 1

// MessagingConfig represents the messaging configuration (config/messaging.json).
// This defines mailing lists, work queues, and announcement channels.
type MessagingConfig struct {
	Type    string `json:"type"`    // "messaging"
	Version int    `json:"version"` // schema version

	// Lists are static mailing lists. Messages are fanned out to all recipients.
	// Each recipient gets their own copy of the message.
	// Example: {"oncall": ["mayor/", "gastown/witness"]}
	Lists map[string][]string `json:"lists,omitempty"`

	// Queues are shared work queues. Only one copy exists; workers claim messages.
	// Messages sit in the queue until explicitly claimed by a worker.
	// Example: {"work/gastown": ["gastown/polecats/*"]}
	Queues map[string]QueueConfig `json:"queues,omitempty"`

	// Announces are bulletin boards. One copy exists; anyone can read, no claiming.
	// Used for broadcast announcements that don't need acknowledgment.
	// Example: {"alerts": {"readers": ["@town"]}}
	Announces map[string]AnnounceConfig `json:"announces,omitempty"`

	// NudgeChannels are named groups for real-time nudge fan-out.
	// Like mailing lists but for tmux send-keys instead of durable mail.
	// Example: {"workers": ["gastown/polecats/*", "gastown/crew/*"], "witnesses": ["*/witness"]}
	NudgeChannels map[string][]string `json:"nudge_channels,omitempty"`
}

// QueueConfig represents a work queue configuration.
type QueueConfig struct {
	// Workers lists addresses eligible to claim from this queue.
	// Supports wildcards: "gastown/polecats/*" matches all polecats in gastown.
	Workers []string `json:"workers"`

	// MaxClaims is the maximum number of concurrent claims (0 = unlimited).
	MaxClaims int `json:"max_claims,omitempty"`
}

// AnnounceConfig represents a bulletin board configuration.
type AnnounceConfig struct {
	// Readers lists addresses eligible to read from this announce channel.
	// Supports @group syntax: "@town", "@rig/gastown", "@witnesses".
	Readers []string `json:"readers"`

	// RetainCount is the number of messages to retain (0 = unlimited).
	RetainCount int `json:"retain_count,omitempty"`
}

// CurrentMessagingVersion is the current schema version for MessagingConfig.
const CurrentMessagingVersion = 1

// NewMessagingConfig creates a new MessagingConfig with defaults.
func NewMessagingConfig() *MessagingConfig {
	return &MessagingConfig{
		Type:          "messaging",
		Version:       CurrentMessagingVersion,
		Lists:         make(map[string][]string),
		Queues:        make(map[string]QueueConfig),
		Announces:     make(map[string]AnnounceConfig),
		NudgeChannels: make(map[string][]string),
	}
}

// EscalationConfig represents escalation routing configuration (settings/escalation.json).
// This defines severity-based routing for escalations to different channels.
type EscalationConfig struct {
	Type    string `json:"type"`    // "escalation"
	Version int    `json:"version"` // schema version

	// Routes maps severity levels to action lists.
	// Actions are executed in order for each escalation.
	// Action formats:
	//   - "bead"        → Create escalation bead (always first, implicit)
	//   - "mail:<target>" → Send gt mail to target (e.g., "mail:mayor")
	//   - "email:human" → Send email to contacts.human_email
	//   - "sms:human"   → Send SMS to contacts.human_sms
	//   - "slack"       → Post to contacts.slack_webhook
	//   - "log"         → Write to escalation log file
	Routes map[string][]string `json:"routes"`

	// Contacts contains contact information for external notification actions.
	Contacts EscalationContacts `json:"contacts"`

	// StaleThreshold is how long before an unacknowledged escalation
	// is considered stale and gets re-escalated.
	// Format: Go duration string (e.g., "4h", "30m", "24h")
	// Default: "4h"
	StaleThreshold string `json:"stale_threshold,omitempty"`

	// MaxReescalations limits how many times an escalation can be
	// re-escalated. Default: 2 (low→medium→high, then stops)
	// Pointer type to distinguish "not configured" (nil) from explicit 0.
	MaxReescalations *int `json:"max_reescalations,omitempty"`
}

// EscalationContacts contains contact information for external notification channels.
type EscalationContacts struct {
	HumanEmail   string `json:"human_email,omitempty"`   // email address for email:human action
	HumanSMS     string `json:"human_sms,omitempty"`     // phone number for sms:human action
	SlackWebhook string `json:"slack_webhook,omitempty"` // webhook URL for slack action
	SMTPHost     string `json:"smtp_host,omitempty"`     // SMTP server host (e.g. "smtp.gmail.com")
	SMTPPort     string `json:"smtp_port,omitempty"`     // SMTP server port (default "587")
	SMTPFrom     string `json:"smtp_from,omitempty"`     // sender address for email notifications
	SMTPUser     string `json:"smtp_user,omitempty"`     // SMTP auth username (optional)
	SMTPPass     string `json:"smtp_pass,omitempty"`     // SMTP auth password (optional)
	SMSWebhook   string `json:"sms_webhook,omitempty"`   // webhook URL for SMS delivery (e.g. Twilio)
}

// CurrentEscalationVersion is the current schema version for EscalationConfig.
const CurrentEscalationVersion = 1

// Escalation severity level constants.
const (
	SeverityCritical = "critical" // P0: immediate attention required
	SeverityHigh     = "high"     // P1: urgent, needs attention soon
	SeverityMedium   = "medium"   // P2: standard escalation (default)
	SeverityLow      = "low"      // P3: informational, can wait
)

// ValidSeverities returns the list of valid severity levels in order of priority.
func ValidSeverities() []string {
	return []string{SeverityLow, SeverityMedium, SeverityHigh, SeverityCritical}
}

// IsValidSeverity checks if a severity level is valid.
func IsValidSeverity(severity string) bool {
	switch severity {
	case SeverityLow, SeverityMedium, SeverityHigh, SeverityCritical:
		return true
	default:
		return false
	}
}

// NextSeverity returns the next higher severity level for re-escalation.
// Returns the same level if already at critical.
func NextSeverity(severity string) string {
	switch severity {
	case SeverityLow:
		return SeverityMedium
	case SeverityMedium:
		return SeverityHigh
	case SeverityHigh:
		return SeverityCritical
	default:
		return SeverityCritical
	}
}

// intPtr returns a pointer to the given int value.
func intPtr(v int) *int { return &v }

// NewEscalationConfig creates a new EscalationConfig with sensible defaults.
func NewEscalationConfig() *EscalationConfig {
	return &EscalationConfig{
		Type:    "escalation",
		Version: CurrentEscalationVersion,
		Routes: map[string][]string{
			SeverityLow:      {"bead"},
			SeverityMedium:   {"bead", "mail:mayor"},
			SeverityHigh:     {"bead", "mail:mayor", "email:human"},
			SeverityCritical: {"bead", "mail:mayor", "email:human", "sms:human"},
		},
		Contacts:         EscalationContacts{},
		StaleThreshold:   "4h",
		MaxReescalations: intPtr(2),
	}
}




----


V

     1 {
     2   "hooks": {
     3     "SessionStart": [{ "command": "{{GT_BIN}} prime --hook && {{GT_BIN}} mail check --inject" }],
     4     "PreCompact":  [{ "command": "{{GT_BIN}} prime --hook" }],
     5     "UserPromptSubmit": [{ "command": "{{GT_BIN}} mail check --inject" }],
     6     "Stop":        [{ "command": "{{GT_BIN}} costs record &" }]
     7   }
     8 }

    Gemini (.gemini/settings.json) — same hooks, different event names:

     1 {
     2   "hooks": {
     3     "SessionStart": [{ "command": "GT_SESSION_ID=${GT_SESSION_ID:-$(uuidgen)} GT_HOOK_SOURCE=startup {{GT_BIN}} prime --hook" }],
     4     "PreCompress":  [{ "command": "GT_HOOK_SOURCE=compact {{GT_BIN}} prime --hook" }],
     5     "BeforeAgent":  [{ "command": "{{GT_BIN}} mail check --inject" }],
     6     "SessionEnd":   [{ "command": "{{GT_BIN}} costs record &" }]
     7   }
     8 }

    Copilot (.github/hooks/gastown.json):

     1 {
     2   "hooks": {
     3     "sessionStart": [{ "bash": "{{GT_BIN}} prime --hook && {{GT_BIN}} mail check --inject" }],
     4     "userPromptSubmitted": [{ "bash": "{{GT_BIN}} mail check --inject" }],
     5     "sessionEnd":   [{ "bash": "{{GT_BIN}} costs record &" }]
     6   }
     7 }

    OpenCode (.opencode/plugins/gastown.js) — JavaScript plugin:

      1 export const GasTown = async ({ $, directory }) => {
      2   return {
      3     event: async ({ event }) => {
      4       if (event?.type === "session.created") {
      5         primePromise = loadPrime();  // Starts gt prime in background
      6       }
      7     },
      8     "experimental.chat.system.transform": async (input, output) => {
      9       const context = await primePromise;  // Injects into system prompt
     10       output.system.push(context);
     11     },
     12   };
     13 };

    Pi (.pi/extensions/gastown-hooks.js) — Pi extension:

     1 pi.on("session_start", async (event, context) => {
     2   const result = await pi.exec("gt", ["prime", "--hook"]);
     3   primeContext = result.stdout.trim();
     4 });
     5 
     6 pi.on("before_agent_start", async (event, context) => {
     7   return { systemPrompt: event.systemPrompt + "\n\n" + primeContext };
     8 });

    Codex (internal/hooks/templates/codex/hooks-autonomous.json):
    Templates exist on disk but are never installed because SupportsHooks: false in the preset. The installer skips Codex entirely.

    ---

    The Complete Startup Comparison
______