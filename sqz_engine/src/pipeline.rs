use crate::ansi_strip::AnsiStripper;
use crate::error::{Result, SqzError};
use crate::preset::Preset;
use crate::prompt_cache::PromptCacheDetector;
use crate::stages::{
    CollapseArraysStage, CondenseStage, CustomTransformsStage, FlattenStage, GitDiffFoldStage,
    KeepFieldsStage, StripFieldsStage, StripNullsStage, TruncateStringsStage,
};
use crate::toon::ToonEncoder;
use crate::types::{CompressedContent, Content, ContentType, StageConfig};

/// Minimal session context passed to the pipeline.
pub struct SessionContext {
    pub session_id: String,
}

/// The 8-stage compression pipeline orchestrator.
pub struct CompressionPipeline {
    stages: Vec<Box<dyn crate::stages::CompressionStage>>,
    toon_encoder: ToonEncoder,
    #[allow(dead_code)]
    prompt_cache_detector: PromptCacheDetector,
}

impl CompressionPipeline {
    /// Construct the pipeline from a preset, creating all 8 built-in stages
    /// sorted by priority.
    pub fn new(_preset: &Preset) -> Self {
        let mut stages: Vec<Box<dyn crate::stages::CompressionStage>> = vec![
            Box::new(AnsiStripper),
            Box::new(KeepFieldsStage),
            Box::new(StripFieldsStage),
            Box::new(CondenseStage),
            Box::new(GitDiffFoldStage),
            Box::new(StripNullsStage),
            Box::new(FlattenStage),
            Box::new(TruncateStringsStage),
            Box::new(CollapseArraysStage),
            Box::new(CustomTransformsStage),
        ];
        stages.sort_by_key(|s| s.priority());

        Self {
            stages,
            toon_encoder: ToonEncoder,
            prompt_cache_detector: PromptCacheDetector,
        }
    }

    /// Run content through all enabled stages then apply TOON encoding if the
    /// result is JSON.
    pub fn compress(
        &self,
        input: &str,
        _ctx: &SessionContext,
        preset: &Preset,
    ) -> Result<CompressedContent> {
        let tokens_original = (input.chars().count() as u32).saturating_add(3) / 4;

        let mut content = Content {
            raw: input.to_owned(),
            content_type: ContentType::PlainText,
            metadata: crate::types::ContentMetadata {
                source: None,
                path: None,
                language: None,
            },
            tokens_original,
        };

        let mut stages_applied: Vec<String> = Vec::new();

        for stage in &self.stages {
            let config = stage_config_from_preset(stage.name(), preset);
            if config.enabled {
                stage.process(&mut content, &config)?;
                stages_applied.push(stage.name().to_owned());
            }
        }

        // Apply TOON encoding if the result is JSON
        let data = if ToonEncoder::is_json(&content.raw) {
            let json: serde_json::Value = serde_json::from_str(&content.raw)
                .map_err(|e| SqzError::Other(format!("pipeline: JSON parse error: {e}")))?;
            let encoded = self.toon_encoder.encode(&json)?;
            stages_applied.push("toon_encode".to_owned());
            encoded
        } else {
            content.raw
        };

        let tokens_compressed = (data.chars().count() as u32).saturating_add(3) / 4;
        let compression_ratio = if tokens_original == 0 {
            1.0
        } else {
            tokens_compressed as f64 / tokens_original as f64
        };

        Ok(CompressedContent {
            data,
            tokens_compressed,
            tokens_original,
            stages_applied,
            compression_ratio,
            provenance: crate::types::Provenance::default(),
            verify: None,
        })
    }

    /// Insert a plugin stage and re-sort by priority.
    pub fn insert_stage(&mut self, stage: Box<dyn crate::stages::CompressionStage>) {
        self.stages.push(stage);
        self.stages.sort_by_key(|s| s.priority());
    }

    /// Rebuild stage list from a new preset (hot-reload support).
    /// Built-in stages are recreated; plugin stages are dropped and must be
    /// re-inserted by the caller.
    pub fn reload_preset(&mut self, _preset: &Preset) -> Result<()> {
        let mut stages: Vec<Box<dyn crate::stages::CompressionStage>> = vec![
            Box::new(AnsiStripper),
            Box::new(KeepFieldsStage),
            Box::new(StripFieldsStage),
            Box::new(CondenseStage),
            Box::new(GitDiffFoldStage),
            Box::new(StripNullsStage),
            Box::new(FlattenStage),
            Box::new(TruncateStringsStage),
            Box::new(CollapseArraysStage),
            Box::new(CustomTransformsStage),
        ];
        stages.sort_by_key(|s| s.priority());
        self.stages = stages;
        Ok(())
    }
}

/// Build a `StageConfig` for a named stage from the preset's compression config.
fn stage_config_from_preset(name: &str, preset: &Preset) -> StageConfig {
    let c = &preset.compression;
    match name {
        "ansi_strip" => StageConfig {
            enabled: true,
            options: serde_json::Value::Object(Default::default()),
        },
        "keep_fields" => {
            if let Some(cfg) = &c.keep_fields {
                StageConfig {
                    enabled: cfg.enabled,
                    options: serde_json::json!({ "fields": cfg.fields }),
                }
            } else {
                StageConfig::default()
            }
        }
        "strip_fields" => {
            if let Some(cfg) = &c.strip_fields {
                StageConfig {
                    enabled: cfg.enabled,
                    options: serde_json::json!({ "fields": cfg.fields }),
                }
            } else {
                StageConfig::default()
            }
        }
        "condense" => {
            if let Some(cfg) = &c.condense {
                StageConfig {
                    enabled: cfg.enabled,
                    options: serde_json::json!({
                        "max_repeated_lines": cfg.max_repeated_lines
                    }),
                }
            } else {
                StageConfig::default()
            }
        }
        "git_diff_fold" => {
            if let Some(cfg) = &c.git_diff_fold {
                StageConfig {
                    enabled: cfg.enabled,
                    options: serde_json::json!({
                        "max_context_lines": cfg.max_context_lines
                    }),
                }
            } else {
                // Default: enabled with 2 context lines
                StageConfig {
                    enabled: true,
                    options: serde_json::json!({ "max_context_lines": 2 }),
                }
            }
        }
        "strip_nulls" => {
            if let Some(cfg) = &c.strip_nulls {
                StageConfig {
                    enabled: cfg.enabled,
                    options: serde_json::Value::Object(Default::default()),
                }
            } else {
                StageConfig::default()
            }
        }
        "flatten" => {
            if let Some(cfg) = &c.flatten {
                StageConfig {
                    enabled: cfg.enabled,
                    options: serde_json::json!({ "max_depth": cfg.max_depth }),
                }
            } else {
                StageConfig::default()
            }
        }
        "truncate_strings" => {
            if let Some(cfg) = &c.truncate_strings {
                StageConfig {
                    enabled: cfg.enabled,
                    options: serde_json::json!({ "max_length": cfg.max_length }),
                }
            } else {
                StageConfig::default()
            }
        }
        "collapse_arrays" => {
            if let Some(cfg) = &c.collapse_arrays {
                StageConfig {
                    enabled: cfg.enabled,
                    options: serde_json::json!({
                        "max_items": cfg.max_items,
                        "summary_template": cfg.summary_template
                    }),
                }
            } else {
                StageConfig::default()
            }
        }
        "custom_transforms" => {
            if let Some(cfg) = &c.custom_transforms {
                StageConfig {
                    enabled: cfg.enabled,
                    options: serde_json::Value::Object(Default::default()),
                }
            } else {
                StageConfig::default()
            }
        }
        _ => StageConfig::default(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::{
        BudgetConfig, CollapseArraysConfig, CompressionConfig, CondenseConfig,
        CustomTransformsConfig, ModelConfig, PresetMeta,
        StripNullsConfig, ToolSelectionConfig, TruncateStringsConfig,
        TerseModeConfig,
    };

    fn default_preset() -> Preset {
        Preset {
            preset: PresetMeta {
                name: "test".into(),
                version: "1.0".into(),
                description: String::new(),
            },
            compression: CompressionConfig {
                stages: vec![],
                keep_fields: None,
                strip_fields: None,
                condense: Some(CondenseConfig {
                    enabled: true,
                    max_repeated_lines: 3,
                }),
                git_diff_fold: None,
                strip_nulls: Some(StripNullsConfig { enabled: true }),
                flatten: None,
                truncate_strings: Some(TruncateStringsConfig {
                    enabled: true,
                    max_length: 500,
                }),
                collapse_arrays: Some(CollapseArraysConfig {
                    enabled: true,
                    max_items: 5,
                    summary_template: "... and {remaining} more items".into(),
                }),
                custom_transforms: Some(CustomTransformsConfig { enabled: true }),
            },
            tool_selection: ToolSelectionConfig {
                max_tools: 5,
                similarity_threshold: 0.7,
                default_tools: vec![],
            },
            budget: BudgetConfig {
                warning_threshold: 0.70,
                ceiling_threshold: 0.85,
                default_window_size: 200_000,
                agents: Default::default(),
            },
            terse_mode: TerseModeConfig {
                enabled: false,
                level: crate::preset::TerseLevel::Moderate,
            },
            model: ModelConfig {
                family: "anthropic".into(),
                primary: "claude-sonnet-4-20250514".into(),
                local: String::new(),
                complexity_threshold: 0.4,
                pricing: None,
            },
        }
    }

    fn ctx() -> SessionContext {
        SessionContext {
            session_id: "test-session".into(),
        }
    }

    #[test]
    fn new_creates_pipeline_with_sorted_stages() {
        let preset = default_preset();
        let pipeline = CompressionPipeline::new(&preset);
        // Verify stages are sorted by priority
        let priorities: Vec<u32> = pipeline.stages.iter().map(|s| s.priority()).collect();
        let mut sorted = priorities.clone();
        sorted.sort();
        assert_eq!(priorities, sorted);
    }

    #[test]
    fn compress_plain_text_passthrough() {
        let preset = default_preset();
        let pipeline = CompressionPipeline::new(&preset);
        let result = pipeline.compress("hello world", &ctx(), &preset).unwrap();
        assert_eq!(result.data, "hello world");
        assert!(!result.stages_applied.contains(&"toon_encode".to_owned()));
    }

    #[test]
    fn compress_json_applies_toon() {
        let preset = default_preset();
        let pipeline = CompressionPipeline::new(&preset);
        let json = r#"{"name":"Alice","age":30}"#;
        let result = pipeline.compress(json, &ctx(), &preset).unwrap();
        assert!(result.data.starts_with("TOON:"), "data: {}", result.data);
        assert!(result.stages_applied.contains(&"toon_encode".to_owned()));
    }

    #[test]
    fn compress_strips_nulls_from_json() {
        let preset = default_preset();
        let pipeline = CompressionPipeline::new(&preset);
        let json = r#"{"a":1,"b":null}"#;
        let result = pipeline.compress(json, &ctx(), &preset).unwrap();
        // After strip_nulls, "b" is gone; TOON encodes the result
        assert!(result.data.starts_with("TOON:"));
        // Decode and verify null is gone
        let decoded = ToonEncoder.decode(&result.data).unwrap();
        assert!(decoded.get("b").is_none());
        assert_eq!(decoded["a"], serde_json::json!(1));
    }

    #[test]
    fn compress_returns_token_counts() {
        let preset = default_preset();
        let pipeline = CompressionPipeline::new(&preset);
        let input = "a".repeat(100);
        let result = pipeline.compress(&input, &ctx(), &preset).unwrap();
        assert!(result.tokens_original > 0);
        assert!(result.tokens_compressed > 0);
    }

    #[test]
    fn compress_ratio_is_reasonable() {
        let preset = default_preset();
        let pipeline = CompressionPipeline::new(&preset);
        let result = pipeline.compress("hello", &ctx(), &preset).unwrap();
        assert!(result.compression_ratio > 0.0);
    }

    #[test]
    fn insert_stage_re_sorts_by_priority() {
        use crate::stages::CompressionStage;
        use crate::types::StageConfig;

        struct LowPriorityStage;
        impl CompressionStage for LowPriorityStage {
            fn name(&self) -> &str {
                "low_priority"
            }
            fn priority(&self) -> u32 {
                5 // lower than all built-in stages
            }
            fn process(
                &self,
                _content: &mut Content,
                _config: &StageConfig,
            ) -> crate::error::Result<()> {
                Ok(())
            }
        }

        let preset = default_preset();
        let mut pipeline = CompressionPipeline::new(&preset);
        pipeline.insert_stage(Box::new(LowPriorityStage));

        let priorities: Vec<u32> = pipeline.stages.iter().map(|s| s.priority()).collect();
        let mut sorted = priorities.clone();
        sorted.sort();
        assert_eq!(priorities, sorted);
        assert_eq!(pipeline.stages[0].name(), "ansi_strip");
        assert_eq!(pipeline.stages[1].name(), "low_priority");
    }

    #[test]
    fn reload_preset_rebuilds_stages() {
        let preset = default_preset();
        let mut pipeline = CompressionPipeline::new(&preset);
        let original_count = pipeline.stages.len();
        pipeline.reload_preset(&preset).unwrap();
        assert_eq!(pipeline.stages.len(), original_count);
    }

    #[test]
    fn compress_keep_fields_filters_json() {
        use crate::preset::KeepFieldsConfig;
        let mut preset = default_preset();
        preset.compression.keep_fields = Some(KeepFieldsConfig {
            enabled: true,
            fields: vec!["id".into(), "name".into()],
        });
        let pipeline = CompressionPipeline::new(&preset);
        let json = r#"{"id":1,"name":"Bob","debug":"x"}"#;
        let result = pipeline.compress(json, &ctx(), &preset).unwrap();
        let decoded = ToonEncoder.decode(&result.data).unwrap();
        assert!(decoded.get("debug").is_none());
        assert_eq!(decoded["id"], serde_json::json!(1));
    }

    #[test]
    fn compress_empty_string() {
        let preset = default_preset();
        let pipeline = CompressionPipeline::new(&preset);
        let result = pipeline.compress("", &ctx(), &preset).unwrap();
        assert_eq!(result.data, "");
        assert_eq!(result.tokens_original, 0);
    }

    #[test]
    fn stage_config_from_preset_unknown_stage() {
        let preset = default_preset();
        let config = stage_config_from_preset("nonexistent", &preset);
        assert!(!config.enabled);
    }

    // ---------------------------------------------------------------------------
    // Property tests
    // ---------------------------------------------------------------------------

    use proptest::prelude::*;

    /// Generate a significant line from a fixed set of meaningful tokens.
    fn significant_line_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("error: connection refused".to_owned()),
            Just("warning: deprecated API usage".to_owned()),
            Just("failed: build step exited with code 1".to_owned()),
            Just("success: deployment complete".to_owned()),
            Just("status: all checks passed".to_owned()),
            Just("error: file not found".to_owned()),
            Just("warning: unused variable detected".to_owned()),
        ]
    }

    /// Generate a noise line (repeated decorative content).
    fn noise_line_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("---".to_owned()),
            Just("Loading...".to_owned()),
            Just("================".to_owned()),
            Just("...".to_owned()),
        ]
    }

    /// Recursive strategy that generates arbitrary serde_json::Value instances.
    /// Mirrors the strategy in toon.rs tests.
    fn arb_json_value() -> impl Strategy<Value = serde_json::Value> {
        let leaf = prop_oneof![
            Just(serde_json::Value::Null),
            any::<bool>().prop_map(serde_json::Value::Bool),
            any::<i64>().prop_map(|n| serde_json::json!(n)),
            any::<f64>()
                .prop_filter("must be finite", |f| f.is_finite())
                .prop_map(|f| serde_json::json!(f)),
            ".*".prop_map(serde_json::Value::String),
        ];

        leaf.prop_recursive(4, 64, 8, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..8)
                    .prop_map(serde_json::Value::Array),
                prop::collection::hash_map(".*", inner, 0..8).prop_map(|m| {
                    serde_json::Value::Object(m.into_iter().collect())
                }),
            ]
        })
    }

    proptest! {
        /// **Validates: Requirements 17.1, 17.2, 13.2**
        ///
        /// Property 22: ASCII-safe output.
        ///
        /// For any JSON input, the Compression_Pipeline SHALL produce output
        /// using only ASCII-safe characters: printable ASCII (0x20–0x7E) plus
        /// standard whitespace (\t = 0x09, \n = 0x0A, \r = 0x0D).
        #[test]
        fn prop_pipeline_ascii_safe_output(v in arb_json_value()) {
            let preset = default_preset();
            let pipeline = CompressionPipeline::new(&preset);

            let json_input = serde_json::to_string(&v).expect("serialize should not fail");
            let result = pipeline.compress(&json_input, &ctx(), &preset)
                .expect("compress should not fail");

            for ch in result.data.chars() {
                let cp = ch as u32;
                let is_printable_ascii = cp >= 0x20 && cp <= 0x7E;
                let is_standard_whitespace = cp == 0x09 || cp == 0x0A || cp == 0x0D;
                prop_assert!(
                    is_printable_ascii || is_standard_whitespace,
                    "non-ASCII-safe character in output: U+{:04X} ({:?})\noutput: {:?}",
                    cp, ch, result.data
                );
            }
        }
    }

    proptest! {
        /// **Validates: Requirements 1.3**
        ///
        /// Property 1: Compression preserves semantically significant content.
        ///
        /// For any CLI output containing significant tokens (errors, warnings,
        /// status messages) mixed with noise (repeated identical lines), the
        /// Compression_Pipeline SHALL produce output that:
        ///   1. Contains all significant lines.
        ///   2. Contains each noise line at most `max_repeated_lines` times.
        #[test]
        fn prop_compression_preserves_significant_content(
            significant_lines in prop::collection::vec(significant_line_strategy(), 1..=5),
            noise_line in noise_line_strategy(),
            noise_repeat in 5u32..=10u32,
        ) {
            let preset = default_preset(); // condense enabled, max_repeated_lines=3
            let pipeline = CompressionPipeline::new(&preset);

            // Build interleaved input: noise, significant, noise, significant, ...
            let mut lines: Vec<String> = Vec::new();
            for sig in &significant_lines {
                for _ in 0..noise_repeat {
                    lines.push(noise_line.clone());
                }
                lines.push(sig.clone());
            }
            // Trailing noise block
            for _ in 0..noise_repeat {
                lines.push(noise_line.clone());
            }

            let input = lines.join("\n");
            let result = pipeline.compress(&input, &ctx(), &preset).unwrap();
            let output = &result.data;

            // 1. All significant lines must appear in the output.
            for sig in &significant_lines {
                prop_assert!(
                    output.contains(sig.as_str()),
                    "significant line missing from output: {:?}\noutput: {:?}",
                    sig,
                    output
                );
            }

            // 2. No consecutive run of the noise line exceeds max_repeated_lines (3).
            let mut max_run = 0usize;
            let mut current_run = 0usize;
            for line in output.lines() {
                if line == noise_line.as_str() {
                    current_run += 1;
                    max_run = max_run.max(current_run);
                } else {
                    current_run = 0;
                }
            }
            prop_assert!(
                max_run <= 3,
                "noise line {:?} has a consecutive run of {} (max 3)\noutput: {:?}",
                noise_line,
                max_run,
                output
            );
        }
    }
}
