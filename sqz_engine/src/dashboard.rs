//! Local web dashboard with real-time metrics via SSE.
//!
//! Serves a self-contained HTML page (inline CSS/JS, zero external network
//! requests) on a configurable port.  An SSE endpoint pushes updated metrics
//! every 5 seconds.

use std::fmt::Write as FmtWrite;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::session_store::SessionSummary;

// ---------------------------------------------------------------------------
// Metrics data model
// ---------------------------------------------------------------------------

/// Per-tool token/cost breakdown.
#[derive(Debug, Clone, Default)]
pub struct ToolBreakdown {
    pub tool_name: String,
    pub tokens_input: u64,
    pub tokens_output: u64,
    pub cost_usd: f64,
    pub call_count: u32,
}

/// Per-command compression breakdown.
#[derive(Debug, Clone, Default)]
pub struct CommandBreakdown {
    pub command: String,
    pub tokens_original: u64,
    pub tokens_compressed: u64,
    pub invocations: u32,
}

/// Session history entry for the dashboard.
#[derive(Debug, Clone)]
pub struct SessionHistoryEntry {
    pub id: String,
    pub project_dir: String,
    pub summary: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

impl From<&SessionSummary> for SessionHistoryEntry {
    fn from(s: &SessionSummary) -> Self {
        SessionHistoryEntry {
            id: s.id.clone(),
            project_dir: s.project_dir.display().to_string(),
            summary: s.compressed_summary.clone(),
            created_at: s.created_at,
            updated_at: s.updated_at,
            total_tokens: 0,
            cost_usd: 0.0,
        }
    }
}

/// All metrics exposed by the dashboard.
#[derive(Debug, Clone)]
pub struct DashboardMetrics {
    // Real-time counters
    pub tokens_saved: u64,
    pub tokens_total: u64,
    pub compression_ratio: f64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cost_savings_usd: f64,
    pub total_cost_usd: f64,

    // Active session
    pub active_session_id: Option<String>,
    pub active_model: Option<String>,
    pub budget_consumed_pct: f64,

    // Breakdowns
    pub per_tool: Vec<ToolBreakdown>,
    pub per_command: Vec<CommandBreakdown>,

    // Session history
    pub sessions: Vec<SessionHistoryEntry>,

    /// Timestamp of this snapshot.
    pub snapshot_at: DateTime<Utc>,
}

impl Default for DashboardMetrics {
    fn default() -> Self {
        DashboardMetrics {
            tokens_saved: 0,
            tokens_total: 0,
            compression_ratio: 0.0,
            cache_hits: 0,
            cache_misses: 0,
            cost_savings_usd: 0.0,
            total_cost_usd: 0.0,
            active_session_id: None,
            active_model: None,
            budget_consumed_pct: 0.0,
            per_tool: Vec::new(),
            per_command: Vec::new(),
            sessions: Vec::new(),
            snapshot_at: Utc::now(),
        }
    }
}

impl DashboardMetrics {
    /// Cache hit rate as a percentage (0.0–100.0).
    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            return 0.0;
        }
        (self.cache_hits as f64 / total as f64) * 100.0
    }

    /// Serialize metrics to a JSON string for SSE delivery.
    pub fn to_json(&self) -> String {
        let mut s = String::with_capacity(2048);
        s.push('{');

        // Scalars
        let _ = write!(
            s,
            "\"tokens_saved\":{},\"tokens_total\":{},\"compression_ratio\":{:.4},\
             \"cache_hit_rate\":{:.2},\"cache_hits\":{},\"cache_misses\":{},\
             \"cost_savings_usd\":{:.6},\"total_cost_usd\":{:.6},\
             \"budget_consumed_pct\":{:.2}",
            self.tokens_saved,
            self.tokens_total,
            self.compression_ratio,
            self.cache_hit_rate(),
            self.cache_hits,
            self.cache_misses,
            self.cost_savings_usd,
            self.total_cost_usd,
            self.budget_consumed_pct,
        );

        // Active session
        if let Some(ref id) = self.active_session_id {
            let _ = write!(s, ",\"active_session_id\":\"{}\"", escape_json(id));
        } else {
            s.push_str(",\"active_session_id\":null");
        }
        if let Some(ref model) = self.active_model {
            let _ = write!(s, ",\"active_model\":\"{}\"", escape_json(model));
        } else {
            s.push_str(",\"active_model\":null");
        }

        // Per-tool
        s.push_str(",\"per_tool\":[");
        for (i, t) in self.per_tool.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let _ = write!(
                s,
                "{{\"tool_name\":\"{}\",\"tokens_input\":{},\"tokens_output\":{},\
                 \"cost_usd\":{:.6},\"call_count\":{}}}",
                escape_json(&t.tool_name),
                t.tokens_input,
                t.tokens_output,
                t.cost_usd,
                t.call_count,
            );
        }
        s.push(']');

        // Per-command
        s.push_str(",\"per_command\":[");
        for (i, c) in self.per_command.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let _ = write!(
                s,
                "{{\"command\":\"{}\",\"tokens_original\":{},\"tokens_compressed\":{},\
                 \"invocations\":{}}}",
                escape_json(&c.command),
                c.tokens_original,
                c.tokens_compressed,
                c.invocations,
            );
        }
        s.push(']');

        // Sessions (compact)
        s.push_str(",\"sessions\":[");
        for (i, sess) in self.sessions.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let _ = write!(
                s,
                "{{\"id\":\"{}\",\"project_dir\":\"{}\",\"summary\":\"{}\",\
                 \"created_at\":\"{}\",\"total_tokens\":{},\"cost_usd\":{:.6}}}",
                escape_json(&sess.id),
                escape_json(&sess.project_dir),
                escape_json(&sess.summary),
                sess.created_at.to_rfc3339(),
                sess.total_tokens,
                sess.cost_usd,
            );
        }
        s.push(']');

        let _ = write!(s, ",\"snapshot_at\":\"{}\"", self.snapshot_at.to_rfc3339());
        s.push('}');
        s
    }
}

/// Minimal JSON string escaping.
fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// HTML generation
// ---------------------------------------------------------------------------

/// Generates a self-contained HTML dashboard page with inline CSS and JS.
pub struct DashboardHtml;

impl DashboardHtml {
    /// Render the full HTML page.  The page uses SSE to auto-refresh metrics
    /// from `/events` every 5 seconds.
    pub fn render(_port: u16) -> String {
        format!(
            r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>sqz dashboard</title>
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:system-ui,-apple-system,sans-serif;background:#0f1117;color:#e1e4e8;padding:1.5rem}}
h1{{font-size:1.4rem;margin-bottom:1rem;color:#58a6ff}}
.grid{{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:1rem;margin-bottom:1.5rem}}
.card{{background:#161b22;border:1px solid #30363d;border-radius:8px;padding:1rem}}
.card .label{{font-size:.75rem;color:#8b949e;text-transform:uppercase;letter-spacing:.05em}}
.card .value{{font-size:1.6rem;font-weight:700;margin-top:.25rem}}
.green{{color:#3fb950}} .blue{{color:#58a6ff}} .orange{{color:#d29922}} .red{{color:#f85149}}
h2{{font-size:1.1rem;margin:1.2rem 0 .6rem;color:#c9d1d9}}
table{{width:100%;border-collapse:collapse;margin-bottom:1rem}}
th,td{{text-align:left;padding:.4rem .6rem;border-bottom:1px solid #21262d;font-size:.85rem}}
th{{color:#8b949e;font-weight:600}}
#search{{background:#0d1117;border:1px solid #30363d;color:#e1e4e8;padding:.4rem .6rem;border-radius:4px;width:260px;margin-bottom:.6rem;font-size:.85rem}}
.status{{font-size:.8rem;color:#8b949e;text-align:right;margin-top:1rem}}
</style>
</head>
<body>
<h1>sqz dashboard</h1>

<div class="grid">
  <div class="card"><div class="label">Tokens Saved</div><div class="value green" id="m-saved">—</div></div>
  <div class="card"><div class="label">Compression Ratio</div><div class="value blue" id="m-ratio">—</div></div>
  <div class="card"><div class="label">Cache Hit Rate</div><div class="value blue" id="m-cache">—</div></div>
  <div class="card"><div class="label">Cost Savings</div><div class="value green" id="m-cost">—</div></div>
  <div class="card"><div class="label">Total Cost</div><div class="value orange" id="m-total">—</div></div>
  <div class="card"><div class="label">Budget Used</div><div class="value" id="m-budget">—</div></div>
</div>

<h2>Per-Tool Breakdown</h2>
<table id="tool-table">
<thead><tr><th>Tool</th><th>Input Tokens</th><th>Output Tokens</th><th>Cost (USD)</th><th>Calls</th></tr></thead>
<tbody></tbody>
</table>

<h2>Per-Command Breakdown</h2>
<table id="cmd-table">
<thead><tr><th>Command</th><th>Original</th><th>Compressed</th><th>Ratio</th><th>Runs</th></tr></thead>
<tbody></tbody>
</table>

<h2>Session History</h2>
<input id="search" placeholder="Search sessions…" aria-label="Search sessions">
<table id="sess-table">
<thead><tr><th>ID</th><th>Project</th><th>Summary</th><th>Created</th><th>Tokens</th><th>Cost</th></tr></thead>
<tbody></tbody>
</table>

<div class="status" id="status">Connecting…</div>

<script>
(function(){{
  var es=new EventSource('/events');
  var statusEl=document.getElementById('status');
  var searchEl=document.getElementById('search');
  var lastData=null;

  es.onmessage=function(e){{
    var d=JSON.parse(e.data);
    lastData=d;
    render(d);
    statusEl.textContent='Updated '+new Date().toLocaleTimeString();
  }};
  es.onerror=function(){{statusEl.textContent='Disconnected — retrying…';}};

  searchEl.addEventListener('input',function(){{if(lastData)renderSessions(lastData.sessions);}});

  function render(d){{
    document.getElementById('m-saved').textContent=fmt(d.tokens_saved);
    document.getElementById('m-ratio').textContent=(d.compression_ratio*100).toFixed(1)+'%';
    document.getElementById('m-cache').textContent=d.cache_hit_rate.toFixed(1)+'%';
    document.getElementById('m-cost').textContent='$'+d.cost_savings_usd.toFixed(4);
    document.getElementById('m-total').textContent='$'+d.total_cost_usd.toFixed(4);
    var bp=d.budget_consumed_pct;
    var budgetEl=document.getElementById('m-budget');
    budgetEl.textContent=bp.toFixed(1)+'%';
    budgetEl.className='value '+(bp>85?'red':bp>70?'orange':'green');

    renderTable('tool-table',d.per_tool,function(t){{
      return '<td>'+esc(t.tool_name)+'</td><td>'+fmt(t.tokens_input)+'</td><td>'+fmt(t.tokens_output)+'</td><td>$'+t.cost_usd.toFixed(4)+'</td><td>'+t.call_count+'</td>';
    }});
    renderTable('cmd-table',d.per_command,function(c){{
      var r=c.tokens_original?((c.tokens_compressed/c.tokens_original)*100).toFixed(1)+'%':'—';
      return '<td>'+esc(c.command)+'</td><td>'+fmt(c.tokens_original)+'</td><td>'+fmt(c.tokens_compressed)+'</td><td>'+r+'</td><td>'+c.invocations+'</td>';
    }});
    renderSessions(d.sessions);
  }}

  function renderSessions(sessions){{
    var q=(searchEl.value||'').toLowerCase();
    var filtered=sessions.filter(function(s){{
      if(!q)return true;
      return (s.id+s.project_dir+s.summary).toLowerCase().indexOf(q)>=0;
    }});
    renderTable('sess-table',filtered,function(s){{
      return '<td>'+esc(s.id)+'</td><td>'+esc(s.project_dir)+'</td><td>'+esc(s.summary)+'</td><td>'+new Date(s.created_at).toLocaleDateString()+'</td><td>'+fmt(s.total_tokens)+'</td><td>$'+s.cost_usd.toFixed(4)+'</td>';
    }});
  }}

  function renderTable(id,rows,rowFn){{
    var tb=document.getElementById(id).querySelector('tbody');
    tb.innerHTML=rows.map(function(r){{return '<tr>'+rowFn(r)+'</tr>';}}).join('');
  }}

  function fmt(n){{
    if(n>=1e6)return (n/1e6).toFixed(1)+'M';
    if(n>=1e3)return (n/1e3).toFixed(1)+'K';
    return ''+n;
  }}

  function esc(s){{
    var d=document.createElement('div');d.textContent=s||'';return d.innerHTML;
  }}
}})();
</script>
</body>
</html>"##,
        )
    }
}


// ---------------------------------------------------------------------------
// Dashboard server (minimal TCP-based)
// ---------------------------------------------------------------------------

/// Configuration for the dashboard server.
#[derive(Debug, Clone)]
pub struct DashboardConfig {
    pub port: u16,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        DashboardConfig { port: 3001 }
    }
}

/// A minimal HTTP server that serves the dashboard HTML and an SSE endpoint.
///
/// The server uses a shared `Arc<Mutex<DashboardMetrics>>` so that an
/// external thread can update the metrics while the server pushes them to
/// connected SSE clients.
pub struct DashboardServer {
    config: DashboardConfig,
    metrics: Arc<Mutex<DashboardMetrics>>,
}

impl DashboardServer {
    /// Create a new server with the given config and a shared metrics handle.
    pub fn new(config: DashboardConfig, metrics: Arc<Mutex<DashboardMetrics>>) -> Self {
        DashboardServer { config, metrics }
    }

    /// Return a clone of the shared metrics handle so callers can update it.
    pub fn metrics_handle(&self) -> Arc<Mutex<DashboardMetrics>> {
        Arc::clone(&self.metrics)
    }

    /// Start listening.  This blocks the calling thread.
    ///
    /// For each incoming connection the server reads the HTTP request line,
    /// then either:
    /// - `GET /`        → responds with the full HTML page
    /// - `GET /events`  → responds with an SSE stream (pushes metrics JSON
    ///                     every 5 seconds until the client disconnects)
    /// - anything else  → 404
    pub fn run(&self) -> crate::error::Result<()> {
        let addr = format!("127.0.0.1:{}", self.config.port);
        let listener = TcpListener::bind(&addr)?;
        eprintln!("[sqz] dashboard listening on http://{addr}");

        let html = DashboardHtml::render(self.config.port);

        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };

            // Read the request line.
            let mut reader = BufReader::new(stream.try_clone().unwrap_or_else(|_| {
                // Fallback: just use the stream directly (shouldn't happen).
                stream.try_clone().expect("clone failed")
            }));
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }

            // Drain remaining headers (we don't need them).
            let mut header = String::new();
            loop {
                header.clear();
                match reader.read_line(&mut header) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if header.trim().is_empty() {
                            break;
                        }
                    }
                }
            }

            if request_line.starts_with("GET /events") {
                // SSE endpoint
                let response_header = "HTTP/1.1 200 OK\r\n\
                    Content-Type: text/event-stream\r\n\
                    Cache-Control: no-cache\r\n\
                    Connection: keep-alive\r\n\
                    Access-Control-Allow-Origin: *\r\n\r\n";
                if stream.write_all(response_header.as_bytes()).is_err() {
                    continue;
                }

                // Push metrics every 5 seconds until the client disconnects.
                loop {
                    let json = {
                        let m = self.metrics.lock().unwrap();
                        m.to_json()
                    };
                    let event = format!("data: {json}\n\n");
                    if stream.write_all(event.as_bytes()).is_err() {
                        break;
                    }
                    if stream.flush().is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(5));
                }
            } else if request_line.starts_with("GET / ")
                || request_line.starts_with("GET / HTTP")
            {
                // Serve HTML
                let response = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: text/html; charset=utf-8\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\r\n{}",
                    html.len(),
                    html,
                );
                let _ = stream.write_all(response.as_bytes());
            } else {
                let body = "404 Not Found";
                let response = format!(
                    "HTTP/1.1 404 Not Found\r\n\
                     Content-Type: text/plain\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                let _ = stream.write_all(response.as_bytes());
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // DashboardMetrics
    // -----------------------------------------------------------------------

    #[test]
    fn test_default_metrics() {
        let m = DashboardMetrics::default();
        assert_eq!(m.tokens_saved, 0);
        assert_eq!(m.tokens_total, 0);
        assert_eq!(m.cache_hits, 0);
        assert_eq!(m.cache_misses, 0);
        assert!((m.compression_ratio - 0.0).abs() < f64::EPSILON);
        assert!(m.per_tool.is_empty());
        assert!(m.per_command.is_empty());
        assert!(m.sessions.is_empty());
    }

    #[test]
    fn test_cache_hit_rate_zero_total() {
        let m = DashboardMetrics::default();
        assert!((m.cache_hit_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cache_hit_rate_calculation() {
        let mut m = DashboardMetrics::default();
        m.cache_hits = 75;
        m.cache_misses = 25;
        assert!((m.cache_hit_rate() - 75.0).abs() < 0.01);
    }

    #[test]
    fn test_cache_hit_rate_all_hits() {
        let mut m = DashboardMetrics::default();
        m.cache_hits = 100;
        m.cache_misses = 0;
        assert!((m.cache_hit_rate() - 100.0).abs() < f64::EPSILON);
    }

    // -----------------------------------------------------------------------
    // JSON serialization
    // -----------------------------------------------------------------------

    #[test]
    fn test_to_json_default_metrics() {
        let m = DashboardMetrics::default();
        let json = m.to_json();
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
        assert!(json.contains("\"tokens_saved\":0"));
        assert!(json.contains("\"per_tool\":[]"));
        assert!(json.contains("\"per_command\":[]"));
        assert!(json.contains("\"sessions\":[]"));
        assert!(json.contains("\"active_session_id\":null"));
    }

    #[test]
    fn test_to_json_with_data() {
        let mut m = DashboardMetrics::default();
        m.tokens_saved = 50_000;
        m.compression_ratio = 0.35;
        m.active_session_id = Some("sess_123".to_string());
        m.per_tool.push(ToolBreakdown {
            tool_name: "read_file".to_string(),
            tokens_input: 1000,
            tokens_output: 500,
            cost_usd: 0.003,
            call_count: 5,
        });
        m.per_command.push(CommandBreakdown {
            command: "cargo build".to_string(),
            tokens_original: 10_000,
            tokens_compressed: 3_500,
            invocations: 3,
        });

        let json = m.to_json();
        assert!(json.contains("\"tokens_saved\":50000"));
        assert!(json.contains("\"active_session_id\":\"sess_123\""));
        assert!(json.contains("\"read_file\""));
        assert!(json.contains("\"cargo build\""));
    }

    #[test]
    fn test_escape_json_special_chars() {
        assert_eq!(escape_json("hello"), "hello");
        assert_eq!(escape_json("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(escape_json("a\\b"), "a\\\\b");
        assert_eq!(escape_json("line1\nline2"), "line1\\nline2");
        assert_eq!(escape_json("tab\there"), "tab\\there");
    }

    // -----------------------------------------------------------------------
    // HTML generation
    // -----------------------------------------------------------------------

    #[test]
    fn test_html_is_self_contained() {
        let html = DashboardHtml::render(3001);
        // Must be valid HTML with inline CSS and JS
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<style>"));
        assert!(html.contains("<script>"));
        assert!(html.contains("</html>"));
        // Must NOT reference external resources
        assert!(!html.contains("https://"));
        assert!(!html.contains("http://"));
        // Must contain SSE connection to /events
        assert!(html.contains("EventSource"));
        assert!(html.contains("/events"));
    }

    #[test]
    fn test_html_contains_metric_elements() {
        let html = DashboardHtml::render(3001);
        assert!(html.contains("id=\"m-saved\""));
        assert!(html.contains("id=\"m-ratio\""));
        assert!(html.contains("id=\"m-cache\""));
        assert!(html.contains("id=\"m-cost\""));
        assert!(html.contains("id=\"m-budget\""));
    }

    #[test]
    fn test_html_contains_tables() {
        let html = DashboardHtml::render(3001);
        assert!(html.contains("id=\"tool-table\""));
        assert!(html.contains("id=\"cmd-table\""));
        assert!(html.contains("id=\"sess-table\""));
    }

    #[test]
    fn test_html_contains_search_input() {
        let html = DashboardHtml::render(3001);
        assert!(html.contains("id=\"search\""));
        assert!(html.contains("Search sessions"));
    }

    // -----------------------------------------------------------------------
    // DashboardServer construction
    // -----------------------------------------------------------------------

    #[test]
    fn test_dashboard_config_default() {
        let cfg = DashboardConfig::default();
        assert_eq!(cfg.port, 3001);
    }

    #[test]
    fn test_dashboard_server_metrics_handle() {
        let metrics = Arc::new(Mutex::new(DashboardMetrics::default()));
        let server = DashboardServer::new(DashboardConfig::default(), Arc::clone(&metrics));

        // Update via the handle
        {
            let handle = server.metrics_handle();
            let mut m = handle.lock().unwrap();
            m.tokens_saved = 42;
        }

        // Original Arc should reflect the change
        let m = metrics.lock().unwrap();
        assert_eq!(m.tokens_saved, 42);
    }

    // -----------------------------------------------------------------------
    // SessionHistoryEntry from SessionSummary
    // -----------------------------------------------------------------------

    #[test]
    fn test_session_history_from_summary() {
        let summary = SessionSummary {
            id: "s1".to_string(),
            project_dir: std::path::PathBuf::from("/tmp/proj"),
            compressed_summary: "working on API".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let entry = SessionHistoryEntry::from(&summary);
        assert_eq!(entry.id, "s1");
        assert_eq!(entry.project_dir, "/tmp/proj");
        assert_eq!(entry.summary, "working on API");
        assert_eq!(entry.total_tokens, 0);
        assert_eq!(entry.cost_usd, 0.0);
    }
}
