// get and logs are left to the generic pipeline, which keeps every field, row and error line.
pub fn format_kubectl(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "describe" => Some(format_kubectl_describe(output)),
        "apply" => format_kubectl_apply(output),
        _ => None,
    }
}

const NORMAL_EVENTS_KEPT: usize = 5;

/// Caps routine Normal events; Warning events are always kept.
fn format_kubectl_describe(output: &str) -> String {
    let mut result = Vec::new();
    let mut in_events = false;
    let mut normal_events = 0;

    for line in output.lines() {
        if line.starts_with("Events:") {
            in_events = true;
            result.push(line.to_string());
            continue;
        }
        if in_events {
            if line.starts_with("  ") || line.starts_with('\t') {
                let first = line.split_whitespace().next().unwrap_or("");
                if first == "Normal" {
                    normal_events += 1;
                    if normal_events > NORMAL_EVENTS_KEPT {
                        continue;
                    }
                }
                result.push(line.to_string());
                continue;
            }
            if normal_events > NORMAL_EVENTS_KEPT {
                result.push(format!("  ...+{} more Normal events", normal_events - NORMAL_EVENTS_KEPT));
            }
            in_events = false;
            normal_events = 0;
        }
        result.push(line.to_string());
    }
    if in_events && normal_events > NORMAL_EVENTS_KEPT {
        result.push(format!("  ...+{} more Normal events", normal_events - NORMAL_EVENTS_KEPT));
    }

    result.join("\n")
}

/// `unchanged` resources become a count; created and configured ones,
/// errors and warnings stay as written.
fn format_kubectl_apply(output: &str) -> Option<String> {
    let mut kept = Vec::new();
    let mut unchanged = 0;
    for line in output.lines() {
        if line.trim_end().ends_with(" unchanged") {
            unchanged += 1;
        } else {
            kept.push(line);
        }
    }
    if unchanged == 0 {
        return None;
    }
    let mut result = kept.join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    result.push_str(&format!("{unchanged} unchanged"));
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kubectl_get_is_left_to_the_pipeline() {
        let table = "NAME    READY   STATUS    RESTARTS   AGE\nnginx   1/1     Running   0          5d\n";
        assert!(format_kubectl(Some("get"), table).is_none());
        let json = r#"{"kind":"PodList","items":[{"metadata":{"name":"a"}},{"metadata":{"name":"b"}}]}"#;
        assert!(format_kubectl(Some("get"), json).is_none());
        let yaml = "apiVersion: v1\nkind: Pod\nmetadata:\n  name: web\n  labels:\n    app: web\n";
        assert!(format_kubectl(Some("get"), yaml).is_none());
        assert!(format_kubectl(Some("logs"), "line\n".repeat(200).as_str()).is_none());
    }

    #[test]
    fn test_kubectl_apply_counts_unchanged_only() {
        let output = "deployment.apps/web created\nservice/web configured\nconfigmap/env unchanged\nsecret/db unchanged\n";
        let result = format_kubectl_apply(output).unwrap();
        assert!(result.contains("deployment.apps/web created"));
        assert!(result.contains("service/web configured"));
        assert!(result.contains("2 unchanged"));
        assert!(!result.contains("configmap/env"));
    }

    #[test]
    fn test_kubectl_apply_keeps_errors() {
        let output = "configmap/env unchanged\nError from server (NotFound): error when creating \"web.yaml\": namespaces \"prod\" not found\n";
        let result = format_kubectl_apply(output).unwrap();
        assert!(result.contains("namespaces \"prod\" not found"), "{result}");
        assert!(format_kubectl_apply("deployment.apps/web created\n").is_none());
    }

    #[test]
    fn test_kubectl_describe_caps_normal_events_only() {
        let mut lines = vec!["Name: nginx".to_string(), "Events:".to_string()];
        for i in 0..20 {
            lines.push(format!("  Normal  Scheduled  {}m ago  default-scheduler  ok", i));
        }
        lines.push("  Warning  BackOff  1m ago  kubelet  Back-off restarting failed container".to_string());
        let output = lines.join("\n");
        let result = format_kubectl_describe(&output);
        assert!(result.contains("...+15 more Normal events"), "{result}");
        assert!(result.contains("Back-off restarting failed container"), "{result}");
    }
}
