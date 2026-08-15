//! UTF-8 boundary-safe slicing helpers.
//!
//! sqz slices user-controlled text (command output, session content,
//! function signatures) at computed byte offsets in many places. Direct
//! `&s[..n]` panics when `n` lands inside a multi-byte character — with
//! the shell hook in the pipeline, that panic swallows the user's entire
//! command output (issue #34, `Привет мир` + a `password:` marker).
//!
//! Every truncation of arbitrary text must go through these helpers.
//! Slicing is only safe to do directly when the surrounding code has
//! already proven the offset is a boundary (an ASCII-only match, a value
//! returned by `find()`, a hex hash).

/// Largest byte index `<= max_bytes` that is a `char` boundary of `s`.
///
/// `O(1)`: a UTF-8 sequence is at most 4 bytes, so the nearest boundary
/// is at most 3 bytes back.
#[inline]
pub fn floor_char_boundary(s: &str, max_bytes: usize) -> usize {
    if max_bytes >= s.len() {
        return s.len();
    }
    let mut i = max_bytes;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Boundary-safe version of `&s[..max_bytes]`: truncates to the largest
/// char boundary at or below `max_bytes`. Never panics.
#[inline]
pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    &s[..floor_char_boundary(s, max_bytes)]
}

/// Boundary-safe version of `s.split_at(mid)`. `mid` is rounded DOWN to
/// the nearest char boundary so the left side never exceeds `mid` bytes.
#[inline]
pub fn split_at_boundary_safe(s: &str, mid: usize) -> (&str, &str) {
    s.split_at(floor_char_boundary(s, mid))
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn ascii_passthrough() {
        assert_eq!(truncate_str("hello world", 5), "hello");
        assert_eq!(truncate_str("hello", 100), "hello");
        assert_eq!(truncate_str("", 5), "");
    }

    #[test]
    fn cyrillic_mid_char() {
        // "Привет" — every char is 2 bytes. Byte 7 splits 'в' (bytes 6..8).
        let s = "Привет мир";
        assert_eq!(truncate_str(s, 7), "При"); // 6 bytes, 3 chars
        assert_eq!(truncate_str(s, 6), "При");
        assert_eq!(truncate_str(s, 8), "Прив");
    }

    #[test]
    fn four_byte_emoji() {
        let s = "ab🦀cd"; // 🦀 = 4 bytes at offset 2..6
        assert_eq!(truncate_str(s, 3), "ab");
        assert_eq!(truncate_str(s, 4), "ab");
        assert_eq!(truncate_str(s, 5), "ab");
        assert_eq!(truncate_str(s, 6), "ab🦀");
    }

    #[test]
    fn split_never_panics_mid_char() {
        let s = "Привет";
        let (l, r) = split_at_boundary_safe(s, 7);
        assert_eq!(l, "При");
        assert_eq!(r, "вет");
        assert_eq!(format!("{l}{r}"), s);
    }

    proptest! {
        /// For ANY string and ANY cut point, truncate_str never panics,
        /// returns a prefix, and never exceeds the requested byte length.
        #[test]
        fn prop_truncate_safe(s in "\\PC*", n in 0usize..512) {
            let t = truncate_str(&s, n);
            prop_assert!(t.len() <= n.min(s.len()));
            prop_assert!(s.starts_with(t));
        }

        /// split_at_boundary_safe always reassembles to the original.
        #[test]
        fn prop_split_reassembles(s in "\\PC*", n in 0usize..512) {
            let (l, r) = split_at_boundary_safe(&s, n);
            let reassembled = format!("{l}{r}");
            let left_len = l.len();
            prop_assert_eq!(reassembled, s.clone());
            prop_assert!(left_len <= n.min(s.len()));
        }
    }
}
