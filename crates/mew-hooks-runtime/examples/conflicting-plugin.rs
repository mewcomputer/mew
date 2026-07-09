//! Conflicting plugin for broker integration tests.
//!
//! Registers a tool named `sample-echo` (same as sample-plugin) to test
//! collision rejection. Transforms `on-system-prompt` by prepending
//! `[conflicting-plugin]` to test last-writer-wins ordering.
//!
//! Build: `cargo build --example conflicting-plugin`

use std::io::{BufRead, BufReader, Write};

fn main() {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut line = String::new();

    eprintln!("conflicting-plugin: ready");

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
            eprintln!("conflicting-plugin: init called");
            serde_json::json!("ok")
        }
        "shutdown" => {
            eprintln!("conflicting-plugin: shutdown called");
            serde_json::json!("ok")
        }
        "on-register-tools" => {
            // Register the SAME tool name as sample-plugin — triggers collision.
            serde_json::json!([{
                "name": "sample-echo",
                "description": "Conflicting echo tool",
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
            // Prepend [conflicting-plugin] — alphabetically after [sample-plugin]
            // so this should win in last-writer-wins ordering.
            let prompt = params["value"].as_str().unwrap_or("");
            eprintln!("conflicting-plugin: on-system-prompt called");
            serde_json::json!(format!("[conflicting-plugin] {prompt}"))
        }
        // Pass-through for all other hooks
        _ => params["value"].clone(),
    }
}
