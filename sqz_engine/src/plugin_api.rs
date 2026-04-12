use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::error::{Result, SqzError};
use crate::stages::CompressionStage;
use crate::types::{Content, StageConfig};

// ---------------------------------------------------------------------------
// SqzPlugin trait
// ---------------------------------------------------------------------------

/// Trait that native Rust plugins implement.
///
/// Plugins are discovered from a plugin directory, loaded at startup, and
/// inserted into the Compression_Pipeline at the position specified by their
/// `priority` value (lower = earlier).
pub trait SqzPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    /// Lower priority number = earlier in the pipeline.
    fn priority(&self) -> u32;
    fn compress(&self, content: &mut Content, config: &serde_json::Value) -> Result<()>;
}

// ---------------------------------------------------------------------------
// WASM plugin interface (documentation)
// ---------------------------------------------------------------------------
//
// Plugins compiled to WASM implement these exported functions:
//   - sqz_plugin_name()     -> *const u8   (null-terminated UTF-8 string)
//   - sqz_plugin_version()  -> *const u8
//   - sqz_plugin_priority() -> u32
//   - sqz_plugin_compress(ptr: *const u8, len: u32) -> *const u8
//
// The compress function receives JSON-encoded Content and returns
// JSON-encoded Content (or a null pointer on error).

// ---------------------------------------------------------------------------
// Plugin manifest (TOML)
// ---------------------------------------------------------------------------

/// Parsed representation of a plugin's `.toml` manifest file.
#[derive(Debug, Clone)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub priority: u32,
    pub config: serde_json::Value,
    pub source: PluginSource,
}

/// Where the plugin binary lives.
#[derive(Debug, Clone)]
pub enum PluginSource {
    /// A native shared library (`.so` / `.dylib` / `.dll`).
    NativeLib(PathBuf),
    /// A WASM module (`.wasm`).
    WasmModule(PathBuf),
}

// ---------------------------------------------------------------------------
// Internal TOML deserialization helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ManifestToml {
    plugin: PluginSection,
}

#[derive(Debug, Deserialize)]
struct PluginSection {
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: String,
    priority: u32,
    #[serde(default)]
    config: Option<toml::Value>,
}

// ---------------------------------------------------------------------------
// LoadedPlugin — wraps a native plugin + its manifest config
// ---------------------------------------------------------------------------

struct LoadedPlugin {
    plugin: Arc<dyn SqzPlugin>,
    config: serde_json::Value,
}

// ---------------------------------------------------------------------------
// PluginLoader
// ---------------------------------------------------------------------------

/// Discovers and loads plugins from a directory.
///
/// For each `.toml` manifest found, `discover_and_load` checks whether a
/// corresponding native library (`.so`/`.dylib`/`.dll`) or WASM module
/// (`.wasm`) exists alongside it.  Native library loading requires unsafe
/// FFI and is left as a stub — the manifest is still returned so callers
/// can inspect what was found.
pub struct PluginLoader {
    plugin_dir: PathBuf,
    loaded: Vec<LoadedPlugin>,
}

impl PluginLoader {
    /// Create a new loader pointing at `plugin_dir`.
    pub fn new(plugin_dir: &Path) -> Self {
        Self {
            plugin_dir: plugin_dir.to_owned(),
            loaded: Vec::new(),
        }
    }

    /// Scan `plugin_dir` for `.toml` manifest files, resolve the binary
    /// path, and return the manifests found.
    ///
    /// Native library loading (dlopen / WASM runtime) is intentionally
    /// stubbed out — this method only reads manifests and resolves paths.
    pub fn discover_and_load(&mut self) -> Result<Vec<PluginManifest>> {
        let mut manifests = Vec::new();

        let entries = match std::fs::read_dir(&self.plugin_dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(manifests),
            Err(e) => return Err(SqzError::Io(e)),
        };

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }

            match self.load_manifest(&path) {
                Ok(manifest) => manifests.push(manifest),
                Err(e) => {
                    eprintln!(
                        "[sqz] warning: failed to load plugin manifest {}: {}",
                        path.display(),
                        e
                    );
                }
            }
        }

        Ok(manifests)
    }

    /// Return `CompressionStage` wrappers for every loaded plugin, sorted
    /// by ascending priority.
    pub fn get_stages(&self) -> Vec<Box<dyn CompressionStage>> {
        let mut stages: Vec<&LoadedPlugin> = self.loaded.iter().collect();
        stages.sort_by_key(|lp| lp.plugin.priority());

        stages
            .into_iter()
            .map(|lp| -> Box<dyn CompressionStage> {
                Box::new(PluginStageWrapper {
                    name: lp.plugin.name().to_owned(),
                    priority: lp.plugin.priority(),
                    plugin: Arc::clone(&lp.plugin),
                    config: lp.config.clone(),
                })
            })
            .collect()
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    fn load_manifest(&self, toml_path: &Path) -> Result<PluginManifest> {
        let raw = std::fs::read_to_string(toml_path)?;
        let parsed: ManifestToml = toml::from_str(&raw)?;
        let p = parsed.plugin;

        // Convert toml::Value config to serde_json::Value
        let config_json = p.config.map(toml_to_json).unwrap_or(serde_json::Value::Object(Default::default()));

        // Determine the binary path: prefer .so/.dylib/.dll, then .wasm
        let stem = toml_path.file_stem().unwrap_or_default();
        let dir = toml_path.parent().unwrap_or(&self.plugin_dir);

        let source = resolve_plugin_source(dir, stem);

        Ok(PluginManifest {
            name: p.name,
            version: p.version,
            description: p.description,
            author: p.author,
            priority: p.priority,
            config: config_json,
            source,
        })
    }

    /// Register a native plugin directly (used in tests and by callers that
    /// construct plugins programmatically).
    pub fn register(&mut self, plugin: Box<dyn SqzPlugin>, config: serde_json::Value) {
        self.loaded.push(LoadedPlugin { plugin: Arc::from(plugin), config });
    }
}

// ---------------------------------------------------------------------------
// PluginStageWrapper — adapts SqzPlugin into CompressionStage
// ---------------------------------------------------------------------------

/// Wraps a `SqzPlugin` as a `CompressionStage`.
///
/// Panics from the plugin are caught via `std::panic::catch_unwind`; on
/// panic the stage is skipped and an error is logged (Requirement 15.5).
struct PluginStageWrapper {
    name: String,
    priority: u32,
    plugin: Arc<dyn SqzPlugin>,
    config: serde_json::Value,
}

impl CompressionStage for PluginStageWrapper {
    fn name(&self) -> &str {
        &self.name
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    fn process(&self, content: &mut Content, _stage_config: &StageConfig) -> Result<()> {
        if !_stage_config.enabled {
            return Ok(());
        }

        let plugin = &*self.plugin;

        // Catch panics so a misbehaving plugin cannot crash the pipeline.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            plugin.compress(content, &self.config)
        }));

        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                eprintln!("[sqz] plugin '{}' error: {}", self.name, e);
                Err(SqzError::Plugin {
                    plugin: self.name.clone(),
                    message: e.to_string(),
                })
            }
            Err(_panic) => {
                eprintln!("[sqz] plugin '{}' panicked — skipping stage", self.name);
                Err(SqzError::Plugin {
                    plugin: self.name.clone(),
                    message: "plugin panicked".to_owned(),
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_plugin_source(dir: &Path, stem: &std::ffi::OsStr) -> PluginSource {
    // Native library extensions in priority order
    let native_exts = if cfg!(target_os = "macos") {
        vec!["dylib"]
    } else if cfg!(target_os = "windows") {
        vec!["dll"]
    } else {
        vec!["so"]
    };

    for ext in &native_exts {
        let candidate = dir.join(stem).with_extension(ext);
        if candidate.exists() {
            return PluginSource::NativeLib(candidate);
        }
    }

    // Fall back to WASM
    let wasm_candidate = dir.join(stem).with_extension("wasm");
    PluginSource::WasmModule(wasm_candidate)
}

fn toml_to_json(value: toml::Value) -> serde_json::Value {
    match value {
        toml::Value::String(s) => serde_json::Value::String(s),
        toml::Value::Integer(i) => serde_json::Value::Number(i.into()),
        toml::Value::Float(f) => serde_json::json!(f),
        toml::Value::Boolean(b) => serde_json::Value::Bool(b),
        toml::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(toml_to_json).collect())
        }
        toml::Value::Table(tbl) => {
            let map = tbl
                .into_iter()
                .map(|(k, v)| (k, toml_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
        toml::Value::Datetime(dt) => serde_json::Value::String(dt.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentMetadata, ContentType};

    // A simple test plugin that uppercases the raw content.
    struct UppercasePlugin {
        prio: u32,
    }

    impl SqzPlugin for UppercasePlugin {
        fn name(&self) -> &str {
            "uppercase"
        }
        fn version(&self) -> &str {
            "0.1.0"
        }
        fn priority(&self) -> u32 {
            self.prio
        }
        fn compress(&self, content: &mut Content, _config: &serde_json::Value) -> Result<()> {
            content.raw = content.raw.to_uppercase();
            Ok(())
        }
    }

    // A plugin that always panics.
    struct PanicPlugin;

    impl SqzPlugin for PanicPlugin {
        fn name(&self) -> &str {
            "panicker"
        }
        fn version(&self) -> &str {
            "0.0.1"
        }
        fn priority(&self) -> u32 {
            99
        }
        fn compress(&self, _content: &mut Content, _config: &serde_json::Value) -> Result<()> {
            panic!("intentional panic for testing");
        }
    }

    fn plain_content(raw: &str) -> Content {
        Content {
            raw: raw.to_owned(),
            content_type: ContentType::PlainText,
            metadata: ContentMetadata {
                source: None,
                path: None,
                language: None,
            },
            tokens_original: 0,
        }
    }

    fn enabled_stage_config() -> StageConfig {
        StageConfig {
            enabled: true,
            options: serde_json::json!({}),
        }
    }

    #[test]
    fn plugin_stage_wrapper_calls_plugin() {
        let mut loader = PluginLoader::new(Path::new("/tmp/nonexistent"));
        loader.register(
            Box::new(UppercasePlugin { prio: 10 }),
            serde_json::json!({}),
        );

        let stages = loader.get_stages();
        assert_eq!(stages.len(), 1);

        let mut content = plain_content("hello world");
        stages[0].process(&mut content, &enabled_stage_config()).unwrap();
        assert_eq!(content.raw, "HELLO WORLD");
    }

    #[test]
    fn plugin_panic_is_caught_and_returns_error() {
        let mut loader = PluginLoader::new(Path::new("/tmp/nonexistent"));
        loader.register(Box::new(PanicPlugin), serde_json::json!({}));

        let stages = loader.get_stages();
        let mut content = plain_content("data");
        let result = stages[0].process(&mut content, &enabled_stage_config());
        assert!(result.is_err());
        // Content should be unchanged after a panic
        assert_eq!(content.raw, "data");
    }

    #[test]
    fn get_stages_sorted_by_priority() {
        let mut loader = PluginLoader::new(Path::new("/tmp/nonexistent"));
        loader.register(Box::new(UppercasePlugin { prio: 50 }), serde_json::json!({}));
        loader.register(Box::new(UppercasePlugin { prio: 10 }), serde_json::json!({}));
        loader.register(Box::new(UppercasePlugin { prio: 30 }), serde_json::json!({}));

        let stages = loader.get_stages();
        let priorities: Vec<u32> = stages.iter().map(|s| s.priority()).collect();
        assert_eq!(priorities, vec![10, 30, 50]);
    }

    #[test]
    fn discover_and_load_empty_dir_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut loader = PluginLoader::new(dir.path());
        let manifests = loader.discover_and_load().unwrap();
        assert!(manifests.is_empty());
    }

    #[test]
    fn discover_and_load_nonexistent_dir_returns_empty() {
        let mut loader = PluginLoader::new(Path::new("/tmp/sqz_nonexistent_plugin_dir_xyz"));
        let manifests = loader.discover_and_load().unwrap();
        assert!(manifests.is_empty());
    }

    #[test]
    fn discover_and_load_reads_toml_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let toml_content = r#"
[plugin]
name = "test-plugin"
version = "1.0.0"
description = "A test plugin"
author = "tester"
priority = 42

[plugin.config]
key = "value"
"#;
        std::fs::write(dir.path().join("test-plugin.toml"), toml_content).unwrap();

        let mut loader = PluginLoader::new(dir.path());
        let manifests = loader.discover_and_load().unwrap();

        assert_eq!(manifests.len(), 1);
        let m = &manifests[0];
        assert_eq!(m.name, "test-plugin");
        assert_eq!(m.version, "1.0.0");
        assert_eq!(m.priority, 42);
        assert_eq!(m.config["key"], "value");
    }

    #[test]
    fn plugin_stage_disabled_is_noop() {
        let mut loader = PluginLoader::new(Path::new("/tmp/nonexistent"));
        loader.register(
            Box::new(UppercasePlugin { prio: 10 }),
            serde_json::json!({}),
        );

        let stages = loader.get_stages();
        let mut content = plain_content("hello");
        let disabled = StageConfig {
            enabled: false,
            options: serde_json::json!({}),
        };
        stages[0].process(&mut content, &disabled).unwrap();
        assert_eq!(content.raw, "hello"); // unchanged
    }
}

// ---------------------------------------------------------------------------
// Property-based tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod prop_tests {
    use super::*;
    use proptest::prelude::*;

    // A minimal plugin whose only interesting attribute is its priority.
    struct PriorityPlugin {
        prio: u32,
        id: String,
    }

    impl SqzPlugin for PriorityPlugin {
        fn name(&self) -> &str {
            &self.id
        }
        fn version(&self) -> &str {
            "0.1.0"
        }
        fn priority(&self) -> u32 {
            self.prio
        }
        fn compress(
            &self,
            _content: &mut Content,
            _config: &serde_json::Value,
        ) -> Result<()> {
            Ok(())
        }
    }

    /// **Property 24: Plugin priority ordering**
    ///
    /// For any set of loaded plugins with distinct priorities,
    /// `get_stages()` SHALL return them sorted in ascending priority order
    /// (lower priority number = earlier execution).
    ///
    /// **Validates: Requirements 15.4**
    #[test]
    fn prop_plugin_priority_ordering() {
        // Generate between 1 and 10 distinct priorities
        proptest!(|(mut priorities in proptest::collection::vec(0u32..1000, 1..=10))| {
            // Deduplicate so all priorities are distinct
            priorities.sort();
            priorities.dedup();

            let mut loader = PluginLoader::new(Path::new("/tmp/nonexistent"));
            // Register plugins in *reverse* order to ensure sorting is not
            // an accident of insertion order.
            for (i, &prio) in priorities.iter().enumerate().rev() {
                loader.register(
                    Box::new(PriorityPlugin {
                        prio,
                        id: format!("plugin-{i}"),
                    }),
                    serde_json::json!({}),
                );
            }

            let stages = loader.get_stages();
            let returned_priorities: Vec<u32> = stages.iter().map(|s| s.priority()).collect();

            // Must be sorted ascending
            let mut expected = returned_priorities.clone();
            expected.sort();
            prop_assert_eq!(returned_priorities, expected,
                "get_stages() must return stages in ascending priority order");
        });
    }
}
