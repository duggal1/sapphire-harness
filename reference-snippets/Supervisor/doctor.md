package doctor

import (
	"errors"
	"fmt"
	"io"
	"time"

	"github.com/steveyegge/gastown/internal/ui"
)

// Doctor manages and executes health checks.
type Doctor struct {
	checks []Check
}

// NewDoctor creates a new Doctor with no registered checks.
func NewDoctor() *Doctor {
	return &Doctor{
		checks: make([]Check, 0),
	}
}

// Register adds a check to the doctor's check list.
func (d *Doctor) Register(check Check) {
	d.checks = append(d.checks, check)
}

// RegisterAll adds multiple checks to the doctor's check list.
func (d *Doctor) RegisterAll(checks ...Check) {
	d.checks = append(d.checks, checks...)
}

// Checks returns the list of registered checks.
func (d *Doctor) Checks() []Check {
	return d.checks
}

// categoryGetter interface for checks that provide a category
type categoryGetter interface {
	Category() string
}

// Run executes all registered checks and returns a report.
func (d *Doctor) Run(ctx *CheckContext) *Report {
	return d.RunStreaming(ctx, nil, 0)
}

// RunStreaming executes all registered checks with optional real-time output.
// If w is non-nil, prints each check name as it starts and result when done.
// If slowThreshold > 0, shows hourglass icon for slow checks.
func (d *Doctor) RunStreaming(ctx *CheckContext, w io.Writer, slowThreshold time.Duration) *Report {
	report := NewReport()

	for _, check := range d.checks {
		// Stream: print check name before running
		if w != nil {
			fmt.Fprintf(w, "  %s  %s...", ui.RenderMuted("○"), check.Name())
		}

		start := time.Now()
		result := check.Run(ctx)
		result.Elapsed = time.Since(start)

		// Ensure check name is populated
		if result.Name == "" {
			result.Name = check.Name()
		}
		// Set category from check if available
		if cg, ok := check.(categoryGetter); ok && result.Category == "" {
			result.Category = cg.Category()
		}

		// Stream: overwrite line with result
		if w != nil {
			var statusIcon string
			switch result.Status {
			case StatusOK:
				statusIcon = ui.RenderPassIcon()
			case StatusWarning:
				statusIcon = ui.RenderWarnIcon()
			case StatusError:
				statusIcon = ui.RenderFailIcon()
			}
			// Check if slow (hourglass replaces spaces to maintain alignment)
			isSlow := slowThreshold > 0 && result.Elapsed >= slowThreshold
			slowIndicator := "  "
			if isSlow {
				report.Summary.Slow++
				slowIndicator = "⏳"
			}
			fmt.Fprintf(w, "\r  %s%s%s", statusIcon, slowIndicator, result.Name)
			if result.Message != "" {
				fmt.Fprintf(w, "%s", ui.RenderMuted(" "+result.Message))
			}
			if isSlow {
				fmt.Fprintf(w, "%s", ui.RenderMuted(" ("+formatDuration(result.Elapsed)+")"))
			}
			fmt.Fprintln(w)
		}

		report.Add(result)
	}

	return report
}

// Fix runs all checks with auto-fix enabled where possible.
// It first runs the check, then if it fails and can be fixed, attempts the fix.
func (d *Doctor) Fix(ctx *CheckContext) *Report {
	return d.FixStreaming(ctx, nil, 0)
}

// safeFixCheck calls check.Fix() with panic recovery. If the Fix method panics
// (e.g., due to a Dolt nil pointer dereference propagating in-process — GH#1769),
// the panic is caught and returned as an error instead of crashing gt doctor.
func safeFixCheck(check Check, ctx *CheckContext) (retErr error) {
	defer func() {
		if r := recover(); r != nil {
			retErr = fmt.Errorf("fix panicked: %v", r)
		}
	}()
	return check.Fix(ctx)
}

// FixStreaming runs all checks with auto-fix and optional real-time output.
// If w is non-nil, prints each check name as it starts and result when done.
// If slowThreshold > 0, shows hourglass icon for slow checks.
func (d *Doctor) FixStreaming(ctx *CheckContext, w io.Writer, slowThreshold time.Duration) *Report {
	report := NewReport()

	for _, check := range d.checks {
		// Stream: print check name before running
		if w != nil {
			fmt.Fprintf(w, "  %s  %s...", ui.RenderMuted("○"), check.Name())
		}

		start := time.Now()
		result := check.Run(ctx)
		if result.Name == "" {
			result.Name = check.Name()
		}
		// Set category from check if available
		if cg, ok := check.(categoryGetter); ok && result.Category == "" {
			result.Category = cg.Category()
		}

		// Attempt fix if check failed and is fixable
		if result.Status != StatusOK && check.CanFix() {
			// Stream: show the problem with fixing indicator (all on same line)
			if w != nil {
				var problemIcon string
				if result.Status == StatusError {
					problemIcon = ui.RenderFailIcon()
				} else {
					problemIcon = ui.RenderWarnIcon()
				}
				// Overwrite the "checking" line with problem status + fixing indicator
				fmt.Fprintf(w, "\r  %s  %s", problemIcon, check.Name())
				if result.Message != "" {
					fmt.Fprintf(w, "%s", ui.RenderMuted(" "+result.Message))
				}
				fmt.Fprintf(w, "%s", ui.RenderMuted(" (fixing)..."))
			}

			err := safeFixCheck(check, ctx)
			if err == nil {
				// Re-run check to verify fix worked
				result = check.Run(ctx)
				if result.Name == "" {
					result.Name = check.Name()
				}
				// Set category again after re-run
				if cg, ok := check.(categoryGetter); ok && result.Category == "" {
					result.Category = cg.Category()
				}
				// Update message to indicate fix was applied
				if result.Status == StatusOK {
					result.Message = result.Message + " (fixed)"
					result.Fixed = true
				}
			} else if errors.Is(err, ErrSkippedNoStart) {
				// Fix skipped due to --no-start flag
				result.Details = append(result.Details, "Skipped: --no-start suppresses startup")
			} else {
				// Fix failed, add error to details
				result.Details = append(result.Details, "Fix failed: "+err.Error())
			}
		}

		// Record total elapsed time including any fix attempts
		result.Elapsed = time.Since(start)

		// Stream: overwrite line with final result
		if w != nil {
			var statusIcon string
			if result.Fixed {
				statusIcon = ui.RenderFixIcon()
			} else {
				switch result.Status {
				case StatusOK:
					statusIcon = ui.RenderPassIcon()
				case StatusWarning:
					statusIcon = ui.RenderWarnIcon()
				case StatusError:
					statusIcon = ui.RenderFailIcon()
				}
			}
			// Check if slow (hourglass replaces spaces to maintain alignment)
			// Fix icon (🔧) is double-width, so use one less padding space
			isSlow := slowThreshold > 0 && result.Elapsed >= slowThreshold
			slowIndicator := "  "
			if result.Fixed {
				slowIndicator = " "
			}
			if isSlow {
				report.Summary.Slow++
				slowIndicator = "⏳"
			}
			fmt.Fprintf(w, "\r  %s%s%s", statusIcon, slowIndicator, result.Name)
			if result.Message != "" {
				fmt.Fprintf(w, "%s", ui.RenderMuted(" "+result.Message))
			}
			if isSlow {
				fmt.Fprintf(w, "%s", ui.RenderMuted(" ("+formatDuration(result.Elapsed)+")"))
			}
			fmt.Fprintln(w)
		}

		report.Add(result)
	}

	return report
}

// BaseCheck provides a base implementation for checks that don't support auto-fix.
// Embed this in custom checks to get default CanFix() and Fix() implementations.
type BaseCheck struct {
	CheckName        string
	CheckDescription string
	CheckCategory    string // Category for grouping (e.g., CategoryCore)
}

// Category returns the check's category for grouping in output.
func (b *BaseCheck) Category() string {
	return b.CheckCategory
}

// Name returns the check name.
func (b *BaseCheck) Name() string {
	return b.CheckName
}

// Description returns the check description.
func (b *BaseCheck) Description() string {
	return b.CheckDescription
}

// CanFix returns false by default.
func (b *BaseCheck) CanFix() bool {
	return false
}

// Fix returns an error indicating this check cannot be auto-fixed.
func (b *BaseCheck) Fix(ctx *CheckContext) error {
	return ErrCannotFix
}

// FixableCheck provides a base implementation for checks that support auto-fix.
// Embed this and override CanFix() to return true, and implement Fix().
type FixableCheck struct {
	BaseCheck
}

// CanFix returns true for fixable checks.
func (f *FixableCheck) CanFix() bool {
	return true
}
// Package doctor provides a framework for running health checks on Gas Town workspaces.
package doctor

import (
	"fmt"
	"io"
	"time"

	"github.com/steveyegge/gastown/internal/ui"
)

// Category constants for grouping checks
const (
	CategoryCore          = "Core"
	CategoryInfrastructure = "Infrastructure"
	CategoryRig           = "Rig"
	CategoryPatrol        = "Patrol"
	CategoryConfig        = "Configuration"
	CategoryCleanup       = "Cleanup"
	CategoryHooks         = "Hooks"
)

// CategoryOrder defines the display order for categories
var CategoryOrder = []string{
	CategoryCore,
	CategoryInfrastructure,
	CategoryRig,
	CategoryPatrol,
	CategoryConfig,
	CategoryCleanup,
	CategoryHooks,
}

// CheckStatus represents the result status of a health check.
type CheckStatus int

const (
	// StatusOK indicates the check passed.
	StatusOK CheckStatus = iota
	// StatusWarning indicates a non-critical issue.
	StatusWarning
	// StatusError indicates a critical problem.
	StatusError
)

// String returns a human-readable status.
func (s CheckStatus) String() string {
	switch s {
	case StatusOK:
		return "OK"
	case StatusWarning:
		return "Warning"
	case StatusError:
		return "Error"
	default:
		return "Unknown"
	}
}

// CheckContext provides context for running checks.
type CheckContext struct {
	TownRoot        string // Root directory of the Gas Town workspace
	RigName         string // Rig name (empty for town-level checks)
	Verbose         bool   // Enable verbose output
	RestartSessions bool   // Restart patrol sessions when fixing (requires explicit --restart-sessions flag)
	NoStart         bool   // Suppress starting daemon/agents during --fix
}

// RigPath returns the full path to the rig directory.
// Returns empty string if RigName is not set.
func (ctx *CheckContext) RigPath() string {
	if ctx.RigName == "" {
		return ""
	}
	return ctx.TownRoot + "/" + ctx.RigName
}

// DefaultSlowThreshold is the default duration above which a check is considered slow.
const DefaultSlowThreshold = 1 * time.Second

// CheckResult represents the outcome of a health check.
type CheckResult struct {
	Name     string        // Check name
	Status   CheckStatus   // Result status
	Message  string        // Primary result message
	Details  []string      // Additional information
	FixHint  string        // Suggestion if not auto-fixable
	Category string        // Category for grouping (e.g., CategoryCore)
	Elapsed  time.Duration // How long the check took to run
	Fixed    bool          // True if this check was auto-fixed
}

// Check defines the interface for a health check.
type Check interface {
	// Name returns the check identifier.
	Name() string

	// Description returns a human-readable description.
	Description() string

	// Run executes the check and returns a result.
	Run(ctx *CheckContext) *CheckResult

	// Fix attempts to automatically fix the issue.
	// Should only be called if CanFix() returns true.
	Fix(ctx *CheckContext) error

	// CanFix returns true if this check can automatically fix issues.
	CanFix() bool
}

// ReportSummary summarizes the results of all checks.
type ReportSummary struct {
	Total       int
	OK          int
	Warnings    int
	Errors      int
	Fixed       int           // Checks that were auto-fixed
	Slow        int           // Checks that took longer than threshold (counted during Print)
	SlowestName string        // Name of the slowest check
	SlowestTime time.Duration // Duration of the slowest check
}

// Report contains all check results and a summary.
type Report struct {
	Timestamp time.Time
	Checks    []*CheckResult
	Summary   ReportSummary
}

// NewReport creates an empty report with the current timestamp.
func NewReport() *Report {
	return &Report{
		Timestamp: time.Now(),
		Checks:    make([]*CheckResult, 0),
	}
}

// Add adds a check result to the report and updates the summary.
func (r *Report) Add(result *CheckResult) {
	r.Checks = append(r.Checks, result)
	r.Summary.Total++

	switch result.Status {
	case StatusOK:
		r.Summary.OK++
	case StatusWarning:
		r.Summary.Warnings++
	case StatusError:
		r.Summary.Errors++
	}

	// Track fixed checks
	if result.Fixed {
		r.Summary.Fixed++
	}

	// Track the slowest check
	if result.Elapsed > r.Summary.SlowestTime {
		r.Summary.SlowestName = result.Name
		r.Summary.SlowestTime = result.Elapsed
	}
}

// HasErrors returns true if any check reported an error.
func (r *Report) HasErrors() bool {
	return r.Summary.Errors > 0
}

// HasWarnings returns true if any check reported a warning.
func (r *Report) HasWarnings() bool {
	return r.Summary.Warnings > 0
}

// IsHealthy returns true if all checks passed without errors or warnings.
func (r *Report) IsHealthy() bool {
	return r.Summary.Errors == 0 && r.Summary.Warnings == 0
}

// PrintSummaryOnly outputs just the summary and warnings section.
// Used after streaming output where checks were already printed as they ran.
// Slow checks are already counted during streaming, so slowThreshold is only
// used for the summary display.
func (r *Report) PrintSummaryOnly(w io.Writer, verbose bool, slowThreshold time.Duration) {
	// Collect warnings/errors for summary section
	var warnings []*CheckResult
	for _, check := range r.Checks {
		if check.Status != StatusOK {
			warnings = append(warnings, check)
		}
	}

	// Print separator and summary
	_, _ = fmt.Fprintln(w, ui.RenderSeparator())
	r.printSummary(w, slowThreshold)

	// Print warnings/errors section with fixes
	r.printWarningsSection(w, warnings)

	// Print details for non-OK checks in verbose mode
	if verbose && len(warnings) > 0 {
		for _, check := range warnings {
			if len(check.Details) > 0 {
				for _, detail := range check.Details {
					_, _ = fmt.Fprintf(w, "     %s%s\n", ui.MutedStyle.Render(ui.TreeLast), ui.RenderMuted(detail))
				}
			}
		}
	}
}

// Print outputs the report to the given writer.
// Matches bd doctor UX: grouped by category, semantic icons, warnings section.
// If slowThreshold > 0, displays elapsed time for checks exceeding the threshold.
func (r *Report) Print(w io.Writer, verbose bool, slowThreshold time.Duration) {
	// Print header with version placeholder (caller should set via PrintWithVersion)
	_, _ = fmt.Fprintln(w)

	// Group checks by category
	checksByCategory := make(map[string][]*CheckResult)
	for _, check := range r.Checks {
		cat := check.Category
		if cat == "" {
			cat = "Other"
		}
		checksByCategory[cat] = append(checksByCategory[cat], check)
	}

	// Track warnings/errors for summary section
	var warnings []*CheckResult

	// Print checks by category in defined order
	for _, category := range CategoryOrder {
		checks, exists := checksByCategory[category]
		if !exists || len(checks) == 0 {
			continue
		}

		// Print category header
		_, _ = fmt.Fprintln(w, ui.RenderCategory(category))

		// Print each check in this category
		for _, check := range checks {
			r.printCheck(w, check, verbose, slowThreshold)
			if check.Status != StatusOK {
				warnings = append(warnings, check)
			}
		}
		_, _ = fmt.Fprintln(w)
	}

	// Print any checks without a category
	if otherChecks, exists := checksByCategory["Other"]; exists && len(otherChecks) > 0 {
		_, _ = fmt.Fprintln(w, ui.RenderCategory("Other"))
		for _, check := range otherChecks {
			r.printCheck(w, check, verbose, slowThreshold)
			if check.Status != StatusOK {
				warnings = append(warnings, check)
			}
		}
		_, _ = fmt.Fprintln(w)
	}

	// Print separator and summary
	_, _ = fmt.Fprintln(w, ui.RenderSeparator())
	r.printSummary(w, slowThreshold)

	// Print warnings/errors section with fixes
	r.printWarningsSection(w, warnings)
}

// printCheck outputs a single check result with semantic styling.
func (r *Report) printCheck(w io.Writer, check *CheckResult, verbose bool, slowThreshold time.Duration) {
	var statusIcon string
	switch check.Status {
	case StatusOK:
		statusIcon = ui.RenderPassIcon()
	case StatusWarning:
		statusIcon = ui.RenderWarnIcon()
	case StatusError:
		statusIcon = ui.RenderFailIcon()
	}

	// Add hourglass for slow checks (only when --slow is enabled)
	isSlow := slowThreshold > 0 && check.Elapsed >= slowThreshold
	if isSlow {
		r.Summary.Slow++ // Count slow checks during print
	}

	// Print check line: icon + name + muted message + optional timing
	// For slow checks, hourglass replaces spaces to maintain alignment
	slowIndicator := "  "
	if isSlow {
		slowIndicator = "⏳"
	}
	_, _ = fmt.Fprintf(w, "  %s%s%s", statusIcon, slowIndicator, check.Name)
	if check.Message != "" {
		_, _ = fmt.Fprintf(w, "%s", ui.RenderMuted(" "+check.Message))
	}
	if isSlow {
		_, _ = fmt.Fprintf(w, "%s", ui.RenderMuted(" ("+formatDuration(check.Elapsed)+")"))
	}
	_, _ = fmt.Fprintln(w)

	// Print details in verbose mode or for non-OK results (with tree connector)
	if len(check.Details) > 0 && (verbose || check.Status != StatusOK) {
		for _, detail := range check.Details {
			_, _ = fmt.Fprintf(w, "     %s%s\n", ui.MutedStyle.Render(ui.TreeLast), ui.RenderMuted(detail))
		}
	}
}

// formatDuration formats a duration in a human-readable way.
// Examples: "1.2s", "45s", "1m 30s", "2h 5m"
func formatDuration(d time.Duration) string {
	if d < time.Minute {
		return fmt.Sprintf("%.1fs", d.Seconds())
	}
	if d < time.Hour {
		m := int(d.Minutes())
		s := int(d.Seconds()) % 60
		if s == 0 {
			return fmt.Sprintf("%dm", m)
		}
		return fmt.Sprintf("%dm %ds", m, s)
	}
	h := int(d.Hours())
	m := int(d.Minutes()) % 60
	if m == 0 {
		return fmt.Sprintf("%dh", h)
	}
	return fmt.Sprintf("%dh %dm", h, m)
}

// printSummary outputs the summary line with semantic icons.
func (r *Report) printSummary(w io.Writer, slowThreshold time.Duration) {
	summary := fmt.Sprintf("%s %d passed  %s %d warnings  %s %d failed",
		ui.RenderPassIcon(), r.Summary.OK,
		ui.RenderWarnIcon(), r.Summary.Warnings,
		ui.RenderFailIcon(), r.Summary.Errors,
	)
	if r.Summary.Fixed > 0 {
		summary += fmt.Sprintf("  🔧 %d fixed", r.Summary.Fixed)
	}
	if slowThreshold > 0 && r.Summary.Slow > 0 {
		summary += fmt.Sprintf("  ⏳ %d slow (slowest: %s %s)",
			r.Summary.Slow,
			r.Summary.SlowestName,
			formatDuration(r.Summary.SlowestTime),
		)
	}
	_, _ = fmt.Fprintln(w, summary)
}

// printWarningsSection outputs separate sections for failures, warnings, and fixed items.
func (r *Report) printWarningsSection(w io.Writer, issues []*CheckResult) {
	// Separate into categories
	var failures, warnings, fixed []*CheckResult
	for _, check := range issues {
		if check.Fixed {
			fixed = append(fixed, check)
		} else if check.Status == StatusError {
			failures = append(failures, check)
		} else {
			warnings = append(warnings, check)
		}
	}

	// Also collect fixed items from all checks (not just issues)
	for _, check := range r.Checks {
		if check.Fixed && check.Status == StatusOK {
			fixed = append(fixed, check)
		}
	}

	// If nothing to report, show success message
	if len(failures) == 0 && len(warnings) == 0 && len(fixed) == 0 {
		_, _ = fmt.Fprintln(w)
		_, _ = fmt.Fprintln(w, ui.RenderPass(ui.IconPass+" All checks passed"))
		return
	}

	// Print FAILURES section
	if len(failures) > 0 {
		_, _ = fmt.Fprintln(w)
		_, _ = fmt.Fprintln(w, ui.RenderFail(ui.IconFail+"  FAILURES"))
		for i, check := range failures {
			line := fmt.Sprintf("%s: %s", check.Name, check.Message)
			_, _ = fmt.Fprintf(w, "  %s  %s %s\n", ui.RenderFailIcon(), ui.RenderFail(fmt.Sprintf("%d.", i+1)), ui.RenderFail(line))
			if check.FixHint != "" {
				_, _ = fmt.Fprintf(w, "        %s%s\n", ui.MutedStyle.Render(ui.TreeLast), check.FixHint)
			}
		}
	}

	// Print WARNINGS section
	if len(warnings) > 0 {
		_, _ = fmt.Fprintln(w)
		_, _ = fmt.Fprintln(w, ui.RenderWarn(ui.IconWarn+"  WARNINGS"))
		for i, check := range warnings {
			line := fmt.Sprintf("%s: %s", check.Name, check.Message)
			_, _ = fmt.Fprintf(w, "  %s  %s %s\n", ui.RenderWarnIcon(), ui.RenderWarn(fmt.Sprintf("%d.", i+1)), line)
			if check.FixHint != "" {
				_, _ = fmt.Fprintf(w, "        %s%s\n", ui.MutedStyle.Render(ui.TreeLast), check.FixHint)
			}
		}
	}

	// Print FIXED section
	if len(fixed) > 0 {
		_, _ = fmt.Fprintln(w)
		_, _ = fmt.Fprintln(w, ui.RenderPass("🔧  FIXED"))
		for i, check := range fixed {
			line := fmt.Sprintf("%s: %s", check.Name, check.Message)
			_, _ = fmt.Fprintf(w, "  %s  %s %s\n", ui.RenderPassIcon(), ui.RenderMuted(fmt.Sprintf("%d.", i+1)), ui.RenderMuted(line))
		}
	}

	// If only fixed items, show success message
	if len(failures) == 0 && len(warnings) == 0 {
		_, _ = fmt.Fprintln(w)
		_, _ = fmt.Fprintln(w, ui.RenderPass(ui.IconPass+" All remaining checks passed"))
	}
}
package doctor

import (
	"fmt"
	"strings"

	"github.com/steveyegge/gastown/internal/session"
	"github.com/steveyegge/gastown/internal/tmux"
)

// LinkedPaneCheck detects tmux sessions that share panes,
// which can cause crosstalk (messages sent to one session appearing in another).
type LinkedPaneCheck struct {
	FixableCheck
	linkedSessions []string // Sessions with linked panes, cached for Fix
}

// NewLinkedPaneCheck creates a new linked pane check.
func NewLinkedPaneCheck() *LinkedPaneCheck {
	return &LinkedPaneCheck{
		FixableCheck: FixableCheck{
			BaseCheck: BaseCheck{
				CheckName:        "linked-panes",
				CheckDescription: "Detect tmux sessions sharing panes (causes crosstalk)",
				CheckCategory:    CategoryInfrastructure,
			},
		},
	}
}

// Run checks for linked panes across Gas Town tmux sessions.
func (c *LinkedPaneCheck) Run(ctx *CheckContext) *CheckResult {
	t := tmux.NewTmux()

	sessions, err := t.ListSessions()
	if err != nil {
		return &CheckResult{
			Name:    c.Name(),
			Status:  StatusWarning,
			Message: "Could not list tmux sessions",
			Details: []string{err.Error()},
		}
	}

	// Filter to Gas Town sessions only
	var gtSessions []string
	for _, s := range sessions {
		if session.IsKnownSession(s) {
			gtSessions = append(gtSessions, s)
		}
	}

	if len(gtSessions) < 2 {
		return &CheckResult{
			Name:    c.Name(),
			Status:  StatusOK,
			Message: "Not enough sessions to check for linking",
		}
	}

	// Map pane IDs to sessions that contain them
	paneToSessions := make(map[string][]string)

	for _, session := range gtSessions {
		panes, err := c.getSessionPanes(session)
		if err != nil {
			continue
		}
		for _, pane := range panes {
			paneToSessions[pane] = append(paneToSessions[pane], session)
		}
	}

	// Find panes shared by multiple sessions
	var conflicts []string
	linkedSessionSet := make(map[string]bool)

	for pane, sessions := range paneToSessions {
		if len(sessions) > 1 {
			conflicts = append(conflicts, fmt.Sprintf("Pane %s shared by: %s", pane, strings.Join(sessions, ", ")))
			for _, s := range sessions {
				linkedSessionSet[s] = true
			}
		}
	}

	// Cache for Fix (exclude mayor session since we don't want to kill it)
	mayorSession := session.MayorSessionName()

	c.linkedSessions = nil
	for sess := range linkedSessionSet {
		if mayorSession == "" || sess != mayorSession {
			c.linkedSessions = append(c.linkedSessions, sess)
		}
	}

	if len(conflicts) == 0 {
		return &CheckResult{
			Name:    c.Name(),
			Status:  StatusOK,
			Message: fmt.Sprintf("All %d Gas Town sessions have independent panes", len(gtSessions)),
		}
	}

	return &CheckResult{
		Name:    c.Name(),
		Status:  StatusError,
		Message: fmt.Sprintf("Found %d linked pane(s) causing crosstalk!", len(conflicts)),
		Details: conflicts,
		FixHint: "Run 'gt doctor --fix' to kill linked sessions (daemon will recreate)",
	}
}

// Fix kills sessions with linked panes (except mayor session).
// The daemon will recreate them with independent panes.
func (c *LinkedPaneCheck) Fix(ctx *CheckContext) error {
	if len(c.linkedSessions) == 0 {
		return nil
	}

	t := tmux.NewTmux()
	var lastErr error

	for _, session := range c.linkedSessions {
		// Use KillSessionWithProcesses to ensure all descendant processes are killed.
		if err := t.KillSessionWithProcesses(session); err != nil {
			lastErr = err
		}
	}

	return lastErr
}

// getSessionPanes returns all pane IDs for a session.
func (c *LinkedPaneCheck) getSessionPanes(session string) ([]string, error) {
	// Get pane IDs using tmux list-panes with format
	// Using #{pane_id} which gives us the unique pane identifier like %123
	// Note: -s flag lists all panes in all windows of this session (not -a which is global)
	out, err := tmux.BuildCommand("list-panes", "-t", session, "-s", "-F", "#{pane_id}").Output()
	if err != nil {
		return nil, err
	}

	var panes []string
	for _, line := range strings.Split(strings.TrimSpace(string(out)), "\n") {
		if line != "" {
			panes = append(panes, line)
		}
	}

	return panes, nil
}
