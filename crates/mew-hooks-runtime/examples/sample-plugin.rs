//! Sample plugin demonstrating all hook points.
//!
//! This binary reads JSON-RPC method calls from stdin and writes
//! responses to stdout. It's loaded by the SubprocessDispatcher.
//!
//! Build: `cargo build --example sample-plugin`
//! Install: copy `target/debug/examples/sample-plugin` to
//!   `~/.config/mew/plugins/` or `<project>/.mew/plugins/`

use std::io::{BufRead, BufReader, Write};

fn main() {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut line = String::new();

    eprintln!("sample-plugin: ready");

    while reader.read_line(&mut line).is_ok() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            line.clear();
            continue;
        }

        let req: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("sample-plugin: parse error: {e}");
                line.clear();
                continue;
            }
        };

        let method = req["method"].as_str().unwrap_or("");
        let params = &req["params"];

        let result = handle(method, params);

        let resp = serde_json::json!({
            "jsonrpc": "2.0",
            "result": result,
            "id": req["id"],
        });

        let mut stdout = std::io::stdout();
        writeln!(stdout, "{}", serde_json::to_string(&resp).unwrap()).unwrap();
        stdout.flush().unwrap();

        line.clear();
    }
}

fn handle(method: &str, params: &serde_json::Value) -> serde_json::Value {
    match method {
        "init" => {
            eprintln!("sample-plugin: init called");
            serde_json::json!("ok")
        }

        "shutdown" => {
            eprintln!("sample-plugin: shutdown called");
            serde_json::json!("ok")
        }

        "on-chat-headers" => {
            // Add a custom header. params["value"] is a JSON string.
            let json_str = params["value"].as_str().unwrap_or("[]");
            let mut headers: Vec<(String, String)> =
                serde_json::from_str(json_str).unwrap_or_default();
            headers.push(("x-plugin".to_string(), "sample-plugin".to_string()));
            eprintln!("sample-plugin: added x-plugin header");
            serde_json::to_value(&headers).unwrap()
        }

        "on-permission-ask" => {
            // Deny "AllowOnce" (simulating a plugin that requires explicit session grant)
            let decision = params["value"].as_str().unwrap_or("");
            let new_decision = if decision == "AllowOnce" {
                eprintln!("sample-plugin: denying AllowOnce → Deny");
                "Deny"
            } else {
                decision
            };
            serde_json::json!(new_decision)
        }

        "on-tool-execute-after" => {
            // Redact "SECRET" from tool output.
            let json_str = params["value"].as_str().unwrap_or("{}");
            let mut output: serde_json::Value = serde_json::from_str(json_str).unwrap_or_default();
            if let Some(out) = output.get("output").and_then(|v| v.as_str()) {
                if out.contains("SECRET") {
                    let redacted = out.replace("SECRET", "***REDACTED***");
                    output["output"] = serde_json::json!(redacted);
                    eprintln!("sample-plugin: redacted SECRET from output");
                }
            }
            output
        }

        "on-register-tools" => {
            // Return a dynamic tool
            serde_json::json!([{
                "name": "sample-echo",
                "description": "Echoes the input back (sample plugin tool)",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "message": { "type": "string" }
                    },
                    "required": ["message"]
                }
            }])
        }

        "on-system-prompt" => {
            let prompt = params["value"].as_str().unwrap_or("");
            eprintln!("sample-plugin: on-system-prompt called, prepending plugin greeting");
            serde_json::json!(format!("[sample-plugin] {prompt}"))
        }

        "on-turn-end" => {
            eprintln!("sample-plugin: on-turn-end called");
            serde_json::json!("ok")
        }

        "on-register-slash-commands" => {
            serde_json::json!([{
                "name": "/sample-plugin",
                "description": "sample plugin slash command",
                "handler_id": "sample-slash-handler"
            }])
        }

        "execute-slash-command" => {
            let command = params["command"].as_str().unwrap_or("");
            eprintln!("sample-plugin: execute-slash-command '{command}'");
            serde_json::json!(format!("sample-plugin executed: {command}"))
        }

        // Pass-through hooks: return value unchanged
        _ => params["value"].clone(),
    }
}
