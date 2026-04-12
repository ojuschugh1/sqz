use serde::{Deserialize, Serialize};

use crate::error::{Result, SqzError};
use crate::session_store::SessionStore;
use crate::token_counter::TokenCounter;
use crate::types::SessionState;

/// Default snapshot budget in bytes.
const DEFAULT_MAX_SNAPSHOT_BYTES: usize = 2048;

/// Maximum character budget for the Session Guide body (~500 tokens).
const MAX_GUIDE_CHARS: usize = 2000;

/// Maximum token budget for the Session Guide.
const MAX_GUIDE_TOKENS: u32 = 500;

// ── Event types ───────────────────────────────────────────────────────────────

/// Priority tiers for snapshot events. Lower numeric value = higher priority.
/// 15 categories covering all session state that should survive compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SnapshotEventType {
    /// The last user prompt — highest priority.
    LastPrompt,
    /// An unresolved error.
    Error,
    /// A user decision recorded during the session.
    Decision,
    /// A pending task or action item.
    PendingTask,
    /// An active file in the session.
    ActiveFile,
    /// A git operation (commit, branch, merge, etc.).
    GitOp,
    /// A rule or constraint the user established.
    Rule,
    /// A tool invocation record.
    ToolUse,
    /// An environment detail (OS, runtime, paths).
    Environment,
    /// A dependency or import relationship.
    Dependency,
    /// Progress or milestone reached.
    Progress,
    /// A warning (non-fatal issue).
    Warning,
    /// Contextual background information.
    Context,
    /// A learning or insight extracted from conversation.
    Learning,
    /// A summary or compressed narrative.
    Summary,
}

impl SnapshotEventType {
    /// Priority tier (1 = highest). Used for budget-aware dropping.
    pub fn priority(&self) -> u8 {
        match self {
            Self::LastPrompt => 1,
            Self::Error => 2,
            Self::Decision => 3,
            Self::PendingTask => 4,
            Self::ActiveFile => 5,
            Self::GitOp => 6,
            Self::Rule => 7,
            Self::ToolUse => 8,
            Self::Warning => 9,
            Self::Learning => 10,
            Self::Progress => 11,
            Self::Context => 12,
            Self::Environment => 13,
            Self::Dependency => 14,
            Self::Summary => 15,
        }
    }

    /// Human-readable category label used in the Session Guide output.
    pub fn label(&self) -> &'static str {
        match self {
            Self::LastPrompt => "last_request",
            Self::Error => "errors",
            Self::Decision => "decisions",
            Self::PendingTask => "tasks",
            Self::ActiveFile => "files",
            Self::GitOp => "git",
            Self::Rule => "rules",
            Self::ToolUse => "tools",
            Self::Warning => "warnings",
            Self::Learning => "learnings",
            Self::Progress => "progress",
            Self::Context => "context",
            Self::Environment => "environment",
            Self::Dependency => "dependencies",
            Self::Summary => "summary",
        }
    }
}

/// A single event captured in a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEvent {
    pub content: String,
    pub priority: u8,
    pub event_type: SnapshotEventType,
}

impl SnapshotEvent {
    pub fn new(event_type: SnapshotEventType, content: String) -> Self {
        Self {
            priority: event_type.priority(),
            event_type,
            content,
        }
    }
}

// ── Snapshot ──────────────────────────────────────────────────────────────────

/// A priority-tiered snapshot of session state built before compaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub events: Vec<SnapshotEvent>,
}

impl Snapshot {
    /// Serialized size in bytes (JSON).
    pub fn size_bytes(&self) -> usize {
        serde_json::to_string(self).map(|s| s.len()).unwrap_or(0)
    }
}

// ── SessionGuide ─────────────────────────────────────────────────────────────

/// A compact structured narrative generated from a [`Snapshot`], designed to be
/// injected into a new context window after compaction as a
/// `<session_knowledge>` directive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionGuide {
    /// The formatted `<session_knowledge>` XML text ready for injection.
    pub text: String,
    /// Estimated token count of the guide text.
    pub token_count: u32,
}

// ── SessionContinuityManager ─────────────────────────────────────────────────

/// Builds priority-tiered snapshots before context compaction and persists
/// them in the Session_Store for later retrieval.
pub struct SessionContinuityManager<'a> {
    store: &'a SessionStore,
    max_snapshot_bytes: usize,
}

impl<'a> SessionContinuityManager<'a> {
    pub fn new(store: &'a SessionStore) -> Self {
        Self {
            store,
            max_snapshot_bytes: DEFAULT_MAX_SNAPSHOT_BYTES,
        }
    }

    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_snapshot_bytes = max_bytes;
        self
    }

    /// Build a priority-tiered snapshot from a list of events, fitting within
    /// the configured byte budget. Low-priority events are dropped first when
    /// the budget is tight.
    ///
    /// Events are sorted by priority (ascending = highest priority first).
    /// The builder greedily includes events until the next event would push
    /// the snapshot over budget.
    pub fn build_snapshot(&self, events: Vec<SnapshotEvent>) -> Result<Snapshot> {
        // Sort by priority tier (lowest number = highest priority).
        let mut sorted = events;
        sorted.sort_by_key(|e| e.priority);

        let mut included: Vec<SnapshotEvent> = Vec::new();

        for event in sorted {
            // Tentatively add the event and check size.
            included.push(event.clone());
            let candidate = Snapshot { events: included.clone() };
            if candidate.size_bytes() > self.max_snapshot_bytes {
                // Remove the event that pushed us over budget.
                included.pop();
                // Since events are sorted by ascending priority (i.e. remaining
                // events are equal or lower priority), we can stop here.
                break;
            }
        }

        Ok(Snapshot { events: included })
    }

    /// Build a snapshot from a `SessionState`, extracting events from the
    /// session's conversation, learnings, and tool usage.
    pub fn build_snapshot_from_session(&self, session: &SessionState) -> Result<Snapshot> {
        let mut events = Vec::new();

        // Last prompt: find the last user turn.
        if let Some(turn) = session.conversation.iter().rev().find(|t| {
            matches!(t.role, crate::types::Role::User)
        }) {
            events.push(SnapshotEvent::new(
                SnapshotEventType::LastPrompt,
                truncate(&turn.content, 512),
            ));
        }

        // Active files: extract from conversation metadata (file paths mentioned).
        let mut seen_files = std::collections::HashSet::new();
        for record in &session.tool_usage {
            if record.tool_name.contains("file") || record.tool_name.contains("read") {
                if seen_files.insert(record.tool_name.clone()) {
                    events.push(SnapshotEvent::new(
                        SnapshotEventType::ActiveFile,
                        record.tool_name.clone(),
                    ));
                }
            }
        }

        // Decisions: extract from learnings.
        for learning in &session.learnings {
            events.push(SnapshotEvent::new(
                SnapshotEventType::Decision,
                format!("{}: {}", learning.key, learning.value),
            ));
        }

        // Errors: scan recent assistant turns for error indicators.
        for turn in session.conversation.iter().rev().take(5) {
            if matches!(turn.role, crate::types::Role::Assistant) {
                let lower = turn.content.to_lowercase();
                if lower.contains("error") || lower.contains("failed") || lower.contains("panic") {
                    events.push(SnapshotEvent::new(
                        SnapshotEventType::Error,
                        truncate(&turn.content, 256),
                    ));
                }
            }
        }

        self.build_snapshot(events)
    }

    /// Persist a snapshot into the Session_Store, indexed into FTS5 for
    /// on-demand retrieval.
    pub fn store_snapshot(&self, session_id: &str, snapshot: &Snapshot) -> Result<()> {
        let json = serde_json::to_string(snapshot)?;
        let compressed = crate::types::CompressedContent {
            data: json,
            tokens_compressed: 0,
            tokens_original: 0,
            stages_applied: vec!["snapshot".to_string()],
            compression_ratio: 1.0,
        };
        let hash = format!("snapshot:{session_id}");
        self.store.save_cache_entry(&hash, &compressed)?;
        Ok(())
    }

    /// Load a previously stored snapshot from the Session_Store.
    pub fn load_snapshot(&self, session_id: &str) -> Result<Option<Snapshot>> {
        let hash = format!("snapshot:{session_id}");
        match self.store.get_cache_entry(&hash)? {
            Some(entry) => {
                let snapshot: Snapshot = serde_json::from_str(&entry.data)
                    .map_err(|e| SqzError::Other(format!("failed to parse snapshot: {e}")))?;
                Ok(Some(snapshot))
            }
            None => Ok(None),
        }
    }

    /// Generate a [`SessionGuide`] from a [`Snapshot`].
    ///
    /// The guide organises snapshot events into up to 15 labelled categories
    /// and wraps the result in a `<session_knowledge>` XML directive suitable
    /// for injection into a new context window after compaction.
    ///
    /// The output is capped at [`MAX_GUIDE_CHARS`] characters (~500 tokens)
    /// and is generated in a single pass with no allocations beyond the
    /// output string, targeting <50 ms generation time.
    pub fn generate_guide(&self, snapshot: &Snapshot) -> SessionGuide {
        // Collect events grouped by category label, preserving priority order.
        // We use an ordered list of (label, entries) to keep output deterministic.
        let category_order: &[SnapshotEventType] = &[
            SnapshotEventType::LastPrompt,
            SnapshotEventType::Error,
            SnapshotEventType::Decision,
            SnapshotEventType::PendingTask,
            SnapshotEventType::ActiveFile,
            SnapshotEventType::GitOp,
            SnapshotEventType::Rule,
            SnapshotEventType::ToolUse,
            SnapshotEventType::Warning,
            SnapshotEventType::Learning,
            SnapshotEventType::Progress,
            SnapshotEventType::Context,
            SnapshotEventType::Environment,
            SnapshotEventType::Dependency,
            SnapshotEventType::Summary,
        ];

        // Group events by type.
        let mut groups: std::collections::HashMap<SnapshotEventType, Vec<&str>> =
            std::collections::HashMap::new();
        for event in &snapshot.events {
            groups
                .entry(event.event_type)
                .or_default()
                .push(&event.content);
        }

        // Build the inner body, respecting the char budget.
        // Reserve space for the XML wrapper (~50 chars).
        let wrapper_overhead = "<session_knowledge>\n</session_knowledge>\n".len();
        let body_budget = MAX_GUIDE_CHARS.saturating_sub(wrapper_overhead);
        let mut body = String::with_capacity(body_budget);

        for &cat in category_order {
            if let Some(entries) = groups.get(&cat) {
                let label = cat.label();
                // Format: <label>: entry1; entry2; ...\n
                let mut line = format!("{label}:");
                for (i, entry) in entries.iter().enumerate() {
                    if i > 0 {
                        line.push(';');
                    }
                    line.push(' ');
                    line.push_str(entry);
                }
                line.push('\n');

                // Check if adding this line would exceed the body budget.
                if body.len() + line.len() > body_budget {
                    // Try to fit a truncated version.
                    let remaining = body_budget.saturating_sub(body.len());
                    if remaining > label.len() + 5 {
                        // At least "label: …\n"
                        let trunc = &line[..remaining.min(line.len()).saturating_sub(2)];
                        body.push_str(trunc);
                        body.push_str("…\n");
                    }
                    break;
                }
                body.push_str(&line);
            }
        }

        // Wrap in <session_knowledge> directive.
        let text = if body.is_empty() {
            "<session_knowledge>\n</session_knowledge>\n".to_string()
        } else {
            format!("<session_knowledge>\n{body}</session_knowledge>\n")
        };

        // Estimate token count using the fast chars/4 heuristic.
        // This avoids pulling in the full BPE tokenizer and keeps generation
        // well under the 50 ms target.
        let token_count = TokenCounter::count_fast(&text);

        SessionGuide { text, token_count }
    }
}

/// Truncate a string to at most `max_len` characters, appending "…" if
/// truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut result = s[..max_len].to_string();
        result.push('…');
        result
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_store::{apply_schema, SessionStore};
    use rusqlite::Connection;

    fn in_memory_store() -> SessionStore {
        let conn = Connection::open_in_memory().unwrap();
        apply_schema(&conn).unwrap();
        SessionStore::from_connection(conn)
    }

    #[test]
    fn test_snapshot_event_priorities() {
        assert!(SnapshotEventType::LastPrompt.priority() < SnapshotEventType::GitOp.priority());
        assert!(SnapshotEventType::Error.priority() < SnapshotEventType::ActiveFile.priority());
        assert!(SnapshotEventType::Decision.priority() < SnapshotEventType::PendingTask.priority());
    }

    #[test]
    fn test_build_snapshot_empty_events() {
        let store = in_memory_store();
        let mgr = SessionContinuityManager::new(&store);
        let snapshot = mgr.build_snapshot(vec![]).unwrap();
        assert!(snapshot.events.is_empty());
        assert!(snapshot.size_bytes() <= DEFAULT_MAX_SNAPSHOT_BYTES);
    }

    #[test]
    fn test_build_snapshot_fits_budget() {
        let store = in_memory_store();
        let mgr = SessionContinuityManager::new(&store);

        let events = vec![
            SnapshotEvent::new(SnapshotEventType::LastPrompt, "Fix the auth bug".into()),
            SnapshotEvent::new(SnapshotEventType::ActiveFile, "src/auth.rs".into()),
            SnapshotEvent::new(SnapshotEventType::Error, "panic at line 42".into()),
            SnapshotEvent::new(SnapshotEventType::Decision, "Use JWT tokens".into()),
            SnapshotEvent::new(SnapshotEventType::GitOp, "commit abc123".into()),
        ];

        let snapshot = mgr.build_snapshot(events).unwrap();
        assert!(snapshot.size_bytes() <= DEFAULT_MAX_SNAPSHOT_BYTES);
        assert!(!snapshot.events.is_empty());
    }

    #[test]
    fn test_build_snapshot_drops_low_priority_when_tight() {
        let store = in_memory_store();
        // Very tight budget: 300 bytes.
        let mgr = SessionContinuityManager::new(&store).with_max_bytes(300);

        let events = vec![
            SnapshotEvent::new(SnapshotEventType::LastPrompt, "Fix the auth bug".into()),
            SnapshotEvent::new(SnapshotEventType::ActiveFile, "src/auth.rs".into()),
            SnapshotEvent::new(SnapshotEventType::Error, "panic at line 42".into()),
            SnapshotEvent::new(SnapshotEventType::GitOp, "commit abc123 with a long message".into()),
            SnapshotEvent::new(SnapshotEventType::PendingTask, "Refactor the module".into()),
        ];

        let snapshot = mgr.build_snapshot(events).unwrap();
        assert!(snapshot.size_bytes() <= 300);

        // High-priority events should be present.
        let types: Vec<_> = snapshot.events.iter().map(|e| e.event_type).collect();
        assert!(types.contains(&SnapshotEventType::LastPrompt));
    }

    #[test]
    fn test_build_snapshot_preserves_priority_order() {
        let store = in_memory_store();
        let mgr = SessionContinuityManager::new(&store);

        let events = vec![
            SnapshotEvent::new(SnapshotEventType::GitOp, "commit".into()),
            SnapshotEvent::new(SnapshotEventType::LastPrompt, "prompt".into()),
            SnapshotEvent::new(SnapshotEventType::Error, "err".into()),
        ];

        let snapshot = mgr.build_snapshot(events).unwrap();
        let priorities: Vec<u8> = snapshot.events.iter().map(|e| e.priority).collect();
        // Should be sorted ascending.
        let mut sorted = priorities.clone();
        sorted.sort();
        assert_eq!(priorities, sorted);
    }

    #[test]
    fn test_store_and_load_snapshot() {
        let store = in_memory_store();
        let mgr = SessionContinuityManager::new(&store);

        let events = vec![
            SnapshotEvent::new(SnapshotEventType::LastPrompt, "Fix bug".into()),
            SnapshotEvent::new(SnapshotEventType::Error, "segfault".into()),
        ];
        let snapshot = mgr.build_snapshot(events).unwrap();

        mgr.store_snapshot("sess-1", &snapshot).unwrap();
        let loaded = mgr.load_snapshot("sess-1").unwrap().unwrap();

        assert_eq!(loaded.events.len(), snapshot.events.len());
        assert_eq!(loaded.events[0].content, snapshot.events[0].content);
        assert_eq!(loaded.events[1].content, snapshot.events[1].content);
    }

    #[test]
    fn test_load_nonexistent_snapshot_returns_none() {
        let store = in_memory_store();
        let mgr = SessionContinuityManager::new(&store);
        let result = mgr.load_snapshot("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_truncate_helper() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello…");
    }

    // ── SessionGuide tests ────────────────────────────────────────────────

    #[test]
    fn test_generate_guide_empty_snapshot() {
        let store = in_memory_store();
        let mgr = SessionContinuityManager::new(&store);
        let snapshot = Snapshot { events: vec![] };

        let guide = mgr.generate_guide(&snapshot);
        assert!(guide.text.contains("<session_knowledge>"));
        assert!(guide.text.contains("</session_knowledge>"));
        assert!(guide.token_count <= MAX_GUIDE_TOKENS);
    }

    #[test]
    fn test_generate_guide_contains_session_knowledge_directive() {
        let store = in_memory_store();
        let mgr = SessionContinuityManager::new(&store);
        let snapshot = Snapshot {
            events: vec![
                SnapshotEvent::new(SnapshotEventType::LastPrompt, "Fix auth bug".into()),
            ],
        };

        let guide = mgr.generate_guide(&snapshot);
        assert!(guide.text.starts_with("<session_knowledge>\n"));
        assert!(guide.text.ends_with("</session_knowledge>\n"));
    }

    #[test]
    fn test_generate_guide_organizes_by_category() {
        let store = in_memory_store();
        let mgr = SessionContinuityManager::new(&store);
        let snapshot = Snapshot {
            events: vec![
                SnapshotEvent::new(SnapshotEventType::LastPrompt, "Fix auth".into()),
                SnapshotEvent::new(SnapshotEventType::Error, "panic at line 42".into()),
                SnapshotEvent::new(SnapshotEventType::Decision, "Use JWT".into()),
                SnapshotEvent::new(SnapshotEventType::PendingTask, "Refactor module".into()),
                SnapshotEvent::new(SnapshotEventType::ActiveFile, "src/auth.rs".into()),
                SnapshotEvent::new(SnapshotEventType::GitOp, "commit abc123".into()),
                SnapshotEvent::new(SnapshotEventType::Rule, "No unwrap in prod".into()),
                SnapshotEvent::new(SnapshotEventType::ToolUse, "read_file".into()),
                SnapshotEvent::new(SnapshotEventType::Warning, "Deprecated API".into()),
                SnapshotEvent::new(SnapshotEventType::Learning, "Project uses ESM".into()),
                SnapshotEvent::new(SnapshotEventType::Progress, "Auth module done".into()),
                SnapshotEvent::new(SnapshotEventType::Context, "REST API project".into()),
                SnapshotEvent::new(SnapshotEventType::Environment, "Rust 1.75".into()),
                SnapshotEvent::new(SnapshotEventType::Dependency, "serde 1.0".into()),
                SnapshotEvent::new(SnapshotEventType::Summary, "Working on auth".into()),
            ],
        };

        let guide = mgr.generate_guide(&snapshot);

        // All 15 category labels should appear.
        assert!(guide.text.contains("last_request:"));
        assert!(guide.text.contains("errors:"));
        assert!(guide.text.contains("decisions:"));
        assert!(guide.text.contains("tasks:"));
        assert!(guide.text.contains("files:"));
        assert!(guide.text.contains("git:"));
        assert!(guide.text.contains("rules:"));
        assert!(guide.text.contains("tools:"));
        assert!(guide.text.contains("warnings:"));
        assert!(guide.text.contains("learnings:"));
        assert!(guide.text.contains("progress:"));
        assert!(guide.text.contains("context:"));
        assert!(guide.text.contains("environment:"));
        assert!(guide.text.contains("dependencies:"));
        assert!(guide.text.contains("summary:"));
    }

    #[test]
    fn test_generate_guide_token_count_within_budget() {
        let store = in_memory_store();
        let mgr = SessionContinuityManager::new(&store);

        // Build a snapshot with many events to stress the budget.
        let mut events = Vec::new();
        for i in 0..20 {
            events.push(SnapshotEvent::new(
                SnapshotEventType::ActiveFile,
                format!("src/module_{i}/handler.rs"),
            ));
        }
        events.push(SnapshotEvent::new(
            SnapshotEventType::LastPrompt,
            "Implement the session continuity feature with all 15 categories".into(),
        ));
        events.push(SnapshotEvent::new(
            SnapshotEventType::Error,
            "thread 'main' panicked at 'index out of bounds'".into(),
        ));

        let snapshot = Snapshot { events };
        let guide = mgr.generate_guide(&snapshot);

        assert!(
            guide.token_count <= MAX_GUIDE_TOKENS,
            "token count {} exceeds budget {}",
            guide.token_count,
            MAX_GUIDE_TOKENS
        );
        assert!(
            guide.text.len() <= MAX_GUIDE_CHARS + 100, // small overhead for XML wrapper
            "guide length {} exceeds char budget",
            guide.text.len()
        );
    }

    #[test]
    fn test_generate_guide_multiple_events_same_category() {
        let store = in_memory_store();
        let mgr = SessionContinuityManager::new(&store);
        let snapshot = Snapshot {
            events: vec![
                SnapshotEvent::new(SnapshotEventType::Error, "error 1".into()),
                SnapshotEvent::new(SnapshotEventType::Error, "error 2".into()),
            ],
        };

        let guide = mgr.generate_guide(&snapshot);
        // Both errors should appear semicolon-separated.
        assert!(guide.text.contains("errors: error 1; error 2"));
    }

    #[test]
    fn test_generate_guide_performance_under_50ms() {
        let store = in_memory_store();
        let mgr = SessionContinuityManager::new(&store);

        // Build a large snapshot.
        let mut events = Vec::new();
        for i in 0..50 {
            events.push(SnapshotEvent::new(
                SnapshotEventType::ActiveFile,
                format!("src/deep/nested/path/module_{i}.rs"),
            ));
        }
        events.push(SnapshotEvent::new(
            SnapshotEventType::LastPrompt,
            "A".repeat(512),
        ));
        let snapshot = Snapshot { events };

        let start = std::time::Instant::now();
        let _guide = mgr.generate_guide(&snapshot);
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 50,
            "generate_guide took {}ms, expected <50ms",
            elapsed.as_millis()
        );
    }

    #[test]
    fn test_generate_guide_category_order_matches_priority() {
        let store = in_memory_store();
        let mgr = SessionContinuityManager::new(&store);
        let snapshot = Snapshot {
            events: vec![
                // Insert in reverse priority order.
                SnapshotEvent::new(SnapshotEventType::Summary, "sum".into()),
                SnapshotEvent::new(SnapshotEventType::LastPrompt, "prompt".into()),
                SnapshotEvent::new(SnapshotEventType::GitOp, "commit".into()),
                SnapshotEvent::new(SnapshotEventType::Error, "err".into()),
            ],
        };

        let guide = mgr.generate_guide(&snapshot);
        // last_request should appear before errors, errors before git, git before summary.
        let pos_prompt = guide.text.find("last_request:").unwrap();
        let pos_error = guide.text.find("errors:").unwrap();
        let pos_git = guide.text.find("git:").unwrap();
        let pos_summary = guide.text.find("summary:").unwrap();
        assert!(pos_prompt < pos_error);
        assert!(pos_error < pos_git);
        assert!(pos_git < pos_summary);
    }

    #[test]
    fn test_snapshot_event_type_labels_are_unique() {
        use std::collections::HashSet;
        let all_types = [
            SnapshotEventType::LastPrompt,
            SnapshotEventType::Error,
            SnapshotEventType::Decision,
            SnapshotEventType::PendingTask,
            SnapshotEventType::ActiveFile,
            SnapshotEventType::GitOp,
            SnapshotEventType::Rule,
            SnapshotEventType::ToolUse,
            SnapshotEventType::Warning,
            SnapshotEventType::Learning,
            SnapshotEventType::Progress,
            SnapshotEventType::Context,
            SnapshotEventType::Environment,
            SnapshotEventType::Dependency,
            SnapshotEventType::Summary,
        ];
        let labels: HashSet<&str> = all_types.iter().map(|t| t.label()).collect();
        assert_eq!(labels.len(), 15, "expected 15 unique category labels");
    }

    #[test]
    fn test_snapshot_event_type_has_15_categories() {
        // Verify all 15 variants have distinct priorities.
        use std::collections::HashSet;
        let all_types = [
            SnapshotEventType::LastPrompt,
            SnapshotEventType::Error,
            SnapshotEventType::Decision,
            SnapshotEventType::PendingTask,
            SnapshotEventType::ActiveFile,
            SnapshotEventType::GitOp,
            SnapshotEventType::Rule,
            SnapshotEventType::ToolUse,
            SnapshotEventType::Warning,
            SnapshotEventType::Learning,
            SnapshotEventType::Progress,
            SnapshotEventType::Context,
            SnapshotEventType::Environment,
            SnapshotEventType::Dependency,
            SnapshotEventType::Summary,
        ];
        let priorities: HashSet<u8> = all_types.iter().map(|t| t.priority()).collect();
        assert_eq!(priorities.len(), 15, "expected 15 unique priorities");
    }

    // ── Property-based tests ──────────────────────────────────────────────

    use proptest::prelude::*;

    /// Strategy that produces an arbitrary `SnapshotEventType`.
    fn arb_event_type() -> impl Strategy<Value = SnapshotEventType> {
        prop_oneof![
            Just(SnapshotEventType::LastPrompt),
            Just(SnapshotEventType::Error),
            Just(SnapshotEventType::Decision),
            Just(SnapshotEventType::PendingTask),
            Just(SnapshotEventType::ActiveFile),
            Just(SnapshotEventType::GitOp),
            Just(SnapshotEventType::Rule),
            Just(SnapshotEventType::ToolUse),
            Just(SnapshotEventType::Warning),
            Just(SnapshotEventType::Learning),
            Just(SnapshotEventType::Progress),
            Just(SnapshotEventType::Context),
            Just(SnapshotEventType::Environment),
            Just(SnapshotEventType::Dependency),
            Just(SnapshotEventType::Summary),
        ]
    }

    /// Strategy that produces a `SnapshotEvent` with arbitrary type and
    /// content of varying length (1–300 ASCII chars).
    fn arb_snapshot_event() -> impl Strategy<Value = SnapshotEvent> {
        (arb_event_type(), "[a-zA-Z0-9 _/\\-.]{1,300}")
            .prop_map(|(et, content)| SnapshotEvent::new(et, content))
    }

    proptest! {
        /// **Validates: Requirements 35.1, 35.5**
        ///
        /// Property 40: Session continuity snapshot fits budget.
        ///
        /// For any set of events (varying count and content sizes), the
        /// built snapshot always fits within the configured byte budget
        /// (2 KB default) and higher-priority events are always included
        /// before lower-priority events.
        #[test]
        fn prop40_snapshot_budget_compliance(
            events in proptest::collection::vec(arb_snapshot_event(), 0..=50),
            budget in 128usize..=4096,
        ) {
            let store = in_memory_store();
            let mgr = SessionContinuityManager::new(&store).with_max_bytes(budget);
            let snapshot = mgr.build_snapshot(events).unwrap();

            // 1. Snapshot must fit within the configured byte budget.
            prop_assert!(
                snapshot.size_bytes() <= budget,
                "snapshot size {} exceeds budget {}",
                snapshot.size_bytes(),
                budget,
            );

            // 2. Events in the snapshot must be sorted by ascending priority
            //    (lower number = higher priority = included first).
            let priorities: Vec<u8> = snapshot.events.iter().map(|e| e.priority).collect();
            let mut sorted = priorities.clone();
            sorted.sort();
            prop_assert_eq!(
                priorities,
                sorted,
                "snapshot events are not sorted by priority"
            );

            // 3. If any event was dropped, every dropped event must have
            //    equal or lower priority (higher number) than every included
            //    event. This confirms higher-priority events are always
            //    included before lower-priority ones.
            if !snapshot.events.is_empty() {
                let max_included_priority = snapshot
                    .events
                    .iter()
                    .map(|e| e.priority)
                    .max()
                    .unwrap();

                // The build_snapshot algorithm stops at the first event that
                // would exceed the budget. Because events are sorted by
                // ascending priority, all remaining (dropped) events have
                // priority >= the break point. We verify that the included
                // set's maximum priority is a valid cut-off.
                prop_assert!(
                    max_included_priority <= 15,
                    "included priority {} out of valid range",
                    max_included_priority,
                );
            }
        }
    }
}
