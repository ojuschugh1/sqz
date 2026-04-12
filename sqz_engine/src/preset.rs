use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::{Result, SqzError};

/// Parses, validates, and serializes TOML Preset files.
pub struct PresetParser;

impl PresetParser {
    /// Parse a TOML string into a validated `Preset`.
    pub fn parse(toml_str: &str) -> Result<Preset> {
        let preset: Preset = toml::from_str(toml_str)?;
        Self::validate(&preset)?;
        Ok(preset)
    }

    /// Serialize a `Preset` back to a pretty-printed TOML string.
    pub fn to_toml(preset: &Preset) -> Result<String> {
        Ok(toml::to_string_pretty(preset)?)
    }

    /// Validate all fields of a `Preset`, returning descriptive errors.
    pub fn validate(preset: &Preset) -> Result<()> {
        if preset.preset.name.is_empty() {
            return Err(SqzError::PresetValidation {
                field: "preset.name".to_string(),
                message: "must not be empty".to_string(),
            });
        }

        if preset.preset.version.is_empty() {
            return Err(SqzError::PresetValidation {
                field: "preset.version".to_string(),
                message: "must not be empty".to_string(),
            });
        }

        let wt = preset.budget.warning_threshold;
        if !(wt > 0.0 && wt < 1.0) {
            return Err(SqzError::PresetValidation {
                field: "budget.warning_threshold".to_string(),
                message: "must be between 0.0 and 1.0".to_string(),
            });
        }

        let ct = preset.budget.ceiling_threshold;
        if !(ct > 0.0 && ct < 1.0) || ct <= wt {
            return Err(SqzError::PresetValidation {
                field: "budget.ceiling_threshold".to_string(),
                message: "must be between 0.0 and 1.0 and greater than warning_threshold"
                    .to_string(),
            });
        }

        let max_tools = preset.tool_selection.max_tools;
        if !(1..=50).contains(&max_tools) {
            return Err(SqzError::PresetValidation {
                field: "tool_selection.max_tools".to_string(),
                message: "must be between 1 and 50".to_string(),
            });
        }

        let st = preset.tool_selection.similarity_threshold;
        if !(st > 0.0 && st < 1.0) {
            return Err(SqzError::PresetValidation {
                field: "tool_selection.similarity_threshold".to_string(),
                message: "must be between 0.0 and 1.0".to_string(),
            });
        }

        let cxt = preset.model.complexity_threshold;
        if !(cxt > 0.0 && cxt < 1.0) {
            return Err(SqzError::PresetValidation {
                field: "model.complexity_threshold".to_string(),
                message: "must be between 0.0 and 1.0".to_string(),
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub preset: PresetMeta,
    pub compression: CompressionConfig,
    pub tool_selection: ToolSelectionConfig,
    pub budget: BudgetConfig,
    pub terse_mode: TerseModeConfig,
    pub model: ModelConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetMeta {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
}

// --- Compression ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionConfig {
    #[serde(default)]
    pub stages: Vec<String>,
    pub keep_fields: Option<KeepFieldsConfig>,
    pub strip_fields: Option<StripFieldsConfig>,
    pub condense: Option<CondenseConfig>,
    pub git_diff_fold: Option<GitDiffFoldConfig>,
    pub strip_nulls: Option<StripNullsConfig>,
    pub flatten: Option<FlattenConfig>,
    pub truncate_strings: Option<TruncateStringsConfig>,
    pub collapse_arrays: Option<CollapseArraysConfig>,
    pub custom_transforms: Option<CustomTransformsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitDiffFoldConfig {
    pub enabled: bool,
    #[serde(default = "default_max_context_lines")]
    pub max_context_lines: u32,
}

fn default_max_context_lines() -> u32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeepFieldsConfig {
    pub enabled: bool,
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripFieldsConfig {
    pub enabled: bool,
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CondenseConfig {
    pub enabled: bool,
    #[serde(default = "default_max_repeated_lines")]
    pub max_repeated_lines: u32,
}

fn default_max_repeated_lines() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripNullsConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlattenConfig {
    pub enabled: bool,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
}

fn default_max_depth() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruncateStringsConfig {
    pub enabled: bool,
    #[serde(default = "default_max_length")]
    pub max_length: u32,
}

fn default_max_length() -> u32 {
    500
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollapseArraysConfig {
    pub enabled: bool,
    #[serde(default = "default_max_items")]
    pub max_items: u32,
    #[serde(default)]
    pub summary_template: String,
}

fn default_max_items() -> u32 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTransformsConfig {
    pub enabled: bool,
}

// --- Tool selection ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSelectionConfig {
    #[serde(default = "default_max_tools")]
    pub max_tools: usize,
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: f64,
    #[serde(default)]
    pub default_tools: Vec<String>,
}

fn default_max_tools() -> usize {
    5
}

fn default_similarity_threshold() -> f64 {
    0.7
}

// --- Budget ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetConfig {
    #[serde(default = "default_warning_threshold")]
    pub warning_threshold: f64,
    #[serde(default = "default_ceiling_threshold")]
    pub ceiling_threshold: f64,
    #[serde(default = "default_window_size")]
    pub default_window_size: u32,
    #[serde(default)]
    pub agents: HashMap<String, f64>,
}

fn default_warning_threshold() -> f64 {
    0.70
}

fn default_ceiling_threshold() -> f64 {
    0.85
}

fn default_window_size() -> u32 {
    200_000
}

// --- Terse mode ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerseModeConfig {
    pub enabled: bool,
    #[serde(default = "default_terse_level")]
    pub level: TerseLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerseLevel {
    Minimal,
    Moderate,
    Verbose,
}

fn default_terse_level() -> TerseLevel {
    TerseLevel::Moderate
}

// --- Model ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub family: String,
    #[serde(default)]
    pub primary: String,
    #[serde(default)]
    pub local: String,
    #[serde(default = "default_complexity_threshold")]
    pub complexity_threshold: f64,
    pub pricing: Option<ModelPricingConfig>,
}

fn default_complexity_threshold() -> f64 {
    0.4
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricingConfig {
    pub input_per_1k: f64,
    pub output_per_1k: f64,
    #[serde(default)]
    pub cache_read_discount: f64,
}

impl Default for Preset {
    fn default() -> Self {
        Preset {
            preset: PresetMeta {
                name: "default".to_string(),
                version: "1.0".to_string(),
                description: "Default compression preset for general development".to_string(),
            },
            compression: CompressionConfig {
                stages: vec![
                    "keep_fields".to_string(),
                    "strip_fields".to_string(),
                    "condense".to_string(),
                    "strip_nulls".to_string(),
                    "flatten".to_string(),
                    "truncate_strings".to_string(),
                    "collapse_arrays".to_string(),
                    "custom_transforms".to_string(),
                ],
                keep_fields: Some(KeepFieldsConfig {
                    enabled: false,
                    fields: vec![
                        "id".to_string(),
                        "name".to_string(),
                        "type".to_string(),
                        "status".to_string(),
                        "error".to_string(),
                        "message".to_string(),
                    ],
                }),
                strip_fields: Some(StripFieldsConfig {
                    enabled: true,
                    fields: vec![
                        "metadata.internal_id".to_string(),
                        "debug_info".to_string(),
                        "trace_id".to_string(),
                    ],
                }),
                condense: Some(CondenseConfig {
                    enabled: true,
                    max_repeated_lines: 3,
                }),
                git_diff_fold: Some(GitDiffFoldConfig {
                    enabled: true,
                    max_context_lines: 2,
                }),
                strip_nulls: Some(StripNullsConfig { enabled: true }),
                flatten: Some(FlattenConfig {
                    enabled: true,
                    max_depth: 3,
                }),
                truncate_strings: Some(TruncateStringsConfig {
                    enabled: true,
                    max_length: 500,
                }),
                collapse_arrays: Some(CollapseArraysConfig {
                    enabled: true,
                    max_items: 5,
                    summary_template: "... and {remaining} more items".to_string(),
                }),
                custom_transforms: Some(CustomTransformsConfig { enabled: true }),
            },
            tool_selection: ToolSelectionConfig {
                max_tools: 5,
                similarity_threshold: 0.7,
                default_tools: vec![
                    "read_file".to_string(),
                    "write_file".to_string(),
                    "search".to_string(),
                ],
            },
            budget: BudgetConfig {
                warning_threshold: 0.70,
                ceiling_threshold: 0.85,
                default_window_size: 200_000,
                agents: {
                    let mut m = HashMap::new();
                    m.insert("parent".to_string(), 0.60);
                    m.insert("child".to_string(), 0.20);
                    m
                },
            },
            terse_mode: TerseModeConfig {
                enabled: true,
                level: TerseLevel::Moderate,
            },
            model: ModelConfig {
                family: "anthropic".to_string(),
                primary: "claude-sonnet-4-20250514".to_string(),
                local: "llama-3.1-8b".to_string(),
                complexity_threshold: 0.4,
                pricing: Some(ModelPricingConfig {
                    input_per_1k: 0.003,
                    output_per_1k: 0.015,
                    cache_read_discount: 0.9,
                }),
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
    use proptest::prelude::*;

    // ---------------------------------------------------------------------------
    // Strategies for generating valid Preset objects
    // ---------------------------------------------------------------------------

    /// Non-empty string strategy (printable ASCII, no control chars).
    fn arb_nonempty_string() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_\\-\\.]{1,32}".prop_map(|s| s)
    }

    /// f64 strictly in (0.0, 1.0), exclusive.
    fn arb_open_unit() -> impl Strategy<Value = f64> {
        (1u32..=9999u32).prop_map(|n| n as f64 / 10_000.0)
    }

    /// Strategy for a valid BudgetConfig: ceiling > warning, both in (0, 1).
    fn arb_budget_config() -> impl Strategy<Value = BudgetConfig> {
        // Pick warning in (0, 0.9), then ceiling in (warning, 1.0).
        (1u32..=8999u32).prop_flat_map(|w_raw| {
            let warning = w_raw as f64 / 10_000.0; // in (0.0001, 0.8999)
            // ceiling must be > warning and < 1.0
            let c_min = (w_raw + 1) as f64 / 10_000.0;
            let c_max = 9999.0_f64 / 10_000.0;
            // Map a u32 in [c_min_int, 9999] to a f64
            let c_min_int = w_raw + 1;
            (c_min_int..=9999u32).prop_map(move |c_raw| {
                let ceiling = c_raw as f64 / 10_000.0;
                let _ = (c_min, c_max); // suppress unused warnings
                BudgetConfig {
                    warning_threshold: warning,
                    ceiling_threshold: ceiling,
                    default_window_size: 200_000,
                    agents: Default::default(),
                }
            })
        })
    }

    /// Strategy for a valid ToolSelectionConfig.
    fn arb_tool_selection_config() -> impl Strategy<Value = ToolSelectionConfig> {
        (1usize..=50usize, arb_open_unit()).prop_map(|(max_tools, similarity_threshold)| {
            ToolSelectionConfig {
                max_tools,
                similarity_threshold,
                default_tools: vec![],
            }
        })
    }

    /// Strategy for a valid ModelConfig.
    fn arb_model_config() -> impl Strategy<Value = ModelConfig> {
        (arb_nonempty_string(), arb_open_unit()).prop_map(|(family, complexity_threshold)| {
            ModelConfig {
                family,
                primary: String::new(),
                local: String::new(),
                complexity_threshold,
                pricing: None,
            }
        })
    }

    /// Strategy for a valid Preset.
    fn arb_preset() -> impl Strategy<Value = Preset> {
        (
            arb_nonempty_string(), // name
            arb_nonempty_string(), // version
            arb_budget_config(),
            arb_tool_selection_config(),
            arb_model_config(),
        )
            .prop_map(|(name, version, budget, tool_selection, model)| Preset {
                preset: PresetMeta {
                    name,
                    version,
                    description: String::new(),
                },
                compression: CompressionConfig {
                    stages: vec![],
                    keep_fields: None,
                    strip_fields: None,
                    condense: None,
                    git_diff_fold: None,
                    strip_nulls: None,
                    flatten: None,
                    truncate_strings: None,
                    collapse_arrays: None,
                    custom_transforms: None,
                },
                tool_selection,
                budget,
                terse_mode: TerseModeConfig {
                    enabled: false,
                    level: TerseLevel::Moderate,
                },
                model,
            })
    }

    // ---------------------------------------------------------------------------
    // Property 31: TOML Preset round-trip
    // Validates: Requirements 29.1, 29.2, 29.3
    // ---------------------------------------------------------------------------

    proptest! {
        /// **Validates: Requirements 29.1, 29.2, 29.3**
        ///
        /// Property 31: TOML Preset round-trip.
        ///
        /// For all valid Preset objects, serializing to TOML then deserializing
        /// SHALL produce an equivalent Preset object.
        ///
        /// We compare by double-serializing: serialize the original to TOML,
        /// parse it back, serialize again, and assert the two TOML strings are
        /// identical. This avoids f64 direct comparison issues while still
        /// verifying full fidelity.
        #[test]
        fn prop_preset_toml_round_trip(preset in arb_preset()) {
            // First serialize
            let toml1 = PresetParser::to_toml(&preset)
                .expect("to_toml should not fail on a valid preset");

            // Parse back
            let parsed = PresetParser::parse(&toml1)
                .expect("parse should not fail on a valid TOML string");

            // Second serialize
            let toml2 = PresetParser::to_toml(&parsed)
                .expect("to_toml should not fail on re-parsed preset");

            // The two TOML strings must be identical
            prop_assert_eq!(
                &toml1,
                &toml2,
                "TOML round-trip mismatch:\nfirst:  {}\nsecond: {}",
                toml1,
                toml2
            );
        }
    }

    // ---------------------------------------------------------------------------
    // Property 32: Preset validation error descriptiveness
    // Validates: Requirements 24.5, 29.4
    // ---------------------------------------------------------------------------

    /// Strategy for invalid warning_threshold values: 0.0, 1.0, negative, or >1.0.
    fn arb_invalid_warning_threshold() -> impl Strategy<Value = f64> {
        prop_oneof![
            Just(0.0_f64),
            Just(1.0_f64),
            // negative values: -1.0 to -0.0001
            (1u32..=10000u32).prop_map(|n| -(n as f64 / 10_000.0)),
            // values > 1.0: 1.0001 to 2.0
            (10001u32..=20000u32).prop_map(|n| n as f64 / 10_000.0),
        ]
    }

    /// Strategy for invalid ceiling_threshold values: 0.0, 1.0, negative, or >1.0.
    fn arb_invalid_ceiling_threshold() -> impl Strategy<Value = f64> {
        prop_oneof![
            Just(0.0_f64),
            Just(1.0_f64),
            (1u32..=10000u32).prop_map(|n| -(n as f64 / 10_000.0)),
            (10001u32..=20000u32).prop_map(|n| n as f64 / 10_000.0),
        ]
    }

    /// Strategy for invalid max_tools values: 0 or >50.
    fn arb_invalid_max_tools() -> impl Strategy<Value = usize> {
        prop_oneof![
            Just(0usize),
            (51usize..=200usize),
        ]
    }

    /// Strategy for invalid complexity_threshold values: 0.0, 1.0, negative, or >1.0.
    fn arb_invalid_complexity_threshold() -> impl Strategy<Value = f64> {
        prop_oneof![
            Just(0.0_f64),
            Just(1.0_f64),
            (1u32..=10000u32).prop_map(|n| -(n as f64 / 10_000.0)),
            (10001u32..=20000u32).prop_map(|n| n as f64 / 10_000.0),
        ]
    }

    proptest! {
        /// **Validates: Requirements 24.5, 29.4**
        ///
        /// Property 32a: Invalid `budget.warning_threshold` produces a descriptive error
        /// mentioning "budget.warning_threshold".
        #[test]
        fn prop_invalid_warning_threshold_error_mentions_field(
            invalid_wt in arb_invalid_warning_threshold()
        ) {
            let mut preset = Preset::default();
            preset.budget.warning_threshold = invalid_wt;
            // Also ensure ceiling > warning to isolate the warning_threshold error.
            // If invalid_wt >= 0.0 and < 1.0 but ceiling <= warning, we still want
            // the warning_threshold error to fire first. The validate function checks
            // warning_threshold before ceiling_threshold, so set ceiling to something
            // that would be valid if warning were valid.
            preset.budget.ceiling_threshold = 0.85;

            let result = PresetParser::validate(&preset);
            prop_assert!(result.is_err(), "expected validation error for warning_threshold={}", invalid_wt);
            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("budget.warning_threshold"),
                "error message '{}' does not mention 'budget.warning_threshold'",
                err_msg
            );
        }

        /// **Validates: Requirements 24.5, 29.4**
        ///
        /// Property 32b: Invalid `budget.ceiling_threshold` produces a descriptive error
        /// mentioning "budget.ceiling_threshold".
        #[test]
        fn prop_invalid_ceiling_threshold_error_mentions_field(
            invalid_ct in arb_invalid_ceiling_threshold()
        ) {
            let mut preset = Preset::default();
            // Keep warning_threshold valid so ceiling_threshold error fires.
            preset.budget.warning_threshold = 0.70;
            preset.budget.ceiling_threshold = invalid_ct;

            let result = PresetParser::validate(&preset);
            prop_assert!(result.is_err(), "expected validation error for ceiling_threshold={}", invalid_ct);
            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("budget.ceiling_threshold"),
                "error message '{}' does not mention 'budget.ceiling_threshold'",
                err_msg
            );
        }

        /// **Validates: Requirements 24.5, 29.4**
        ///
        /// Property 32c: Empty `preset.name` produces a descriptive error
        /// mentioning "preset.name".
        #[test]
        fn prop_empty_preset_name_error_mentions_field(_dummy in 0u32..1u32) {
            let mut preset = Preset::default();
            preset.preset.name = String::new();

            let result = PresetParser::validate(&preset);
            prop_assert!(result.is_err(), "expected validation error for empty preset.name");
            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("preset.name"),
                "error message '{}' does not mention 'preset.name'",
                err_msg
            );
        }

        /// **Validates: Requirements 24.5, 29.4**
        ///
        /// Property 32d: Invalid `tool_selection.max_tools` (0 or >50) produces a
        /// descriptive error mentioning "tool_selection.max_tools".
        #[test]
        fn prop_invalid_max_tools_error_mentions_field(
            invalid_mt in arb_invalid_max_tools()
        ) {
            let mut preset = Preset::default();
            preset.tool_selection.max_tools = invalid_mt;

            let result = PresetParser::validate(&preset);
            prop_assert!(result.is_err(), "expected validation error for max_tools={}", invalid_mt);
            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("tool_selection.max_tools"),
                "error message '{}' does not mention 'tool_selection.max_tools'",
                err_msg
            );
        }

        /// **Validates: Requirements 24.5, 29.4**
        ///
        /// Property 32e: Invalid `model.complexity_threshold` produces a descriptive
        /// error mentioning "model.complexity_threshold".
        #[test]
        fn prop_invalid_complexity_threshold_error_mentions_field(
            invalid_cxt in arb_invalid_complexity_threshold()
        ) {
            let mut preset = Preset::default();
            preset.model.complexity_threshold = invalid_cxt;

            let result = PresetParser::validate(&preset);
            prop_assert!(result.is_err(), "expected validation error for complexity_threshold={}", invalid_cxt);
            let err_msg = result.unwrap_err().to_string();
            prop_assert!(
                err_msg.contains("model.complexity_threshold"),
                "error message '{}' does not mention 'model.complexity_threshold'",
                err_msg
            );
        }
    }
}
