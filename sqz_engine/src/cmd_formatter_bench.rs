//! Token-reduction benchmarks for the per-command formatters.
//!
//! Each case feeds a representative command output through `format_command`
//! and asserts a minimum reduction. The inputs here ARE the fixtures behind
//! the numbers quoted in `BENCHMARKS.md` — keep the two in sync. Print lines
//! (`cargo test -p sqz-engine cmd_formatter_bench -- --nocapture`) report the
//! exact measured reduction so the docs can be re-sourced after any change.

#[cfg(test)]
mod tests {
    use crate::cmd_formatters::format_command;

    fn tok(s: &str) -> usize {
        (s.len() + 3) / 4
    }

    /// Run a formatter, print the measured reduction, and gate on a floor.
    fn bench(name: &str, cmd: &str, input: &str, min_reduction: f64) {
        let out = format_command(cmd, input)
            .unwrap_or_else(|| panic!("[{name}] no formatter matched `{cmd}`"));
        let (ti, to) = (tok(input), tok(&out));
        let pct = 100.0 - (to as f64 / ti.max(1) as f64 * 100.0);
        println!("[cmd-bench] {name}: {ti}->{to} tokens = {pct:.0}% reduction");
        assert!(
            pct >= min_reduction,
            "[{name}] reduction {pct:.0}% < floor {min_reduction:.0}% (in={ti}, out={to})"
        );
    }

    fn cargo_test_15_pass() -> String {
        let mut s = String::new();
        for suite in 0..3 {
            s.push_str("running 5 tests\n");
            for i in 0..5 {
                s.push_str(&format!("test suite{suite}::case_{i} ... ok\n"));
            }
            s.push_str("test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n\n");
        }
        s
    }

    fn cargo_build_success() -> String {
        let mut s = String::new();
        for i in 0..30 {
            s.push_str(&format!("   Compiling crate_{i} v0.1.0\n"));
        }
        s.push_str("    Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.2s\n");
        s
    }

    fn cargo_clippy_5_warn() -> String {
        let mut s = String::new();
        for i in 0..5 {
            s.push_str(&format!(
                "warning: unused variable: `x{i}`\n --> src/lib.rs:{}:9\n  |\n{} |     let x{i} = 1;\n  |         ^^ help: prefix with underscore\n\n",
                i + 10, i + 10
            ));
        }
        s.push_str("warning: `mycrate` generated 5 warnings\n");
        s
    }

    fn npm_install_200() -> String {
        let mut s = String::new();
        for i in 0..200 {
            s.push_str(&format!("added package_{i}@1.0.0\n"));
        }
        s.push_str("added 200 packages, and audited 201 packages in 3s\n");
        s
    }

    fn git_status_10() -> String {
        "On branch main\nYour branch is up to date with 'origin/main'.\n\n\
         Changes to be committed:\n  (use \"git restore --staged <file>...\")\n\
         \tmodified:   src/a.rs\n\tnew file:   src/b.rs\n\tnew file:   src/c.rs\n\n\
         Changes not staged for commit:\n  (use \"git add <file>...\")\n\
         \tmodified:   src/d.rs\n\tmodified:   src/e.rs\n\tdeleted:    src/f.rs\n\n\
         Untracked files:\n  (use \"git add <file>...\")\n\
         \tsrc/g.rs\n\tsrc/h.rs\n\tsrc/i.rs\n\tsrc/j.rs\n".to_string()
    }

    fn grep_100() -> String {
        let mut s = String::new();
        for i in 0..100 {
            s.push_str(&format!("src/file{}.rs:{}:    let result = compute_value(input);\n", i % 5, i + 1));
        }
        s
    }

    fn terraform_plan() -> String {
        "Terraform used the selected providers to generate the following execution plan.\n\
         Resource actions are indicated with the following symbols:\n  + create\n  ~ update in-place\n  - destroy\n\n\
         Terraform will perform the following actions:\n\n\
         # aws_instance.web will be created\n  + resource \"aws_instance\" \"web\" {\n      + ami = \"ami-123\"\n    }\n\
         # aws_s3_bucket.data will be created\n  + resource \"aws_s3_bucket\" \"data\" {\n      + bucket = \"my-data\"\n    }\n\n\
         Plan: 2 to add, 0 to change, 0 to destroy.\n".to_string()
    }

    #[test]
    fn bench_cargo_test_all_pass() {
        bench("cargo test (15 pass, 3 suites)", "cargo test", &cargo_test_15_pass(), 85.0);
    }

    #[test]
    fn bench_cargo_build_success() {
        bench("cargo build (success, 30 crates)", "cargo build", &cargo_build_success(), 80.0);
    }

    #[test]
    fn bench_cargo_clippy_warnings() {
        bench("cargo clippy (5 warnings)", "cargo clippy", &cargo_clippy_5_warn(), 50.0);
    }

    #[test]
    fn bench_npm_install() {
        bench("npm install (200 packages)", "npm install", &npm_install_200(), 90.0);
    }

    #[test]
    fn bench_git_status_verbose() {
        bench("git status (10 files)", "git status", &git_status_10(), 40.0);
    }

    #[test]
    fn bench_grep_many_matches() {
        bench("grep (100 matches, 5 files)", "grep foo", &grep_100(), 70.0);
    }

    #[test]
    fn bench_terraform_plan() {
        bench("terraform plan (2 resources)", "terraform plan", &terraform_plan(), 40.0);
    }

    /// Re-measure every fixture with the real cl100k BPE tokenizer and
    /// compare against the chars/4 unit the table above uses. Guards the
    /// claim that reported reductions are not an artifact of the token
    /// estimate: the two aggregates must agree within 10 points.
    #[test]
    fn bench_reductions_hold_under_real_bpe() {
        let cases: Vec<(&str, &str, String)> = vec![
            ("cargo test", "cargo test", cargo_test_15_pass()),
            ("cargo build", "cargo build", cargo_build_success()),
            ("cargo clippy", "cargo clippy", cargo_clippy_5_warn()),
            ("npm install", "npm install", npm_install_200()),
            ("git status", "git status", git_status_10()),
            ("grep", "grep foo", grep_100()),
            ("terraform plan", "terraform plan", terraform_plan()),
        ];

        let counter = crate::token_counter::TokenCounter::new();
        let bpe = |s: &str| counter.count(s, &crate::types::ModelFamily::AnthropicClaude) as usize;

        let (mut in4, mut out4, mut inb, mut outb) = (0usize, 0usize, 0usize, 0usize);
        for (name, cmd, input) in &cases {
            let out = format_command(cmd, input)
                .unwrap_or_else(|| panic!("[{name}] no formatter matched"));
            in4 += tok(input);
            out4 += tok(&out);
            inb += bpe(input);
            outb += bpe(&out);
        }

        let red4 = 100.0 - (out4 as f64 / in4.max(1) as f64 * 100.0);
        let redb = 100.0 - (outb as f64 / inb.max(1) as f64 * 100.0);
        println!(
            "[bpe-verify] chars/4: {in4}->{out4} = {red4:.1}% | cl100k: {inb}->{outb} = {redb:.1}% | divergence {:.1} pts",
            (red4 - redb).abs()
        );
        assert!(
            (red4 - redb).abs() < 10.0,
            "reduction under real BPE ({redb:.1}%) diverges from chars/4 ({red4:.1}%) by ≥10 points"
        );
        assert!(redb > 50.0, "aggregate BPE reduction should stay substantial, got {redb:.1}%");
    }
}
