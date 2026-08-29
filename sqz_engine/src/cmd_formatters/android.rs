//! adb logcat formatter. Log dumps are dominated by verbose/debug/info
//! chatter; the signal an agent needs is errors, fatals, and (capped)
//! warnings. Consecutive identical messages collapse to one line with a
//! repeat count.

use super::truncate::CAP_ERRORS;

const CAP_WARNINGS: usize = 10;

pub fn format_logcat(output: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    let mut warnings_kept = 0usize;
    let mut warnings_total = 0usize;
    let mut errors_total = 0usize;
    let mut dropped = 0usize;
    let mut last_kept_body: Option<String> = None;
    let mut repeat = 0usize;

    let flush_repeat = |kept: &mut Vec<String>, repeat: &mut usize| {
        if *repeat > 0 {
            if let Some(last) = kept.last_mut() {
                last.push_str(&format!(" [×{}]", *repeat + 1));
            }
            *repeat = 0;
        }
    };

    for line in output.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() || trimmed.starts_with("--------- beginning of") {
            continue;
        }
        match logcat_level(trimmed) {
            Some('E') | Some('F') => {
                errors_total += 1;
                let body = message_body(trimmed);
                if last_kept_body.as_deref() == Some(body) {
                    repeat += 1;
                    continue;
                }
                flush_repeat(&mut kept, &mut repeat);
                last_kept_body = Some(body.to_string());
                kept.push(trimmed.to_string());
            }
            Some('W') => {
                warnings_total += 1;
                if warnings_kept < CAP_WARNINGS {
                    let body = message_body(trimmed);
                    if last_kept_body.as_deref() == Some(body) {
                        repeat += 1;
                        continue;
                    }
                    flush_repeat(&mut kept, &mut repeat);
                    last_kept_body = Some(body.to_string());
                    kept.push(trimmed.to_string());
                    warnings_kept += 1;
                }
            }
            Some(_) => dropped += 1,
            // Continuation lines (stack frames under an E entry) have no
            // level column; keep them when they follow a kept line.
            None => {
                if last_kept_body.is_some() && kept.len() <= CAP_ERRORS * 8 {
                    flush_repeat(&mut kept, &mut repeat);
                    kept.push(trimmed.to_string());
                } else {
                    dropped += 1;
                }
            }
        }
    }
    flush_repeat(&mut kept, &mut repeat);

    if kept.is_empty() {
        return format!("logcat: no errors or warnings in {dropped} lines");
    }
    let mut out = format!(
        "logcat: {errors_total} error, {warnings_total} warning lines (dropped {dropped} verbose/debug/info)\n"
    );
    for line in &kept {
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Extract the level char from threadtime ("08-29 10:00:00.000 123 456 E Tag: msg")
/// or brief ("E/Tag( 123): msg") format lines.
fn logcat_level(line: &str) -> Option<char> {
    // Brief format: "E/Tag(  123): message"
    let mut chars = line.chars();
    let first = chars.next()?;
    if "VDIWEF".contains(first) && chars.next() == Some('/') {
        return Some(first);
    }
    // Threadtime: five whitespace-separated fields then the level.
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() >= 6
        && fields[4].len() == 1
        && "VDIWEF".contains(fields[4])
        && fields[2].chars().all(|c| c.is_ascii_digit())
        && fields[3].chars().all(|c| c.is_ascii_digit())
    {
        // fields: date time pid tid LEVEL tag...
        return fields[4].chars().next();
    }
    if fields.len() >= 5
        && fields[4].len() == 1
        && "VDIWEF".contains(fields[4])
    {
        return fields[4].chars().next();
    }
    None
}

/// The tag+message part, used for consecutive-duplicate collapsing so
/// the same message from different timestamps folds together.
fn message_body(line: &str) -> &str {
    match line.find(": ") {
        Some(idx) => &line[idx + 2..],
        None => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_errors_drops_chatter_counts_summary() {
        let output = "\
--------- beginning of main
08-29 10:00:01.000  1234  1234 D WindowManager: relayout w=1080 h=2400
08-29 10:00:01.100  1234  1234 I ActivityManager: Displayed com.app/.MainActivity
08-29 10:00:01.200  1234  1234 V AudioFlinger: mixer thread tick
08-29 10:00:02.000  1234  1234 E AndroidRuntime: FATAL EXCEPTION: main
08-29 10:00:02.001  1234  1234 E AndroidRuntime: java.lang.NullPointerException: token was null
08-29 10:00:02.500  1234  1234 W InputMethodManager: startInputReason unspecified
08-29 10:00:03.000  1234  1234 D WindowManager: relayout w=1080 h=2400
";
        let result = format_logcat(output);
        assert!(result.starts_with("logcat: 2 error, 1 warning lines"));
        assert!(result.contains("FATAL EXCEPTION"));
        assert!(result.contains("NullPointerException"));
        assert!(result.contains("InputMethodManager"));
        assert!(!result.contains("relayout"), "debug noise must be dropped");
        assert!(!result.contains("AudioFlinger"));
        assert!(result.len() < output.len());
    }

    #[test]
    fn consecutive_identical_errors_collapse() {
        let output = "\
08-29 10:00:01.000  1  1 E NetQueue: request timed out after 30s
08-29 10:00:02.000  1  1 E NetQueue: request timed out after 30s
08-29 10:00:03.000  1  1 E NetQueue: request timed out after 30s
";
        let result = format_logcat(output);
        assert!(result.contains("[×3]"), "{result}");
        assert_eq!(result.matches("request timed out").count(), 1);
    }

    #[test]
    fn brief_format_and_clean_log_handled() {
        let brief = "\
E/CameraService( 512): connect denied for uid 10077
D/Zygote(  100): fork child 4242
";
        let result = format_logcat(brief);
        assert!(result.contains("connect denied"));
        assert!(!result.contains("fork child"));

        let clean = "\
08-29 10:00:01.000  1  1 I Boot: step one done
08-29 10:00:01.100  1  1 D Boot: step two done
";
        assert!(format_logcat(clean).starts_with("logcat: no errors or warnings"));
    }
}
