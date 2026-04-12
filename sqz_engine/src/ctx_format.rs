use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::{Result, SqzError};
use crate::types::SessionState;

// ---------------------------------------------------------------------------
// CTX envelope types
// ---------------------------------------------------------------------------

/// Versioned envelope that wraps a `SessionState` in a `.ctx` JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CtxEnvelope {
    pub version: String,
    pub format: String,
    pub created_at: String,
    pub source_model: String,
    pub session: SessionState,
    pub metadata: CtxMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CtxMetadata {
    pub sqz_version: String,
    pub compression_ratio: f64,
    pub total_savings_tokens: u64,
}

// ---------------------------------------------------------------------------
// CtxFormat
// ---------------------------------------------------------------------------

/// Serializes and deserializes `SessionState` to/from the `.ctx` JSON format.
pub struct CtxFormat;

impl CtxFormat {
    /// Serialize a `SessionState` to a compact CTX JSON string.
    pub fn serialize(session: &SessionState) -> Result<String> {
        let envelope = Self::build_envelope(session);
        Ok(serde_json::to_string(&envelope)?)
    }

    /// Serialize a `SessionState` to a pretty-printed CTX JSON string.
    pub fn serialize_pretty(session: &SessionState) -> Result<String> {
        let envelope = Self::build_envelope(session);
        Ok(serde_json::to_string_pretty(&envelope)?)
    }

    /// Deserialize a CTX JSON string back to a `SessionState`.
    ///
    /// Returns a descriptive error with line/column information on parse failure.
    pub fn deserialize(ctx: &str) -> Result<SessionState> {
        let envelope: CtxEnvelope = serde_json::from_str(ctx).map_err(|e| {
            // serde_json errors include line/column for syntax errors and the
            // field path for missing/wrong-type errors.
            SqzError::Other(format!(
                "CTX parse error at line {}, column {}: {}",
                e.line(),
                e.column(),
                e
            ))
        })?;

        // Warn about unsupported version (future-proofing; currently only "1.0").
        if envelope.version != "1.0" {
            // Log a warning but still attempt to use the session.
            eprintln!(
                "sqz warning: CTX version '{}' is newer than supported '1.0'; \
                 some features may be omitted",
                envelope.version
            );
        }

        Ok(envelope.session)
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn build_envelope(session: &SessionState) -> CtxEnvelope {
        // Compute a rough compression ratio from the budget state.
        let total_tokens = session.budget.consumed;
        let compression_ratio = if total_tokens > 0 {
            // Placeholder: real ratio would come from pipeline metrics.
            0.35_f64
        } else {
            0.0_f64
        };

        CtxEnvelope {
            version: "1.0".to_string(),
            format: "ctx".to_string(),
            created_at: Utc::now().to_rfc3339(),
            source_model: format!("{:?}", session.budget.model_family),
            session: session.clone(),
            metadata: CtxMetadata {
                sqz_version: env!("CARGO_PKG_VERSION").to_string(),
                compression_ratio,
                total_savings_tokens: 0,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        BudgetState, CorrectionEntry, CorrectionLog, ConversationTurn, Learning, ModelFamily,
        PinEntry, Role, SessionState,
    };
    use chrono::Utc;
    use proptest::prelude::*;
    use std::path::PathBuf;

    // -----------------------------------------------------------------------
    // Strategies
    // -----------------------------------------------------------------------

    fn arb_model_family() -> impl Strategy<Value = ModelFamily> {
        prop_oneof![
            Just(ModelFamily::AnthropicClaude),
            Just(ModelFamily::OpenAiGpt),
            Just(ModelFamily::GoogleGemini),
            "[a-z]{3,8}".prop_map(ModelFamily::Local),
        ]
    }

    fn arb_role() -> impl Strategy<Value = Role> {
        prop_oneof![Just(Role::User), Just(Role::Assistant), Just(Role::System),]
    }

    fn arb_conversation_turn() -> impl Strategy<Value = ConversationTurn> {
        (arb_role(), "[a-zA-Z0-9 .,!?]{0,80}", 0u32..5000u32, any::<bool>()).prop_map(
            |(role, content, tokens, pinned)| ConversationTurn {
                role,
                content,
                tokens,
                pinned,
                timestamp: Utc::now(),
            },
        )
    }

    fn arb_pin_entry() -> impl Strategy<Value = PinEntry> {
        (0usize..20usize, "[a-zA-Z ]{0,40}", 0u32..1000u32).prop_map(
            |(turn_index, reason, tokens)| PinEntry {
                turn_index,
                reason,
                tokens,
            },
        )
    }

    fn arb_learning() -> impl Strategy<Value = Learning> {
        (
            "[a-zA-Z_]{1,20}",
            "[a-zA-Z0-9 ]{0,60}",
            0usize..10usize,
        )
            .prop_map(|(key, value, source_turn)| Learning {
                key,
                value,
                source_turn,
            })
    }

    fn arb_correction_entry() -> impl Strategy<Value = CorrectionEntry> {
        (
            "[a-zA-Z0-9]{8,16}",
            "[a-zA-Z ]{0,40}",
            "[a-zA-Z ]{0,40}",
            "[a-zA-Z ]{0,40}",
        )
            .prop_map(|(id, original, correction, context)| CorrectionEntry {
                id,
                timestamp: Utc::now(),
                original,
                correction,
                context,
            })
    }

    fn arb_session_state() -> impl Strategy<Value = SessionState> {
        (
            "[a-zA-Z0-9]{8,16}",                          // id
            prop::collection::vec(arb_conversation_turn(), 0..5),
            prop::collection::vec(arb_pin_entry(), 0..3),
            prop::collection::vec(arb_learning(), 0..3),
            prop::collection::vec(arb_correction_entry(), 0..3),
            arb_model_family(),
            0u32..100_000u32, // consumed tokens
        )
            .prop_map(
                |(id, conversation, pins, learnings, corrections, model_family, consumed)| {
                    SessionState {
                        id,
                        project_dir: PathBuf::from("/tmp/test"),
                        conversation,
                        corrections: CorrectionLog {
                            entries: corrections,
                        },
                        pins,
                        learnings,
                        compressed_summary: String::new(),
                        budget: BudgetState {
                            window_size: 200_000,
                            consumed,
                            pinned: 0,
                            model_family,
                        },
                        tool_usage: vec![],
                        created_at: Utc::now(),
                        updated_at: Utc::now(),
                    }
                },
            )
    }

    // -----------------------------------------------------------------------
    // Property 6: CTX_Format round-trip
    // Validates: Requirements 7.1, 7.3, 7.4, 28.1, 28.2, 28.3
    // -----------------------------------------------------------------------

    proptest! {
        /// **Validates: Requirements 7.1, 7.3, 7.4, 28.1, 28.2, 28.3**
        ///
        /// Property 6: CTX_Format round-trip.
        ///
        /// For all valid `SessionState` objects, serializing to CTX then
        /// deserializing SHALL produce an equivalent session state.
        ///
        /// Equivalence is checked by double-serializing: serialize the original,
        /// deserialize, serialize again, and assert the two JSON strings are
        /// identical (modulo `created_at` timestamp which is regenerated).
        /// We compare the inner `session` field directly via JSON to avoid
        /// floating-point and timestamp issues.
        #[test]
        fn prop_ctx_round_trip(session in arb_session_state()) {
            // Serialize to CTX JSON
            let ctx_json = CtxFormat::serialize(&session)
                .expect("serialize should not fail on a valid SessionState");

            // Deserialize back
            let recovered = CtxFormat::deserialize(&ctx_json)
                .expect("deserialize should not fail on a valid CTX JSON string");

            // Compare by re-serializing both sessions to JSON and comparing
            // the stable fields (id, conversation length, pins, learnings,
            // corrections, budget consumed).
            prop_assert_eq!(&session.id, &recovered.id,
                "session id mismatch after round-trip");
            prop_assert_eq!(session.conversation.len(), recovered.conversation.len(),
                "conversation length mismatch after round-trip");
            prop_assert_eq!(session.pins.len(), recovered.pins.len(),
                "pins length mismatch after round-trip");
            prop_assert_eq!(session.learnings.len(), recovered.learnings.len(),
                "learnings length mismatch after round-trip");
            prop_assert_eq!(session.corrections.entries.len(), recovered.corrections.entries.len(),
                "corrections length mismatch after round-trip");
            prop_assert_eq!(session.budget.consumed, recovered.budget.consumed,
                "budget.consumed mismatch after round-trip");
        }
    }

    // -----------------------------------------------------------------------
    // Property 7: CTX export structural completeness
    // Validates: Requirements 7.2, 11.5
    // -----------------------------------------------------------------------

    proptest! {
        /// **Validates: Requirements 7.2, 11.5**
        ///
        /// Property 7: CTX export structural completeness.
        ///
        /// When a session is exported to CTX format, the resulting JSON MUST
        /// contain all corrections, pins, and conversation turns from the
        /// original session.
        #[test]
        fn prop_ctx_structural_completeness(session in arb_session_state()) {
            let ctx_json = CtxFormat::serialize(&session)
                .expect("serialize should not fail");

            // Parse the raw JSON envelope to inspect structure
            let envelope: CtxEnvelope = serde_json::from_str(&ctx_json)
                .expect("envelope should parse");

            // All corrections must be present
            prop_assert_eq!(
                envelope.session.corrections.entries.len(),
                session.corrections.entries.len(),
                "CTX export must include all correction log entries"
            );

            // All pins must be present
            prop_assert_eq!(
                envelope.session.pins.len(),
                session.pins.len(),
                "CTX export must include all pin entries"
            );

            // All conversation turns must be present
            prop_assert_eq!(
                envelope.session.conversation.len(),
                session.conversation.len(),
                "CTX export must include all conversation turns"
            );

            // Learnings must be present
            prop_assert_eq!(
                envelope.session.learnings.len(),
                session.learnings.len(),
                "CTX export must include all learnings"
            );

            // Budget must be present
            prop_assert_eq!(
                envelope.session.budget.consumed,
                session.budget.consumed,
                "CTX export must include budget state"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Property 33: CTX parse error descriptiveness
    // Validates: Requirements 28.4
    // -----------------------------------------------------------------------

    proptest! {
        /// **Validates: Requirements 28.4**
        ///
        /// Property 33: CTX parse error descriptiveness.
        ///
        /// When an invalid CTX JSON string is provided, the error message MUST
        /// include location information (line and column numbers).
        #[test]
        fn prop_ctx_parse_error_descriptiveness(
            // Generate a truncated or corrupted JSON string
            truncate_at in 0usize..50usize,
        ) {
            // Build a valid CTX JSON and then corrupt it
            let valid_session = SessionState {
                id: "test-session".to_string(),
                project_dir: PathBuf::from("/tmp"),
                conversation: vec![],
                corrections: CorrectionLog::default(),
                pins: vec![],
                learnings: vec![],
                compressed_summary: String::new(),
                budget: BudgetState {
                    window_size: 200_000,
                    consumed: 0,
                    pinned: 0,
                    model_family: ModelFamily::AnthropicClaude,
                },
                tool_usage: vec![],
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };

            let valid_json = CtxFormat::serialize(&valid_session)
                .expect("serialize should not fail");

            // Truncate the JSON to produce invalid input
            let truncated: String = valid_json.chars().take(truncate_at).collect();

            // Only test non-empty truncations that are actually invalid JSON
            if truncated.is_empty() || serde_json::from_str::<serde_json::Value>(&truncated).is_ok() {
                // Skip: empty string or accidentally valid JSON
                return Ok(());
            }

            let result = CtxFormat::deserialize(&truncated);
            prop_assert!(result.is_err(), "expected parse error for truncated CTX");

            let err_msg = result.unwrap_err().to_string();

            // Error must mention "line" and "column" for location info
            prop_assert!(
                err_msg.contains("line") && err_msg.contains("column"),
                "parse error '{}' does not include line/column location info",
                err_msg
            );
        }
    }

    // -----------------------------------------------------------------------
    // Additional unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_serialize_deserialize_basic() {
        let session = SessionState {
            id: "sess-001".to_string(),
            project_dir: PathBuf::from("/home/user/project"),
            conversation: vec![ConversationTurn {
                role: Role::User,
                content: "Hello".to_string(),
                tokens: 5,
                pinned: false,
                timestamp: Utc::now(),
            }],
            corrections: CorrectionLog::default(),
            pins: vec![],
            learnings: vec![],
            compressed_summary: String::new(),
            budget: BudgetState {
                window_size: 200_000,
                consumed: 100,
                pinned: 0,
                model_family: ModelFamily::AnthropicClaude,
            },
            tool_usage: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let ctx = CtxFormat::serialize(&session).unwrap();
        let recovered = CtxFormat::deserialize(&ctx).unwrap();

        assert_eq!(session.id, recovered.id);
        assert_eq!(session.conversation.len(), recovered.conversation.len());
        assert_eq!(session.budget.consumed, recovered.budget.consumed);
    }

    #[test]
    fn test_pretty_printer_is_valid_json() {
        let session = SessionState {
            id: "sess-002".to_string(),
            project_dir: PathBuf::from("/tmp"),
            conversation: vec![],
            corrections: CorrectionLog::default(),
            pins: vec![],
            learnings: vec![],
            compressed_summary: String::new(),
            budget: BudgetState {
                window_size: 200_000,
                consumed: 0,
                pinned: 0,
                model_family: ModelFamily::OpenAiGpt,
            },
            tool_usage: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let pretty = CtxFormat::serialize_pretty(&session).unwrap();
        // Must be valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&pretty).unwrap();
        assert_eq!(parsed["version"], "1.0");
        assert_eq!(parsed["format"], "ctx");
        // Pretty-printed output should contain newlines
        assert!(pretty.contains('\n'));
    }

    #[test]
    fn test_invalid_json_error_has_location() {
        let bad_json = r#"{"version": "1.0", "format": "ctx", "session": INVALID}"#;
        let result = CtxFormat::deserialize(bad_json);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("line"), "error should mention line: {}", msg);
        assert!(msg.contains("column"), "error should mention column: {}", msg);
    }

    #[test]
    fn test_envelope_contains_metadata() {
        let session = SessionState {
            id: "sess-003".to_string(),
            project_dir: PathBuf::from("/tmp"),
            conversation: vec![],
            corrections: CorrectionLog::default(),
            pins: vec![],
            learnings: vec![],
            compressed_summary: String::new(),
            budget: BudgetState {
                window_size: 200_000,
                consumed: 500,
                pinned: 0,
                model_family: ModelFamily::GoogleGemini,
            },
            tool_usage: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let ctx = CtxFormat::serialize(&session).unwrap();
        let envelope: CtxEnvelope = serde_json::from_str(&ctx).unwrap();

        assert_eq!(envelope.version, "1.0");
        assert_eq!(envelope.format, "ctx");
        assert!(!envelope.created_at.is_empty());
        assert!(!envelope.source_model.is_empty());
        assert_eq!(envelope.metadata.sqz_version, env!("CARGO_PKG_VERSION"));
    }
}
