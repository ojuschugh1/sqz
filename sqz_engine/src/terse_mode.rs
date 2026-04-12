use crate::preset::{TerseLevel, TerseModeConfig};

/// Injects terse mode modifiers into system prompts based on Preset configuration.
pub struct TerseMode;

impl TerseMode {
    /// Inject a terse mode modifier at the beginning of `system_prompt`.
    /// Returns the modified prompt if terse mode is enabled, or the original
    /// prompt unchanged if disabled.
    pub fn inject(&self, system_prompt: &str, config: &TerseModeConfig) -> String {
        if !config.enabled {
            return system_prompt.to_string();
        }
        let modifier = Self::modifier_for_level(&config.level);
        format!("{}\n\n{}", modifier, system_prompt)
    }

    /// Returns the modifier string for the given terse level.
    pub fn modifier_for_level(level: &TerseLevel) -> &'static str {
        match level {
            TerseLevel::Minimal => {
                "Be extremely concise. Use minimal words. Omit all pleasantries and explanations."
            }
            TerseLevel::Moderate => "Be concise and direct. Avoid unnecessary verbosity.",
            TerseLevel::Verbose => "Provide thorough explanations when helpful.",
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::{TerseLevel, TerseModeConfig};
    use proptest::prelude::*;

    fn terse_config(enabled: bool, level: TerseLevel) -> TerseModeConfig {
        TerseModeConfig { enabled, level }
    }

    fn arb_terse_level() -> impl Strategy<Value = TerseLevel> {
        prop_oneof![
            Just(TerseLevel::Minimal),
            Just(TerseLevel::Moderate),
            Just(TerseLevel::Verbose),
        ]
    }

    fn arb_system_prompt() -> impl Strategy<Value = String> {
        // Printable ASCII strings of length 0..=200
        "[ -~]{0,200}".prop_map(|s| s)
    }

    // -----------------------------------------------------------------------
    // Property 30: Terse mode injection
    // Validates: Requirements 25.1, 25.2, 25.3, 25.4
    // -----------------------------------------------------------------------

    proptest! {
        /// **Validates: Requirements 25.1, 25.2, 25.3, 25.4**
        ///
        /// Property 30a: When terse mode is disabled, the output equals the
        /// original system prompt unchanged.
        #[test]
        fn prop_terse_disabled_returns_original(
            prompt in arb_system_prompt(),
            level in arb_terse_level(),
        ) {
            let tm = TerseMode;
            let config = terse_config(false, level);
            let result = tm.inject(&prompt, &config);
            prop_assert_eq!(
                &result,
                &prompt,
                "disabled terse mode must return original prompt unchanged"
            );
        }

        /// **Validates: Requirements 25.1, 25.2, 25.3, 25.4**
        ///
        /// Property 30b: When terse mode is enabled, the output starts with
        /// the modifier string for the configured level.
        #[test]
        fn prop_terse_enabled_output_starts_with_modifier(
            prompt in arb_system_prompt(),
            level in arb_terse_level(),
        ) {
            let tm = TerseMode;
            let modifier = TerseMode::modifier_for_level(&level);
            let config = terse_config(true, level);
            let result = tm.inject(&prompt, &config);
            prop_assert!(
                result.starts_with(modifier),
                "enabled terse mode output must start with modifier '{}', got '{}'",
                modifier,
                result
            );
        }

        /// **Validates: Requirements 25.4**
        ///
        /// Property 30c: When terse mode is enabled, the modifier is at
        /// position 0 (the very beginning) of the output.
        #[test]
        fn prop_terse_modifier_at_position_zero(
            prompt in arb_system_prompt(),
            level in arb_terse_level(),
        ) {
            let tm = TerseMode;
            let modifier = TerseMode::modifier_for_level(&level);
            let config = terse_config(true, level);
            let result = tm.inject(&prompt, &config);
            prop_assert_eq!(
                result.find(modifier),
                Some(0),
                "modifier must appear at byte position 0 in the output"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_modifier_for_minimal() {
        assert_eq!(
            TerseMode::modifier_for_level(&TerseLevel::Minimal),
            "Be extremely concise. Use minimal words. Omit all pleasantries and explanations."
        );
    }

    #[test]
    fn test_modifier_for_moderate() {
        assert_eq!(
            TerseMode::modifier_for_level(&TerseLevel::Moderate),
            "Be concise and direct. Avoid unnecessary verbosity."
        );
    }

    #[test]
    fn test_modifier_for_verbose() {
        assert_eq!(
            TerseMode::modifier_for_level(&TerseLevel::Verbose),
            "Provide thorough explanations when helpful."
        );
    }

    #[test]
    fn test_inject_disabled_returns_original() {
        let tm = TerseMode;
        let prompt = "You are a helpful assistant.";
        let config = terse_config(false, TerseLevel::Moderate);
        assert_eq!(tm.inject(prompt, &config), prompt);
    }

    #[test]
    fn test_inject_enabled_prepends_modifier() {
        let tm = TerseMode;
        let prompt = "You are a helpful assistant.";
        let config = terse_config(true, TerseLevel::Moderate);
        let result = tm.inject(prompt, &config);
        assert_eq!(
            result,
            "Be concise and direct. Avoid unnecessary verbosity.\n\nYou are a helpful assistant."
        );
    }

    #[test]
    fn test_inject_enabled_empty_prompt() {
        let tm = TerseMode;
        let config = terse_config(true, TerseLevel::Minimal);
        let result = tm.inject("", &config);
        assert!(result.starts_with(
            "Be extremely concise. Use minimal words. Omit all pleasantries and explanations."
        ));
    }
}
