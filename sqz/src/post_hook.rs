//! Claude Code PostToolUse: shrink a successful Bash result that the
//! PreToolUse rewrite left alone.

use crate::cli_proxy::{catch_panic, CliProxy, InterceptOptions};

/// Below this many tokens saved, the original result is kept.
const MIN_SAVING_TOKENS: usize = 16;

/// The hook's JSON reply, or `None` to leave the result as it is.
pub fn claude_post_tool_use(input: &str) -> Option<String> {
    let target = sqz_engine::claude_post_tool_use_target(input)?;
    let proxy = CliProxy::new().ok()?;
    let opts = InterceptOptions::from_env();
    let mut response = target.response;
    let mut changed = false;
    for field in ["stdout", "stderr"] {
        let Some(text) = response.get(field).and_then(|v| v.as_str()) else { continue };
        if text.is_empty() {
            continue;
        }
        let compressed =
            catch_panic(|| proxy.intercept_output_with_options(&target.label, text, opts))?;
        if text.len().saturating_sub(compressed.len()) / 4 >= MIN_SAVING_TOKENS {
            response.insert(field.to_string(), serde_json::Value::String(compressed));
            changed = true;
        }
    }
    if !changed {
        return None;
    }
    let reply = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "updatedToolOutput": response,
        }
    });
    serde_json::to_string(&reply).ok()
}
