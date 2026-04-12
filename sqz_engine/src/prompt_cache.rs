use crate::types::Provider;

/// A detected prompt cache boundary in a message sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheBoundary {
    /// Byte offset in the concatenated content string where the boundary falls.
    /// Content before this offset is cached; content at/after is not.
    pub offset: usize,
    /// The provider that owns this cache boundary.
    pub provider: Provider,
    /// The cache discount fraction (0.9 for Anthropic, 0.5 for OpenAI).
    pub discount: f64,
}

/// A single message in a conversation, used for cache boundary detection.
#[derive(Debug, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
    /// Anthropic-style cache control marker.
    /// When `Some("ephemeral")`, this message has a cache_control marker.
    pub cache_control: Option<String>,
}

/// Detects prompt cache boundaries for Anthropic and OpenAI API formats.
///
/// - **Anthropic**: uses `cache_control: {"type": "ephemeral"}` markers on
///   individual messages. The boundary is placed after the last marked message.
/// - **OpenAI**: automatically caches the first 1024 tokens of a prompt.
///   Detected heuristically: if total content length > 4096 chars (≈1024 tokens),
///   the boundary is at char position 4096.
/// - **Google**: no cache boundary concept; always returns `None`.
pub struct PromptCacheDetector;

impl PromptCacheDetector {
    /// Detect a cache boundary in the given message sequence for the specified
    /// provider. Returns `None` if no boundary is detected.
    pub fn detect_boundary(
        &self,
        messages: &[Message],
        provider: Provider,
    ) -> Option<CacheBoundary> {
        match provider {
            Provider::Anthropic => self.detect_anthropic(messages),
            Provider::OpenAI => self.detect_openai(messages),
            Provider::Google => None,
        }
    }

    /// Split `content` at the given boundary offset.
    ///
    /// Returns `(before, after)` where `before` is byte-identical to the
    /// original content up to `boundary.offset`, and `after` is the remainder.
    ///
    /// If `boundary.offset >= content.len()`, `after` will be empty.
    pub fn split_at_boundary(&self, content: &str, boundary: &CacheBoundary) -> (String, String) {
        let offset = boundary.offset.min(content.len());
        let before = content[..offset].to_owned();
        let after = content[offset..].to_owned();
        (before, after)
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn detect_anthropic(&self, messages: &[Message]) -> Option<CacheBoundary> {
        // Find the last message that carries a cache_control marker.
        // The boundary offset is the byte position after that message's content.
        let mut offset: usize = 0;
        let mut boundary_offset: Option<usize> = None;

        for msg in messages {
            offset += msg.content.len();
            if msg.cache_control.as_deref() == Some("ephemeral") {
                boundary_offset = Some(offset);
            }
        }

        boundary_offset.map(|off| CacheBoundary {
            offset: off,
            provider: Provider::Anthropic,
            discount: 0.9,
        })
    }

    fn detect_openai(&self, messages: &[Message]) -> Option<CacheBoundary> {
        // OpenAI automatically caches the first 1024 tokens.
        // Heuristic: if total content length > 4096 chars, the boundary is at
        // char position 4096 (≈1024 tokens at ~4 chars/token).
        const OPENAI_CACHE_CHAR_THRESHOLD: usize = 4096;

        let total_len: usize = messages.iter().map(|m| m.content.len()).sum();
        if total_len > OPENAI_CACHE_CHAR_THRESHOLD {
            Some(CacheBoundary {
                offset: OPENAI_CACHE_CHAR_THRESHOLD,
                provider: Provider::OpenAI,
                discount: 0.5,
            })
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> Message {
        Message {
            role: role.to_owned(),
            content: content.to_owned(),
            cache_control: None,
        }
    }

    fn cached_msg(role: &str, content: &str) -> Message {
        Message {
            role: role.to_owned(),
            content: content.to_owned(),
            cache_control: Some("ephemeral".to_owned()),
        }
    }

    // -----------------------------------------------------------------------
    // Anthropic tests
    // -----------------------------------------------------------------------

    #[test]
    fn anthropic_no_cache_control_returns_none() {
        let detector = PromptCacheDetector;
        let messages = vec![msg("user", "hello"), msg("assistant", "world")];
        assert!(detector.detect_boundary(&messages, Provider::Anthropic).is_none());
    }

    #[test]
    fn anthropic_single_cached_message() {
        let detector = PromptCacheDetector;
        let messages = vec![cached_msg("user", "hello world")];
        let boundary = detector
            .detect_boundary(&messages, Provider::Anthropic)
            .unwrap();
        assert_eq!(boundary.offset, "hello world".len());
        assert_eq!(boundary.discount, 0.9);
        assert_eq!(boundary.provider, Provider::Anthropic);
    }

    #[test]
    fn anthropic_boundary_after_last_cached_message() {
        let detector = PromptCacheDetector;
        // Two cached messages; boundary should be after the second one.
        let messages = vec![
            cached_msg("user", "first"),   // offset 5
            msg("assistant", "middle"),    // offset 5+6=11
            cached_msg("user", "second"),  // offset 11+6=17
            msg("assistant", "tail"),      // offset 17+4=21
        ];
        let boundary = detector
            .detect_boundary(&messages, Provider::Anthropic)
            .unwrap();
        // "first" (5) + "middle" (6) + "second" (6) = 17
        assert_eq!(boundary.offset, 17);
    }

    #[test]
    fn anthropic_only_first_message_cached() {
        let detector = PromptCacheDetector;
        let messages = vec![
            cached_msg("system", "sys"),  // offset 3
            msg("user", "query"),         // offset 3+5=8
        ];
        let boundary = detector
            .detect_boundary(&messages, Provider::Anthropic)
            .unwrap();
        assert_eq!(boundary.offset, 3);
    }

    // -----------------------------------------------------------------------
    // OpenAI tests
    // -----------------------------------------------------------------------

    #[test]
    fn openai_short_content_returns_none() {
        let detector = PromptCacheDetector;
        let messages = vec![msg("user", "short")];
        assert!(detector.detect_boundary(&messages, Provider::OpenAI).is_none());
    }

    #[test]
    fn openai_long_content_returns_boundary_at_4096() {
        let detector = PromptCacheDetector;
        let long_content = "x".repeat(5000);
        let messages = vec![msg("user", &long_content)];
        let boundary = detector
            .detect_boundary(&messages, Provider::OpenAI)
            .unwrap();
        assert_eq!(boundary.offset, 4096);
        assert_eq!(boundary.discount, 0.5);
        assert_eq!(boundary.provider, Provider::OpenAI);
    }

    #[test]
    fn openai_exactly_at_threshold_returns_none() {
        let detector = PromptCacheDetector;
        let content = "x".repeat(4096);
        let messages = vec![msg("user", &content)];
        // total_len == 4096, not > 4096, so no boundary
        assert!(detector.detect_boundary(&messages, Provider::OpenAI).is_none());
    }

    // -----------------------------------------------------------------------
    // Google tests
    // -----------------------------------------------------------------------

    #[test]
    fn google_always_returns_none() {
        let detector = PromptCacheDetector;
        let messages = vec![cached_msg("user", "x".repeat(10000).as_str())];
        assert!(detector.detect_boundary(&messages, Provider::Google).is_none());
    }

    // -----------------------------------------------------------------------
    // split_at_boundary tests
    // -----------------------------------------------------------------------

    #[test]
    fn split_at_boundary_basic() {
        let detector = PromptCacheDetector;
        let content = "hello world";
        let boundary = CacheBoundary {
            offset: 5,
            provider: Provider::Anthropic,
            discount: 0.9,
        };
        let (before, after) = detector.split_at_boundary(content, &boundary);
        assert_eq!(before, "hello");
        assert_eq!(after, " world");
    }

    #[test]
    fn split_at_boundary_offset_zero() {
        let detector = PromptCacheDetector;
        let content = "hello";
        let boundary = CacheBoundary {
            offset: 0,
            provider: Provider::Anthropic,
            discount: 0.9,
        };
        let (before, after) = detector.split_at_boundary(content, &boundary);
        assert_eq!(before, "");
        assert_eq!(after, "hello");
    }

    #[test]
    fn split_at_boundary_offset_beyond_end() {
        let detector = PromptCacheDetector;
        let content = "hello";
        let boundary = CacheBoundary {
            offset: 100,
            provider: Provider::Anthropic,
            discount: 0.9,
        };
        let (before, after) = detector.split_at_boundary(content, &boundary);
        assert_eq!(before, "hello");
        assert_eq!(after, "");
    }

    #[test]
    fn split_before_is_byte_identical() {
        let detector = PromptCacheDetector;
        let content = "abcdefghij";
        let boundary = CacheBoundary {
            offset: 5,
            provider: Provider::OpenAI,
            discount: 0.5,
        };
        let (before, _) = detector.split_at_boundary(content, &boundary);
        // before must be byte-identical to the original prefix
        assert_eq!(before.as_bytes(), &content.as_bytes()[..5]);
    }

    // -----------------------------------------------------------------------
    // Property tests
    // ---------------------------------------------------------------------------

    use proptest::prelude::*;

    /// Strategy: generate a list of messages where at least one has a
    /// cache_control marker (Anthropic format).
    fn anthropic_messages_with_boundary() -> impl Strategy<Value = Vec<Message>> {
        // Generate 1-8 messages; at least one will be marked as cached.
        (1usize..=8usize).prop_flat_map(|n| {
            // For each message, generate content and whether it's cached.
            let msg_strategy = (
                prop_oneof![Just("user"), Just("assistant"), Just("system")],
                "[a-z]{1,50}",
                any::<bool>(),
            );
            prop::collection::vec(msg_strategy, n).prop_filter(
                "at least one cached message",
                |msgs| msgs.iter().any(|(_, _, cached)| *cached),
            )
        })
        .prop_map(|msgs| {
            msgs.into_iter()
                .map(|(role, content, cached)| Message {
                    role: role.to_owned(),
                    content,
                    cache_control: if cached {
                        Some("ephemeral".to_owned())
                    } else {
                        None
                    },
                })
                .collect()
        })
    }

    proptest! {
        /// **Validates: Requirements 4.1, 4.2**
        ///
        /// Property 3: Prompt cache boundary preservation.
        ///
        /// For any message sequence containing a prompt cache boundary marker
        /// (Anthropic or OpenAI format), the Prompt_Cache_Detector SHALL
        /// identify the boundary, and content preceding the boundary SHALL be
        /// byte-identical to the original.
        #[test]
        fn prop_prompt_cache_boundary_preservation(
            messages in anthropic_messages_with_boundary(),
        ) {
            let detector = PromptCacheDetector;

            // 1. Boundary must be detected (at least one cached message exists).
            let boundary = detector.detect_boundary(&messages, Provider::Anthropic);
            prop_assert!(
                boundary.is_some(),
                "expected boundary to be detected for messages with cache_control markers"
            );
            let boundary = boundary.unwrap();

            // 2. Discount must be 0.9 for Anthropic.
            prop_assert_eq!(boundary.discount, 0.9);

            // 3. Content before the boundary must be byte-identical to the original.
            //    Concatenate all message contents to form the "full content".
            let full_content: String = messages.iter().map(|m| m.content.as_str()).collect();
            let (before, _after) = detector.split_at_boundary(&full_content, &boundary);

            let expected_prefix = &full_content[..boundary.offset.min(full_content.len())];
            prop_assert_eq!(
                before.as_bytes(),
                expected_prefix.as_bytes(),
                "content before boundary must be byte-identical to original prefix"
            );

            // 4. The boundary offset must be at or after the last cached message's
            //    content end, and must not exceed the total content length.
            prop_assert!(
                boundary.offset <= full_content.len(),
                "boundary offset {} exceeds content length {}",
                boundary.offset,
                full_content.len()
            );

            // 5. Verify the boundary is placed after the last cached message.
            let mut cumulative = 0usize;
            let mut last_cached_end = 0usize;
            for msg in &messages {
                cumulative += msg.content.len();
                if msg.cache_control.as_deref() == Some("ephemeral") {
                    last_cached_end = cumulative;
                }
            }
            prop_assert_eq!(
                boundary.offset,
                last_cached_end,
                "boundary offset should be after the last cached message"
            );
        }
    }

    proptest! {
        /// Property 3 (OpenAI variant): For any message sequence whose total
        /// content length exceeds 4096 chars, the detector SHALL identify a
        /// boundary at offset 4096, and content before that offset SHALL be
        /// byte-identical to the original.
        #[test]
        fn prop_openai_cache_boundary_preservation(
            // Generate content that is definitely > 4096 chars total
            extra in 1usize..=1000usize,
            content in "[a-z]{4097,5000}",
        ) {
            let _ = extra; // suppress unused warning
            let detector = PromptCacheDetector;
            let messages = vec![Message {
                role: "user".to_owned(),
                content: content.clone(),
                cache_control: None,
            }];

            let boundary = detector.detect_boundary(&messages, Provider::OpenAI);
            prop_assert!(boundary.is_some(), "expected OpenAI boundary for long content");
            let boundary = boundary.unwrap();

            prop_assert_eq!(boundary.offset, 4096);
            prop_assert_eq!(boundary.discount, 0.5);

            let (before, _) = detector.split_at_boundary(&content, &boundary);
            prop_assert_eq!(
                before.as_bytes(),
                &content.as_bytes()[..4096],
                "content before OpenAI boundary must be byte-identical"
            );
        }
    }
}
