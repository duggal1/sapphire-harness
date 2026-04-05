// Package mail provides messaging for agent communication via beads.
package mail

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"strings"
	"time"
)

// Priority levels for messages.
type Priority string

const (
	// PriorityLow is for non-urgent messages.
	PriorityLow Priority = "low"

	// PriorityNormal is the default priority.
	PriorityNormal Priority = "normal"

	// PriorityHigh indicates an important message.
	PriorityHigh Priority = "high"

	// PriorityUrgent indicates an urgent message requiring immediate attention.
	PriorityUrgent Priority = "urgent"
)

// MessageType indicates the purpose of a message.
type MessageType string

const (
	// TypeTask indicates a message requiring action from the recipient.
	TypeTask MessageType = "task"

	// TypeEscalation indicates a structured escalation copy persisted in mail.
	TypeEscalation MessageType = "escalation"

	// TypeScavenge indicates optional first-come-first-served work.
	TypeScavenge MessageType = "scavenge"

	// TypeNotification is an informational message (default).
	TypeNotification MessageType = "notification"

	// TypeReply is a response to another message.
	TypeReply MessageType = "reply"
)

// Delivery specifies how a message is delivered to the recipient.
type Delivery string

const (
	// DeliveryQueue creates the message in the mailbox for periodic checking.
	// This is the default delivery mode. Agent checks with `gt mail check`.
	DeliveryQueue Delivery = "queue"

	// DeliveryInterrupt injects a system-reminder directly into the agent's session.
	// Use for lifecycle events, URGENT priority, or stuck detection.
	DeliveryInterrupt Delivery = "interrupt"
)

// Message represents a mail message between agents.
// This is the GGT-side representation; it gets translated to/from beads messages.
type Message struct {
	// ID is a unique message identifier (beads issue ID like "bd-abc123").
	ID string `json:"id"`

	// From is the sender address (e.g., "gastown/Toast" or "mayor/").
	From string `json:"from"`

	// To is the recipient address.
	To string `json:"to"`

	// Subject is a brief summary.
	Subject string `json:"subject"`

	// Body is the full message content.
	Body string `json:"body"`

	// Timestamp is when the message was sent.
	Timestamp time.Time `json:"timestamp"`

	// Read indicates if the message has been read (closed in beads).
	Read bool `json:"read"`

	// Priority is the message priority.
	Priority Priority `json:"priority"`

	// Type indicates the message type (task, escalation, scavenge, notification, reply).
	Type MessageType `json:"type"`

	// Delivery specifies how the message is delivered (queue or interrupt).
	// Queue: agent checks periodically. Interrupt: inject into session.
	Delivery Delivery `json:"delivery,omitempty"`

	// ThreadID groups related messages into a conversation thread.
	ThreadID string `json:"thread_id,omitempty"`

	// ReplyTo is the ID of the message this is replying to.
	ReplyTo string `json:"reply_to,omitempty"`

	// Pinned marks the message as pinned (won't be auto-archived).
	Pinned bool `json:"pinned,omitempty"`

	// Wisp marks this as a transient message (stored in same DB but not synced to git).
	// Wisp messages auto-cleanup on patrol squash.
	Wisp bool `json:"wisp,omitempty"`

	// CC contains addresses that should receive a copy of this message.
	// CC'd recipients see the message in their inbox but are not the primary recipient.
	CC []string `json:"cc,omitempty"`

	// Queue is the queue name for queue-routed messages.
	// Mutually exclusive with To and Channel - a message is either direct, queued, or broadcast.
	Queue string `json:"queue,omitempty"`

	// Channel is the channel name for broadcast messages.
	// Mutually exclusive with To and Queue - a message is either direct, queued, or broadcast.
	Channel string `json:"channel,omitempty"`

	// ClaimedBy is the agent that claimed this queue message.
	// Only set for queue messages after claiming.
	ClaimedBy string `json:"claimed_by,omitempty"`

	// ClaimedAt is when the queue message was claimed.
	// Only set for queue messages after claiming.
	ClaimedAt *time.Time `json:"claimed_at,omitempty"`

	// DeliveryState tracks two-phase mailbox delivery state: pending or acked.
	DeliveryState string `json:"delivery_state,omitempty"`
	// DeliveryAckedBy is the recipient identity that acknowledged receipt.
	DeliveryAckedBy string `json:"delivery_acked_by,omitempty"`
	// DeliveryAckedAt is when receipt was acknowledged.
	DeliveryAckedAt *time.Time `json:"delivery_acked_at,omitempty"`

	// SuppressNotify tells the router to skip all recipient notification
	// (no nudge, no banner). Set by the CLI when --no-notify is passed.
	// In-memory only — not serialized.
	SuppressNotify bool `json:"-"`
}

// NewMessage creates a new message with a generated ID and thread ID.
func NewMessage(from, to, subject, body string) *Message {
	return &Message{
		ID:        GenerateID(),
		From:      from,
		To:        to,
		Subject:   subject,
		Body:      body,
		Timestamp: time.Now(),
		Read:      false,
		Priority:  PriorityNormal,
		Type:      TypeNotification,
		ThreadID:  generateThreadID(),
	}
}

// NewReplyMessage creates a reply message that inherits the thread from the original.
func NewReplyMessage(from, to, subject, body string, original *Message) *Message {
	return &Message{
		ID:        GenerateID(),
		From:      from,
		To:        to,
		Subject:   subject,
		Body:      body,
		Timestamp: time.Now(),
		Read:      false,
		Priority:  PriorityNormal,
		Type:      TypeReply,
		ThreadID:  original.ThreadID,
		ReplyTo:   original.ID,
	}
}

// NewQueueMessage creates a message destined for a queue.
// Queue messages have no direct recipient - they are claimed by eligible agents.
func NewQueueMessage(from, queue, subject, body string) *Message {
	return &Message{
		ID:        GenerateID(),
		From:      from,
		Queue:     queue,
		Subject:   subject,
		Body:      body,
		Timestamp: time.Now(),
		Read:      false,
		Priority:  PriorityNormal,
		Type:      TypeTask, // Queue messages are typically tasks
		ThreadID:  generateThreadID(),
	}
}

// NewChannelMessage creates a broadcast message for a channel.
// Channel messages are visible to all readers of the channel.
func NewChannelMessage(from, channel, subject, body string) *Message {
	return &Message{
		ID:        GenerateID(),
		From:      from,
		Channel:   channel,
		Subject:   subject,
		Body:      body,
		Timestamp: time.Now(),
		Read:      false,
		Priority:  PriorityNormal,
		Type:      TypeNotification,
		ThreadID:  generateThreadID(),
	}
}

// IsQueueMessage returns true if this is a queue-routed message.
func (m *Message) IsQueueMessage() bool {
	return m.Queue != ""
}

// IsChannelMessage returns true if this is a channel broadcast message.
func (m *Message) IsChannelMessage() bool {
	return m.Channel != ""
}

// IsDirectMessage returns true if this is a direct (To-addressed) message.
func (m *Message) IsDirectMessage() bool {
	return m.Queue == "" && m.Channel == "" && m.To != ""
}

// IsClaimed returns true if this queue message has been claimed.
func (m *Message) IsClaimed() bool {
	return m.ClaimedBy != ""
}

// Validate checks that the message has valid required fields and routing configuration.
// Returns an error if required fields are missing or routing targets are not mutually exclusive.
func (m *Message) Validate() error {
	// Required fields
	if m.ID == "" {
		return fmt.Errorf("message must have an ID")
	}
	if m.From == "" {
		return fmt.Errorf("message must have a From address")
	}
	if m.Subject == "" {
		return fmt.Errorf("message must have a Subject")
	}

	// Routing: exactly one of To, Queue, or Channel
	count := 0
	if m.To != "" {
		count++
	}
	if m.Queue != "" {
		count++
	}
	if m.Channel != "" {
		count++
	}

	if count == 0 {
		return fmt.Errorf("message must have exactly one of: to, queue, or channel")
	}
	if count > 1 {
		return fmt.Errorf("message cannot have multiple routing targets (to, queue, channel are mutually exclusive)")
	}

	// ClaimedBy/ClaimedAt only valid for queue messages
	if m.ClaimedBy != "" && m.Queue == "" {
		return fmt.Errorf("claimed_by is only valid for queue messages")
	}
	if m.ClaimedAt != nil && m.Queue == "" {
		return fmt.Errorf("claimed_at is only valid for queue messages")
	}

	return nil
}

// GenerateID creates a random message ID for in-memory tracking (notifications, logging).
// Falls back to time-based ID if crypto/rand fails (extremely rare).
// NOTE: This ID is NOT passed to bd create — bd auto-generates IDs with the correct
// database prefix. This is only used for msg.ID in the Message struct.
func GenerateID() string {
	b := make([]byte, 8)
	if _, err := rand.Read(b); err != nil {
		// Fallback to time-based ID instead of panicking
		return fmt.Sprintf("msg-%x", time.Now().UnixNano())
	}
	return "msg-" + hex.EncodeToString(b)
}

// generateThreadID creates a random thread ID.
// Falls back to time-based ID if crypto/rand fails (extremely rare).
func generateThreadID() string {
	b := make([]byte, 6)
	if _, err := rand.Read(b); err != nil {
		// Fallback to time-based ID instead of panicking
		return fmt.Sprintf("thread-%x", time.Now().UnixNano())
	}
	return "thread-" + hex.EncodeToString(b)
}

// BeadsMessage represents a message as returned by bd list/show commands.
// Messages are beads issues with type=message and metadata stored in labels.
type BeadsMessage struct {
	ID          string    `json:"id"`
	Title       string    `json:"title"`       // Subject
	Description string    `json:"description"` // Body
	Assignee    string    `json:"assignee"`    // To identity (for direct messages)
	Priority    int       `json:"priority"`    // 0=urgent, 1=high, 2=normal, 3=low
	Status      string    `json:"status"`      // open=unread, closed=read
	CreatedAt   time.Time `json:"created_at"`
	Labels      []string  `json:"labels"` // Metadata labels (from:X, thread:X, reply-to:X, msg-type:X, cc:X, queue:X, channel:X, claimed-by:X, claimed-at:X)
	Pinned      bool      `json:"pinned,omitempty"`
	Wisp        bool      `json:"wisp,omitempty"` // Ephemeral message (not synced to git)

	// Cached parsed values (populated by ParseLabels)
	sender    string
	threadID  string
	replyTo   string
	msgType   string
	cc        []string   // CC recipients
	queue     string     // Queue name (for queue messages)
	channel   string     // Channel name (for broadcast messages)
	claimedBy string     // Who claimed the queue message
	claimedAt *time.Time // When the queue message was claimed
	// Two-phase delivery metadata
	deliveryState   string
	deliveryAckedBy string
	deliveryAckedAt *time.Time
}

// ParseLabels extracts metadata from the labels array.
// Safe to call multiple times - resets parsed state before re-parsing.
func (bm *BeadsMessage) ParseLabels() {
	bm.sender = ""
	bm.threadID = ""
	bm.replyTo = ""
	bm.msgType = ""
	bm.cc = nil
	bm.queue = ""
	bm.channel = ""
	bm.claimedBy = ""
	bm.claimedAt = nil
	bm.deliveryState = ""
	bm.deliveryAckedBy = ""
	bm.deliveryAckedAt = nil

	for _, label := range bm.Labels {
		if strings.HasPrefix(label, "from:") {
			bm.sender = strings.TrimPrefix(label, "from:")
		} else if strings.HasPrefix(label, "thread:") {
			bm.threadID = strings.TrimPrefix(label, "thread:")
		} else if strings.HasPrefix(label, "reply-to:") {
			bm.replyTo = strings.TrimPrefix(label, "reply-to:")
		} else if strings.HasPrefix(label, "msg-type:") {
			bm.msgType = strings.TrimPrefix(label, "msg-type:")
		} else if strings.HasPrefix(label, "cc:") {
			bm.cc = append(bm.cc, strings.TrimPrefix(label, "cc:"))
		} else if strings.HasPrefix(label, "queue:") {
			bm.queue = strings.TrimPrefix(label, "queue:")
		} else if strings.HasPrefix(label, "channel:") {
			bm.channel = strings.TrimPrefix(label, "channel:")
		} else if strings.HasPrefix(label, "claimed-by:") {
			bm.claimedBy = strings.TrimPrefix(label, "claimed-by:")
		} else if strings.HasPrefix(label, "claimed-at:") {
			ts := strings.TrimPrefix(label, "claimed-at:")
			if t, err := time.Parse(time.RFC3339, ts); err == nil {
				bm.claimedAt = &t
			}
		}
	}

	bm.deliveryState, bm.deliveryAckedBy, bm.deliveryAckedAt = ParseDeliveryLabels(bm.Labels)
}

// GetCC returns the parsed CC recipients.
func (bm *BeadsMessage) GetCC() []string {
	return bm.cc
}

// IsCCRecipient checks if the given identity is in the CC list.
func (bm *BeadsMessage) IsCCRecipient(identity string) bool {
	for _, cc := range bm.cc {
		if cc == identity {
			return true
		}
	}
	return false
}

// ToMessage converts a BeadsMessage to a GGT Message.
func (bm *BeadsMessage) ToMessage() *Message {
	// Parse labels to extract metadata
	bm.ParseLabels()

	// Convert beads priority (0=urgent, 1=high, 2=normal, 3=low) to GGT Priority
	var priority Priority
	switch bm.Priority {
	case 0:
		priority = PriorityUrgent
	case 1:
		priority = PriorityHigh
	case 3:
		priority = PriorityLow
	default:
		priority = PriorityNormal
	}

	// Convert message type, default to notification
	msgType := TypeNotification
	switch MessageType(bm.msgType) {
	case TypeTask, TypeEscalation, TypeScavenge, TypeReply:
		msgType = MessageType(bm.msgType)
	}

	// Convert CC identities to addresses
	var ccAddrs []string
	for _, cc := range bm.cc {
		ccAddrs = append(ccAddrs, identityToAddress(cc))
	}

	return &Message{
		ID:              bm.ID,
		From:            identityToAddress(bm.sender),
		To:              identityToAddress(bm.Assignee),
		Subject:         bm.Title,
		Body:            bm.Description,
		Timestamp:       bm.CreatedAt,
		Read:            bm.Status == "closed" || bm.HasLabel("read"),
		Priority:        priority,
		Type:            msgType,
		ThreadID:        bm.threadID,
		ReplyTo:         bm.replyTo,
		Wisp:            bm.Wisp,
		CC:              ccAddrs,
		Queue:           bm.queue,
		Channel:         bm.channel,
		ClaimedBy:       bm.claimedBy,
		ClaimedAt:       bm.claimedAt,
		DeliveryState:   bm.deliveryState,
		DeliveryAckedBy: bm.deliveryAckedBy,
		DeliveryAckedAt: bm.deliveryAckedAt,
	}
}

// GetQueue returns the queue name for queue messages.
func (bm *BeadsMessage) GetQueue() string {
	return bm.queue
}

// GetChannel returns the channel name for broadcast messages.
func (bm *BeadsMessage) GetChannel() string {
	return bm.channel
}

// GetClaimedBy returns who claimed the queue message.
func (bm *BeadsMessage) GetClaimedBy() string {
	return bm.claimedBy
}

// GetClaimedAt returns when the queue message was claimed.
func (bm *BeadsMessage) GetClaimedAt() *time.Time {
	return bm.claimedAt
}

// IsQueueMessage returns true if this is a queue-routed message.
func (bm *BeadsMessage) IsQueueMessage() bool {
	bm.ParseLabels()
	return bm.queue != ""
}

// IsChannelMessage returns true if this is a channel broadcast message.
func (bm *BeadsMessage) IsChannelMessage() bool {
	bm.ParseLabels()
	return bm.channel != ""
}

// IsDirectMessage returns true if this is a direct (To-addressed) message.
func (bm *BeadsMessage) IsDirectMessage() bool {
	bm.ParseLabels()
	return bm.queue == "" && bm.channel == "" && bm.Assignee != ""
}

// HasLabel checks if the message has a specific label.
func (bm *BeadsMessage) HasLabel(label string) bool {
	for _, l := range bm.Labels {
		if l == label {
			return true
		}
	}
	return false
}

// PriorityToBeads converts a GGT Priority to beads priority integer.
// Returns: 0=urgent, 1=high, 2=normal, 3=low
func PriorityToBeads(p Priority) int {
	switch p {
	case PriorityUrgent:
		return 0
	case PriorityHigh:
		return 1
	case PriorityLow:
		return 3
	default:
		return 2 // normal
	}
}

// ParsePriority parses a priority string, returning PriorityNormal for invalid values.
func ParsePriority(s string) Priority {
	switch Priority(s) {
	case PriorityLow, PriorityNormal, PriorityHigh, PriorityUrgent:
		return Priority(s)
	default:
		return PriorityNormal
	}
}

// PriorityFromInt converts a beads-style integer priority to a Priority.
// Accepts: 0=urgent, 1=high, 2=normal, 3=low, 4=backlog (treated as low).
// Invalid values default to PriorityNormal.
func PriorityFromInt(p int) Priority {
	switch p {
	case 0:
		return PriorityUrgent
	case 1:
		return PriorityHigh
	case 2:
		return PriorityNormal
	case 3, 4:
		return PriorityLow
	default:
		return PriorityNormal
	}
}

// ParseMessageType parses a message type string, returning TypeNotification for invalid values.
func ParseMessageType(s string) MessageType {
	switch MessageType(s) {
	case TypeTask, TypeEscalation, TypeScavenge, TypeNotification, TypeReply:
		return MessageType(s)
	default:
		return TypeNotification
	}
}

// normalizeAddress handles the common normalization logic shared by
// AddressToIdentity and identityToAddress.
//
// Liberal normalization (Postel's Law - be liberal in what you accept):
//   - "overseer" → "overseer" (human operator, no trailing slash)
//   - "mayor" or "mayor/" → "mayor/" (town-level, trailing slash)
//   - "deacon" or "deacon/" → "deacon/" (town-level, trailing slash)
//   - "gastown/polecats/Toast" → "gastown/Toast" (crew/polecats normalized)
//   - "gastown/crew/max" → "gastown/max" (crew/polecats normalized)
//   - "gastown/Toast" → "gastown/Toast" (already canonical)
//   - "gastown/refinery" → "gastown/refinery"
func normalizeAddress(s string) string {
	// Overseer (human operator) - no trailing slash, distinct from agents
	if s == "overseer" {
		return "overseer"
	}

	// Town-level agents: mayor and deacon keep trailing slash
	if s == "mayor" || s == "mayor/" {
		return "mayor/"
	}
	if s == "deacon" || s == "deacon/" {
		return "deacon/"
	}

	// Resolve rig-scoped town-level roles to their canonical form (gt-te23).
	// "gastown/mayor" → "mayor/", "gastown/deacon" → "deacon/"
	// Mayor and deacon are town-level singletons, not rig-level agents.
	parts := strings.Split(s, "/")
	if len(parts) == 2 {
		switch parts[1] {
		case "mayor":
			return "mayor/"
		case "deacon":
			return "deacon/"
		}
	}

	// Normalize crew/ and polecats/ to canonical form:
	// "rig/crew/name" → "rig/name"
	// "rig/polecats/name" → "rig/name"
	if len(parts) == 3 && (parts[1] == "crew" || parts[1] == "polecats") {
		return parts[0] + "/" + parts[2]
	}

	return s
}

// AddressToIdentity converts a GGT address to a beads identity.
//
// Addresses use slash format:
//   - "overseer" → "overseer" (human operator, no trailing slash)
//   - "mayor/" → "mayor/"
//   - "mayor" → "mayor/"
//   - "deacon/" → "deacon/"
//   - "deacon" → "deacon/"
//   - "gastown/polecats/Toast" → "gastown/Toast" (normalized)
//   - "gastown/crew/max" → "gastown/max" (normalized)
//   - "gastown/Toast" → "gastown/Toast" (already canonical)
//   - "gastown/refinery" → "gastown/refinery"
//   - "gastown/" → "gastown" (rig broadcast)
func AddressToIdentity(address string) string {
	// Trim trailing slash for rig-level addresses before normalization.
	// normalizeAddress handles mayor/ and deacon/ correctly even after trimming.
	if len(address) > 0 && address[len(address)-1] == '/' {
		address = address[:len(address)-1]
	}
	return normalizeAddress(address)
}

// identityToAddress converts a beads identity back to a GGT address.
//
// Examples:
//   - "overseer" → "overseer" (human operator)
//   - "mayor/" → "mayor/"
//   - "deacon/" → "deacon/"
//   - "gastown/polecats/Toast" → "gastown/Toast" (normalized)
//   - "gastown/crew/max" → "gastown/max" (normalized)
//   - "gastown/Toast" → "gastown/Toast" (already canonical)
//   - "gastown/refinery" → "gastown/refinery"
func identityToAddress(identity string) string {
	return normalizeAddress(identity)
}
// Package mail: in-process beadsdk.Storage integration for mail operations.
//
// When a beadsdk.Storage is set on a Mailbox (via SetStore), methods bypass
// the bd subprocess and use the store directly. This eliminates ~600ms per
// operation for mail queries (inbox, get, mark-read).
package mail

import (
	"context"
	"fmt"
	"strings"
	"time"

	beadsdk "github.com/steveyegge/beads"
	"github.com/steveyegge/gastown/internal/runtime"
	"github.com/steveyegge/gastown/internal/telemetry"
)

// SetStore configures an in-process beadsdk.Storage for this Mailbox.
// When set, beads-mode methods use the store directly instead of shelling
// out to the bd CLI. Legacy JSONL mode is unaffected.
//
// Callers are responsible for closing the store when done.
func (m *Mailbox) SetStore(store beadsdk.Storage) {
	m.store = store
}

// Store returns the in-process beadsdk.Storage, or nil if not set.
func (m *Mailbox) Store() beadsdk.Storage {
	return m.store
}

// NewMailboxBeadsWithStore creates a mailbox backed by an in-process beads store.
func NewMailboxBeadsWithStore(identity, workDir string, store beadsdk.Storage) *Mailbox {
	return &Mailbox{
		identity: identity,
		workDir:  workDir,
		legacy:   false,
		store:    store,
	}
}

// NewMailboxWithBeadsDirAndStore creates a mailbox with an explicit beads
// directory and an in-process store.
func NewMailboxWithBeadsDirAndStore(address, workDir, beadsDir string, store beadsdk.Storage) *Mailbox {
	return &Mailbox{
		identity: AddressToIdentity(address),
		workDir:  workDir,
		beadsDir: beadsDir,
		legacy:   false,
		store:    store,
	}
}

// mailStoreCtx returns a context with a standard timeout for mail store operations.
func mailStoreCtx() (context.Context, context.CancelFunc) {
	return context.WithTimeout(context.Background(), 30*time.Second)
}

// storeListFromDir queries messages using the in-process store.
// Returns messages where identity is the assignee.
func (m *Mailbox) storeListFromDir() ([]*Message, error) {
	ctx, cancel := mailStoreCtx()
	defer cancel()

	identities := m.identityVariants()

	seen := make(map[string]bool)
	messages := make([]*Message, 0)

	// Query by assignee for each identity variant. We omit the status filter
	// so that a single query returns both "open" and "hooked" messages,
	// avoiding a redundant second round-trip per identity variant.
	for _, id := range identities {
		filter := beadsdk.IssueFilter{
			Labels:   []string{"gt:message"},
			Assignee: &id,
			Limit:    0, // No limit
		}

		sdkIssues, err := m.store.SearchIssues(ctx, "", filter)
		if err != nil {
			return nil, fmt.Errorf("store list messages: %w", err)
		}

		for _, si := range sdkIssues {
			if seen[si.ID] {
				continue
			}
			if si.Status == beadsdk.StatusOpen || string(si.Status) == "hooked" {
				seen[si.ID] = true
				messages = append(messages, sdkIssueToMessage(si))
			}
		}
	}

	return messages, nil
}

// storeGetFromDir retrieves a message using the in-process store.
func (m *Mailbox) storeGetFromDir(id string) (*Message, error) {
	ctx, cancel := mailStoreCtx()
	defer cancel()

	si, err := m.store.GetIssue(ctx, id)
	if err != nil {
		if strings.Contains(err.Error(), "not found") {
			return nil, ErrMessageNotFound
		}
		return nil, fmt.Errorf("store get message: %w", err)
	}

	return sdkIssueToMessage(si), nil
}

// storeCloseInDir closes a message using the in-process store.
func (m *Mailbox) storeCloseInDir(id string) error {
	ctx, cancel := mailStoreCtx()
	defer cancel()

	sessionID := runtime.SessionIDFromEnv()
	err := m.store.CloseIssue(ctx, id, "", "", sessionID)
	telemetry.RecordMailMessage(context.Background(), "read", telemetry.MailMessageInfo{
		ID: id,
		To: m.identity,
	}, err)
	if err != nil {
		if strings.Contains(err.Error(), "not found") {
			return ErrMessageNotFound
		}
		return fmt.Errorf("store close message: %w", err)
	}
	return nil
}

// storeMarkReadOnly adds a "read" label using the in-process store.
func (m *Mailbox) storeMarkReadOnly(id string) error {
	ctx, cancel := mailStoreCtx()
	defer cancel()

	err := m.store.AddLabel(ctx, id, "read", "")
	if err != nil {
		if strings.Contains(err.Error(), "not found") {
			return ErrMessageNotFound
		}
		return fmt.Errorf("store mark read: %w", err)
	}
	return nil
}

// storeMarkUnreadOnly removes a "read" label using the in-process store.
func (m *Mailbox) storeMarkUnreadOnly(id string) error {
	ctx, cancel := mailStoreCtx()
	defer cancel()

	err := m.store.RemoveLabel(ctx, id, "read", "")
	if err != nil {
		if strings.Contains(err.Error(), "not found") {
			return ErrMessageNotFound
		}
		// Ignore error if label doesn't exist
		if strings.Contains(err.Error(), "does not have label") {
			return nil
		}
		return fmt.Errorf("store mark unread: %w", err)
	}
	return nil
}

// storeMarkUnread reopens a message using the in-process store.
func (m *Mailbox) storeMarkUnread(id string) error {
	ctx, cancel := mailStoreCtx()
	defer cancel()

	updates := map[string]interface{}{
		"status": "open",
	}
	err := m.store.UpdateIssue(ctx, id, updates, "")
	if err != nil {
		if strings.Contains(err.Error(), "not found") {
			return ErrMessageNotFound
		}
		return fmt.Errorf("store reopen message: %w", err)
	}
	return nil
}

// sdkIssueToMessage converts a beadsdk Issue to a mail Message by routing
// through BeadsMessage for correct label parsing and type conversion.
func sdkIssueToMessage(si *beadsdk.Issue) *Message {
	if si == nil {
		return nil
	}

	// Build a BeadsMessage from SDK issue fields, then use its ToMessage()
	// method for correct label parsing (from:, thread:, cc:, etc.).
	bm := &BeadsMessage{
		ID:          si.ID,
		Title:       si.Title,
		Description: si.Description,
		Assignee:    si.Assignee,
		Priority:    si.Priority,
		Status:      string(si.Status),
		CreatedAt:   si.CreatedAt,
		Labels:      si.Labels,
		Pinned:      si.Pinned,
		Wisp:        si.Ephemeral,
	}

	return bm.ToMessage()
}
package mail

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"sync"
	"time"

	beadsdk "github.com/steveyegge/beads"
	"github.com/gofrs/flock"
	"github.com/steveyegge/gastown/internal/beads"
	"github.com/steveyegge/gastown/internal/runtime"
	"github.com/steveyegge/gastown/internal/telemetry"
)

// timeNow is a function that returns the current time. It can be overridden in tests.
var timeNow = time.Now

// Common errors
var (
	ErrMessageNotFound = errors.New("message not found")
	ErrEmptyInbox      = errors.New("inbox is empty")
)

// Mailbox manages messages for an identity via beads.
// When store is non-nil, beads-mode methods use the in-process beadsdk.Storage
// directly instead of shelling out to the bd CLI.
type Mailbox struct {
	identity string // beads identity (e.g., "gastown/polecats/Toast")
	workDir  string // directory to run bd commands in
	beadsDir string // explicit .beads directory path (set via BEADS_DIR)
	path     string // for legacy JSONL mode (crew workers)
	legacy   bool   // true = use JSONL files, false = use beads

	// store is an optional in-process beadsdk.Storage. When set, beads-mode
	// methods bypass the bd subprocess and use the store directly.
	// Callers are responsible for closing the store.
	store beadsdk.Storage
}

// NewMailbox creates a mailbox for the given JSONL path (legacy mode).
// Used by crew workers that have local JSONL inboxes.
func NewMailbox(path string) *Mailbox {
	return &Mailbox{
		path:   filepath.Join(path, "inbox.jsonl"),
		legacy: true,
	}
}

// NewMailboxBeads creates a mailbox backed by beads.
func NewMailboxBeads(identity, workDir string) *Mailbox {
	return &Mailbox{
		identity: identity,
		workDir:  workDir,
		legacy:   false,
	}
}

// NewMailboxFromAddress creates a beads-backed mailbox from a GGT address.
// Follows .beads/redirect for crew workers and polecats using shared beads.
func NewMailboxFromAddress(address, workDir string) *Mailbox {
	beadsDir := beads.ResolveBeadsDir(workDir)
	return &Mailbox{
		identity: AddressToIdentity(address),
		workDir:  workDir,
		beadsDir: beadsDir,
		legacy:   false,
	}
}

// NewMailboxWithBeadsDir creates a mailbox with an explicit beads directory.
func NewMailboxWithBeadsDir(address, workDir, beadsDir string) *Mailbox {
	return &Mailbox{
		identity: AddressToIdentity(address),
		workDir:  workDir,
		beadsDir: beadsDir,
		legacy:   false,
	}
}

// Identity returns the beads identity for this mailbox.
func (m *Mailbox) Identity() string {
	return m.identity
}

// Path returns the JSONL path for legacy mailboxes.
func (m *Mailbox) Path() string {
	return m.path
}

// lockLegacy acquires an exclusive flock for legacy mailbox operations.
// Callers must defer Unlock on the returned flock. The lock file is
// separate from the data file to avoid interfering with reads.
func (m *Mailbox) lockLegacy() (*flock.Flock, error) {
	fl := flock.New(m.path + ".lock")
	if err := fl.Lock(); err != nil {
		return nil, fmt.Errorf("acquiring mailbox lock: %w", err)
	}
	return fl, nil
}

// List returns all open messages in the mailbox.
func (m *Mailbox) List() ([]*Message, error) {
	if m.legacy {
		return m.listLegacy()
	}
	return m.listBeads()
}

func (m *Mailbox) listBeads() ([]*Message, error) {
	// Single query to beads - returns both persistent and wisp messages
	// Wisps are stored in same DB with wisp=true flag, not synced to git
	messages, err := m.listFromDir(m.beadsDir)
	if err != nil {
		return nil, err
	}

	// Sort by priority (higher first), then timestamp (newest first).
	sort.Slice(messages, func(i, j int) bool {
		pi, pj := PriorityToBeads(messages[i].Priority), PriorityToBeads(messages[j].Priority)
		if pi != pj {
			return pi < pj // lower beads int = higher priority
		}
		return messages[i].Timestamp.After(messages[j].Timestamp)
	})

	return messages, nil
}

// listFromDir queries messages from a beads directory.
// Returns messages where identity is the assignee OR a CC recipient.
// Includes both open and hooked messages (hooked = auto-assigned handoff mail).
//
// Uses per-identity --assignee queries to push filtering to Dolt, reducing
// memory footprint under concurrent agent load. A separate CC query fetches
// messages where this identity is CC'd.
func (m *Mailbox) listFromDir(beadsDir string) ([]*Message, error) {
	// Use in-process store when available
	if m.store != nil {
		return m.storeListFromDir()
	}

	identities := m.identityVariants()

	if err := beads.EnsureCustomTypes(beadsDir); err != nil {
		return nil, fmt.Errorf("ensuring custom types: %w", err)
	}

	// Deduplicate messages across queries (assignee + CC may overlap)
	seen := make(map[string]bool)
	messages := make([]*Message, 0)

	// Query 1: assignee match (per identity variant)
	for _, id := range identities {
		args := []string{"list",
			"--label", "gt:message",
			"--assignee", id,
			"--json",
			"--limit", "0",
		}

		ctx, cancel := bdReadCtx()
		stdout, err := runBdCommand(ctx, args, m.workDir, beadsDir)
		cancel()
		if err != nil {
			return nil, err
		}

		// bd v0.58.0 returns plain text (e.g. "No issues found.") for
		// empty result sets instead of JSON. Skip non-JSON output.
		if !isJSON(stdout) {
			continue
		}
		var msgs []BeadsMessage
		trimmed := bytes.TrimSpace(stdout)
		if len(trimmed) == 0 || string(trimmed) == "null" || (trimmed[0] != '[' && trimmed[0] != '{') {
			// bd v0.58.0 returns plain text (e.g. "No issues found.") for
			// empty result sets instead of JSON. Skip non-JSON output.
			continue
		}
		if err := json.Unmarshal(stdout, &msgs); err != nil {
			return nil, err
		}

		for i := range msgs {
			bm := &msgs[i]
			if seen[bm.ID] {
				continue
			}
			// Assignee match: open or hooked status
			if bm.Status == "open" || bm.Status == "hooked" {
				seen[bm.ID] = true
				messages = append(messages, bm.ToMessage())
			}
		}
	}

	// Query 2: CC match — fetch messages with cc:<identity> label
	for _, id := range identities {
		ccLabel := "cc:" + id
		args := []string{"list",
			"--label", "gt:message",
			"--label", ccLabel,
			"--json",
			"--limit", "0",
		}

		ctx, cancel := bdReadCtx()
		stdout, err := runBdCommand(ctx, args, m.workDir, beadsDir)
		cancel()
		if err != nil {
			// CC query failure is non-fatal — assignee messages are primary
			continue
		}

		if !isJSON(stdout) {
			continue
		}
		var msgs []BeadsMessage
		trimmedCC := bytes.TrimSpace(stdout)
		if len(trimmedCC) == 0 || string(trimmedCC) == "null" || (trimmedCC[0] != '[' && trimmedCC[0] != '{') {
			continue
		}
		if err := json.Unmarshal(stdout, &msgs); err != nil {
			continue // Non-fatal for CC
		}

		for i := range msgs {
			bm := &msgs[i]
			if seen[bm.ID] {
				continue
			}
			// CC match: open status only
			if bm.Status == "open" {
				seen[bm.ID] = true
				messages = append(messages, bm.ToMessage())
			}
		}
	}

	// Query 3: Wisps table — ephemeral messages (protocol/lifecycle) are stored
	// as wisps by shouldBeWisp(), but bd list only queries the issues table.
	wispMessages := m.listWispMessages(beadsDir, identities, seen)
	messages = append(messages, wispMessages...)

	return messages, nil
}

// listWispMessages queries the wisps table for ephemeral messages matching the identity.
// Protocol/lifecycle messages are stored as wisps by shouldBeWisp(), but bd list only
// queries the issues table. Uses bd sql --json for full wisp data.
func (m *Mailbox) listWispMessages(beadsDir string, identities []string, seen map[string]bool) []*Message {
	var messages []*Message

	// Query 3a: assignee match via SQL on wisps table
	for _, id := range identities {
		wispMsgs := m.queryWispMessagesByAssignee(beadsDir, id)
		for _, bm := range wispMsgs {
			if seen[bm.ID] {
				continue
			}
			if bm.Status == "open" || bm.Status == "hooked" {
				seen[bm.ID] = true
				messages = append(messages, bm.ToMessage())
			}
		}
	}

	// Query 3b: CC match via SQL on wisps table
	for _, id := range identities {
		wispMsgs := m.queryWispMessagesByCC(beadsDir, id)
		for _, bm := range wispMsgs {
			if seen[bm.ID] {
				continue
			}
			if bm.Status == "open" {
				seen[bm.ID] = true
				messages = append(messages, bm.ToMessage())
			}
		}
	}

	return messages
}

// queryWispMessagesByAssignee queries wisps table for messages assigned to identity.
func (m *Mailbox) queryWispMessagesByAssignee(beadsDir, identity string) []BeadsMessage {
	query := fmt.Sprintf(
		"SELECT w.id, w.title, w.description, w.status, w.priority, w.assignee, w.created_at, w.updated_at, "+
			"GROUP_CONCAT(al.label) as labels_csv "+
			"FROM wisps w "+
			"JOIN wisp_labels l ON w.id = l.issue_id "+
			"JOIN wisp_labels al ON w.id = al.issue_id "+
			"WHERE l.label = 'gt:message' AND w.status IN ('open', 'hooked') AND w.assignee = '%s' "+
			"GROUP BY w.id, w.title, w.description, w.status, w.priority, w.assignee, w.created_at, w.updated_at",
		escapeSQLString(identity))
	return m.runWispSQL(beadsDir, query)
}

// queryWispMessagesByCC queries wisps table for messages where identity is CC'd.
func (m *Mailbox) queryWispMessagesByCC(beadsDir, identity string) []BeadsMessage {
	ccLabel := "cc:" + identity
	query := fmt.Sprintf(
		"SELECT w.id, w.title, w.description, w.status, w.priority, w.assignee, w.created_at, w.updated_at, "+
			"GROUP_CONCAT(al.label) as labels_csv "+
			"FROM wisps w "+
			"JOIN wisp_labels l1 ON w.id = l1.issue_id "+
			"JOIN wisp_labels l2 ON w.id = l2.issue_id "+
			"JOIN wisp_labels al ON w.id = al.issue_id "+
			"WHERE l1.label = 'gt:message' AND l2.label = '%s' AND w.status IN ('open', 'hooked') "+
			"GROUP BY w.id, w.title, w.description, w.status, w.priority, w.assignee, w.created_at, w.updated_at",
		escapeSQLString(ccLabel))
	return m.runWispSQL(beadsDir, query)
}

// wispSQLRow represents a row from the wisps SQL query with aggregated labels.
type wispSQLRow struct {
	ID          string `json:"id"`
	Title       string `json:"title"`
	Description string `json:"description"`
	Status      string `json:"status"`
	Priority    int    `json:"priority"`
	Assignee    string `json:"assignee"`
	CreatedAt   string `json:"created_at"`
	UpdatedAt   string `json:"updated_at"`
	LabelsCSV   string `json:"labels_csv"`
}

// runWispSQL executes a bd sql --json query and converts results to BeadsMessages.
func (m *Mailbox) runWispSQL(beadsDir, query string) []BeadsMessage {
	args := []string{"sql", "--json", query}
	ctx, cancel := bdReadCtx()
	stdout, err := runBdCommand(ctx, args, m.workDir, beadsDir)
	cancel()
	if err != nil {
		return nil // Wisps table may not exist yet
	}
	if !isJSON(stdout) {
		return nil
	}

	var rows []wispSQLRow
	if err := json.Unmarshal(stdout, &rows); err != nil {
		return nil
	}

	msgs := make([]BeadsMessage, 0, len(rows))
	for _, row := range rows {
		bm := BeadsMessage{
			ID:          row.ID,
			Title:       row.Title,
			Description: row.Description,
			Status:      row.Status,
			Priority:    row.Priority,
			Assignee:    row.Assignee,
			Wisp:        true,
		}
		if t, err := time.Parse(time.RFC3339, row.CreatedAt); err == nil {
			bm.CreatedAt = t
		} else if t, err := time.Parse("2006-01-02 15:04:05 +0000 UTC", row.CreatedAt); err == nil {
			bm.CreatedAt = t
		}
		if row.LabelsCSV != "" {
			bm.Labels = strings.Split(row.LabelsCSV, ",")
		}
		msgs = append(msgs, bm)
	}
	return msgs
}

// escapeSQLString escapes single quotes for SQL string literals.
func escapeSQLString(s string) string {
	return strings.ReplaceAll(s, "'", "''")
}

// identityVariants returns all identity formats to query.
// For town-level agents (mayor/, deacon/), also includes the variant without
// trailing slash for backwards compatibility with legacy messages.
func (m *Mailbox) identityVariants() []string {
	variants := []string{m.identity}

	// Town-level agents may have legacy messages without trailing slash
	if m.identity == "mayor/" {
		variants = append(variants, "mayor")
	} else if m.identity == "deacon/" {
		variants = append(variants, "deacon")
	}

	return variants
}

func (m *Mailbox) listLegacy() ([]*Message, error) {
	file, err := os.Open(m.path)
	if err != nil {
		if os.IsNotExist(err) {
			return make([]*Message, 0), nil
		}
		return nil, err
	}
	defer func() { _ = file.Close() }() // non-fatal: OS will close on exit

	messages := make([]*Message, 0)
	scanner := bufio.NewScanner(file)
	lineNum := 0
	for scanner.Scan() {
		lineNum++
		line := scanner.Text()
		if line == "" {
			continue
		}

		var msg Message
		if err := json.Unmarshal([]byte(line), &msg); err != nil {
			return nil, fmt.Errorf("corrupt mailbox %s line %d: %w", m.path, lineNum, err)
		}
		messages = append(messages, &msg)
	}

	if err := scanner.Err(); err != nil {
		return nil, err
	}

	// Sort by priority (higher first), then timestamp (newest first).
	sort.Slice(messages, func(i, j int) bool {
		pi, pj := PriorityToBeads(messages[i].Priority), PriorityToBeads(messages[j].Priority)
		if pi != pj {
			return pi < pj
		}
		return messages[i].Timestamp.After(messages[j].Timestamp)
	})

	return messages, nil
}

// ListUnread returns unread (open) messages.
// Filters out messages marked as read (via "read" label in beads mode).
func (m *Mailbox) ListUnread() ([]*Message, error) {
	all, err := m.List()
	if err != nil {
		return nil, err
	}
	unread := make([]*Message, 0)
	for _, msg := range all {
		if !msg.Read {
			unread = append(unread, msg)
		}
	}
	return unread, nil
}

// Get returns a message by ID.
func (m *Mailbox) Get(id string) (*Message, error) {
	if m.legacy {
		return m.getLegacy(id)
	}
	return m.getBeads(id)
}

func (m *Mailbox) getBeads(id string) (*Message, error) {
	// Resolve correct beadsDir based on bead ID prefix (GH#2423)
	primary := beads.ResolveBeadsDirForID(m.beadsDir, id)
	msg, err := m.getFromDir(id, primary)
	if errors.Is(err, ErrMessageNotFound) && primary != m.beadsDir {
		// Cross-rig bead IDs (e.g. ne-*) may live in the home DB when created
		// via the mail router (which always uses town beads). Fall back to
		// m.beadsDir before giving up. See ne-bgr.
		return m.getFromDir(id, m.beadsDir)
	}
	return msg, err
}

// getFromDir retrieves a message from a beads directory.
func (m *Mailbox) getFromDir(id, beadsDir string) (*Message, error) {
	if m.store != nil {
		return m.storeGetFromDir(id)
	}

	args := []string{"show", id, "--json"}

	ctx, cancel := bdReadCtx()
	defer cancel()
	stdout, err := runBdCommand(ctx, args, m.workDir, beadsDir)
	if err != nil {
		if bdErr, ok := err.(*bdError); ok && bdErr.ContainsError("not found") {
			return nil, ErrMessageNotFound
		}
		return nil, err
	}

	// bd show --json returns an array
	if !isJSON(stdout) {
		return nil, ErrMessageNotFound
	}
	var bms []BeadsMessage
	if err := json.Unmarshal(stdout, &bms); err != nil {
		return nil, err
	}
	if len(bms) == 0 {
		return nil, ErrMessageNotFound
	}

	// Wisp status comes from beads issue.wisp field via ToMessage()
	return bms[0].ToMessage(), nil
}

func (m *Mailbox) getLegacy(id string) (*Message, error) {
	messages, err := m.List()
	if err != nil {
		return nil, err
	}
	for _, msg := range messages {
		if msg.ID == id {
			return msg, nil
		}
	}
	return nil, ErrMessageNotFound
}

// MarkRead marks a message as read.
func (m *Mailbox) MarkRead(id string) error {
	if m.legacy {
		return m.markReadLegacy(id)
	}
	return m.markReadBeads(id)
}

func (m *Mailbox) markReadBeads(id string) error {
	// Resolve correct beadsDir based on bead ID prefix (GH#2423)
	primary := beads.ResolveBeadsDirForID(m.beadsDir, id)
	err := m.closeInDir(id, primary)
	if errors.Is(err, ErrMessageNotFound) && primary != m.beadsDir {
		// Cross-rig bead IDs (e.g. ne-*) may live in the home DB when created
		// via the mail router (which always uses town beads). Fall back to
		// m.beadsDir before giving up. See ne-bgr.
		return m.closeInDir(id, m.beadsDir)
	}
	return err
}

// closeInDir closes a message in a specific beads directory.
func (m *Mailbox) closeInDir(id, beadsDir string) error {
	if m.store != nil {
		return m.storeCloseInDir(id)
	}

	args := []string{"close", id}
	// Pass session ID for work attribution if available
	if sessionID := runtime.SessionIDFromEnv(); sessionID != "" {
		args = append(args, "--session="+sessionID)
	}

	ctx, cancel := bdWriteCtx()
	defer cancel()
	_, err := runBdCommand(ctx, args, m.workDir, beadsDir)
	telemetry.RecordMailMessage(context.Background(), "read", telemetry.MailMessageInfo{
		ID: id,
		To: m.identity,
	}, err)
	if err != nil {
		if bdErr, ok := err.(*bdError); ok && bdErr.ContainsError("not found") {
			return ErrMessageNotFound
		}
		return err
	}

	return nil
}

func (m *Mailbox) markReadLegacy(id string) error {
	fl, err := m.lockLegacy()
	if err != nil {
		return err
	}
	defer func() { _ = fl.Unlock() }()

	messages, err := m.List()
	if err != nil {
		return err
	}

	found := false
	for _, msg := range messages {
		if msg.ID == id {
			msg.Read = true
			found = true
		}
	}

	if !found {
		return ErrMessageNotFound
	}

	return m.rewriteLegacy(messages)
}

// MarkReadOnly marks a message as read WITHOUT archiving/closing it.
// For beads mode, this adds a "read" label to the message.
// For legacy mode, this sets the Read field to true.
// The message remains in the inbox but is displayed as read.
func (m *Mailbox) MarkReadOnly(id string) error {
	if m.legacy {
		return m.markReadLegacy(id)
	}
	return m.markReadOnlyBeads(id)
}

func (m *Mailbox) markReadOnlyBeads(id string) error {
	if m.store != nil {
		return m.storeMarkReadOnly(id)
	}

	// Add "read" label to mark as read without closing
	args := []string{"label", "add", id, "read"}
	primary := beads.ResolveBeadsDirForID(m.beadsDir, id)

	ctx, cancel := bdWriteCtx()
	defer cancel()
	_, err := runBdCommand(ctx, args, m.workDir, primary)
	if err != nil {
		if bdErr, ok := err.(*bdError); ok && bdErr.ContainsError("not found") {
			if primary != m.beadsDir {
				// Cross-rig bead IDs (e.g. ne-*) may live in the home DB. See ne-bgr.
				ctx2, cancel2 := bdWriteCtx()
				defer cancel2()
				_, err2 := runBdCommand(ctx2, args, m.workDir, m.beadsDir)
				if err2 != nil {
					if bdErr2, ok := err2.(*bdError); ok && bdErr2.ContainsError("not found") {
						return ErrMessageNotFound
					}
					return err2
				}
				return nil
			}
			return ErrMessageNotFound
		}
		return err
	}

	return nil
}

// MarkUnreadOnly marks a message as unread (removes "read" label).
// For beads mode, this removes the "read" label from the message.
// For legacy mode, this sets the Read field to false.
func (m *Mailbox) MarkUnreadOnly(id string) error {
	if m.legacy {
		return m.markUnreadLegacy(id)
	}
	return m.markUnreadOnlyBeads(id)
}

func (m *Mailbox) markUnreadOnlyBeads(id string) error {
	if m.store != nil {
		return m.storeMarkUnreadOnly(id)
	}

	// Remove "read" label to mark as unread
	args := []string{"label", "remove", id, "read"}
	primary := beads.ResolveBeadsDirForID(m.beadsDir, id)

	ctx, cancel := bdWriteCtx()
	defer cancel()
	_, err := runBdCommand(ctx, args, m.workDir, primary)
	if err != nil {
		if bdErr, ok := err.(*bdError); ok && bdErr.ContainsError("not found") {
			if primary != m.beadsDir {
				// Cross-rig bead IDs (e.g. ne-*) may live in the home DB. See ne-bgr.
				ctx2, cancel2 := bdWriteCtx()
				defer cancel2()
				_, err2 := runBdCommand(ctx2, args, m.workDir, m.beadsDir)
				if err2 != nil {
					if bdErr2, ok := err2.(*bdError); ok && bdErr2.ContainsError("not found") {
						return ErrMessageNotFound
					}
					if bdErr2, ok := err2.(*bdError); ok && bdErr2.ContainsError("does not have label") {
						return nil
					}
					return err2
				}
				return nil
			}
			return ErrMessageNotFound
		}
		// Ignore error if label doesn't exist
		if bdErr, ok := err.(*bdError); ok && bdErr.ContainsError("does not have label") {
			return nil
		}
		return err
	}

	return nil
}

// MarkUnread marks a message as unread (reopens in beads).
func (m *Mailbox) MarkUnread(id string) error {
	if m.legacy {
		return m.markUnreadLegacy(id)
	}
	return m.markUnreadBeads(id)
}

func (m *Mailbox) markUnreadBeads(id string) error {
	if m.store != nil {
		return m.storeMarkUnread(id)
	}

	args := []string{"reopen", id}
	primary := beads.ResolveBeadsDirForID(m.beadsDir, id)

	ctx, cancel := bdWriteCtx()
	defer cancel()
	_, err := runBdCommand(ctx, args, m.workDir, primary)
	if err != nil {
		if bdErr, ok := err.(*bdError); ok && bdErr.ContainsError("not found") {
			if primary != m.beadsDir {
				// Cross-rig bead IDs (e.g. ne-*) may live in the home DB. See ne-bgr.
				ctx2, cancel2 := bdWriteCtx()
				defer cancel2()
				_, err2 := runBdCommand(ctx2, args, m.workDir, m.beadsDir)
				if err2 != nil {
					if bdErr2, ok := err2.(*bdError); ok && bdErr2.ContainsError("not found") {
						return ErrMessageNotFound
					}
					return err2
				}
				return nil
			}
			return ErrMessageNotFound
		}
		return err
	}

	return nil
}

func (m *Mailbox) markUnreadLegacy(id string) error {
	fl, err := m.lockLegacy()
	if err != nil {
		return err
	}
	defer func() { _ = fl.Unlock() }()

	messages, err := m.List()
	if err != nil {
		return err
	}

	found := false
	for _, msg := range messages {
		if msg.ID == id {
			msg.Read = false
			found = true
		}
	}

	if !found {
		return ErrMessageNotFound
	}

	return m.rewriteLegacy(messages)
}

// Delete removes a message.
func (m *Mailbox) Delete(id string) error {
	if m.legacy {
		return m.deleteLegacy(id)
	}
	return m.MarkRead(id) // beads: just acknowledge/close
}

func (m *Mailbox) deleteLegacy(id string) error {
	fl, err := m.lockLegacy()
	if err != nil {
		return err
	}
	defer func() { _ = fl.Unlock() }()

	messages, err := m.List()
	if err != nil {
		return err
	}

	var filtered []*Message
	found := false
	for _, msg := range messages {
		if msg.ID == id {
			found = true
		} else {
			filtered = append(filtered, msg)
		}
	}

	if !found {
		return ErrMessageNotFound
	}

	return m.rewriteLegacy(filtered)
}

// Archive moves a message to the archive file and removes it from inbox.
func (m *Mailbox) Archive(id string) error {
	if m.legacy {
		return m.archiveLegacy(id)
	}
	// Beads mode: append to archive then close
	msg, err := m.Get(id)
	if err != nil {
		return err
	}
	if err := m.appendToArchive(msg); err != nil {
		return err
	}
	return m.Delete(id)
}

// archiveLegacy moves a message to the archive file atomically.
// A single flock covers the entire read-archive-rewrite cycle so that
// a crash between appendToArchive and the inbox rewrite cannot lose the
// message (worst case: duplicate in both archive and inbox).
func (m *Mailbox) archiveLegacy(id string) error {
	fl, err := m.lockLegacy()
	if err != nil {
		return err
	}
	defer func() { _ = fl.Unlock() }()

	// Read inbox
	messages, err := m.listLegacy()
	if err != nil {
		return err
	}

	// Find and extract target
	var target *Message
	var remaining []*Message
	for _, msg := range messages {
		if msg.ID == id {
			target = msg
		} else {
			remaining = append(remaining, msg)
		}
	}
	if target == nil {
		return ErrMessageNotFound
	}

	// Append to archive first (safe failure mode: duplicate, not loss)
	if err := m.appendToArchive(target); err != nil {
		return err
	}

	// Rewrite inbox without the target
	return m.rewriteLegacy(remaining)
}

// ArchivePath returns the path to the archive file.
func (m *Mailbox) ArchivePath() string {
	if m.legacy {
		return m.path + ".archive"
	}
	// For beads, use archive.jsonl in the same directory as beads
	return filepath.Join(m.beadsDir, "archive.jsonl")
}

func (m *Mailbox) appendToArchive(msg *Message) error {
	archivePath := m.ArchivePath()

	// Ensure directory exists
	dir := filepath.Dir(archivePath)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}

	// Open for append
	file, err := os.OpenFile(archivePath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644) //nolint:gosec // G302: archive is non-sensitive operational data
	if err != nil {
		return err
	}
	defer func() { _ = file.Close() }()

	data, err := json.Marshal(msg)
	if err != nil {
		return err
	}

	_, err = file.WriteString(string(data) + "\n")
	return err
}

// ListArchived returns all messages in the archive file.
func (m *Mailbox) ListArchived() ([]*Message, error) {
	archivePath := m.ArchivePath()

	file, err := os.Open(archivePath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	defer func() { _ = file.Close() }()

	var messages []*Message
	scanner := bufio.NewScanner(file)
	lineNum := 0
	for scanner.Scan() {
		lineNum++
		line := scanner.Text()
		if line == "" {
			continue
		}

		var msg Message
		if err := json.Unmarshal([]byte(line), &msg); err != nil {
			return nil, fmt.Errorf("corrupt archive %s line %d: %w", archivePath, lineNum, err)
		}
		messages = append(messages, &msg)
	}

	if err := scanner.Err(); err != nil {
		return nil, err
	}

	return messages, nil
}

// PurgeArchive removes messages from the archive, optionally filtering by age.
// If olderThanDays is 0, removes all archived messages.
func (m *Mailbox) PurgeArchive(olderThanDays int) (int, error) {
	if m.legacy {
		fl, err := m.lockLegacy()
		if err != nil {
			return 0, err
		}
		defer func() { _ = fl.Unlock() }()
	}

	messages, err := m.ListArchived()
	if err != nil {
		return 0, err
	}

	if len(messages) == 0 {
		return 0, nil
	}

	// If no age filter, remove all
	if olderThanDays <= 0 {
		if err := os.Remove(m.ArchivePath()); err != nil && !os.IsNotExist(err) {
			return 0, err
		}
		return len(messages), nil
	}

	// Filter by age
	cutoff := timeNow().AddDate(0, 0, -olderThanDays)
	var keep []*Message
	purged := 0

	for _, msg := range messages {
		if msg.Timestamp.Before(cutoff) {
			purged++
		} else {
			keep = append(keep, msg)
		}
	}

	// Rewrite archive with remaining messages
	if len(keep) == 0 {
		if err := os.Remove(m.ArchivePath()); err != nil && !os.IsNotExist(err) {
			return 0, err
		}
	} else {
		if err := m.rewriteArchive(keep); err != nil {
			return 0, err
		}
	}

	return purged, nil
}

func (m *Mailbox) rewriteArchive(messages []*Message) error {
	archivePath := m.ArchivePath()
	tmpPath := archivePath + ".tmp"

	file, err := os.Create(tmpPath)
	if err != nil {
		return err
	}

	for _, msg := range messages {
		data, err := json.Marshal(msg)
		if err != nil {
			_ = file.Close()
			_ = os.Remove(tmpPath)
			return err
		}
		if _, err := file.WriteString(string(data) + "\n"); err != nil {
			_ = file.Close()
			_ = os.Remove(tmpPath)
			return fmt.Errorf("writing archive: %w", err)
		}
	}

	if err := file.Close(); err != nil {
		_ = os.Remove(tmpPath)
		return err
	}

	return os.Rename(tmpPath, archivePath)
}

// SearchOptions specifies search parameters.
type SearchOptions struct {
	Query       string // Regex pattern to search for
	FromFilter  string // Optional: only match messages from this sender
	SubjectOnly bool   // Only search subject
	BodyOnly    bool   // Only search body
}

// Search finds messages matching the given criteria.
// Returns messages from both inbox and archive.
// Query and FromFilter are treated as literal strings (not regex) to prevent ReDoS.
func (m *Mailbox) Search(opts SearchOptions) ([]*Message, error) {
	// Use QuoteMeta to escape special regex chars - prevents ReDoS attacks
	// and provides intuitive literal string matching for users
	re, err := regexp.Compile("(?i)" + regexp.QuoteMeta(opts.Query))
	if err != nil {
		return nil, fmt.Errorf("invalid search pattern: %w", err)
	}

	var fromRe *regexp.Regexp
	if opts.FromFilter != "" {
		fromRe, err = regexp.Compile("(?i)" + regexp.QuoteMeta(opts.FromFilter))
		if err != nil {
			return nil, fmt.Errorf("invalid from pattern: %w", err)
		}
	}

	// Get inbox messages
	inbox, err := m.List()
	if err != nil {
		return nil, err
	}

	// Get archived messages
	archived, err := m.ListArchived()
	if err != nil && !os.IsNotExist(err) {
		return nil, err
	}

	// Combine and search
	all := append(inbox, archived...)
	var matches []*Message

	for _, msg := range all {
		// Apply from filter
		if fromRe != nil && !fromRe.MatchString(msg.From) {
			continue
		}

		// Search in specified fields
		matched := false
		if opts.SubjectOnly {
			matched = re.MatchString(msg.Subject)
		} else if opts.BodyOnly {
			matched = re.MatchString(msg.Body)
		} else {
			// Search in both subject and body
			matched = re.MatchString(msg.Subject) || re.MatchString(msg.Body)
		}

		if matched {
			matches = append(matches, msg)
		}
	}

	// Sort by priority (higher first), then timestamp (newest first).
	sort.Slice(matches, func(i, j int) bool {
		pi, pj := PriorityToBeads(matches[i].Priority), PriorityToBeads(matches[j].Priority)
		if pi != pj {
			return pi < pj
		}
		return matches[i].Timestamp.After(matches[j].Timestamp)
	})

	return matches, nil
}

// Count returns the total and unread message counts.
func (m *Mailbox) Count() (total, unread int, err error) {
	messages, err := m.List()
	if err != nil {
		return 0, 0, err
	}

	total = len(messages)
	// Count messages that are NOT marked as read (including via "read" label)
	for _, msg := range messages {
		if !msg.Read {
			unread++
		}
	}

	return total, unread, nil
}

// AcknowledgeDeliveries marks delivery receipt for unread messages where this
// mailbox is the primary recipient. This is phase-2 of two-phase delivery
// tracking (phase-1 is written at send time as delivery:pending).
// Acks are run concurrently (bounded to 8) to avoid N+1 sequential subprocess
// spawns on the hot path.
func (m *Mailbox) AcknowledgeDeliveries(recipientAddress string, messages []*Message) error {
	if m.legacy || len(messages) == 0 {
		return nil
	}

	recipientIdentity := AddressToIdentity(recipientAddress)

	// Collect messages that need acking.
	var toAck []*Message
	for _, msg := range messages {
		if msg == nil || msg.ID == "" {
			continue
		}
		if AddressToIdentity(msg.To) != recipientIdentity {
			continue
		}
		if msg.DeliveryState == "" || msg.DeliveryState == DeliveryStateAcked {
			continue
		}
		toAck = append(toAck, msg)
	}
	if len(toAck) == 0 {
		return nil
	}

	// Run acks concurrently with bounded parallelism.
	const maxConcurrentAckOps = 8
	sem := make(chan struct{}, maxConcurrentAckOps)
	var mu sync.Mutex
	var errs []string
	var wg sync.WaitGroup

	for _, msg := range toAck {
		wg.Add(1)
		sem <- struct{}{} // acquire
		go func(id string) {
			defer wg.Done()
			defer func() { <-sem }() // release
			if err := AcknowledgeDeliveryBead(m.workDir, m.beadsDir, id, recipientIdentity); err != nil {
				mu.Lock()
				errs = append(errs, fmt.Sprintf("%s: %v", id, err))
				mu.Unlock()
			}
		}(msg.ID)
	}
	wg.Wait()

	if len(errs) > 0 {
		return fmt.Errorf("acknowledging deliveries failed: %s", strings.Join(errs, "; "))
	}
	return nil
}

// Append adds a message to the mailbox (legacy mode only).
// For beads mode, use Router.Send() instead.
func (m *Mailbox) Append(msg *Message) error {
	if !m.legacy {
		return errors.New("use Router.Send() to send messages via beads")
	}
	return m.appendLegacy(msg)
}

func (m *Mailbox) appendLegacy(msg *Message) error {
	// Ensure directory exists before acquiring lock (lock file is in same dir)
	dir := filepath.Dir(m.path)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}

	fl, err := m.lockLegacy()
	if err != nil {
		return err
	}
	defer func() { _ = fl.Unlock() }()

	// Open for append
	file, err := os.OpenFile(m.path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0600)
	if err != nil {
		return err
	}
	defer func() { _ = file.Close() }() // non-fatal: OS will close on exit

	data, err := json.Marshal(msg)
	if err != nil {
		return err
	}

	_, err = file.WriteString(string(data) + "\n")
	return err
}

// rewriteLegacy rewrites the mailbox with the given messages.
func (m *Mailbox) rewriteLegacy(messages []*Message) error {
	// Sort by timestamp (oldest first for JSONL)
	sort.Slice(messages, func(i, j int) bool {
		return messages[i].Timestamp.Before(messages[j].Timestamp)
	})

	// Write to temp file
	tmpPath := m.path + ".tmp"
	file, err := os.Create(tmpPath)
	if err != nil {
		return err
	}

	for _, msg := range messages {
		data, err := json.Marshal(msg)
		if err != nil {
			_ = file.Close()       // best-effort cleanup
			_ = os.Remove(tmpPath) // best-effort cleanup
			return err
		}
		if _, err := file.WriteString(string(data) + "\n"); err != nil {
			_ = file.Close()
			_ = os.Remove(tmpPath)
			return fmt.Errorf("writing mailbox: %w", err)
		}
	}

	if err := file.Close(); err != nil {
		_ = os.Remove(tmpPath) // best-effort cleanup
		return err
	}

	// Atomic rename
	return os.Rename(tmpPath, m.path)
}

// ListByThread returns all messages in a given thread.
func (m *Mailbox) ListByThread(threadID string) ([]*Message, error) {
	if m.legacy {
		return m.listByThreadLegacy(threadID)
	}
	return m.listByThreadBeads(threadID)
}

func (m *Mailbox) listByThreadBeads(threadID string) ([]*Message, error) {
	args := []string{"message", "thread", threadID, "--json"}

	ctx, cancel := bdReadCtx()
	defer cancel()
	stdout, err := runBdCommand(ctx, args, m.workDir, m.beadsDir, "BD_IDENTITY="+m.identity)
	if err != nil {
		return nil, err
	}

	if !isJSON(stdout) {
		return nil, nil
	}
	var beadsMsgs []BeadsMessage
	if err := json.Unmarshal(stdout, &beadsMsgs); err != nil {
		return nil, err
	}

	var messages []*Message
	for _, bm := range beadsMsgs {
		messages = append(messages, bm.ToMessage())
	}

	// Sort by timestamp (oldest first for thread view)
	sort.Slice(messages, func(i, j int) bool {
		return messages[i].Timestamp.Before(messages[j].Timestamp)
	})

	return messages, nil
}

func (m *Mailbox) listByThreadLegacy(threadID string) ([]*Message, error) {
	messages, err := m.List()
	if err != nil {
		return nil, err
	}

	var thread []*Message
	for _, msg := range messages {
		if msg.ThreadID == threadID {
			thread = append(thread, msg)
		}
	}

	// Sort by timestamp (oldest first for thread view)
	sort.Slice(thread, func(i, j int) bool {
		return thread[i].Timestamp.Before(thread[j].Timestamp)
	})

	return thread, nil
}

// isJSON returns true if the byte slice looks like JSON (starts with [ or {).
// bd list --json may return plain text like "No issues found." instead of JSON
// when there are no results.
func isJSON(b []byte) bool {
	for _, c := range b {
		switch c {
		case ' ', '\t', '\n', '\r':
			continue
		case '[', '{':
			return true
		default:
			return false
		}
	}
	return false
}
package mail

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"sync"
	"time"

	beadsdk "github.com/steveyegge/beads"
	"github.com/gofrs/flock"
	"github.com/steveyegge/gastown/internal/beads"
	"github.com/steveyegge/gastown/internal/runtime"
	"github.com/steveyegge/gastown/internal/telemetry"
)

// timeNow is a function that returns the current time. It can be overridden in tests.
var timeNow = time.Now

// Common errors
var (
	ErrMessageNotFound = errors.New("message not found")
	ErrEmptyInbox      = errors.New("inbox is empty")
)

// Mailbox manages messages for an identity via beads.
// When store is non-nil, beads-mode methods use the in-process beadsdk.Storage
// directly instead of shelling out to the bd CLI.
type Mailbox struct {
	identity string // beads identity (e.g., "gastown/polecats/Toast")
	workDir  string // directory to run bd commands in
	beadsDir string // explicit .beads directory path (set via BEADS_DIR)
	path     string // for legacy JSONL mode (crew workers)
	legacy   bool   // true = use JSONL files, false = use beads

	// store is an optional in-process beadsdk.Storage. When set, beads-mode
	// methods bypass the bd subprocess and use the store directly.
	// Callers are responsible for closing the store.
	store beadsdk.Storage
}

// NewMailbox creates a mailbox for the given JSONL path (legacy mode).
// Used by crew workers that have local JSONL inboxes.
func NewMailbox(path string) *Mailbox {
	return &Mailbox{
		path:   filepath.Join(path, "inbox.jsonl"),
		legacy: true,
	}
}

// NewMailboxBeads creates a mailbox backed by beads.
func NewMailboxBeads(identity, workDir string) *Mailbox {
	return &Mailbox{
		identity: identity,
		workDir:  workDir,
		legacy:   false,
	}
}

// NewMailboxFromAddress creates a beads-backed mailbox from a GGT address.
// Follows .beads/redirect for crew workers and polecats using shared beads.
func NewMailboxFromAddress(address, workDir string) *Mailbox {
	beadsDir := beads.ResolveBeadsDir(workDir)
	return &Mailbox{
		identity: AddressToIdentity(address),
		workDir:  workDir,
		beadsDir: beadsDir,
		legacy:   false,
	}
}

// NewMailboxWithBeadsDir creates a mailbox with an explicit beads directory.
func NewMailboxWithBeadsDir(address, workDir, beadsDir string) *Mailbox {
	return &Mailbox{
		identity: AddressToIdentity(address),
		workDir:  workDir,
		beadsDir: beadsDir,
		legacy:   false,
	}
}

// Identity returns the beads identity for this mailbox.
func (m *Mailbox) Identity() string {
	return m.identity
}

// Path returns the JSONL path for legacy mailboxes.
func (m *Mailbox) Path() string {
	return m.path
}

// lockLegacy acquires an exclusive flock for legacy mailbox operations.
// Callers must defer Unlock on the returned flock. The lock file is
// separate from the data file to avoid interfering with reads.
func (m *Mailbox) lockLegacy() (*flock.Flock, error) {
	fl := flock.New(m.path + ".lock")
	if err := fl.Lock(); err != nil {
		return nil, fmt.Errorf("acquiring mailbox lock: %w", err)
	}
	return fl, nil
}

// List returns all open messages in the mailbox.
func (m *Mailbox) List() ([]*Message, error) {
	if m.legacy {
		return m.listLegacy()
	}
	return m.listBeads()
}

func (m *Mailbox) listBeads() ([]*Message, error) {
	// Single query to beads - returns both persistent and wisp messages
	// Wisps are stored in same DB with wisp=true flag, not synced to git
	messages, err := m.listFromDir(m.beadsDir)
	if err != nil {
		return nil, err
	}

	// Sort by priority (higher first), then timestamp (newest first).
	sort.Slice(messages, func(i, j int) bool {
		pi, pj := PriorityToBeads(messages[i].Priority), PriorityToBeads(messages[j].Priority)
		if pi != pj {
			return pi < pj // lower beads int = higher priority
		}
		return messages[i].Timestamp.After(messages[j].Timestamp)
	})

	return messages, nil
}

// listFromDir queries messages from a beads directory.
// Returns messages where identity is the assignee OR a CC recipient.
// Includes both open and hooked messages (hooked = auto-assigned handoff mail).
//
// Uses per-identity --assignee queries to push filtering to Dolt, reducing
// memory footprint under concurrent agent load. A separate CC query fetches
// messages where this identity is CC'd.
func (m *Mailbox) listFromDir(beadsDir string) ([]*Message, error) {
	// Use in-process store when available
	if m.store != nil {
		return m.storeListFromDir()
	}

	identities := m.identityVariants()

	if err := beads.EnsureCustomTypes(beadsDir); err != nil {
		return nil, fmt.Errorf("ensuring custom types: %w", err)
	}

	// Deduplicate messages across queries (assignee + CC may overlap)
	seen := make(map[string]bool)
	messages := make([]*Message, 0)

	// Query 1: assignee match (per identity variant)
	for _, id := range identities {
		args := []string{"list",
			"--label", "gt:message",
			"--assignee", id,
			"--json",
			"--limit", "0",
		}

		ctx, cancel := bdReadCtx()
		stdout, err := runBdCommand(ctx, args, m.workDir, beadsDir)
		cancel()
		if err != nil {
			return nil, err
		}

		// bd v0.58.0 returns plain text (e.g. "No issues found.") for
		// empty result sets instead of JSON. Skip non-JSON output.
		if !isJSON(stdout) {
			continue
		}
		var msgs []BeadsMessage
		trimmed := bytes.TrimSpace(stdout)
		if len(trimmed) == 0 || string(trimmed) == "null" || (trimmed[0] != '[' && trimmed[0] != '{') {
			// bd v0.58.0 returns plain text (e.g. "No issues found.") for
			// empty result sets instead of JSON. Skip non-JSON output.
			continue
		}
		if err := json.Unmarshal(stdout, &msgs); err != nil {
			return nil, err
		}

		for i := range msgs {
			bm := &msgs[i]
			if seen[bm.ID] {
				continue
			}
			// Assignee match: open or hooked status
			if bm.Status == "open" || bm.Status == "hooked" {
				seen[bm.ID] = true
				messages = append(messages, bm.ToMessage())
			}
		}
	}

	// Query 2: CC match — fetch messages with cc:<identity> label
	for _, id := range identities {
		ccLabel := "cc:" + id
		args := []string{"list",
			"--label", "gt:message",
			"--label", ccLabel,
			"--json",
			"--limit", "0",
		}

		ctx, cancel := bdReadCtx()
		stdout, err := runBdCommand(ctx, args, m.workDir, beadsDir)
		cancel()
		if err != nil {
			// CC query failure is non-fatal — assignee messages are primary
			continue
		}

		if !isJSON(stdout) {
			continue
		}
		var msgs []BeadsMessage
		trimmedCC := bytes.TrimSpace(stdout)
		if len(trimmedCC) == 0 || string(trimmedCC) == "null" || (trimmedCC[0] != '[' && trimmedCC[0] != '{') {
			continue
		}
		if err := json.Unmarshal(stdout, &msgs); err != nil {
			continue // Non-fatal for CC
		}

		for i := range msgs {
			bm := &msgs[i]
			if seen[bm.ID] {
				continue
			}
			// CC match: open status only
			if bm.Status == "open" {
				seen[bm.ID] = true
				messages = append(messages, bm.ToMessage())
			}
		}
	}

	// Query 3: Wisps table — ephemeral messages (protocol/lifecycle) are stored
	// as wisps by shouldBeWisp(), but bd list only queries the issues table.
	wispMessages := m.listWispMessages(beadsDir, identities, seen)
	messages = append(messages, wispMessages...)

	return messages, nil
}

// listWispMessages queries the wisps table for ephemeral messages matching the identity.
// Protocol/lifecycle messages are stored as wisps by shouldBeWisp(), but bd list only
// queries the issues table. Uses bd sql --json for full wisp data.
func (m *Mailbox) listWispMessages(beadsDir string, identities []string, seen map[string]bool) []*Message {
	var messages []*Message

	// Query 3a: assignee match via SQL on wisps table
	for _, id := range identities {
		wispMsgs := m.queryWispMessagesByAssignee(beadsDir, id)
		for _, bm := range wispMsgs {
			if seen[bm.ID] {
				continue
			}
			if bm.Status == "open" || bm.Status == "hooked" {
				seen[bm.ID] = true
				messages = append(messages, bm.ToMessage())
			}
		}
	}

	// Query 3b: CC match via SQL on wisps table
	for _, id := range identities {
		wispMsgs := m.queryWispMessagesByCC(beadsDir, id)
		for _, bm := range wispMsgs {
			if seen[bm.ID] {
				continue
			}
			if bm.Status == "open" {
				seen[bm.ID] = true
				messages = append(messages, bm.ToMessage())
			}
		}
	}

	return messages
}

// queryWispMessagesByAssignee queries wisps table for messages assigned to identity.
func (m *Mailbox) queryWispMessagesByAssignee(beadsDir, identity string) []BeadsMessage {
	query := fmt.Sprintf(
		"SELECT w.id, w.title, w.description, w.status, w.priority, w.assignee, w.created_at, w.updated_at, "+
			"GROUP_CONCAT(al.label) as labels_csv "+
			"FROM wisps w "+
			"JOIN wisp_labels l ON w.id = l.issue_id "+
			"JOIN wisp_labels al ON w.id = al.issue_id "+
			"WHERE l.label = 'gt:message' AND w.status IN ('open', 'hooked') AND w.assignee = '%s' "+
			"GROUP BY w.id, w.title, w.description, w.status, w.priority, w.assignee, w.created_at, w.updated_at",
		escapeSQLString(identity))
	return m.runWispSQL(beadsDir, query)
}

// queryWispMessagesByCC queries wisps table for messages where identity is CC'd.
func (m *Mailbox) queryWispMessagesByCC(beadsDir, identity string) []BeadsMessage {
	ccLabel := "cc:" + identity
	query := fmt.Sprintf(
		"SELECT w.id, w.title, w.description, w.status, w.priority, w.assignee, w.created_at, w.updated_at, "+
			"GROUP_CONCAT(al.label) as labels_csv "+
			"FROM wisps w "+
			"JOIN wisp_labels l1 ON w.id = l1.issue_id "+
			"JOIN wisp_labels l2 ON w.id = l2.issue_id "+
			"JOIN wisp_labels al ON w.id = al.issue_id "+
			"WHERE l1.label = 'gt:message' AND l2.label = '%s' AND w.status IN ('open', 'hooked') "+
			"GROUP BY w.id, w.title, w.description, w.status, w.priority, w.assignee, w.created_at, w.updated_at",
		escapeSQLString(ccLabel))
	return m.runWispSQL(beadsDir, query)
}

// wispSQLRow represents a row from the wisps SQL query with aggregated labels.
type wispSQLRow struct {
	ID          string `json:"id"`
	Title       string `json:"title"`
	Description string `json:"description"`
	Status      string `json:"status"`
	Priority    int    `json:"priority"`
	Assignee    string `json:"assignee"`
	CreatedAt   string `json:"created_at"`
	UpdatedAt   string `json:"updated_at"`
	LabelsCSV   string `json:"labels_csv"`
}

// runWispSQL executes a bd sql --json query and converts results to BeadsMessages.
func (m *Mailbox) runWispSQL(beadsDir, query string) []BeadsMessage {
	args := []string{"sql", "--json", query}
	ctx, cancel := bdReadCtx()
	stdout, err := runBdCommand(ctx, args, m.workDir, beadsDir)
	cancel()
	if err != nil {
		return nil // Wisps table may not exist yet
	}
	if !isJSON(stdout) {
		return nil
	}

	var rows []wispSQLRow
	if err := json.Unmarshal(stdout, &rows); err != nil {
		return nil
	}

	msgs := make([]BeadsMessage, 0, len(rows))
	for _, row := range rows {
		bm := BeadsMessage{
			ID:          row.ID,
			Title:       row.Title,
			Description: row.Description,
			Status:      row.Status,
			Priority:    row.Priority,
			Assignee:    row.Assignee,
			Wisp:        true,
		}
		if t, err := time.Parse(time.RFC3339, row.CreatedAt); err == nil {
			bm.CreatedAt = t
		} else if t, err := time.Parse("2006-01-02 15:04:05 +0000 UTC", row.CreatedAt); err == nil {
			bm.CreatedAt = t
		}
		if row.LabelsCSV != "" {
			bm.Labels = strings.Split(row.LabelsCSV, ",")
		}
		msgs = append(msgs, bm)
	}
	return msgs
}

// escapeSQLString escapes single quotes for SQL string literals.
func escapeSQLString(s string) string {
	return strings.ReplaceAll(s, "'", "''")
}

// identityVariants returns all identity formats to query.
// For town-level agents (mayor/, deacon/), also includes the variant without
// trailing slash for backwards compatibility with legacy messages.
func (m *Mailbox) identityVariants() []string {
	variants := []string{m.identity}

	// Town-level agents may have legacy messages without trailing slash
	if m.identity == "mayor/" {
		variants = append(variants, "mayor")
	} else if m.identity == "deacon/" {
		variants = append(variants, "deacon")
	}

	return variants
}

func (m *Mailbox) listLegacy() ([]*Message, error) {
	file, err := os.Open(m.path)
	if err != nil {
		if os.IsNotExist(err) {
			return make([]*Message, 0), nil
		}
		return nil, err
	}
	defer func() { _ = file.Close() }() // non-fatal: OS will close on exit

	messages := make([]*Message, 0)
	scanner := bufio.NewScanner(file)
	lineNum := 0
	for scanner.Scan() {
		lineNum++
		line := scanner.Text()
		if line == "" {
			continue
		}

		var msg Message
		if err := json.Unmarshal([]byte(line), &msg); err != nil {
			return nil, fmt.Errorf("corrupt mailbox %s line %d: %w", m.path, lineNum, err)
		}
		messages = append(messages, &msg)
	}

	if err := scanner.Err(); err != nil {
		return nil, err
	}

	// Sort by priority (higher first), then timestamp (newest first).
	sort.Slice(messages, func(i, j int) bool {
		pi, pj := PriorityToBeads(messages[i].Priority), PriorityToBeads(messages[j].Priority)
		if pi != pj {
			return pi < pj
		}
		return messages[i].Timestamp.After(messages[j].Timestamp)
	})

	return messages, nil
}

// ListUnread returns unread (open) messages.
// Filters out messages marked as read (via "read" label in beads mode).
func (m *Mailbox) ListUnread() ([]*Message, error) {
	all, err := m.List()
	if err != nil {
		return nil, err
	}
	unread := make([]*Message, 0)
	for _, msg := range all {
		if !msg.Read {
			unread = append(unread, msg)
		}
	}
	return unread, nil
}

// Get returns a message by ID.
func (m *Mailbox) Get(id string) (*Message, error) {
	if m.legacy {
		return m.getLegacy(id)
	}
	return m.getBeads(id)
}

func (m *Mailbox) getBeads(id string) (*Message, error) {
	// Resolve correct beadsDir based on bead ID prefix (GH#2423)
	primary := beads.ResolveBeadsDirForID(m.beadsDir, id)
	msg, err := m.getFromDir(id, primary)
	if errors.Is(err, ErrMessageNotFound) && primary != m.beadsDir {
		// Cross-rig bead IDs (e.g. ne-*) may live in the home DB when created
		// via the mail router (which always uses town beads). Fall back to
		// m.beadsDir before giving up. See ne-bgr.
		return m.getFromDir(id, m.beadsDir)
	}
	return msg, err
}

// getFromDir retrieves a message from a beads directory.
func (m *Mailbox) getFromDir(id, beadsDir string) (*Message, error) {
	if m.store != nil {
		return m.storeGetFromDir(id)
	}

	args := []string{"show", id, "--json"}

	ctx, cancel := bdReadCtx()
	defer cancel()
	stdout, err := runBdCommand(ctx, args, m.workDir, beadsDir)
	if err != nil {
		if bdErr, ok := err.(*bdError); ok && bdErr.ContainsError("not found") {
			return nil, ErrMessageNotFound
		}
		return nil, err
	}

	// bd show --json returns an array
	if !isJSON(stdout) {
		return nil, ErrMessageNotFound
	}
	var bms []BeadsMessage
	if err := json.Unmarshal(stdout, &bms); err != nil {
		return nil, err
	}
	if len(bms) == 0 {
		return nil, ErrMessageNotFound
	}

	// Wisp status comes from beads issue.wisp field via ToMessage()
	return bms[0].ToMessage(), nil
}

func (m *Mailbox) getLegacy(id string) (*Message, error) {
	messages, err := m.List()
	if err != nil {
		return nil, err
	}
	for _, msg := range messages {
		if msg.ID == id {
			return msg, nil
		}
	}
	return nil, ErrMessageNotFound
}

// MarkRead marks a message as read.
func (m *Mailbox) MarkRead(id string) error {
	if m.legacy {
		return m.markReadLegacy(id)
	}
	return m.markReadBeads(id)
}

func (m *Mailbox) markReadBeads(id string) error {
	// Resolve correct beadsDir based on bead ID prefix (GH#2423)
	primary := beads.ResolveBeadsDirForID(m.beadsDir, id)
	err := m.closeInDir(id, primary)
	if errors.Is(err, ErrMessageNotFound) && primary != m.beadsDir {
		// Cross-rig bead IDs (e.g. ne-*) may live in the home DB when created
		// via the mail router (which always uses town beads). Fall back to
		// m.beadsDir before giving up. See ne-bgr.
		return m.closeInDir(id, m.beadsDir)
	}
	return err
}

// closeInDir closes a message in a specific beads directory.
func (m *Mailbox) closeInDir(id, beadsDir string) error {
	if m.store != nil {
		return m.storeCloseInDir(id)
	}

	args := []string{"close", id}
	// Pass session ID for work attribution if available
	if sessionID := runtime.SessionIDFromEnv(); sessionID != "" {
		args = append(args, "--session="+sessionID)
	}

	ctx, cancel := bdWriteCtx()
	defer cancel()
	_, err := runBdCommand(ctx, args, m.workDir, beadsDir)
	telemetry.RecordMailMessage(context.Background(), "read", telemetry.MailMessageInfo{
		ID: id,
		To: m.identity,
	}, err)
	if err != nil {
		if bdErr, ok := err.(*bdError); ok && bdErr.ContainsError("not found") {
			return ErrMessageNotFound
		}
		return err
	}

	return nil
}

func (m *Mailbox) markReadLegacy(id string) error {
	fl, err := m.lockLegacy()
	if err != nil {
		return err
	}
	defer func() { _ = fl.Unlock() }()

	messages, err := m.List()
	if err != nil {
		return err
	}

	found := false
	for _, msg := range messages {
		if msg.ID == id {
			msg.Read = true
			found = true
		}
	}

	if !found {
		return ErrMessageNotFound
	}

	return m.rewriteLegacy(messages)
}

// MarkReadOnly marks a message as read WITHOUT archiving/closing it.
// For beads mode, this adds a "read" label to the message.
// For legacy mode, this sets the Read field to true.
// The message remains in the inbox but is displayed as read.
func (m *Mailbox) MarkReadOnly(id string) error {
	if m.legacy {
		return m.markReadLegacy(id)
	}
	return m.markReadOnlyBeads(id)
}

func (m *Mailbox) markReadOnlyBeads(id string) error {
	if m.store != nil {
		return m.storeMarkReadOnly(id)
	}

	// Add "read" label to mark as read without closing
	args := []string{"label", "add", id, "read"}
	primary := beads.ResolveBeadsDirForID(m.beadsDir, id)

	ctx, cancel := bdWriteCtx()
	defer cancel()
	_, err := runBdCommand(ctx, args, m.workDir, primary)
	if err != nil {
		if bdErr, ok := err.(*bdError); ok && bdErr.ContainsError("not found") {
			if primary != m.beadsDir {
				// Cross-rig bead IDs (e.g. ne-*) may live in the home DB. See ne-bgr.
				ctx2, cancel2 := bdWriteCtx()
				defer cancel2()
				_, err2 := runBdCommand(ctx2, args, m.workDir, m.beadsDir)
				if err2 != nil {
					if bdErr2, ok := err2.(*bdError); ok && bdErr2.ContainsError("not found") {
						return ErrMessageNotFound
					}
					return err2
				}
				return nil
			}
			return ErrMessageNotFound
		}
		return err
	}

	return nil
}

// MarkUnreadOnly marks a message as unread (removes "read" label).
// For beads mode, this removes the "read" label from the message.
// For legacy mode, this sets the Read field to false.
func (m *Mailbox) MarkUnreadOnly(id string) error {
	if m.legacy {
		return m.markUnreadLegacy(id)
	}
	return m.markUnreadOnlyBeads(id)
}

func (m *Mailbox) markUnreadOnlyBeads(id string) error {
	if m.store != nil {
		return m.storeMarkUnreadOnly(id)
	}

	// Remove "read" label to mark as unread
	args := []string{"label", "remove", id, "read"}
	primary := beads.ResolveBeadsDirForID(m.beadsDir, id)

	ctx, cancel := bdWriteCtx()
	defer cancel()
	_, err := runBdCommand(ctx, args, m.workDir, primary)
	if err != nil {
		if bdErr, ok := err.(*bdError); ok && bdErr.ContainsError("not found") {
			if primary != m.beadsDir {
				// Cross-rig bead IDs (e.g. ne-*) may live in the home DB. See ne-bgr.
				ctx2, cancel2 := bdWriteCtx()
				defer cancel2()
				_, err2 := runBdCommand(ctx2, args, m.workDir, m.beadsDir)
				if err2 != nil {
					if bdErr2, ok := err2.(*bdError); ok && bdErr2.ContainsError("not found") {
						return ErrMessageNotFound
					}
					if bdErr2, ok := err2.(*bdError); ok && bdErr2.ContainsError("does not have label") {
						return nil
					}
					return err2
				}
				return nil
			}
			return ErrMessageNotFound
		}
		// Ignore error if label doesn't exist
		if bdErr, ok := err.(*bdError); ok && bdErr.ContainsError("does not have label") {
			return nil
		}
		return err
	}

	return nil
}

// MarkUnread marks a message as unread (reopens in beads).
func (m *Mailbox) MarkUnread(id string) error {
	if m.legacy {
		return m.markUnreadLegacy(id)
	}
	return m.markUnreadBeads(id)
}

func (m *Mailbox) markUnreadBeads(id string) error {
	if m.store != nil {
		return m.storeMarkUnread(id)
	}

	args := []string{"reopen", id}
	primary := beads.ResolveBeadsDirForID(m.beadsDir, id)

	ctx, cancel := bdWriteCtx()
	defer cancel()
	_, err := runBdCommand(ctx, args, m.workDir, primary)
	if err != nil {
		if bdErr, ok := err.(*bdError); ok && bdErr.ContainsError("not found") {
			if primary != m.beadsDir {
				// Cross-rig bead IDs (e.g. ne-*) may live in the home DB. See ne-bgr.
				ctx2, cancel2 := bdWriteCtx()
				defer cancel2()
				_, err2 := runBdCommand(ctx2, args, m.workDir, m.beadsDir)
				if err2 != nil {
					if bdErr2, ok := err2.(*bdError); ok && bdErr2.ContainsError("not found") {
						return ErrMessageNotFound
					}
					return err2
				}
				return nil
			}
			return ErrMessageNotFound
		}
		return err
	}

	return nil
}

func (m *Mailbox) markUnreadLegacy(id string) error {
	fl, err := m.lockLegacy()
	if err != nil {
		return err
	}
	defer func() { _ = fl.Unlock() }()

	messages, err := m.List()
	if err != nil {
		return err
	}

	found := false
	for _, msg := range messages {
		if msg.ID == id {
			msg.Read = false
			found = true
		}
	}

	if !found {
		return ErrMessageNotFound
	}

	return m.rewriteLegacy(messages)
}

// Delete removes a message.
func (m *Mailbox) Delete(id string) error {
	if m.legacy {
		return m.deleteLegacy(id)
	}
	return m.MarkRead(id) // beads: just acknowledge/close
}

func (m *Mailbox) deleteLegacy(id string) error {
	fl, err := m.lockLegacy()
	if err != nil {
		return err
	}
	defer func() { _ = fl.Unlock() }()

	messages, err := m.List()
	if err != nil {
		return err
	}

	var filtered []*Message
	found := false
	for _, msg := range messages {
		if msg.ID == id {
			found = true
		} else {
			filtered = append(filtered, msg)
		}
	}

	if !found {
		return ErrMessageNotFound
	}

	return m.rewriteLegacy(filtered)
}

// Archive moves a message to the archive file and removes it from inbox.
func (m *Mailbox) Archive(id string) error {
	if m.legacy {
		return m.archiveLegacy(id)
	}
	// Beads mode: append to archive then close
	msg, err := m.Get(id)
	if err != nil {
		return err
	}
	if err := m.appendToArchive(msg); err != nil {
		return err
	}
	return m.Delete(id)
}

// archiveLegacy moves a message to the archive file atomically.
// A single flock covers the entire read-archive-rewrite cycle so that
// a crash between appendToArchive and the inbox rewrite cannot lose the
// message (worst case: duplicate in both archive and inbox).
func (m *Mailbox) archiveLegacy(id string) error {
	fl, err := m.lockLegacy()
	if err != nil {
		return err
	}
	defer func() { _ = fl.Unlock() }()

	// Read inbox
	messages, err := m.listLegacy()
	if err != nil {
		return err
	}

	// Find and extract target
	var target *Message
	var remaining []*Message
	for _, msg := range messages {
		if msg.ID == id {
			target = msg
		} else {
			remaining = append(remaining, msg)
		}
	}
	if target == nil {
		return ErrMessageNotFound
	}

	// Append to archive first (safe failure mode: duplicate, not loss)
	if err := m.appendToArchive(target); err != nil {
		return err
	}

	// Rewrite inbox without the target
	return m.rewriteLegacy(remaining)
}

// ArchivePath returns the path to the archive file.
func (m *Mailbox) ArchivePath() string {
	if m.legacy {
		return m.path + ".archive"
	}
	// For beads, use archive.jsonl in the same directory as beads
	return filepath.Join(m.beadsDir, "archive.jsonl")
}

func (m *Mailbox) appendToArchive(msg *Message) error {
	archivePath := m.ArchivePath()

	// Ensure directory exists
	dir := filepath.Dir(archivePath)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}

	// Open for append
	file, err := os.OpenFile(archivePath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644) //nolint:gosec // G302: archive is non-sensitive operational data
	if err != nil {
		return err
	}
	defer func() { _ = file.Close() }()

	data, err := json.Marshal(msg)
	if err != nil {
		return err
	}

	_, err = file.WriteString(string(data) + "\n")
	return err
}

// ListArchived returns all messages in the archive file.
func (m *Mailbox) ListArchived() ([]*Message, error) {
	archivePath := m.ArchivePath()

	file, err := os.Open(archivePath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	defer func() { _ = file.Close() }()

	var messages []*Message
	scanner := bufio.NewScanner(file)
	lineNum := 0
	for scanner.Scan() {
		lineNum++
		line := scanner.Text()
		if line == "" {
			continue
		}

		var msg Message
		if err := json.Unmarshal([]byte(line), &msg); err != nil {
			return nil, fmt.Errorf("corrupt archive %s line %d: %w", archivePath, lineNum, err)
		}
		messages = append(messages, &msg)
	}

	if err := scanner.Err(); err != nil {
		return nil, err
	}

	return messages, nil
}

// PurgeArchive removes messages from the archive, optionally filtering by age.
// If olderThanDays is 0, removes all archived messages.
func (m *Mailbox) PurgeArchive(olderThanDays int) (int, error) {
	if m.legacy {
		fl, err := m.lockLegacy()
		if err != nil {
			return 0, err
		}
		defer func() { _ = fl.Unlock() }()
	}

	messages, err := m.ListArchived()
	if err != nil {
		return 0, err
	}

	if len(messages) == 0 {
		return 0, nil
	}

	// If no age filter, remove all
	if olderThanDays <= 0 {
		if err := os.Remove(m.ArchivePath()); err != nil && !os.IsNotExist(err) {
			return 0, err
		}
		return len(messages), nil
	}

	// Filter by age
	cutoff := timeNow().AddDate(0, 0, -olderThanDays)
	var keep []*Message
	purged := 0

	for _, msg := range messages {
		if msg.Timestamp.Before(cutoff) {
			purged++
		} else {
			keep = append(keep, msg)
		}
	}

	// Rewrite archive with remaining messages
	if len(keep) == 0 {
		if err := os.Remove(m.ArchivePath()); err != nil && !os.IsNotExist(err) {
			return 0, err
		}
	} else {
		if err := m.rewriteArchive(keep); err != nil {
			return 0, err
		}
	}

	return purged, nil
}

func (m *Mailbox) rewriteArchive(messages []*Message) error {
	archivePath := m.ArchivePath()
	tmpPath := archivePath + ".tmp"

	file, err := os.Create(tmpPath)
	if err != nil {
		return err
	}

	for _, msg := range messages {
		data, err := json.Marshal(msg)
		if err != nil {
			_ = file.Close()
			_ = os.Remove(tmpPath)
			return err
		}
		if _, err := file.WriteString(string(data) + "\n"); err != nil {
			_ = file.Close()
			_ = os.Remove(tmpPath)
			return fmt.Errorf("writing archive: %w", err)
		}
	}

	if err := file.Close(); err != nil {
		_ = os.Remove(tmpPath)
		return err
	}

	return os.Rename(tmpPath, archivePath)
}

// SearchOptions specifies search parameters.
type SearchOptions struct {
	Query       string // Regex pattern to search for
	FromFilter  string // Optional: only match messages from this sender
	SubjectOnly bool   // Only search subject
	BodyOnly    bool   // Only search body
}

// Search finds messages matching the given criteria.
// Returns messages from both inbox and archive.
// Query and FromFilter are treated as literal strings (not regex) to prevent ReDoS.
func (m *Mailbox) Search(opts SearchOptions) ([]*Message, error) {
	// Use QuoteMeta to escape special regex chars - prevents ReDoS attacks
	// and provides intuitive literal string matching for users
	re, err := regexp.Compile("(?i)" + regexp.QuoteMeta(opts.Query))
	if err != nil {
		return nil, fmt.Errorf("invalid search pattern: %w", err)
	}

	var fromRe *regexp.Regexp
	if opts.FromFilter != "" {
		fromRe, err = regexp.Compile("(?i)" + regexp.QuoteMeta(opts.FromFilter))
		if err != nil {
			return nil, fmt.Errorf("invalid from pattern: %w", err)
		}
	}

	// Get inbox messages
	inbox, err := m.List()
	if err != nil {
		return nil, err
	}

	// Get archived messages
	archived, err := m.ListArchived()
	if err != nil && !os.IsNotExist(err) {
		return nil, err
	}

	// Combine and search
	all := append(inbox, archived...)
	var matches []*Message

	for _, msg := range all {
		// Apply from filter
		if fromRe != nil && !fromRe.MatchString(msg.From) {
			continue
		}

		// Search in specified fields
		matched := false
		if opts.SubjectOnly {
			matched = re.MatchString(msg.Subject)
		} else if opts.BodyOnly {
			matched = re.MatchString(msg.Body)
		} else {
			// Search in both subject and body
			matched = re.MatchString(msg.Subject) || re.MatchString(msg.Body)
		}

		if matched {
			matches = append(matches, msg)
		}
	}

	// Sort by priority (higher first), then timestamp (newest first).
	sort.Slice(matches, func(i, j int) bool {
		pi, pj := PriorityToBeads(matches[i].Priority), PriorityToBeads(matches[j].Priority)
		if pi != pj {
			return pi < pj
		}
		return matches[i].Timestamp.After(matches[j].Timestamp)
	})

	return matches, nil
}

// Count returns the total and unread message counts.
func (m *Mailbox) Count() (total, unread int, err error) {
	messages, err := m.List()
	if err != nil {
		return 0, 0, err
	}

	total = len(messages)
	// Count messages that are NOT marked as read (including via "read" label)
	for _, msg := range messages {
		if !msg.Read {
			unread++
		}
	}

	return total, unread, nil
}

// AcknowledgeDeliveries marks delivery receipt for unread messages where this
// mailbox is the primary recipient. This is phase-2 of two-phase delivery
// tracking (phase-1 is written at send time as delivery:pending).
// Acks are run concurrently (bounded to 8) to avoid N+1 sequential subprocess
// spawns on the hot path.
func (m *Mailbox) AcknowledgeDeliveries(recipientAddress string, messages []*Message) error {
	if m.legacy || len(messages) == 0 {
		return nil
	}

	recipientIdentity := AddressToIdentity(recipientAddress)

	// Collect messages that need acking.
	var toAck []*Message
	for _, msg := range messages {
		if msg == nil || msg.ID == "" {
			continue
		}
		if AddressToIdentity(msg.To) != recipientIdentity {
			continue
		}
		if msg.DeliveryState == "" || msg.DeliveryState == DeliveryStateAcked {
			continue
		}
		toAck = append(toAck, msg)
	}
	if len(toAck) == 0 {
		return nil
	}

	// Run acks concurrently with bounded parallelism.
	const maxConcurrentAckOps = 8
	sem := make(chan struct{}, maxConcurrentAckOps)
	var mu sync.Mutex
	var errs []string
	var wg sync.WaitGroup

	for _, msg := range toAck {
		wg.Add(1)
		sem <- struct{}{} // acquire
		go func(id string) {
			defer wg.Done()
			defer func() { <-sem }() // release
			if err := AcknowledgeDeliveryBead(m.workDir, m.beadsDir, id, recipientIdentity); err != nil {
				mu.Lock()
				errs = append(errs, fmt.Sprintf("%s: %v", id, err))
				mu.Unlock()
			}
		}(msg.ID)
	}
	wg.Wait()

	if len(errs) > 0 {
		return fmt.Errorf("acknowledging deliveries failed: %s", strings.Join(errs, "; "))
	}
	return nil
}

// Append adds a message to the mailbox (legacy mode only).
// For beads mode, use Router.Send() instead.
func (m *Mailbox) Append(msg *Message) error {
	if !m.legacy {
		return errors.New("use Router.Send() to send messages via beads")
	}
	return m.appendLegacy(msg)
}

func (m *Mailbox) appendLegacy(msg *Message) error {
	// Ensure directory exists before acquiring lock (lock file is in same dir)
	dir := filepath.Dir(m.path)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}

	fl, err := m.lockLegacy()
	if err != nil {
		return err
	}
	defer func() { _ = fl.Unlock() }()

	// Open for append
	file, err := os.OpenFile(m.path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0600)
	if err != nil {
		return err
	}
	defer func() { _ = file.Close() }() // non-fatal: OS will close on exit

	data, err := json.Marshal(msg)
	if err != nil {
		return err
	}

	_, err = file.WriteString(string(data) + "\n")
	return err
}

// rewriteLegacy rewrites the mailbox with the given messages.
func (m *Mailbox) rewriteLegacy(messages []*Message) error {
	// Sort by timestamp (oldest first for JSONL)
	sort.Slice(messages, func(i, j int) bool {
		return messages[i].Timestamp.Before(messages[j].Timestamp)
	})

	// Write to temp file
	tmpPath := m.path + ".tmp"
	file, err := os.Create(tmpPath)
	if err != nil {
		return err
	}

	for _, msg := range messages {
		data, err := json.Marshal(msg)
		if err != nil {
			_ = file.Close()       // best-effort cleanup
			_ = os.Remove(tmpPath) // best-effort cleanup
			return err
		}
		if _, err := file.WriteString(string(data) + "\n"); err != nil {
			_ = file.Close()
			_ = os.Remove(tmpPath)
			return fmt.Errorf("writing mailbox: %w", err)
		}
	}

	if err := file.Close(); err != nil {
		_ = os.Remove(tmpPath) // best-effort cleanup
		return err
	}

	// Atomic rename
	return os.Rename(tmpPath, m.path)
}

// ListByThread returns all messages in a given thread.
func (m *Mailbox) ListByThread(threadID string) ([]*Message, error) {
	if m.legacy {
		return m.listByThreadLegacy(threadID)
	}
	return m.listByThreadBeads(threadID)
}

func (m *Mailbox) listByThreadBeads(threadID string) ([]*Message, error) {
	args := []string{"message", "thread", threadID, "--json"}

	ctx, cancel := bdReadCtx()
	defer cancel()
	stdout, err := runBdCommand(ctx, args, m.workDir, m.beadsDir, "BD_IDENTITY="+m.identity)
	if err != nil {
		return nil, err
	}

	if !isJSON(stdout) {
		return nil, nil
	}
	var beadsMsgs []BeadsMessage
	if err := json.Unmarshal(stdout, &beadsMsgs); err != nil {
		return nil, err
	}

	var messages []*Message
	for _, bm := range beadsMsgs {
		messages = append(messages, bm.ToMessage())
	}

	// Sort by timestamp (oldest first for thread view)
	sort.Slice(messages, func(i, j int) bool {
		return messages[i].Timestamp.Before(messages[j].Timestamp)
	})

	return messages, nil
}

func (m *Mailbox) listByThreadLegacy(threadID string) ([]*Message, error) {
	messages, err := m.List()
	if err != nil {
		return nil, err
	}

	var thread []*Message
	for _, msg := range messages {
		if msg.ThreadID == threadID {
			thread = append(thread, msg)
		}
	}

	// Sort by timestamp (oldest first for thread view)
	sort.Slice(thread, func(i, j int) bool {
		return thread[i].Timestamp.Before(thread[j].Timestamp)
	})

	return thread, nil
}

// isJSON returns true if the byte slice looks like JSON (starts with [ or {).
// bd list --json may return plain text like "No issues found." instead of JSON
// when there are no results.
func isJSON(b []byte) bool {
	for _, c := range b {
		switch c {
		case ' ', '\t', '\n', '\r':
			continue
		case '[', '{':
			return true
		default:
			return false
		}
	}
	return false
}
package mail

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/steveyegge/gastown/internal/beads"
)

const (
	// DeliveryStatePending indicates a message has been durably written but not
	// yet acknowledged by a worker/recipient.
	DeliveryStatePending = "pending"
	// DeliveryStateAcked indicates receipt has been acknowledged.
	DeliveryStateAcked = "acked"

	// Label keys used for two-phase delivery tracking.
	DeliveryLabelPending       = "delivery:pending"
	DeliveryLabelAcked         = "delivery:acked"
	DeliveryLabelAckedByPrefix = "delivery-acked-by:"
	DeliveryLabelAckedAtPrefix = "delivery-acked-at:"
)

// DeliverySendLabels returns labels written during phase-1 (send).
func DeliverySendLabels() []string {
	return []string{DeliveryLabelPending}
}

// DeliveryAckLabelSequence returns labels for phase-2 (ack). The ordering is
// intentional for crash safety: state remains pending until the final ack label
// write succeeds.
func DeliveryAckLabelSequence(recipientIdentity string, at time.Time) []string {
	ackedAt := at.UTC().Format(time.RFC3339)
	return []string{
		DeliveryLabelAckedByPrefix + recipientIdentity,
		DeliveryLabelAckedAtPrefix + ackedAt,
		DeliveryLabelAcked,
	}
}

// DeliveryAckLabelSequenceIdempotent returns ack labels, reusing an existing
// timestamp from existingLabels if one is present AND the recipient identity
// matches. This ensures retries produce the exact same label set instead of
// appending duplicate timestamps. If the recipient differs (e.g., after a
// claim-release-reclaim cycle), a fresh timestamp is generated.
//
// The scan is order-independent because bd show --json returns labels in
// lexicographic order, not insertion order. We collect all acked-by and
// acked-at values and only reuse a timestamp when this recipient is the
// sole acker (no mixed state from crash recovery).
func DeliveryAckLabelSequenceIdempotent(recipientIdentity string, at time.Time, existingLabels []string) []string {
	ts := at.UTC().Format(time.RFC3339)
	var recipients []string
	var timestamps []string
	for _, label := range existingLabels {
		if strings.HasPrefix(label, DeliveryLabelAckedByPrefix) {
			recipients = append(recipients, strings.TrimPrefix(label, DeliveryLabelAckedByPrefix))
		}
		if strings.HasPrefix(label, DeliveryLabelAckedAtPrefix) {
			timestamps = append(timestamps, strings.TrimPrefix(label, DeliveryLabelAckedAtPrefix))
		}
	}
	// Reuse existing timestamp only when this recipient is the sole acker
	// and exactly one timestamp exists. Multiple acked-by labels indicate
	// mixed state from crash recovery — use fresh timestamp to avoid
	// cross-recipient leakage.
	if len(recipients) == 1 && recipients[0] == recipientIdentity && len(timestamps) == 1 {
		ts = timestamps[0]
	}
	return []string{
		DeliveryLabelAckedByPrefix + recipientIdentity,
		DeliveryLabelAckedAtPrefix + ts,
		DeliveryLabelAcked,
	}
}

// AcknowledgeDeliveryBead writes phase-2 delivery ack labels for a bead.
// It reads existing labels for idempotent retry (reusing prior timestamps),
// then writes the ack label sequence. Uses runBdCommand with timeouts.
// Resolves the correct beadsDir based on the bead ID prefix (GH#2423).
func AcknowledgeDeliveryBead(workDir, beadsDir, beadID, recipientIdentity string) error {
	beadsDir = beads.ResolveBeadsDirForID(beadsDir, beadID)
	existingLabels, readErr := readBeadLabelsShared(workDir, beadsDir, beadID)
	if readErr != nil {
		// Log but proceed with empty labels — fresh timestamp is acceptable
		// degradation vs blocking the ack entirely.
		fmt.Fprintf(os.Stderr, "delivery ack: could not read labels for %s: %v (proceeding with fresh timestamp)\n", beadID, readErr)
	}

	for _, label := range DeliveryAckLabelSequenceIdempotent(recipientIdentity, timeNow().UTC(), existingLabels) {
		args := []string{"label", "add", beadID, label}
		ctx, cancel := bdWriteCtx()
		_, err := runBdCommand(ctx, args, workDir, beadsDir)
		cancel()
		if err == nil {
			continue // bd label add silently succeeds on duplicate labels.
		}
		if bdErr, ok := err.(*bdError); ok && bdErr.ContainsError("not found") {
			return ErrMessageNotFound
		}
		return err
	}
	return nil
}

// readBeadLabelsShared reads the labels for a bead, returning an error on failure
// instead of silently swallowing it.
func readBeadLabelsShared(workDir, beadsDir, id string) ([]string, error) {
	args := []string{"show", id, "--json"}
	ctx, cancel := bdReadCtx()
	defer cancel()
	stdout, err := runBdCommand(ctx, args, workDir, beadsDir)
	if err != nil {
		return nil, fmt.Errorf("bd show %s: %w", id, err)
	}
	var bms []BeadsMessage
	if err := json.Unmarshal(stdout, &bms); err != nil {
		return nil, fmt.Errorf("parsing bd show %s: %w", id, err)
	}
	if len(bms) == 0 {
		return nil, nil
	}
	return bms[0].Labels, nil
}

// ParseDeliveryLabels derives delivery state and ack metadata from labels.
// The state is append-only:
// - `delivery:pending` means pending
// - once `delivery:acked` appears, state is acked (even if pending remains)
//
// Note: bd show --json returns labels in lexicographic order, so this parser
// must be order-independent. It uses last-wins for both acked-by and acked-at.
// For RFC3339 timestamps, lexicographic last-wins is chronologically correct.
func ParseDeliveryLabels(labels []string) (state, ackedBy string, ackedAt *time.Time) {
	hasPending := false
	hasAcked := false

	for _, label := range labels {
		switch {
		case label == DeliveryLabelPending:
			hasPending = true
		case label == DeliveryLabelAcked:
			hasAcked = true
		case strings.HasPrefix(label, DeliveryLabelAckedByPrefix):
			ackedBy = strings.TrimPrefix(label, DeliveryLabelAckedByPrefix)
		case strings.HasPrefix(label, DeliveryLabelAckedAtPrefix):
			ts := strings.TrimPrefix(label, DeliveryLabelAckedAtPrefix)
			if t, err := time.Parse(time.RFC3339, ts); err == nil {
				ackedAt = &t
			}
		}
	}

	if hasAcked {
		return DeliveryStateAcked, ackedBy, ackedAt
	}
	if hasPending {
		return DeliveryStatePending, "", nil
	}
	return "", "", nil
}
package cmd

import (
	"github.com/spf13/cobra"
)

// Mail command flags
var (
	mailSubject       string
	mailBody          string
	mailPriority      int
	mailUrgent        bool
	mailPinned        bool
	mailWisp          bool
	mailPermanent     bool
	mailType          string
	mailReplyTo       string
	mailNotify        bool
	mailNoNotify      bool // Suppress auto-nudge notification to recipient
	mailTo            string   // --to flag (alternative to positional arg)
	mailFrom          string   // --from flag (override sender, for relay/bridge use)
	mailSendSelf      bool
	mailCC            []string // CC recipients
	mailInboxJSON     bool
	mailReadJSON      bool
	mailInboxUnread   bool
	mailInboxAll      bool
	mailInboxIdentity string
	mailCheckInject   bool
	mailCheckJSON     bool
	mailCheckIdentity string
	mailThreadJSON    bool
	mailReplySubject  string
	mailReplyMessage  string
	mailStdin         bool // Read message body from stdin

	// Search flags
	mailSearchFrom    string
	mailSearchSubject bool
	mailSearchBody    bool
	mailSearchArchive bool
	mailSearchJSON    bool

	// Announces flags
	mailAnnouncesJSON bool

	// Clear flags
	mailClearAll bool

	// Archive flags
	mailArchiveStale  bool
	mailArchiveDryRun bool
)

var mailCmd = &cobra.Command{
	Use:         "mail",
	GroupID:     GroupComm,
	Annotations: map[string]string{AnnotationPolecatSafe: "true"},
	Short:       "Agent messaging system",
	RunE:        requireSubcommand,
	Long: `Send and receive messages between agents.

The mail system allows Mayor, polecats, and the Refinery to communicate.
Messages are stored in beads as issues with type=message.

MAIL ROUTING:
  ┌─────────────────────────────────────────────────────┐
  │                    Town (.beads/)                   │
  │  ┌─────────────────────────────────────────────┐   │
  │  │                 Mayor Inbox                 │   │
  │  │  └── mayor/                                 │   │
  │  └─────────────────────────────────────────────┘   │
  │                                                     │
  │  ┌─────────────────────────────────────────────┐   │
  │  │           gastown/ (rig mailboxes)          │   │
  │  │  ├── witness      ← greenplace/witness         │   │
  │  │  ├── refinery     ← greenplace/refinery        │   │
  │  │  ├── Toast        ← greenplace/Toast           │   │
  │  │  └── crew/max     ← greenplace/crew/max        │   │
  │  └─────────────────────────────────────────────┘   │
  └─────────────────────────────────────────────────────┘

ADDRESS FORMATS:
  mayor/              → Mayor inbox
  <rig>/witness       → Rig's Witness
  <rig>/refinery      → Rig's Refinery
  <rig>/<polecat>     → Polecat (e.g., greenplace/Toast)
  <rig>/crew/<name>   → Crew worker (e.g., greenplace/crew/max)
  --human             → Special: human overseer

COMMANDS:
  inbox     View your inbox
  send      Send a message
  read      Read a specific message
  mark      Mark messages read/unread`,
}

var mailSendCmd = &cobra.Command{
	Use:   "send <address>",
	Short: "Send a message",
	Long: `Send a message to an agent.

Addresses:
  mayor/           - Send to Mayor
  <rig>/refinery   - Send to a rig's Refinery
  <rig>/<polecat>  - Send to a specific polecat
  <rig>/           - Broadcast to a rig
  list:<name>      - Send to a mailing list (fans out to all members)

Mailing lists are defined in ~/gt/config/messaging.json and allow
sending to multiple recipients at once. Each recipient gets their
own copy of the message.

Message types:
  task          - Required processing
  scavenge      - Optional first-come work
  notification  - Informational (default)
  reply         - Response to message

Priority levels:
  0 - urgent/critical
  1 - high
  2 - normal (default)
  3 - low
  4 - backlog

Use --urgent as shortcut for --priority 0.

Examples:
  gt mail send greenplace/Toast -s "Status check" -m "How's that bug fix going?"
  gt mail send mayor/ -s "Work complete" -m "Finished gt-abc"
  gt mail send gastown/ -s "All hands" -m "Swarm starting" --notify
  gt mail send greenplace/Toast -s "Task" -m "Fix bug" --type task --priority 1
  gt mail send greenplace/Toast -s "Urgent" -m "Help!" --urgent
  gt mail send mayor/ -s "Re: Status" -m "Done" --reply-to msg-abc123
  gt mail send --self -s "Handoff" -m "Context for next session"
  gt mail send greenplace/Toast -s "Update" -m "Progress report" --cc overseer
  gt mail send list:oncall -s "Alert" -m "System down"

  # Read body from stdin (avoids shell quoting issues):
  gt mail send mayor/ -s "Update" --stdin <<'BODY'
  Message with 'quotes' and "quotes" and $variables.
  BODY`,
	Args: cobra.MaximumNArgs(1),
	RunE: runMailSend,
}

var mailInboxCmd = &cobra.Command{
	Use:   "inbox [address]",
	Short: "Check inbox",
	Long: `Check messages in an inbox.

If no address is specified, shows the current context's inbox.
Use --identity for polecats to explicitly specify their identity.

By default, shows all messages. Use --unread to filter to unread only,
or --all to explicitly show all messages (read and unread).

Examples:
  gt mail inbox                       # Current context (auto-detected)
  gt mail inbox --all                 # Explicitly show all messages
  gt mail inbox --unread              # Show only unread messages
  gt mail inbox mayor/                # Mayor's inbox
  gt mail inbox greenplace/Toast         # Polecat's inbox
  gt mail inbox --identity greenplace/Toast  # Explicit polecat identity`,
	Args: cobra.MaximumNArgs(1),
	RunE: runMailInbox,
}

var mailReadCmd = &cobra.Command{
	Use:   "read <message-id|index>",
	Short: "Read a message",
	Long: `Read a specific message (does not mark as read).

You can specify a message by its ID or by its numeric index from the inbox.
The index corresponds to the number shown in 'gt mail inbox' (1-based).

Examples:
  gt mail read hq-abc123    # Read by message ID
  gt mail read 3            # Read the 3rd message in inbox

Use 'gt mail inbox' to list messages and their IDs.
Use 'gt mail mark-read' to mark messages as read.`,
	Aliases: []string{"show"},
	Args:    cobra.MaximumNArgs(1),
	RunE:    runMailRead,
}

var mailPeekCmd = &cobra.Command{
	Use:   "peek",
	Short: "Show preview of first unread message",
	Long: `Display a compact preview of the first unread message.

Useful for status bar popups - shows subject, sender, and body preview.
Exits silently with code 1 if no unread messages.`,
	RunE: runMailPeek,
}

var mailDeleteCmd = &cobra.Command{
	Use:   "delete <message-id> [message-id...]",
	Short: "Delete messages",
	Long: `Delete (acknowledge) one or more messages.

This closes the messages in beads.

Examples:
  gt mail delete hq-abc123
  gt mail delete hq-abc123 hq-def456 hq-ghi789`,
	Args: cobra.MinimumNArgs(1),
	RunE: runMailDelete,
}

var mailArchiveCmd = &cobra.Command{
	Use:   "archive [message-id...]",
	Short: "Archive messages",
	Long: `Archive one or more messages.

Removes the messages from your inbox by closing them in beads.

Use --stale to archive messages sent before your current session started.

Examples:
	gt mail archive hq-abc123
	gt mail archive hq-abc123 hq-def456 hq-ghi789
	gt mail archive --stale
	gt mail archive --stale --dry-run`,
	Args: func(cmd *cobra.Command, args []string) error {
		return nil
	},
	RunE: runMailArchive,
}

var (
	mailMarkReadAll bool
)

var mailMarkReadCmd = &cobra.Command{
	Use:     "mark-read [message-id...]",
	Aliases: []string{"ack"},
	Short:   "Mark messages as read without archiving",
	Long: `Mark one or more messages as read without removing them from inbox.

This adds a 'read' label to the message, which is reflected in the inbox display.
The message remains in your inbox (unlike archive which closes/removes it).

Use --all to mark all unread messages as read (silences hook re-notifications).

Examples:
  gt mail mark-read hq-abc123
  gt mail mark-read hq-abc123 hq-def456
  gt mail mark-read --all`,
	RunE: runMailMarkRead,
}

var mailMarkUnreadCmd = &cobra.Command{
	Use:   "mark-unread <message-id> [message-id...]",
	Short: "Mark messages as unread",
	Long: `Mark one or more messages as unread.

This removes the 'read' label from the message.

Examples:
  gt mail mark-unread hq-abc123
  gt mail mark-unread hq-abc123 hq-def456`,
	Args: cobra.MinimumNArgs(1),
	RunE: runMailMarkUnread,
}

var mailCheckCmd = &cobra.Command{
	Use:   "check",
	Short: "Check for new mail (for hooks)",
	Long: `Check for new mail - useful for Claude Code hooks.

Exit codes (normal mode):
  0 - New mail available
  1 - No new mail

Exit codes (--inject mode):
  0 - Always (hooks should never block)
  Output: system-reminder if mail exists, silent if no mail

Use --identity for polecats to explicitly specify their identity.

Examples:
  gt mail check                           # Simple check (auto-detect identity)
  gt mail check --inject                  # For hooks
  gt mail check --identity greenplace/Toast  # Explicit polecat identity`,
	RunE: runMailCheck,
}

var mailThreadCmd = &cobra.Command{
	Use:   "thread <thread-id>",
	Short: "View a message thread",
	Long: `View all messages in a conversation thread.

Shows messages in chronological order (oldest first).

Examples:
  gt mail thread thread-abc123`,
	Args: cobra.ExactArgs(1),
	RunE: runMailThread,
}

var mailReplyCmd = &cobra.Command{
	Use:   "reply <message-id> [message]",
	Short: "Reply to a message",
	Long: `Reply to a specific message.

This is a convenience command that automatically:
- Sets the reply-to field to the original message
- Prefixes the subject with "Re: " (if not already present)
- Sends to the original sender

The message body can be provided as a positional argument or via -m flag.

Examples:
  gt mail reply msg-abc123 "Thanks, working on it now"
  gt mail reply msg-abc123 -m "Thanks, working on it now"
  gt mail reply msg-abc123 -s "Custom subject" -m "Reply body"`,
	Args: cobra.RangeArgs(1, 2),
	RunE: runMailReply,
}

var mailClaimCmd = &cobra.Command{
	Use:   "claim [queue-name]",
	Short: "Claim a message from a queue",
	Long: `Claim the oldest unclaimed message from a work queue.

SYNTAX:
  gt mail claim [queue-name]

BEHAVIOR:
1. If queue specified, claim from that queue
2. If no queue specified, claim from any eligible queue
3. Add claimed-by and claimed-at labels to the message
4. Print claimed message details

ELIGIBILITY:
The caller must match the queue's claim_pattern (stored in the queue bead).
Pattern examples: "*" (anyone), "gastown/polecats/*" (specific rig crew).

Examples:
  gt mail claim work-requests   # Claim from specific queue
  gt mail claim                 # Claim from any eligible queue`,
	Args: cobra.MaximumNArgs(1),
	RunE: runMailClaim,
}

var mailReleaseCmd = &cobra.Command{
	Use:   "release <message-id>",
	Short: "Release a claimed queue message",
	Long: `Release a previously claimed message back to its queue.

SYNTAX:
  gt mail release <message-id>

BEHAVIOR:
1. Find the message by ID
2. Verify caller is the one who claimed it (claimed-by label matches)
3. Remove claimed-by and claimed-at labels
4. Message returns to queue for others to claim

ERROR CASES:
- Message not found
- Message is not a queue message
- Message not claimed
- Caller did not claim this message

Examples:
  gt mail release hq-abc123    # Release a claimed message`,
	Args: cobra.ExactArgs(1),
	RunE: runMailRelease,
}

var mailClearCmd = &cobra.Command{
	Use:   "clear [target]",
	Short: "Clear all messages from an inbox",
	Long: `Clear (delete) all messages from an inbox.

SYNTAX:
  gt mail clear              # Clear your own inbox
  gt mail clear <target>     # Clear another agent's inbox

BEHAVIOR:
1. List all messages in the target inbox
2. Delete each message
3. Print count of deleted messages

Use case: Town quiescence - reset all inboxes across workers efficiently.

Examples:
  gt mail clear                      # Clear your inbox
  gt mail clear gastown/polecats/joe # Clear joe's inbox
  gt mail clear mayor/               # Clear mayor's inbox`,
	Args: cobra.MaximumNArgs(1),
	RunE: runMailClear,
}

var mailSearchCmd = &cobra.Command{
	Use:   "search <query>",
	Short: "Search messages by content",
	Long: `Search inbox for messages matching a pattern.

SYNTAX:
  gt mail search <query> [flags]

The query is a regular expression pattern. Search is case-insensitive by default.

FLAGS:
  --from <sender>   Filter by sender address (substring match)
  --subject         Only search subject lines
  --body            Only search message body
  --archive         Include archived (closed) messages
  --json            Output as JSON

By default, searches both subject and body text.

Examples:
  gt mail search "urgent"                    # Find messages with "urgent"
  gt mail search "status.*check" --subject   # Regex in subjects only
  gt mail search "error" --from witness      # From witness, containing "error"
  gt mail search "handoff" --archive         # Include archived messages
  gt mail search "" --from mayor/            # All messages from mayor`,
	Args: cobra.ExactArgs(1),
	RunE: runMailSearch,
}

var mailAnnouncesCmd = &cobra.Command{
	Use:   "announces [channel]",
	Short: "List or read announce channels",
	Long: `List available announce channels or read messages from a channel.

SYNTAX:
  gt mail announces              # List all announce channels
  gt mail announces <channel>    # Read messages from a channel

Announce channels are bulletin boards defined in ~/gt/config/messaging.json.
Messages are broadcast to readers and persist until retention limit is reached.
Unlike regular mail, announce messages are NOT removed when read.

BEHAVIOR for 'gt mail announces':
- Loads messaging.json
- Lists all announce channel names
- Shows reader patterns and retain_count for each

BEHAVIOR for 'gt mail announces <channel>':
- Validates channel exists
- Queries beads for messages with announce_channel=<channel>
- Displays in reverse chronological order (newest first)
- Does NOT mark as read or remove messages

Examples:
  gt mail announces              # List all channels
  gt mail announces alerts       # Read messages from 'alerts' channel
  gt mail announces --json       # List channels as JSON`,
	Args: cobra.MaximumNArgs(1),
	RunE: runMailAnnounces,
}

func init() {
	// Send flags
	mailSendCmd.Flags().StringVarP(&mailSubject, "subject", "s", "", "Message subject (required)")
	mailSendCmd.Flags().StringVarP(&mailBody, "message", "m", "", "Message body")
	mailSendCmd.Flags().StringVar(&mailBody, "body", "", "Alias for --message")
	mailSendCmd.Flags().BoolVar(&mailStdin, "stdin", false, "Read message body from stdin (avoids shell quoting issues)")
	mailSendCmd.Flags().IntVar(&mailPriority, "priority", 2, "Message priority (0=urgent, 1=high, 2=normal, 3=low, 4=backlog)")
	mailSendCmd.Flags().BoolVar(&mailUrgent, "urgent", false, "Set priority=0 (urgent)")
	mailSendCmd.Flags().StringVar(&mailType, "type", "notification", "Message type (task, scavenge, notification, reply)")
	mailSendCmd.Flags().StringVar(&mailReplyTo, "reply-to", "", "Message ID this is replying to")
	mailSendCmd.Flags().BoolVarP(&mailNotify, "notify", "n", false, "Bump priority to high (notification is automatic; use --no-notify to suppress)")
	mailSendCmd.Flags().BoolVar(&mailNoNotify, "no-notify", false, "Suppress auto-nudge notification to recipient")
	mailSendCmd.MarkFlagsMutuallyExclusive("notify", "no-notify")
	mailSendCmd.Flags().BoolVar(&mailPinned, "pinned", false, "Pin message (for handoff context that persists)")
	mailSendCmd.Flags().BoolVar(&mailWisp, "wisp", true, "Send as wisp (ephemeral, default)")
	mailSendCmd.Flags().BoolVar(&mailPermanent, "permanent", false, "Send as permanent (not ephemeral, synced to remote)")
	mailSendCmd.Flags().StringVar(&mailTo, "to", "", "Recipient address (alternative to positional argument)")
	mailSendCmd.Flags().StringVar(&mailFrom, "from", "", "Override sender address (for relay/bridge use)")
	mailSendCmd.Flags().BoolVar(&mailSendSelf, "self", false, "Send to self (auto-detect from cwd)")
	mailSendCmd.Flags().StringArrayVar(&mailCC, "cc", nil, "CC recipients (can be used multiple times)")
	_ = mailSendCmd.MarkFlagRequired("subject") // cobra flags: error only at runtime if missing

	// Inbox flags
	mailInboxCmd.Flags().BoolVar(&mailInboxJSON, "json", false, "Output as JSON")
	mailInboxCmd.Flags().BoolVarP(&mailInboxUnread, "unread", "u", false, "Show only unread messages")
	mailInboxCmd.Flags().BoolVarP(&mailInboxAll, "all", "a", false, "Show all messages (read and unread)")
	mailInboxCmd.Flags().StringVar(&mailInboxIdentity, "identity", "", "Explicit identity for inbox (e.g., greenplace/Toast)")
	mailInboxCmd.Flags().StringVar(&mailInboxIdentity, "address", "", "Alias for --identity")

	// Read flags
	mailReadCmd.Flags().BoolVar(&mailReadJSON, "json", false, "Output as JSON")

	// Check flags
	mailCheckCmd.Flags().BoolVar(&mailCheckInject, "inject", false, "Output format for Claude Code hooks")
	mailCheckCmd.Flags().BoolVar(&mailCheckJSON, "json", false, "Output as JSON")
	mailCheckCmd.Flags().StringVar(&mailCheckIdentity, "identity", "", "Explicit identity for inbox (e.g., greenplace/Toast)")
	mailCheckCmd.Flags().StringVar(&mailCheckIdentity, "address", "", "Alias for --identity")

	// Thread flags
	mailThreadCmd.Flags().BoolVar(&mailThreadJSON, "json", false, "Output as JSON")

	// Reply flags
	mailReplyCmd.Flags().StringVarP(&mailReplySubject, "subject", "s", "", "Override reply subject (default: Re: <original>)")
	mailReplyCmd.Flags().StringVarP(&mailReplyMessage, "message", "m", "", "Reply message body")
	mailReplyCmd.Flags().StringVar(&mailReplyMessage, "body", "", "Reply message body (alias for --message)")

	// Search flags
	mailSearchCmd.Flags().StringVar(&mailSearchFrom, "from", "", "Filter by sender address")
	mailSearchCmd.Flags().BoolVar(&mailSearchSubject, "subject", false, "Only search subject lines")
	mailSearchCmd.Flags().BoolVar(&mailSearchBody, "body", false, "Only search message body")
	mailSearchCmd.Flags().BoolVar(&mailSearchArchive, "archive", false, "Include archived messages")
	mailSearchCmd.Flags().BoolVar(&mailSearchJSON, "json", false, "Output as JSON")

	// Announces flags
	mailAnnouncesCmd.Flags().BoolVar(&mailAnnouncesJSON, "json", false, "Output as JSON")

	// Clear flags
	mailClearCmd.Flags().BoolVar(&mailClearAll, "all", false, "Clear all messages (default behavior)")

	// Archive flags
	mailArchiveCmd.Flags().BoolVar(&mailArchiveStale, "stale", false, "Archive messages sent before session start")
	mailArchiveCmd.Flags().BoolVarP(&mailArchiveDryRun, "dry-run", "n", false, "Show what would be archived without archiving")

	// Add subcommands
	mailCmd.AddCommand(mailSendCmd)
	mailCmd.AddCommand(mailInboxCmd)
	mailCmd.AddCommand(mailReadCmd)
	mailCmd.AddCommand(mailPeekCmd)
	mailCmd.AddCommand(mailDeleteCmd)
	mailCmd.AddCommand(mailArchiveCmd)
	mailMarkReadCmd.Flags().BoolVar(&mailMarkReadAll, "all", false, "Mark all unread messages as read")
	mailCmd.AddCommand(mailMarkReadCmd)
	mailCmd.AddCommand(mailMarkUnreadCmd)
	mailCmd.AddCommand(mailCheckCmd)
	mailCmd.AddCommand(mailThreadCmd)
	mailCmd.AddCommand(mailReplyCmd)
	mailCmd.AddCommand(mailClaimCmd)
	mailCmd.AddCommand(mailReleaseCmd)
	mailCmd.AddCommand(mailClearCmd)
	mailCmd.AddCommand(mailSearchCmd)
	mailCmd.AddCommand(mailAnnouncesCmd)
	mailCmd.AddCommand(mailDrainCmd)

	rootCmd.AddCommand(mailCmd)
}
package cmd

import (
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"os"
	"strings"

	"github.com/spf13/cobra"
	"github.com/steveyegge/gastown/internal/beads"
	"github.com/steveyegge/gastown/internal/events"
	"github.com/steveyegge/gastown/internal/mail"
	"github.com/steveyegge/gastown/internal/style"
	"github.com/steveyegge/gastown/internal/workspace"
)

func runMailSend(cmd *cobra.Command, args []string) error {
	// Handle --stdin: read message body from stdin (avoids shell quoting issues)
	if mailStdin {
		if mailBody != "" {
			return fmt.Errorf("cannot use --stdin with --message/-m")
		}
		data, err := io.ReadAll(os.Stdin)
		if err != nil {
			return fmt.Errorf("reading stdin: %w", err)
		}
		mailBody = strings.TrimRight(string(data), "\n")
	}

	var to string

	if mailSendSelf {
		// Auto-detect identity from cwd
		cwd, err := os.Getwd()
		if err != nil {
			return fmt.Errorf("getting current directory: %w", err)
		}
		townRoot, err := workspace.FindFromCwd()
		if err != nil || townRoot == "" {
			return fmt.Errorf("not in a Gas Town workspace")
		}
		roleInfo, err := GetRoleWithContext(cwd, townRoot)
		if err != nil {
			return fmt.Errorf("detecting role: %w", err)
		}
		ctx := RoleContext{
			Role:     roleInfo.Role,
			Rig:      roleInfo.Rig,
			Polecat:  roleInfo.Polecat,
			TownRoot: townRoot,
			WorkDir:  cwd,
		}
		to = buildAgentIdentity(ctx)
		if to == "" {
			return fmt.Errorf("cannot determine identity (role: %s)", ctx.Role)
		}
	} else if mailTo != "" {
		to = mailTo
	} else if len(args) > 0 {
		to = args[0]
	} else {
		return fmt.Errorf("address required (use positional arg, --to, or --self)")
	}

	// All mail uses town beads (two-level architecture)
	workDir, err := findMailWorkDir()
	if err != nil {
		return fmt.Errorf("not in a Gas Town workspace: %w", err)
	}

	// Determine sender (--from overrides auto-detection, for relay/bridge use)
	from := mailFrom
	if from == "" {
		from = detectSender()
	}

	// Create message with auto-generated ID and thread ID
	msg := mail.NewMessage(from, to, mailSubject, mailBody)

	// Set priority (--urgent overrides --priority)
	if mailUrgent {
		msg.Priority = mail.PriorityUrgent
	} else {
		msg.Priority = mail.PriorityFromInt(mailPriority)
	}
	if mailNotify && msg.Priority == mail.PriorityNormal {
		msg.Priority = mail.PriorityHigh
	}

	// Set message type
	msg.Type = mail.ParseMessageType(mailType)

	// Set pinned flag
	msg.Pinned = mailPinned

	// Set wisp flag (ephemeral message) - default true, --permanent overrides
	msg.Wisp = mailWisp && !mailPermanent

	// Set CC recipients
	msg.CC = mailCC

	// Suppress router-side notification when --no-notify is passed.
	// Otherwise the router handles idle-aware notification per-recipient,
	// which also works correctly for fan-out (groups, lists, channels).
	if mailNoNotify {
		msg.SuppressNotify = true
	}

	// Handle reply-to: auto-set type to reply and look up thread
	if mailReplyTo != "" {
		msg.ReplyTo = mailReplyTo
		if msg.Type == mail.TypeNotification {
			msg.Type = mail.TypeReply
		}

		// Look up original message in current user's mailbox to get thread ID.
		// The message we're replying to lives in our inbox (we received it),
		// so we look it up via our own identity (from), not the recipient (to).
		router := mail.NewRouter(workDir)
		mailbox, err := router.GetMailbox(from)
		if err != nil {
			style.PrintWarning("could not open mailbox for thread lookup: %v", err)
		} else {
			original, err := mailbox.Get(mailReplyTo)
			if err != nil {
				style.PrintWarning("could not find original message %s for threading (new thread will be created)", mailReplyTo)
			} else {
				msg.ThreadID = original.ThreadID
			}
		}
	}

	// Generate thread ID for new threads
	if msg.ThreadID == "" {
		msg.ThreadID = generateThreadID()
	}

	// Use address resolver for new address types
	townRoot, _ := workspace.FindFromCwd()
	b := beads.New(townRoot)
	resolver := mail.NewResolver(b, townRoot)

	recipients, err := resolver.Resolve(to)
	if err != nil {
		// Validation errors are definitive — do not fall back to legacy routing,
		// which would silently deliver to a dead inbox.
		// See: https://github.com/steveyegge/gastown/issues/2038
		if errors.Is(err, mail.ErrUnknownRecipient) {
			return err
		}
		// Fall back to legacy routing for infrastructure errors (beads down, etc.)
		router := mail.NewRouter(workDir)
		defer router.WaitPendingNotifications()
		if err := router.Send(msg); err != nil {
			return fmt.Errorf("sending message: %w", err)
		}
		_ = events.LogFeed(events.TypeMail, from, events.MailPayload(to, mailSubject))
		fmt.Printf("%s Message sent to %s\n", style.Bold.Render("✓"), to)
		fmt.Printf("  Subject: %s\n", mailSubject)
		return nil
	}

	// Route based on recipient type, collecting errors instead of failing early
	router := mail.NewRouter(workDir)
	defer router.WaitPendingNotifications()
	var recipientAddrs []string
	var sendErrs []string

	for _, rec := range recipients {
		switch rec.Type {
		case mail.RecipientQueue:
			// Queue messages: single message, workers claim
			msg.To = rec.Address
			if err := router.Send(msg); err != nil {
				sendErrs = append(sendErrs, fmt.Sprintf("queue %s: %v", rec.Address, err))
				continue
			}
			recipientAddrs = append(recipientAddrs, rec.Address)

		case mail.RecipientChannel:
			// Channel messages: single message, broadcast
			msg.To = rec.Address
			if err := router.Send(msg); err != nil {
				sendErrs = append(sendErrs, fmt.Sprintf("channel %s: %v", rec.Address, err))
				continue
			}
			recipientAddrs = append(recipientAddrs, rec.Address)

		default:
			// Direct/agent messages: fan out to each recipient
			msgCopy := *msg
			msgCopy.To = rec.Address
			msgCopy.ID = "" // Each fan-out copy gets its own unique ID
			if err := router.Send(&msgCopy); err != nil {
				sendErrs = append(sendErrs, fmt.Sprintf("%s: %v", rec.Address, err))
				continue
			}
			recipientAddrs = append(recipientAddrs, rec.Address)
		}
	}

	if len(sendErrs) > 0 {
		if len(recipientAddrs) == 0 {
			return fmt.Errorf("all sends failed: %s", strings.Join(sendErrs, "; "))
		}
		fmt.Fprintf(os.Stderr, "⚠ Some deliveries failed: %s\n", strings.Join(sendErrs, "; "))
	}

	// Log mail event to activity feed
	_ = events.LogFeed(events.TypeMail, from, events.MailPayload(to, mailSubject))

	fmt.Printf("%s Message sent to %s\n", style.Bold.Render("✓"), to)
	fmt.Printf("  Subject: %s\n", mailSubject)

	// Show resolved recipients if fan-out occurred
	if len(recipientAddrs) > 1 || (len(recipientAddrs) == 1 && recipientAddrs[0] != to) {
		fmt.Printf("  Recipients: %s\n", strings.Join(recipientAddrs, ", "))
	}

	if len(msg.CC) > 0 {
		fmt.Printf("  CC: %s\n", strings.Join(msg.CC, ", "))
	}
	if msg.Type != mail.TypeNotification {
		fmt.Printf("  Type: %s\n", msg.Type)
	}

	return nil
}

// generateThreadID creates a random thread ID for new message threads.
func generateThreadID() string {
	b := make([]byte, 6)
	_, _ = rand.Read(b) // crypto/rand.Read only fails on broken system
	return "thread-" + hex.EncodeToString(b)
}package cmd

import (
	"github.com/spf13/cobra"
)

// Flags for mail hook command (mirror of hook command flags)
var (
	mailHookSubject string
	mailHookMessage string
	mailHookDryRun  bool
	mailHookForce   bool
)

var mailHookCmd = &cobra.Command{
	Use:   "hook <mail-id>",
	Short: "Attach mail to your hook (alias for 'gt hook attach')",
	Long: `Attach a mail message to your hook.

This is an alias for 'gt hook attach <mail-id>'. It attaches the specified
mail message to your hook so you can work on it.

The hook is the "durability primitive" - work on your hook survives session
restarts, context compaction, and handoffs.

Examples:
  gt mail hook msg-abc123                    # Attach mail to your hook
  gt mail hook msg-abc123 -s "Fix the bug"   # With subject for handoff
  gt mail hook msg-abc123 --force            # Replace existing incomplete work

Related commands:
  gt hook <bead>     # Attach any bead to your hook
  gt hook status     # Show what's on your hook
  gt unsling         # Remove work from hook`,
	Args: cobra.ExactArgs(1),
	RunE: runMailHook,
}

func init() {
	mailHookCmd.Flags().StringVarP(&mailHookSubject, "subject", "s", "", "Subject for handoff mail (optional)")
	mailHookCmd.Flags().StringVarP(&mailHookMessage, "message", "m", "", "Message for handoff mail (optional)")
	mailHookCmd.Flags().BoolVarP(&mailHookDryRun, "dry-run", "n", false, "Show what would be done")
	mailHookCmd.Flags().BoolVarP(&mailHookForce, "force", "f", false, "Replace existing incomplete hooked bead")

	mailCmd.AddCommand(mailHookCmd)
}

// runMailHook attaches mail to the hook - delegates to the hook command's logic
func runMailHook(cmd *cobra.Command, args []string) error {
	// Copy flags to hook command's globals (they share the same functionality)
	hookSubject = mailHookSubject
	hookMessage = mailHookMessage
	hookDryRun = mailHookDryRun
	hookForce = mailHookForce

	// Delegate to the hook command's run function
	return runHook(cmd, args)
}

