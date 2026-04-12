use crate::error::Result;
use crate::stages::CompressionStage;
use crate::types::{Content, StageConfig};

/// Pipeline stage 0: strips ANSI escape sequences from content.
///
/// Removes SGR (colors/styles), cursor movement, erase sequences, and OSC
/// sequences while preserving all semantic text content.  Runs before every
/// other compression stage so that downstream stages never see raw escape
/// codes.
pub struct AnsiStripper;

impl CompressionStage for AnsiStripper {
    fn name(&self) -> &str {
        "ansi_strip"
    }

    fn priority(&self) -> u32 {
        0 // runs before all other stages (lowest priority number)
    }

    fn process(&self, content: &mut Content, _config: &StageConfig) -> Result<()> {
        content.raw = strip_ansi(&content.raw);
        Ok(())
    }
}

/// Strip all ANSI escape sequences from `input`, preserving semantic text.
///
/// Handles:
/// - CSI sequences  (ESC `[` … final byte 0x40–0x7E) — covers SGR, cursor
///   movement, erase line/screen, scroll, etc.
/// - OSC sequences   (ESC `]` … ST) where ST is either ESC `\` or BEL (0x07).
/// - Two-byte escape sequences (ESC followed by a byte in 0x40–0x5F that is
///   *not* `[` or `]`).
///
/// Uses a simple state machine so there are no regex or extra dependencies.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Start of an escape sequence
            match chars.peek() {
                Some('[') => {
                    // CSI sequence: ESC [ <params> <final byte 0x40-0x7E>
                    chars.next(); // consume '['
                    // Skip parameter bytes (0x30–0x3F) and intermediate bytes (0x20–0x2F)
                    // until we hit the final byte (0x40–0x7E) or run out of input.
                    for c in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC sequence: ESC ] … ST
                    // ST is ESC \ or BEL (0x07)
                    chars.next(); // consume ']'
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break; // BEL terminator
                        }
                        if c == '\x1b' {
                            // Check for ST = ESC '\'
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                Some(&c) if ('\x40'..='\x5f').contains(&c) => {
                    // Other two-byte escape (e.g. ESC D, ESC M, ESC 7, ESC 8)
                    chars.next();
                }
                _ => {
                    // Bare ESC with no recognized follower — drop it
                }
            }
        } else {
            out.push(ch);
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentMetadata, ContentType};

    fn text_content(raw: &str) -> Content {
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

    // --- strip_ansi unit tests ---

    #[test]
    fn strips_sgr_color_codes() {
        // Bold red "hello" then reset
        let input = "\x1b[1;31mhello\x1b[0m";
        assert_eq!(strip_ansi(input), "hello");
    }

    #[test]
    fn strips_multiple_sgr_sequences() {
        let input = "\x1b[32mgreen\x1b[0m and \x1b[34mblue\x1b[0m";
        assert_eq!(strip_ansi(input), "green and blue");
    }

    #[test]
    fn strips_cursor_movement() {
        // Cursor up 2 lines, then text
        let input = "\x1b[2Ahello";
        assert_eq!(strip_ansi(input), "hello");
    }

    #[test]
    fn strips_erase_sequences() {
        // Erase entire line, then text
        let input = "\x1b[2Koutput here";
        assert_eq!(strip_ansi(input), "output here");
    }

    #[test]
    fn strips_osc_with_bel_terminator() {
        // OSC to set window title, terminated by BEL
        let input = "\x1b]0;My Title\x07real content";
        assert_eq!(strip_ansi(input), "real content");
    }

    #[test]
    fn strips_osc_with_st_terminator() {
        // OSC terminated by ESC backslash (ST)
        let input = "\x1b]0;My Title\x1b\\real content";
        assert_eq!(strip_ansi(input), "real content");
    }

    #[test]
    fn preserves_plain_text() {
        let input = "no escape codes here\njust plain text";
        assert_eq!(strip_ansi(input), input);
    }

    #[test]
    fn preserves_empty_string() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn strips_mixed_sequences_preserves_text() {
        let input = "\x1b[1m\x1b[33mWARNING:\x1b[0m something happened\n\x1b[2Kerror: bad input\x1b[0m";
        assert_eq!(strip_ansi(input), "WARNING: something happened\nerror: bad input");
    }

    #[test]
    fn strips_two_byte_escape_sequences() {
        // ESC D = index (scroll up)
        let input = "before\x1bDafter";
        assert_eq!(strip_ansi(input), "beforeafter");
    }

    #[test]
    fn handles_bare_esc_at_end_of_input() {
        let input = "text\x1b";
        assert_eq!(strip_ansi(input), "text");
    }

    // --- CompressionStage trait tests ---

    #[test]
    fn stage_name_is_ansi_strip() {
        assert_eq!(AnsiStripper.name(), "ansi_strip");
    }

    #[test]
    fn stage_priority_is_zero() {
        assert_eq!(AnsiStripper.priority(), 0);
    }

    #[test]
    fn stage_process_strips_ansi_from_content() {
        let mut c = text_content("\x1b[31mred text\x1b[0m");
        let cfg = StageConfig {
            enabled: true,
            options: serde_json::Value::Object(Default::default()),
        };
        AnsiStripper.process(&mut c, &cfg).unwrap();
        assert_eq!(c.raw, "red text");
    }

    #[test]
    fn stage_process_preserves_clean_content() {
        let mut c = text_content("clean text");
        let cfg = StageConfig {
            enabled: true,
            options: serde_json::Value::Object(Default::default()),
        };
        AnsiStripper.process(&mut c, &cfg).unwrap();
        assert_eq!(c.raw, "clean text");
    }
}
