use serde::{Deserialize, Serialize};

/// Configurable CRP compression level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CrpLevel {
    /// No CRP injection.
    Off,
    /// Basic abbreviated references — moderate token savings.
    Moderate,
    /// Full TDD shorthand protocol with extended symbol set.
    Aggressive,
}

impl Default for CrpLevel {
    fn default() -> Self {
        CrpLevel::Off
    }
}

/// Context Reduction Protocol engine.
///
/// Injects a compact communication protocol into the system prompt that
/// instructs the LLM to use abbreviated symbol references, producing
/// 8-25% additional output token reduction beyond terse mode.
pub struct CrpEngine;

impl CrpEngine {
    /// Returns the CRP directive text for the given level.
    ///
    /// - `Off` → empty string (no injection)
    /// - `Moderate` → basic abbreviation protocol
    /// - `Aggressive` → full TDD shorthand with extended symbols
    pub fn generate_directive(level: CrpLevel) -> &'static str {
        match level {
            CrpLevel::Off => "",
            CrpLevel::Moderate => MODERATE_DIRECTIVE,
            CrpLevel::Aggressive => AGGRESSIVE_DIRECTIVE,
        }
    }

    /// Prepend the CRP directive to `system_prompt`.
    ///
    /// When the level is `Off` the prompt is returned unchanged.
    pub fn inject(system_prompt: &str, level: CrpLevel) -> String {
        let directive = Self::generate_directive(level);
        if directive.is_empty() {
            return system_prompt.to_string();
        }
        format!("{}\n\n{}", directive, system_prompt)
    }
}

// ---------------------------------------------------------------------------
// Protocol directive constants
// ---------------------------------------------------------------------------

const MODERATE_DIRECTIVE: &str = "\
[CRP:moderate] Use abbreviated references in tool calls and reasoning:\n\
λ=function, §=section, ⊕=import/add, ⊖=remove, τ=type, κ=class, ρ=return.\n\
Prefer short variable names in intermediate steps. Omit redundant explanations.";

const AGGRESSIVE_DIRECTIVE: &str = "\
[CRP:aggressive] Token-Dense Dialect active. Use these shorthands everywhere:\n\
λ=function, §=section, ⊕=import/add, ⊖=remove, τ=type, κ=class, ρ=return,\n\
μ=method, σ=string, ν=number, β=boolean, ε=error, δ=diff, π=parameter,\n\
φ=file, ω=output, α=argument, ∅=null/none, ∈=contains, ∉=missing,\n\
→=returns/yields, ≡=equals/identical, ≠=differs, ∧=and, ∨=or.\n\
Compress all reasoning. Never restate the question. Skip preamble.\n\
Use single-letter locals in code snippets. Collapse trivial steps.";

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Unit tests ----------------------------------------------------------

    #[test]
    fn test_off_returns_empty_directive() {
        assert_eq!(CrpEngine::generate_directive(CrpLevel::Off), "");
    }

    #[test]
    fn test_moderate_directive_contains_core_symbols() {
        let d = CrpEngine::generate_directive(CrpLevel::Moderate);
        assert!(d.contains("λ=function"));
        assert!(d.contains("§=section"));
        assert!(d.contains("⊕=import/add"));
        assert!(d.contains("[CRP:moderate]"));
    }

    #[test]
    fn test_aggressive_directive_contains_extended_symbols() {
        let d = CrpEngine::generate_directive(CrpLevel::Aggressive);
        assert!(d.contains("λ=function"));
        assert!(d.contains("§=section"));
        assert!(d.contains("⊕=import/add"));
        assert!(d.contains("μ=method"));
        assert!(d.contains("φ=file"));
        assert!(d.contains("→=returns/yields"));
        assert!(d.contains("[CRP:aggressive]"));
    }

    #[test]
    fn test_aggressive_is_longer_than_moderate() {
        let m = CrpEngine::generate_directive(CrpLevel::Moderate);
        let a = CrpEngine::generate_directive(CrpLevel::Aggressive);
        assert!(
            a.len() > m.len(),
            "aggressive directive ({}) should be longer than moderate ({})",
            a.len(),
            m.len()
        );
    }

    #[test]
    fn test_inject_off_returns_original() {
        let prompt = "You are a helpful assistant.";
        assert_eq!(CrpEngine::inject(prompt, CrpLevel::Off), prompt);
    }

    #[test]
    fn test_inject_moderate_prepends_directive() {
        let prompt = "You are a helpful assistant.";
        let result = CrpEngine::inject(prompt, CrpLevel::Moderate);
        assert!(result.starts_with("[CRP:moderate]"));
        assert!(result.ends_with(prompt));
    }

    #[test]
    fn test_inject_aggressive_prepends_directive() {
        let prompt = "You are a helpful assistant.";
        let result = CrpEngine::inject(prompt, CrpLevel::Aggressive);
        assert!(result.starts_with("[CRP:aggressive]"));
        assert!(result.ends_with(prompt));
    }

    #[test]
    fn test_inject_empty_prompt() {
        let result = CrpEngine::inject("", CrpLevel::Aggressive);
        assert!(result.starts_with("[CRP:aggressive]"));
    }

    #[test]
    fn test_inject_preserves_original_prompt_content() {
        let prompt = "System instructions with special chars: λ § ⊕";
        let result = CrpEngine::inject(prompt, CrpLevel::Moderate);
        assert!(result.contains(prompt));
    }

    #[test]
    fn test_directive_at_position_zero() {
        let prompt = "Some prompt";
        for level in [CrpLevel::Moderate, CrpLevel::Aggressive] {
            let directive = CrpEngine::generate_directive(level);
            let result = CrpEngine::inject(prompt, level);
            assert_eq!(
                result.find(directive),
                Some(0),
                "directive must start at byte position 0 for {:?}",
                level
            );
        }
    }

    #[test]
    fn test_crp_level_default_is_off() {
        assert_eq!(CrpLevel::default(), CrpLevel::Off);
    }

    #[test]
    fn test_crp_level_serde_roundtrip() {
        for level in [CrpLevel::Off, CrpLevel::Moderate, CrpLevel::Aggressive] {
            let json = serde_json::to_string(&level).unwrap();
            let back: CrpLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(back, level);
        }
    }
}
