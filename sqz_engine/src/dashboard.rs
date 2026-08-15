//! Local web dashboard with real-time metrics via SSE.
//!
//! Serves a self-contained HTML page (inline CSS/JS, zero external network
//! requests) on a configurable port.  An SSE endpoint pushes updated metrics
//! every 5 seconds.

use std::fmt::Write as FmtWrite;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
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
    /// Each accepted connection is served on its own thread, so a
    /// long-lived `/events` SSE stream (or a half-open client) can never
    /// block the accept loop. See [`Self::run_on`] for the routing table
    /// and the history of issue #36.
    pub fn run(&self) -> crate::error::Result<()> {
        let addr = format!("127.0.0.1:{}", self.config.port);
        let listener = TcpListener::bind(&addr)?;
        eprintln!("[sqz] dashboard listening on http://{addr}");
        self.run_on(listener)
    }

    /// Serve on an already-bound listener. This blocks the calling thread.
    ///
    /// Split out from [`Self::run`] so tests can bind port 0 and learn the
    /// real port via `listener.local_addr()` before starting the server.
    ///
    /// For each incoming connection the server reads the HTTP request line,
    /// then either:
    /// - `GET /`        → responds with the full HTML page
    /// - `GET /events`  → responds with an SSE stream (pushes metrics JSON
    ///                     every 5 seconds until the client disconnects)
    /// - anything else  → 404
    ///
    /// Concurrency model: one thread per connection. The previous design
    /// served every connection inline on the accept thread, which meant a
    /// single connected SSE client parked the whole server in its push
    /// loop, and an interrupted SSE client parked it for up to two 5-second
    /// ticks (the first write after a peer FIN succeeds into the send
    /// buffer; only the next one observes the RST — and `TcpStream::flush`
    /// is a no-op, so it detects nothing). Meanwhile every other connection
    /// sat unaccepted in the kernel backlog showing CLOSE_WAIT, and the
    /// dashboard page's EventSource auto-reconnect turned the pile-up into
    /// a permanent hang (issue #36).
    pub fn run_on(&self, listener: TcpListener) -> crate::error::Result<()> {
        let html = Arc::new(DashboardHtml::render(self.config.port));

        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let metrics = Arc::clone(&self.metrics);
            let html = Arc::clone(&html);
            std::thread::spawn(move || {
                handle_connection(stream, metrics, &html);
            });
        }

        Ok(())
    }
}

/// How long an SSE connection may sit idle before we probe it. Also the
/// upper bound on how long a half-open client can hold its handler thread
/// between events.
const SSE_TICK: Duration = Duration::from_secs(5);

/// Read timeout for the initial request line + headers. A client that
/// connects and never sends a request gets its thread reclaimed after this
/// long, instead of holding it forever.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Serve a single accepted connection. Runs on its own thread.
fn handle_connection(mut stream: TcpStream, metrics: Arc<Mutex<DashboardMetrics>>, html: &str) {
    // Bound the initial read so half-open clients can't pin the thread.
    let _ = stream.set_read_timeout(Some(REQUEST_READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(REQUEST_READ_TIMEOUT));

    // Read the request line.
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
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
        serve_sse(stream, metrics);
    } else if request_line.starts_with("GET / ") || request_line.starts_with("GET / HTTP") {
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
    // `stream` (and the BufReader clone) drop here, fully closing the
    // socket — no CLOSE_WAIT lingering past the handler's lifetime.
}

/// Push metrics to an SSE client until it disconnects.
///
/// Disconnect detection: instead of sleeping blindly between events, we
/// spend the inter-event gap in a read with a short timeout. SSE clients
/// never send data after the request, so the read returns:
/// - `Ok(0)` → orderly EOF: the client closed (curl --max-time, tab
///   closed). Tear down immediately — this is what turns the peer's FIN
///   into an instant close instead of a CLOSE_WAIT that lingers until the
///   next write cycle.
/// - `Err(WouldBlock/TimedOut)` → still connected, keep going.
/// - `Err(other)` / RST → tear down.
fn serve_sse(mut stream: TcpStream, metrics: Arc<Mutex<DashboardMetrics>>) {
    let response_header = "HTTP/1.1 200 OK\r\n\
        Content-Type: text/event-stream\r\n\
        Cache-Control: no-cache\r\n\
        Connection: keep-alive\r\n\
        Access-Control-Allow-Origin: *\r\n\r\n";
    if stream.write_all(response_header.as_bytes()).is_err() {
        return;
    }

    let mut buf = [0u8; 512];
    loop {
        let json = {
            let m = metrics.lock().unwrap();
            m.to_json()
        };
        let event = format!("data: {json}\n\n");
        if stream.write_all(event.as_bytes()).is_err() {
            return;
        }

        // Wait out the tick watching the socket for FIN/RST.
        if stream.set_read_timeout(Some(SSE_TICK)).is_err() {
            return;
        }
        match stream.read(&mut buf) {
            // EOF — the client closed its end.
            Ok(0) => return,
            // Clients aren't supposed to send anything on an SSE stream;
            // tolerate stray bytes and keep streaming.
            Ok(_) => {}
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Tick elapsed with the connection still healthy.
            }
            // RST or anything else — connection is gone.
            Err(_) => return,
        }
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
    // Live server: issue #36 regression (SSE must not wedge the server)
    // -----------------------------------------------------------------------

    use std::io::Read as IoRead;
    use std::net::{SocketAddr, TcpStream as ClientStream};

    /// Bind port 0, start the server on a background thread, return the
    /// real address. The thread is detached — `run_on` never returns — and
    /// the OS reclaims the socket when the test process exits.
    fn start_test_server() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test port");
        let addr = listener.local_addr().expect("local addr");
        let metrics = Arc::new(Mutex::new(DashboardMetrics::default()));
        let server = DashboardServer::new(DashboardConfig { port: addr.port() }, metrics);
        std::thread::spawn(move || {
            let _ = server.run_on(listener);
        });
        addr
    }

    /// GET `path` and return the raw response bytes read within `timeout`.
    fn http_get(addr: SocketAddr, path: &str, timeout: Duration) -> std::io::Result<Vec<u8>> {
        let mut s = ClientStream::connect_timeout(&addr.into(), timeout)?;
        s.set_read_timeout(Some(timeout))?;
        s.set_write_timeout(Some(timeout))?;
        write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n")?;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match s.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    // For SSE the stream never ends; stop after the first data frame.
                    if buf.windows(6).any(|w| w == b"data: ") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        Ok(buf)
    }

    /// Regression for issue #36: interrupted `/events` clients (connect,
    /// read a little, hard-close — curl --max-time behaviour) must not
    /// stop the server from answering subsequent requests.
    #[test]
    fn interrupted_sse_clients_do_not_block_server() {
        let addr = start_test_server();

        // Six aborted SSE connections, like the reproduction script.
        for _ in 0..6 {
            let mut s = ClientStream::connect_timeout(&addr.into(), Duration::from_secs(2))
                .expect("connect sse");
            let _ = write!(s, "GET /events HTTP/1.1\r\nHost: localhost\r\n\r\n");
            let mut first = [0u8; 256];
            s.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
            let _ = s.read(&mut first);
            drop(s); // hard close while the server-side stream is open
        }

        // The old single-threaded server would sit in the dead SSE loops
        // for many seconds; the fixed server must answer immediately.
        let start = std::time::Instant::now();
        let resp = http_get(addr, "/", Duration::from_secs(3)).expect("GET / after aborted SSE");
        let text = String::from_utf8_lossy(&resp);
        assert!(
            text.starts_with("HTTP/1.1 200 OK"),
            "expected 200 after aborted SSE clients, got: {}",
            &text[..text.len().min(80)]
        );
        assert!(text.contains("<!DOCTYPE html>"), "should serve the dashboard page");
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "server must stay responsive, took {:?}",
            start.elapsed()
        );
    }

    /// A healthy, connected SSE client must not block other requests
    /// either (this is what made the hang permanent: the dashboard tab's
    /// EventSource reconnects and then holds the connection).
    #[test]
    fn live_sse_client_does_not_block_other_requests() {
        let addr = start_test_server();

        // Open an SSE stream and keep it open.
        let mut sse = ClientStream::connect_timeout(&addr.into(), Duration::from_secs(2))
            .expect("connect sse");
        write!(sse, "GET /events HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        sse.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut first = [0u8; 512];
        let n = sse.read(&mut first).expect("first SSE frame");
        assert!(
            String::from_utf8_lossy(&first[..n]).contains("text/event-stream"),
            "SSE header expected"
        );

        // While it's connected, `/` must still answer.
        let resp = http_get(addr, "/", Duration::from_secs(3)).expect("GET / during live SSE");
        assert!(
            String::from_utf8_lossy(&resp).starts_with("HTTP/1.1 200 OK"),
            "GET / must succeed while an SSE client is connected"
        );

        // And a second SSE client can connect concurrently.
        let resp2 = http_get(addr, "/events", Duration::from_secs(3)).expect("second SSE client");
        let text2 = String::from_utf8_lossy(&resp2);
        assert!(text2.contains("data: "), "second SSE client should receive a frame");
        drop(sse);
    }

    /// Unknown paths still 404 under the threaded handler.
    #[test]
    fn unknown_path_returns_404() {
        let addr = start_test_server();
        let resp = http_get(addr, "/nope", Duration::from_secs(3)).expect("GET /nope");
        assert!(String::from_utf8_lossy(&resp).starts_with("HTTP/1.1 404"));
    }

    /// A client that connects and never sends a request must not hold its
    /// handler thread past the request read timeout — and must never affect
    /// other requests at all.
    #[test]
    fn half_open_client_does_not_block_server() {
        let addr = start_test_server();

        // Connect and send nothing.
        let idle = ClientStream::connect_timeout(&addr.into(), Duration::from_secs(2))
            .expect("idle connect");

        let resp = http_get(addr, "/", Duration::from_secs(3)).expect("GET / with idle peer");
        assert!(String::from_utf8_lossy(&resp).starts_with("HTTP/1.1 200 OK"));
        drop(idle);
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
