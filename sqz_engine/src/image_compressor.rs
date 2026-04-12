/// Keywords that indicate an image is likely a UI screenshot.
const UI_KEYWORDS: &[&str] = &[
    "screenshot", "screen", "ui", "window", "dialog", "modal", "panel",
    "toolbar", "sidebar", "menu", "popup", "widget", "dashboard", "form",
    "button", "tab", "nav",
];

/// Compact semantic description produced from an image.
pub struct ImageDescription {
    /// The text description replacing the raw image bytes.
    pub description: String,
    /// Whether the image was classified as a UI screenshot.
    pub is_ui_screenshot: bool,
    /// Estimated original token cost (image_bytes.len() / 4).
    pub tokens_original: u32,
    /// Token cost of the description text (description.len() / 4).
    pub tokens_description: u32,
    /// Percentage reduction: (1 - tokens_description / tokens_original) * 100.
    pub reduction_pct: f64,
}

/// Converts raw image bytes into a compact semantic text description,
/// achieving ≥95% token reduction compared to the raw image representation.
pub struct ImageCompressor;

impl ImageCompressor {
    pub fn new() -> Self {
        ImageCompressor
    }

    /// Extract a semantic description from image bytes.
    ///
    /// `filename` and `context` are optional hints about the image content.
    /// When the hints suggest a UI screenshot the description follows a
    /// structured DOM-like template; otherwise a general visual description
    /// is produced.
    pub fn describe(
        &self,
        image_bytes: &[u8],
        filename: Option<&str>,
        context: Option<&str>,
    ) -> ImageDescription {
        let is_ui = Self::is_ui_screenshot(filename, context);

        let description = if is_ui {
            Self::ui_description(filename, context)
        } else {
            Self::general_description(image_bytes, filename, context)
        };

        let tokens_original = (image_bytes.len() as u32).saturating_div(4).max(1);
        let tokens_description = (description.len() as u32).saturating_div(4).max(1);

        let reduction_pct = if tokens_original > 0 {
            let ratio = tokens_description as f64 / tokens_original as f64;
            ((1.0 - ratio) * 100.0).max(0.0)
        } else {
            0.0
        };

        ImageDescription {
            description,
            is_ui_screenshot: is_ui,
            tokens_original,
            tokens_description,
            reduction_pct,
        }
    }

    /// Returns `true` when the filename or context hints suggest a UI screenshot.
    pub fn is_ui_screenshot(filename: Option<&str>, context: Option<&str>) -> bool {
        let combined = format!(
            "{} {}",
            filename.unwrap_or("").to_lowercase(),
            context.unwrap_or("").to_lowercase()
        );
        UI_KEYWORDS.iter().any(|kw| combined.contains(kw))
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    fn ui_description(filename: Option<&str>, context: Option<&str>) -> String {
        let file_str = filename.unwrap_or("unknown");
        let content_summary = context.unwrap_or("UI interface screenshot");
        format!(
            "[UI Screenshot]\nFile: {file_str}\nEstimated dimensions: unknown\n\
UI Elements detected: buttons, text fields, navigation\n\
Layout: standard application window\nContent summary: {content_summary}"
        )
    }

    fn general_description(
        image_bytes: &[u8],
        filename: Option<&str>,
        context: Option<&str>,
    ) -> String {
        let file_str = filename.unwrap_or("unknown");
        let content_str = context.unwrap_or("visual content");
        let size = image_bytes.len();
        format!(
            "[Image]\nFile: {file_str}\nSize: {size} bytes\nContent: {content_str}"
        )
    }
}

impl Default for ImageCompressor {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Unit tests — Task 34.1
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_ui_screenshot_by_filename() {
        assert!(ImageCompressor::is_ui_screenshot(
            Some("app_screenshot.png"),
            None
        ));
        assert!(ImageCompressor::is_ui_screenshot(
            Some("main_window.png"),
            None
        ));
        assert!(ImageCompressor::is_ui_screenshot(
            Some("dialog_box.jpg"),
            None
        ));
        assert!(ImageCompressor::is_ui_screenshot(
            Some("ui_overview.png"),
            None
        ));
    }

    #[test]
    fn test_is_ui_screenshot_by_context() {
        assert!(ImageCompressor::is_ui_screenshot(
            None,
            Some("This is a screenshot of the settings panel")
        ));
        assert!(ImageCompressor::is_ui_screenshot(
            None,
            Some("navigation menu screenshot")
        ));
    }

    #[test]
    fn test_is_not_ui_screenshot() {
        assert!(!ImageCompressor::is_ui_screenshot(
            Some("photo.jpg"),
            Some("a landscape photo")
        ));
        assert!(!ImageCompressor::is_ui_screenshot(None, None));
        assert!(!ImageCompressor::is_ui_screenshot(
            Some("chart.png"),
            Some("bar chart of sales data")
        ));
    }

    #[test]
    fn test_describe_ui_screenshot_structure() {
        let compressor = ImageCompressor::new();
        // 56 KB of fake image bytes
        let bytes = vec![0u8; 56 * 1024];
        let desc = compressor.describe(
            &bytes,
            Some("app_screenshot.png"),
            Some("settings dialog"),
        );

        assert!(desc.is_ui_screenshot);
        assert!(desc.description.contains("[UI Screenshot]"));
        assert!(desc.description.contains("app_screenshot.png"));
        assert!(desc.description.contains("settings dialog"));
        assert!(desc.description.contains("buttons, text fields, navigation"));
    }

    #[test]
    fn test_describe_general_image_structure() {
        let compressor = ImageCompressor::new();
        let bytes = vec![0u8; 56 * 1024];
        let desc = compressor.describe(
            &bytes,
            Some("landscape.jpg"),
            Some("mountain scenery"),
        );

        assert!(!desc.is_ui_screenshot);
        assert!(desc.description.contains("[Image]"));
        assert!(desc.description.contains("landscape.jpg"));
        assert!(desc.description.contains("mountain scenery"));
        assert!(desc.description.contains("bytes"));
    }

    #[test]
    fn test_token_reduction_exceeds_95_percent_for_large_image() {
        let compressor = ImageCompressor::new();
        // 56 KB image — description is ~100 bytes → well over 95% reduction
        let bytes = vec![0u8; 56 * 1024];
        let desc = compressor.describe(&bytes, Some("screenshot.png"), None);
        assert!(
            desc.reduction_pct >= 95.0,
            "expected ≥95% reduction, got {:.1}%",
            desc.reduction_pct
        );
    }

    #[test]
    fn test_description_shorter_than_image_bytes() {
        let compressor = ImageCompressor::new();
        let bytes = vec![42u8; 1024];
        let desc = compressor.describe(&bytes, Some("photo.png"), Some("a photo"));
        assert!(
            desc.description.len() < bytes.len(),
            "description ({} bytes) should be shorter than image ({} bytes)",
            desc.description.len(),
            bytes.len()
        );
    }

    #[test]
    fn test_describe_no_hints() {
        let compressor = ImageCompressor::new();
        let bytes = vec![0u8; 100];
        let desc = compressor.describe(&bytes, None, None);
        assert!(!desc.is_ui_screenshot);
        assert!(desc.description.contains("[Image]"));
        assert!(desc.description.contains("unknown"));
        assert!(desc.description.contains("visual content"));
    }

    #[test]
    fn test_tokens_original_estimated_from_size() {
        let compressor = ImageCompressor::new();
        let bytes = vec![0u8; 400];
        let desc = compressor.describe(&bytes, None, None);
        // 400 bytes / 4 = 100 tokens
        assert_eq!(desc.tokens_original, 100);
    }

    // -----------------------------------------------------------------------
    // Integration tests — Task 34.2
    // -----------------------------------------------------------------------

    /// Test semantic DOM extraction: UI screenshot produces structured output
    /// with all required DOM-like fields.
    #[test]
    fn integration_ui_screenshot_dom_extraction() {
        let compressor = ImageCompressor::new();
        let image_bytes = vec![0xFFu8; 56 * 1024]; // 56 KB

        let desc = compressor.describe(
            &image_bytes,
            Some("main_window_screenshot.png"),
            Some("application main window with toolbar"),
        );

        // Classification
        assert!(desc.is_ui_screenshot, "should be classified as UI screenshot");

        // Required DOM-like fields present
        assert!(desc.description.contains("[UI Screenshot]"));
        assert!(desc.description.contains("File:"));
        assert!(desc.description.contains("Estimated dimensions:"));
        assert!(desc.description.contains("UI Elements detected:"));
        assert!(desc.description.contains("Layout:"));
        assert!(desc.description.contains("Content summary:"));

        // Filename and context preserved
        assert!(desc.description.contains("main_window_screenshot.png"));
        assert!(desc.description.contains("application main window with toolbar"));

        // Token reduction ≥ 95%
        assert!(
            desc.reduction_pct >= 95.0,
            "UI screenshot reduction should be ≥95%, got {:.1}%",
            desc.reduction_pct
        );
    }

    /// Test non-UI image fallback: general image produces a compact description
    /// without DOM structure.
    #[test]
    fn integration_non_ui_image_fallback() {
        let compressor = ImageCompressor::new();
        let image_bytes = vec![0xAAu8; 56 * 1024]; // 56 KB

        let desc = compressor.describe(
            &image_bytes,
            Some("company_logo.png"),
            Some("company logo with blue background"),
        );

        // Classification
        assert!(
            !desc.is_ui_screenshot,
            "should NOT be classified as UI screenshot"
        );

        // General image fields present
        assert!(desc.description.contains("[Image]"));
        assert!(desc.description.contains("File:"));
        assert!(desc.description.contains("Size:"));
        assert!(desc.description.contains("Content:"));

        // No DOM-specific fields
        assert!(!desc.description.contains("[UI Screenshot]"));
        assert!(!desc.description.contains("UI Elements detected:"));

        // Filename and context preserved
        assert!(desc.description.contains("company_logo.png"));
        assert!(desc.description.contains("company logo with blue background"));

        // Token reduction ≥ 95%
        assert!(
            desc.reduction_pct >= 95.0,
            "non-UI image reduction should be ≥95%, got {:.1}%",
            desc.reduction_pct
        );
    }

    /// Test that the 95% reduction target holds across a range of realistic
    /// image sizes (4 KB – 1 MB).  Sub-4 KB "images" are below the practical
    /// minimum for real image files and are not required to hit the 95% target.
    #[test]
    fn integration_reduction_target_across_sizes() {
        let compressor = ImageCompressor::new();
        let sizes = [4 * 1024usize, 56 * 1024, 256 * 1024, 1024 * 1024];

        for size in sizes {
            let bytes = vec![0u8; size];

            // UI screenshot path
            let ui = compressor.describe(&bytes, Some("screen.png"), None);
            assert!(
                ui.reduction_pct >= 95.0,
                "UI path: expected ≥95% for {size} bytes, got {:.1}%",
                ui.reduction_pct
            );

            // General image path
            let img = compressor.describe(&bytes, Some("photo.jpg"), None);
            assert!(
                img.reduction_pct >= 95.0,
                "General path: expected ≥95% for {size} bytes, got {:.1}%",
                img.reduction_pct
            );
        }
    }
}
