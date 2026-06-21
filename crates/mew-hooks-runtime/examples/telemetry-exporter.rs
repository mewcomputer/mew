//! Prometheus exporter plugin for mew.
//!
//! Collects model token usage, cost, tool call counts, and turn latency
//! from the mew hooks system and exposes them as Prometheus metrics on
//! `http://localhost:9090/metrics`.
//!
//! Build: `cargo build --example telemetry-exporter`
//! Install: copy `target/debug/examples/telemetry-exporter` to
//!   `~/.config/mew/plugins/` or `<project>/.mew/plugins/`
//! View: `curl http://localhost:9090/metrics`
//!
//! Port can be overridden with `MEW_METRICS_PORT` (default 9090).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

/// Accumulated metrics state, shared between the stdin reader and the
/// HTTP server thread.
#[derive(Default)]
struct Metrics {
    // Model metrics
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    cost: f64,
    model_responses: u64,
    finishes_by_reason: std::collections::HashMap<String, u64>,

    // Tool metrics
    tool_calls: std::collections::HashMap<String, u64>,
    tool_errors: std::collections::HashMap<String, u64>,
    tool_durations_ms: std::collections::HashMap<String, Vec<u64>>,

    // Turn metrics
    turns: u64,

    // In-flight tool tracking (call_id → (tool_name, start_time))
    tool_in_flight: std::collections::HashMap<String, (String, Instant)>,
}

impl Metrics {
    fn render_prometheus(&self) -> String {
        let mut out = String::new();

        // -- Model metrics --
        out.push_str("# HELP mew_input_tokens_total Total input tokens consumed\n");
        out.push_str("# TYPE mew_input_tokens_total counter\n");
        out.push_str(&format!("mew_input_tokens_total {}\n", self.input_tokens));

        out.push_str("# HELP mew_output_tokens_total Total output tokens generated\n");
        out.push_str("# TYPE mew_output_tokens_total counter\n");
        out.push_str(&format!("mew_output_tokens_total {}\n", self.output_tokens));

        out.push_str("# HELP mew_reasoning_tokens_total Total reasoning tokens used\n");
        out.push_str("# TYPE mew_reasoning_tokens_total counter\n");
        out.push_str(&format!(
            "mew_reasoning_tokens_total {}\n",
            self.reasoning_tokens
        ));

        out.push_str("# HELP mew_cost_total Total cost in USD\n");
        out.push_str("# TYPE mew_cost_total counter\n");
        out.push_str(&format!("mew_cost_total {}\n", self.cost));

        out.push_str("# HELP mew_model_responses_total Number of model responses\n");
        out.push_str("# TYPE mew_model_responses_total counter\n");
        out.push_str(&format!(
            "mew_model_responses_total {}\n",
            self.model_responses
        ));

        for (reason, count) in &self.finishes_by_reason {
            out.push_str(&format!(
                "mew_model_finish_total{{finish=\"{}\"}} {}\n",
                reason, count
            ));
        }

        // -- Tool metrics --
        out.push_str("# HELP mew_tool_calls_total Tool invocations by name\n");
        out.push_str("# TYPE mew_tool_calls_total counter\n");
        for (name, count) in &self.tool_calls {
            out.push_str(&format!(
                "mew_tool_calls_total{{tool=\"{}\"}} {}\n",
                name, count
            ));
        }

        for (name, count) in &self.tool_errors {
            out.push_str(&format!(
                "mew_tool_errors_total{{tool=\"{}\"}} {}\n",
                name, count
            ));
        }

        // Tool duration percentiles (simple p50/p90)
        out.push_str("# HELP mew_tool_duration_ms Tool execution duration in ms\n");
        out.push_str("# TYPE mew_tool_duration_ms summary\n");
        for (name, durations) in &self.tool_durations_ms {
            if durations.is_empty() {
                continue;
            }
            let mut sorted = durations.clone();
            sorted.sort();
            let p50 = sorted[sorted.len() / 2];
            let p90_idx = (sorted.len() as f64 * 0.9) as usize;
            let p90 = sorted[p90_idx.min(sorted.len() - 1)];
            out.push_str(&format!(
                "mew_tool_duration_ms{{tool=\"{}\",quantile=\"0.5\"}} {}\n",
                name, p50
            ));
            out.push_str(&format!(
                "mew_tool_duration_ms{{tool=\"{}\",quantile=\"0.9\"}} {}\n",
                name, p90
            ));
        }

        // -- Turn metrics --
        out.push_str("# HELP mew_turns_total Total turns completed\n");
        out.push_str("# TYPE mew_turns_total counter\n");
        out.push_str(&format!("mew_turns_total {}\n", self.turns));

        out
    }
}

fn main() {
    let metrics = Arc::new(Mutex::new(Metrics::default()));
    let port: u16 = std::env::var("MEW_METRICS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9090);

    // Start HTTP server thread.
    let http_metrics = metrics.clone();
    thread::spawn(move || {
        let listener = match TcpListener::bind(format!("127.0.0.1:{}", port)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("telemetry-exporter: failed to bind :{} — {}", port, e);
                return;
            }
        };
        eprintln!(
            "telemetry-exporter: serving /metrics on http://localhost:{}",
            port
        );

        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            // Read the request line (we don't care about the method).
            let mut buf = [0u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut buf);

            let body = {
                let m = http_metrics.lock().unwrap();
                m.render_prometheus()
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    // JSON-RPC loop on stdin.
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut line = String::new();

    eprintln!("telemetry-exporter: ready");

    while reader.read_line(&mut line).is_ok() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            line.clear();
            continue;
        }

        let req: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => {
                line.clear();
                continue;
            }
        };

        let method = req["method"].as_str().unwrap_or("");
        let params = &req["params"];
        let result = handle_hook(method, params, &metrics);

        let resp = serde_json::json!({
            "jsonrpc": "2.0",
            "result": result,
            "id": req["id"],
        });

        let mut stdout = std::io::stdout();
        let _ = writeln!(stdout, "{}", serde_json::to_string(&resp).unwrap());
        let _ = stdout.flush();

        line.clear();
    }
}

fn handle_hook(
    method: &str,
    params: &serde_json::Value,
    metrics: &Arc<Mutex<Metrics>>,
) -> serde_json::Value {
    match method {
        "init" => {
            eprintln!("telemetry-exporter: init");
            serde_json::json!("ok")
        }

        "shutdown" => serde_json::json!("ok"),

        "on-model-finish" => {
            let finish = params["finish"].as_str().unwrap_or("unknown");
            let input = params["input_tokens"].as_u64().unwrap_or(0);
            let output = params["output_tokens"].as_u64().unwrap_or(0);
            let cost = params["cost"].as_f64().unwrap_or(0.0);

            let mut m = metrics.lock().unwrap();
            m.input_tokens += input;
            m.output_tokens += output;
            m.cost += cost;
            m.model_responses += 1;
            *m.finishes_by_reason.entry(finish.to_string()).or_insert(0) += 1;

            serde_json::json!("ok")
        }

        "on-tool-execute-before" => {
            // The value is the tool's input JSON. The tool name comes
            // from the notification params.
            let tool_name = params["tool_name"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let call_id = params["call_id"].as_str().unwrap_or("").to_string();

            let mut m = metrics.lock().unwrap();
            *m.tool_calls.entry(tool_name.clone()).or_insert(0) += 1;
            m.tool_in_flight
                .insert(call_id, (tool_name, Instant::now()));

            // Return the input unchanged (observation only).
            params["value"].clone()
        }

        "on-tool-execute-after" => {
            let call_id = params["call_id"].as_str().unwrap_or("").to_string();

            let mut m = metrics.lock().unwrap();
            if let Some((name, start)) = m.tool_in_flight.remove(&call_id) {
                let dur = start.elapsed().as_millis() as u64;
                m.tool_durations_ms.entry(name).or_default().push(dur);
            }

            // Return the output unchanged.
            params["value"].clone()
        }

        "on-tool-error" => {
            let tool_name = params["tool_name"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let mut m = metrics.lock().unwrap();
            *m.tool_errors.entry(tool_name).or_insert(0) += 1;
            serde_json::json!("ok")
        }

        "on-turn-end" => {
            let mut m = metrics.lock().unwrap();
            m.turns += 1;
            serde_json::json!("ok")
        }

        // All other hooks: passthrough (observation only, no mutation).
        _ => params
            .get("value")
            .cloned()
            .unwrap_or(serde_json::json!("ok")),
    }
}
