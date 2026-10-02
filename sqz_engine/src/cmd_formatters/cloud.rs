// aws and gcloud are left to the generic pipeline; terraform drops only progress lines.

pub fn format_terraform(subcmd: Option<&str>, output: &str) -> Option<String> {
    match subcmd? {
        "plan" | "apply" | "destroy" | "refresh" => drop_lines(output, is_tf_progress),
        "init" => drop_lines(output, is_tf_init_noise),
        _ => None,
    }
}

/// `output` without the lines `noise` matches, or None when nothing matched.
fn drop_lines(output: &str, noise: fn(&str) -> bool) -> Option<String> {
    let kept: Vec<&str> = output.lines().filter(|l| !noise(l.trim())).collect();
    if kept.len() == output.lines().count() {
        return None;
    }
    Some(kept.join("\n"))
}

/// Per-resource progress: `aws_instance.web: Refreshing state... [id=i-1]`,
/// `Reading...`, `Read complete after 1s`, `Still creating... [10s elapsed]`.
fn is_tf_progress(line: &str) -> bool {
    let Some((_, status)) = line.split_once(": ") else {
        return false;
    };
    status.starts_with("Refreshing state...")
        || status.starts_with("Reading...")
        || status.starts_with("Read complete after")
        || (status.starts_with("Still ") && status.ends_with(" elapsed]"))
}

fn is_tf_init_noise(line: &str) -> bool {
    line.starts_with("- Downloading")
        || line.starts_with("- Installing")
        || line.starts_with("- Finding")
        || line.starts_with("- Using previously-installed")
        || line.starts_with("- Reusing previous version")
        || line.starts_with("- Installed ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tf_plan_keeps_the_plan() {
        let output = "aws_instance.web: Refreshing state... [id=i-0abc]\n\
                      data.aws_ami.ubuntu: Reading...\n\
                      data.aws_ami.ubuntu: Read complete after 1s [id=ami-123]\n\n\
                      Terraform will perform the following actions:\n\n\
                      # aws_instance.web must be replaced\n\
                      -/+ resource \"aws_instance\" \"web\" {\n\
                      ~ ami = \"ami-old\" -> \"ami-123\" # forces replacement\n    }\n\n\
                      Plan: 1 to add, 0 to change, 1 to destroy.\n";
        let result = format_terraform(Some("plan"), output).unwrap();
        assert!(!result.contains("Refreshing state"), "{result}");
        assert!(!result.contains("Read complete"), "{result}");
        assert!(result.contains("# forces replacement"), "{result}");
        assert!(result.contains("Plan: 1 to add, 0 to change, 1 to destroy."), "{result}");
    }

    #[test]
    fn test_tf_apply_keeps_outputs_and_errors() {
        let output = "aws_instance.web: Creating...\n\
                      aws_instance.web: Still creating... [10s elapsed]\n\
                      aws_instance.web: Creation complete after 12s [id=i-0abc]\n\n\
                      Apply complete! Resources: 1 added, 0 changed, 0 destroyed.\n\n\
                      Outputs:\n\nendpoint = \"10.0.0.7\"\n";
        let result = format_terraform(Some("apply"), output).unwrap();
        assert!(!result.contains("Still creating"), "{result}");
        assert!(result.contains("Creation complete after 12s [id=i-0abc]"));
        assert!(result.contains("endpoint = \"10.0.0.7\""), "{result}");

        let failed = "Error: creating EC2 Instance: InvalidAMIID.NotFound\n\n  with aws_instance.web,\n";
        assert!(format_terraform(Some("apply"), failed).is_none(), "nothing to drop");
    }

    #[test]
    fn test_tf_init_drops_downloads_keeps_warnings() {
        let output = "Initializing the backend...\n\
                      - Finding hashicorp/aws versions matching \"~> 5.0\"...\n\
                      - Installing hashicorp/aws v5.1.0...\n\
                      - Installed hashicorp/aws v5.1.0 (signed by HashiCorp)\n\n\
                      Warning: Incomplete lock file information for providers\n\n\
                      Terraform has been successfully initialized!\n";
        let result = format_terraform(Some("init"), output).unwrap();
        assert!(!result.contains("Installing"), "{result}");
        assert!(result.contains("Warning: Incomplete lock file"), "{result}");
        assert!(result.contains("successfully initialized"), "{result}");
    }

    #[test]
    fn test_tf_no_changes_passes_through() {
        assert!(format_terraform(Some("plan"), "No changes. Infrastructure is up-to-date.\n").is_none());
    }
}
