//! Ruby ecosystem formatters: rspec, rubocop, rake/minitest, bundle.
//!
//! Execution-wrapping compressors can inject `--format json` at spawn time;
//! sqz is post-hoc and only sees whatever output already happened. So each
//! formatter tries JSON first (in case the user passed `--format json`
//! themselves) and falls back to a text parser.
//!
//! One module per runner. Shared helpers live here; child modules reach them
//! via `super::`.

mod bundle;
mod rake;
mod rspec;
mod rubocop;

pub use bundle::format_bundle;
pub use rake::format_rake;
pub use rspec::format_rspec;
pub use rubocop::format_rubocop;

/// Truncate to `max` chars, appending an ellipsis when cut. Shared by the
/// rspec and rake formatters for one-line message clamping.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut)
}
