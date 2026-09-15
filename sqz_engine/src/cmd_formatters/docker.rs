pub fn format_docker(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "ps" => Some(format_docker_ps(output)),
        "images" => Some(format_docker_images(output)),
        "logs" => None,
        _ => None,
    }
}

fn format_docker_ps(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.is_empty() { return output.to_string(); }

    // Fixed-width table: slice rows at the header's column offsets. Splitting
    // on whitespace breaks on COMMAND ("uvicorn app:app") and CREATED ("2 hours ago").
    let starts = column_starts(lines[0]);
    let header = slice_columns(lines[0], &starts);
    let col = |name: &str| header.iter().position(|h| h == name);
    let (Some(name_i), Some(image_i), Some(status_i)) = (col("NAMES"), col("IMAGE"), col("STATUS")) else {
        return output.to_string();
    };
    let ports_i = col("PORTS");

    let mut result = vec!["NAME | IMAGE | STATUS | PORTS".to_string()];
    for line in &lines[1..] {
        if line.trim().is_empty() { continue; }
        let cells = slice_columns(line, &starts);
        let cell = |i: usize| cells.get(i).map(String::as_str).unwrap_or("");
        let ports = ports_i.map(cell).unwrap_or("");
        result.push(format!("{} | {} | {} | {}", cell(name_i), cell(image_i), cell(status_i), ports));
    }
    result.join("\n")
}

fn column_starts(header: &str) -> Vec<usize> {
    let chars: Vec<char> = header.chars().collect();
    let mut starts = Vec::new();
    for i in 0..chars.len() {
        if chars[i] == ' ' { continue; }
        let gap_before = i == 0 || (i >= 2 && chars[i - 1] == ' ' && chars[i - 2] == ' ');
        if gap_before { starts.push(i); }
    }
    starts
}

fn slice_columns(line: &str, starts: &[usize]) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    starts
        .iter()
        .enumerate()
        .map(|(k, &s)| {
            let e = starts.get(k + 1).copied().unwrap_or(chars.len()).min(chars.len());
            if s >= e { String::new() } else { chars[s..e].iter().collect::<String>().trim().to_string() }
        })
        .collect()
}

fn format_docker_images(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.is_empty() { return output.to_string(); }

    let mut result = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            result.push("REPO:TAG | SIZE".to_string());
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 5 {
            let repo = parts[0];
            let tag = parts[1];
            let size = parts.last().unwrap_or(&"");
            result.push(format!("{}:{} | {}", repo, tag, size));
        }
    }
    result.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docker_ps_compact() {
        let output = "CONTAINER ID   IMAGE     COMMAND   CREATED   STATUS    PORTS     NAMES\nabc123def456   nginx     \"nginx\"   2h ago    Up 2h     80/tcp    web\n";
        let result = format_docker_ps(output);
        assert!(result.contains("NAME | IMAGE | STATUS"));
        assert!(result.contains("web | nginx | Up 2h | 80/tcp"));
    }

    #[test]
    fn test_docker_ps_columns_with_spaces() {
        let output = "CONTAINER ID   IMAGE          COMMAND                  CREATED        STATUS                          PORTS                    NAMES\n\
a1b2c3d4e5f6   api:local      \"uvicorn app:app --…\"    2 hours ago    Restarting (1) 12 seconds ago                            api-web-1\n\
3f1a2b4c5d6e   postgres:16    \"docker-entrypoint.s…\"   3 days ago     Up 3 days (healthy)             0.0.0.0:5432->5432/tcp   api-db-1\n";
        let result = format_docker_ps(output);
        assert!(result.contains("api-web-1 | api:local | Restarting (1) 12 seconds ago | "), "{result}");
        assert!(result.contains("api-db-1 | postgres:16 | Up 3 days (healthy) | 0.0.0.0:5432->5432/tcp"), "{result}");
        assert!(!result.contains("uvicorn"));
    }

    #[test]
    fn test_docker_ps_unknown_header_passes_through() {
        let output = "something unexpected\nrow one\n";
        assert_eq!(format_docker_ps(output), output);
    }
}
