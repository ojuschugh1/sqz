pub fn format_tsc(output: &str) -> String {
    let mut by_file: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    let mut error_count = 0;
    let mut found_line = None;

    for line in output.lines() {
        // `file(line,col): error TS…` when piped, `file:line:col - error TS…` with --pretty
        if line.contains("): error TS") || line.contains("): warning TS") {
            error_count += 1;
            let file = line.find('(').map(|p| &line[..p]).unwrap_or("unknown");
            by_file.entry(file.to_string()).or_default().push(line.to_string());
        } else if line.contains(" - error TS") || line.contains(" - warning TS") {
            error_count += 1;
            let loc = line.find(" - ").map(|p| &line[..p]).unwrap_or(line);
            let file = loc.rsplitn(3, ':').last().unwrap_or(loc);
            by_file.entry(file.to_string()).or_default().push(line.to_string());
        } else if line.starts_with("Found ") && line.contains("error") {
            found_line = Some(line.trim_end_matches('.').to_string());
        }
    }

    if error_count == 0 {
        if output.contains("Found 0 errors") || output.trim().is_empty() {
            return "ok: 0 errors".to_string();
        }
        return output.to_string();
    }

    let mut result = Vec::new();
    result.push(found_line.unwrap_or_else(|| format!("Found {} errors in {} files", error_count, by_file.len())));
    for (file, errors) in &by_file {
        result.push(format!("  {} ({}):", file, errors.len()));
        for e in errors.iter().take(5) {
            result.push(format!("    {}", e));
        }
        if errors.len() > 5 {
            result.push(format!("    ...+{} more", errors.len() - 5));
        }
    }
    result.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tsc_no_errors() {
        let result = format_tsc("Found 0 errors.\n");
        assert_eq!(result, "ok: 0 errors");
    }

    #[test]
    fn test_tsc_piped_format_groups_by_file() {
        let output = "src/a.ts(3,5): error TS2322: Type 'string' is not assignable to type 'number'.\n\
src/a.ts(9,1): error TS2304: Cannot find name 'foo'.\n\
src/b.ts(1,1): error TS1005: ';' expected.\n";
        let result = format_tsc(output);
        assert!(result.starts_with("Found 3 errors in 2 files"), "{result}");
        assert!(result.contains("  src/a.ts (2):"));
        assert!(result.contains("TS2304"));
    }

    #[test]
    fn test_tsc_pretty_format_drops_source_excerpts() {
        let output = "src/billing/refund.ts:41:23 - error TS2345: Argument of type 'string' is not assignable to parameter of type 'number'.\n\n\
41     return this.gateway.refund(charge.id, amountCents);\n                         ~~~~~~~~~~~\n\n\
src/billing/ledger.ts:12:5 - error TS2322: Type 'Refund | undefined' is not assignable to type 'Refund'.\n\n\
12     this.entries.push(refund);\n       ~~~~~~~~~~~~~~~~~~~~~~~~~~\n\n\nFound 2 errors in 2 files.\n\nErrors  Files\n     1  src/billing/refund.ts:41\n     1  src/billing/ledger.ts:12\n";
        let result = format_tsc(output);
        assert!(result.starts_with("Found 2 errors in 2 files\n"), "{result}");
        assert!(result.contains("  src/billing/refund.ts (1):"), "{result}");
        assert!(result.contains("src/billing/ledger.ts:12:5 - error TS2322"));
        assert!(!result.contains("~~~~"));
        assert!(!result.contains("Errors  Files"));
    }
}
