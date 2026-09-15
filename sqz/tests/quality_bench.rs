//! Quality benchmark for the `sqz` binary.
//!
//! Every fixture is realistic command output with a list of facts an agent
//! would need from it. Each one is pushed through the real binary exactly
//! the way the shell hook does (`sqz compress --cmd <cmd>` on stdin) and we
//! measure, per fixture:
//!
//! - token reduction (chars/4, the same estimate sqz prints)
//! - critical-fact recall: facts that survive verbatim in the output the
//!   agent sees (stdout plus sqz's own stderr lines)
//! - noise dropped: lines we expect compression to remove
//! - verbatim preservation for content that must never be touched
//! - byte-exact recovery: a second run yields a `§ref:HASH§` and
//!   `sqz expand` returns the original bytes
//!
//! The asserts at the bottom are regression gates. Run with a table:
//!
//!   cargo test -p sqz-cli --test quality_bench -- --nocapture

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

struct Fixture {
    name: &'static str,
    cmd: &'static str,
    input: String,
    facts: &'static [&'static str],
    noise: &'static [&'static str],
    verbatim: bool,
    sensitive: bool,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Recovery {
    Exact,
    Mismatch,
    NotStored,
}

struct Outcome {
    name: &'static str,
    cmd: &'static str,
    tokens_in: usize,
    tokens_out: usize,
    facts_total: usize,
    facts_kept: usize,
    missing: Vec<&'static str>,
    noise_total: usize,
    noise_dropped: usize,
    verbatim_ok: Option<bool>,
    sensitive: bool,
    recovery: Recovery,
}

impl Outcome {
    fn reduction(&self) -> f64 {
        if self.tokens_in == 0 {
            return 0.0;
        }
        (1.0 - self.tokens_out as f64 / self.tokens_in as f64) * 100.0
    }
}

fn sqz_bin() -> PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.push("sqz");
    p
}

fn sqz(db: &Path, args: &[&str], stdin: Option<&[u8]>) -> Output {
    let mut cmd = Command::new(sqz_bin());
    cmd.args(args)
        .env("SQZ_DB_PATH", db)
        .env_remove("SQZ_NO_DEDUP")
        .env_remove("SQZ_NO_ABBREV")
        .env_remove("SQZ_CMD")
        .env("NO_COLOR", "1")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("failed to spawn sqz");
    if let Some(bytes) = stdin {
        child.stdin.take().unwrap().write_all(bytes).unwrap();
    }
    child.wait_with_output().unwrap()
}

fn tokens(s: &str) -> usize {
    s.len().div_ceil(4)
}

fn extract_ref(s: &str) -> Option<&str> {
    let start = s.find("§ref:")? + "§ref:".len();
    let rest = &s[start..];
    let end = rest.find('§')?;
    Some(&rest[..end])
}

fn measure(db: &Path, fx: &Fixture) -> Outcome {
    let first = sqz(db, &["compress", "--cmd", fx.cmd], Some(fx.input.as_bytes()));
    assert!(first.status.success(), "[{}] sqz compress failed: {}", fx.name, String::from_utf8_lossy(&first.stderr));
    let out = String::from_utf8_lossy(&first.stdout).to_string();
    let err = String::from_utf8_lossy(&first.stderr).to_string();
    // Everything the agent sees: compressed body plus sqz's own lines.
    let seen = format!("{out}{err}");

    let missing: Vec<&'static str> = fx.facts.iter().copied().filter(|f| !out.contains(f)).collect();
    let noise_dropped = fx.noise.iter().filter(|n| !out.contains(*n)).count();
    let verbatim_ok = fx.verbatim.then(|| out.trim_end() == fx.input.trim_end());

    let second = sqz(db, &["compress", "--cmd", fx.cmd], Some(fx.input.as_bytes()));
    let second_out = String::from_utf8_lossy(&second.stdout).to_string();
    let recovery = match extract_ref(&second_out) {
        Some(prefix) => {
            let expanded = sqz(db, &["expand", prefix], None);
            if expanded.stdout == fx.input.as_bytes() {
                Recovery::Exact
            } else {
                Recovery::Mismatch
            }
        }
        None => Recovery::NotStored,
    };

    Outcome {
        name: fx.name,
        cmd: fx.cmd,
        tokens_in: tokens(&fx.input),
        tokens_out: tokens(&seen),
        facts_total: fx.facts.len(),
        facts_kept: fx.facts.len() - missing.len(),
        missing,
        noise_total: fx.noise.len(),
        noise_dropped,
        verbatim_ok,
        sensitive: fx.sensitive,
        recovery,
    }
}

fn fx(
    name: &'static str,
    cmd: &'static str,
    input: impl Into<String>,
    facts: &'static [&'static str],
    noise: &'static [&'static str],
) -> Fixture {
    Fixture { name, cmd, input: input.into(), facts, noise, verbatim: false, sensitive: false }
}

fn verbatim(name: &'static str, cmd: &'static str, input: impl Into<String>, facts: &'static [&'static str]) -> Fixture {
    Fixture { name, cmd, input: input.into(), facts, noise: &[], verbatim: true, sensitive: false }
}

fn sensitive(name: &'static str, cmd: &'static str, input: impl Into<String>, facts: &'static [&'static str]) -> Fixture {
    Fixture { name, cmd, input: input.into(), facts, noise: &[], verbatim: true, sensitive: true }
}

fn app_log() -> String {
    let mut s = String::from("2026-09-14T10:21:58Z INFO  worker started pid=4412 queue=refunds\n");
    for i in 0..40 {
        s.push_str(&format!("2026-09-14T10:22:{:02}Z INFO  GET /healthz 200 1ms\n", i % 60));
    }
    s.push_str("2026-09-14T10:22:41Z WARN  retrying stripe call attempt=2 backoff=400ms\n");
    for i in 0..40 {
        s.push_str(&format!("2026-09-14T10:23:{:02}Z INFO  GET /healthz 200 1ms\n", i % 60));
    }
    s.push_str("2026-09-14T10:23:41Z ERROR refund failed order=ord_8812 code=card_declined\n");
    s.push_str("2026-09-14T10:23:41Z INFO  GET /healthz 200 1ms\n");
    s
}

fn fixtures() -> Vec<Fixture> {
    vec![
        fx(
            "cargo test failure",
            "cargo test",
            "   Compiling sqz-engine v1.6.1 (/work/sqz/sqz_engine)\n   Compiling sqz-cli v1.6.1 (/work/sqz/sqz)\n    Finished `test` profile [unoptimized + debuginfo] target(s) in 12.41s\n     Running unittests src/lib.rs (target/debug/deps/sqz_engine-9f3c2a1b)\n\nrunning 8 tests\ntest cache::tests::evicts_oldest_first ... ok\ntest cache::tests::hash_is_stable ... ok\ntest pipeline::tests::json_roundtrip ... ok\ntest pipeline::tests::strips_ansi ... ok\ntest session::tests::persists_across_reopen ... ok\ntest session::tests::regret_window_default ... ok\ntest tokens::tests::estimate_matches_tiktoken_within_5pct ... FAILED\ntest tokens::tests::zero_length ... ok\n\nfailures:\n\n---- tokens::tests::estimate_matches_tiktoken_within_5pct stdout ----\nthread 'tokens::tests::estimate_matches_tiktoken_within_5pct' panicked at sqz_engine/src/tokens.rs:88:9:\nassertion `left == right` failed: estimate drifted\n  left: 1042\n right: 987\nnote: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n\n\nfailures:\n    tokens::tests::estimate_matches_tiktoken_within_5pct\n\ntest result: FAILED. 7 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s\n\nerror: test failed, to rerun pass `-p sqz-engine --lib`\n",
            &["estimate_matches_tiktoken_within_5pct", "sqz_engine/src/tokens.rs:88:9", "left: 1042", "right: 987", "7 passed; 1 failed"],
            &["evicts_oldest_first ... ok", "Compiling sqz-engine", "RUST_BACKTRACE=1"],
        ),
        fx(
            "pytest failure",
            "pytest",
            "============================= test session starts ==============================\nplatform linux -- Python 3.12.4, pytest-8.3.2, pluggy-1.5.0\nrootdir: /work/api\nconfigfile: pyproject.toml\nplugins: cov-5.0.0, anyio-4.4.0\ncollected 42 items\n\ntests/test_auth.py ..........                                            [ 23%]\ntests/test_billing.py ........F...                                       [ 52%]\ntests/test_models.py ............                                        [ 80%]\ntests/test_webhooks.py ........                                          [100%]\n\n=================================== FAILURES ===================================\n_______________________ test_refund_exceeds_charge_rejected ____________________\n\n    def test_refund_exceeds_charge_rejected():\n        charge = Charge(amount_cents=5000)\n>       with pytest.raises(RefundError):\nE       Failed: DID NOT RAISE <class 'billing.errors.RefundError'>\n\ntests/test_billing.py:71: Failed\n=========================== short test summary info ============================\nFAILED tests/test_billing.py::test_refund_exceeds_charge_rejected - Failed: DID NOT RAISE <class 'billing.errors.RefundError'>\n========================= 1 failed, 41 passed in 3.42s =========================\n",
            &["test_refund_exceeds_charge_rejected", "DID NOT RAISE", "billing.errors.RefundError", "tests/test_billing.py:71", "1 failed, 41 passed"],
            &["tests/test_auth.py ..........", "plugins: cov-5.0.0", "rootdir: /work/api"],
        ),
        fx(
            "tsc errors",
            "tsc --noEmit",
            "src/billing/refund.ts:41:23 - error TS2345: Argument of type 'string' is not assignable to parameter of type 'number'.\n\n41     return this.gateway.refund(charge.id, amountCents);\n                         ~~~~~~~~~~~\n\nsrc/billing/ledger.ts:12:5 - error TS2322: Type 'Refund | undefined' is not assignable to type 'Refund'.\n\n12     this.entries.push(refund);\n       ~~~~~~~~~~~~~~~~~~~~~~~~~~\n\n\nFound 2 errors in 2 files.\n\nErrors  Files\n     1  src/billing/refund.ts:41\n     1  src/billing/ledger.ts:12\n",
            &["src/billing/refund.ts:41:23", "TS2345", "not assignable to parameter of type 'number'", "src/billing/ledger.ts:12:5", "TS2322", "2 errors"],
            &["~~~~~~~~~~~", "Errors  Files"],
        ),
        fx(
            "git diff",
            "git diff",
            "diff --git a/src/billing/refund.py b/src/billing/refund.py\nindex 3f1a2b4..8c9d0e1 100644\n--- a/src/billing/refund.py\n+++ b/src/billing/refund.py\n@@ -12,9 +12,12 @@ class RefundService:\n     def refund(self, charge: Charge, amount_cents: int) -> Refund:\n-        if amount_cents <= 0:\n+        if amount_cents <= 0 or amount_cents > charge.amount_cents:\n             raise RefundError(\"invalid refund amount\")\n-        return self.gateway.refund(charge.id, amount_cents)\n+        refund = self.gateway.refund(charge.id, amount_cents)\n+        self.ledger.record(refund)\n+        return refund\n",
            &[
                "src/billing/refund.py",
                "-        if amount_cents <= 0:",
                "+        if amount_cents <= 0 or amount_cents > charge.amount_cents:",
                "-        return self.gateway.refund(charge.id, amount_cents)",
                "+        refund = self.gateway.refund(charge.id, amount_cents)",
                "+        self.ledger.record(refund)",
                "+        return refund",
            ],
            &["index 3f1a2b4..8c9d0e1 100644"],
        ),
        fx(
            "git status",
            "git status",
            "On branch feature/refund-guard\nYour branch is ahead of 'origin/feature/refund-guard' by 2 commits.\n  (use \"git push\" to publish your local commits)\n\nChanges to be committed:\n  (use \"git restore --staged <file>...\" to unstage)\n\tmodified:   src/billing/refund.py\n\tnew file:   tests/test_refund_guard.py\n\nChanges not staged for commit:\n  (use \"git add <file>...\" to update what will be committed)\n  (use \"git restore <file>...\" to discard changes in working directory)\n\tmodified:   README.md\n\tmodified:   src/billing/ledger.py\n\nUntracked files:\n  (use \"git add <file>...\" to include in what will be committed)\n\tnotes.txt\n\n",
            &["feature/refund-guard", "src/billing/refund.py", "tests/test_refund_guard.py", "README.md", "src/billing/ledger.py", "notes.txt"],
            &["(use \"git push\"", "(use \"git restore --staged", "(use \"git add <file>...\" to include"],
        ),
        fx(
            "npm install",
            "npm install",
            "npm warn deprecated inflight@1.0.6: This module is not supported, and leaks memory. Do not use it. Check out lru-cache if you want a good and tested way to coalesce async requests by a key value, which is much more comprehensive and powerful.\nnpm warn deprecated glob@7.2.3: Glob versions prior to v9 are no longer supported\nnpm warn deprecated rimraf@3.0.2: Rimraf versions prior to v4 are no longer supported\n\nadded 412 packages, and audited 413 packages in 9s\n\n58 packages are looking for funding\n  run `npm fund` for details\n\n3 vulnerabilities (1 moderate, 2 high)\n\nTo address all issues, run:\n  npm audit fix\n\nRun `npm audit` for details.\n",
            &["412 packages", "3 vulnerabilities", "2 high", "npm audit fix"],
            &["looking for funding", "leaks memory"],
        ),
        fx(
            "kubectl get pods",
            "kubectl get pods",
            "NAME                                READY   STATUS             RESTARTS      AGE\napi-7f8d9c0b1a-def34                1/1     Running            2 (3h ago)    42d\napi-7f8d9c0b1a-xk2p9                1/1     Running            0             42d\ncache-1a2b3c4d5e-jkl78              1/1     Running            0             42d\ndb-9z8y7x6w5v-mno90                 1/1     Running            0             42d\ningress-nginx-controller-5b7c8      1/1     Running            0             120d\nweb-6d4cf56db6-abc12                1/1     Running            0             42d\nweb-6d4cf56db6-qq7zt                1/1     Running            0             42d\nworker-5a6b7c8d9e-ghi56             0/1     CrashLoopBackOff   17 (2m ago)   3h\n",
            &["worker-5a6b7c8d9e-ghi56", "CrashLoopBackOff", "17", "api-7f8d9c0b1a-def34", "api-7f8d9c0b1a-xk2p9", "cache-1a2b3c4d5e-jkl78", "db-9z8y7x6w5v-mno90", "ingress-nginx-controller-5b7c8", "web-6d4cf56db6-abc12", "web-6d4cf56db6-qq7zt"],
            &[],
        ),
        fx(
            "docker ps",
            "docker ps",
            "CONTAINER ID   IMAGE                          COMMAND                  CREATED        STATUS                          PORTS                    NAMES\n3f1a2b4c5d6e   postgres:16                    \"docker-entrypoint.s…\"   3 days ago     Up 3 days (healthy)             0.0.0.0:5432->5432/tcp   api-db-1\n8c9d0e1f2a3b   redis:7-alpine                 \"docker-entrypoint.s…\"   3 days ago     Up 3 days                       6379/tcp                 api-cache-1\na1b2c3d4e5f6   api:local                      \"uvicorn app:app --…\"    2 hours ago    Restarting (1) 12 seconds ago                            api-web-1\nb2c3d4e5f6a7   nginx:1.27                     \"/docker-entrypoint.…\"   3 days ago     Up 3 days                       0.0.0.0:80->80/tcp       api-proxy-1\n",
            &["api-web-1", "Restarting (1)", "api-db-1", "5432", "api-cache-1", "api-proxy-1"],
            &["docker-entrypoint.s"],
        ),
        fx(
            "JSON API response",
            "curl https://api.example.com/v1/orders/ord_8812",
            r#"{"id":"ord_8812","status":"failed","created_at":"2026-09-14T10:22:03Z","updated_at":"2026-09-14T10:22:07Z","customer":{"id":"cus_4471","email":"dev@example.com","name":null,"phone":null},"items":[{"sku":"SKU-100","qty":2,"unit_cents":1999,"metadata":null},{"sku":"SKU-205","qty":1,"unit_cents":4999,"metadata":null}],"payment":{"provider":"stripe","charge_id":"ch_3Pq9","error":{"code":"card_declined","decline_code":"insufficient_funds","message":"Your card has insufficient funds."}},"shipping":null,"notes":null}"#,
            &["ord_8812", "failed", "cus_4471", "dev@example.com", "SKU-100", "SKU-205", "4999", "ch_3Pq9", "card_declined", "insufficient_funds", "Your card has insufficient funds"],
            &["null"],
        ),
        fx(
            "app log with repeats",
            "tail -n 200 app.log",
            app_log(),
            &["worker started pid=4412", "retrying stripe call attempt=2", "refund failed order=ord_8812 code=card_declined"],
            &["10:22:07Z INFO  GET /healthz", "10:23:17Z INFO  GET /healthz"],
        ),
        fx(
            "source file read",
            "cat src/billing/refund.py",
            "from dataclasses import dataclass\n\nfrom .errors import RefundError\nfrom .gateway import Gateway\nfrom .ledger import Ledger\nfrom .models import Charge, Refund\n\n\n@dataclass\nclass RefundService:\n    gateway: Gateway\n    ledger: Ledger\n\n    def refund(self, charge: Charge, amount_cents: int) -> Refund:\n        if amount_cents <= 0 or amount_cents > charge.amount_cents:\n            raise RefundError(\"invalid refund amount\")\n        refund = self.gateway.refund(charge.id, amount_cents)\n        self.ledger.record(refund)\n        return refund\n\n    def partial(self, charge: Charge, fraction: float) -> Refund:\n        if not 0.0 < fraction <= 1.0:\n            raise RefundError(\"fraction out of range\")\n        return self.refund(charge, int(charge.amount_cents * fraction))\n\n    def _audit(self, refund: Refund) -> None:\n        self.ledger.record(refund)\n        self.ledger.flush()\n",
            &["class RefundService", "def refund(self, charge: Charge, amount_cents: int) -> Refund", "def partial(self, charge: Charge, fraction: float) -> Refund", "from .errors import RefundError", "raise RefundError(\"invalid refund amount\")"],
            &[],
        ),
        verbatim(
            "python traceback",
            "python app.py",
            "Traceback (most recent call last):\n  File \"/work/api/app.py\", line 42, in <module>\n    main()\n  File \"/work/api/app.py\", line 38, in main\n    svc.refund(charge, 9000)\n  File \"/work/api/src/billing/refund.py\", line 15, in refund\n    raise RefundError(\"invalid refund amount\")\nbilling.errors.RefundError: invalid refund amount\n",
            &["/work/api/app.py\", line 42", "/work/api/app.py\", line 38", "/work/api/src/billing/refund.py\", line 15", "billing.errors.RefundError: invalid refund amount"],
        ),
        sensitive(
            "dotenv with secrets",
            "cat .env",
            "DATABASE_URL=postgres://app:s3cr3t-pw@db.internal:5432/app\nREDIS_URL=redis://cache.internal:6379/0\nSTRIPE_SECRET_KEY=sk_test_51Pq9AbCdEfGhIjKlMnOpQrStUvWxYz0123456789\nAWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\nAWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\nLOG_LEVEL=debug\nFEATURE_REFUND_GUARD=true\n",
            &["sk_test_51Pq9AbCdEfGhIjKlMnOpQrStUvWxYz0123456789", "AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "FEATURE_REFUND_GUARD=true"],
        ),
    ]
}

fn print_table(rows: &[Outcome]) {
    println!();
    println!("| Fixture | Command | Tokens in | Tokens out | Reduction | Facts kept | Noise dropped | Recovery |");
    println!("|---|---|---:|---:|---:|---:|---:|---|");
    for r in rows {
        let noise = if r.noise_total == 0 { "n/a".to_string() } else { format!("{}/{}", r.noise_dropped, r.noise_total) };
        let recovery = match (r.sensitive, r.recovery) {
            (true, Recovery::NotStored) => "passthrough, never cached",
            (_, Recovery::Exact) => "byte-exact",
            (_, Recovery::Mismatch) => "MISMATCH",
            (false, Recovery::NotStored) => "not cached",
        };
        println!(
            "| {} | `{}` | {} | {} | {:.1}% | {}/{} | {} | {} |",
            r.name, r.cmd, r.tokens_in, r.tokens_out, r.reduction(), r.facts_kept, r.facts_total, noise, recovery
        );
    }
    let tin: usize = rows.iter().map(|r| r.tokens_in).sum();
    let tout: usize = rows.iter().map(|r| r.tokens_out).sum();
    let facts: usize = rows.iter().map(|r| r.facts_total).sum();
    let kept: usize = rows.iter().map(|r| r.facts_kept).sum();
    println!();
    println!(
        "aggregate: {tin} -> {tout} tokens ({:.1}% reduction), facts kept {kept}/{facts} ({:.1}%)",
        (1.0 - tout as f64 / tin as f64) * 100.0,
        kept as f64 / facts as f64 * 100.0
    );
    for r in rows.iter().filter(|r| !r.missing.is_empty()) {
        println!("missing in [{}]: {:?}", r.name, r.missing);
    }
    println!();
}

#[test]
fn quality_gates() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("quality.db");
    let fixtures = fixtures();
    let rows: Vec<Outcome> = fixtures.iter().map(|f| measure(&db, f)).collect();
    print_table(&rows);

    for r in &rows {
        assert!(r.missing.is_empty(), "[{}] lost critical facts: {:?}", r.name, r.missing);
        assert_ne!(r.recovery, Recovery::Mismatch, "[{}] expand returned different bytes", r.name);
        if let Some(ok) = r.verbatim_ok {
            assert!(ok, "[{}] content that must pass through untouched was altered", r.name);
        }
        if r.sensitive {
            assert_eq!(r.recovery, Recovery::NotStored, "[{}] sensitive content must not be persisted", r.name);
        } else {
            assert_eq!(r.recovery, Recovery::Exact, "[{}] compressed content must be recoverable", r.name);
        }
    }

    let tin: usize = rows.iter().filter(|r| r.verbatim_ok.is_none()).map(|r| r.tokens_in).sum();
    let tout: usize = rows.iter().filter(|r| r.verbatim_ok.is_none()).map(|r| r.tokens_out).sum();
    let reduction = (1.0 - tout as f64 / tin as f64) * 100.0;
    assert!(reduction >= 40.0, "aggregate reduction on compressible fixtures fell to {reduction:.1}%");
}
