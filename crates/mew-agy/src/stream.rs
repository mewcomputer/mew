use std::io;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliEvent {
    Text(String),
    Usage {
        input: u64,
        output: u64,
        cache_read: u64,
    },
    Finished,
}

/// Parses bounded newline-delimited CLI events without losing split UTF-8.
pub struct CliStream {
    buffer: Vec<u8>,
    max_frame_bytes: usize,
    finished: bool,
}

impl CliStream {
    pub fn new(max_frame_bytes: usize) -> Self {
        Self {
            buffer: Vec::new(),
            max_frame_bytes,
            finished: false,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> io::Result<Vec<CliEvent>> {
        let mut events = Vec::new();
        for part in bytes.split_inclusive(|byte| *byte == b'\n') {
            if self.buffer.len().saturating_add(part.len()) > self.max_frame_bytes {
                return Err(invalid("AGY stream frame is too large"));
            }
            self.buffer.extend_from_slice(part);
            if part.last() != Some(&b'\n') {
                continue;
            }
            let frame = std::mem::take(&mut self.buffer);
            if frame.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            if self.finished {
                return Err(invalid("AGY emitted an event after its result"));
            }
            let value: Value =
                serde_json::from_slice(&frame).map_err(|_| invalid("invalid AGY stream frame"))?;
            match value.get("event").and_then(Value::as_str) {
                Some("init") => {}
                Some("step_update") => {
                    let step = value
                        .get("step_update")
                        .and_then(Value::as_object)
                        .ok_or_else(|| invalid("invalid AGY step update"))?;
                    if step.get("step_type").and_then(Value::as_str) == Some("agent_response") {
                        if let Some(text) = step
                            .get("text_delta")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                        {
                            events.push(CliEvent::Text(text.to_owned()));
                        }
                    }
                    if step.get("state").and_then(Value::as_str) == Some("DONE") {
                        if let Some(usage) = step.get("usage").and_then(Value::as_object) {
                            let count = |key| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
                            events.push(CliEvent::Usage {
                                input: count("input_tokens"),
                                output: count("output_tokens"),
                                cache_read: count("cache_read_tokens"),
                            });
                        }
                    }
                }
                Some("result") => {
                    let result = value
                        .get("result")
                        .and_then(Value::as_object)
                        .ok_or_else(|| invalid("invalid AGY result"))?;
                    if result.get("status").and_then(Value::as_str) != Some("SUCCESS") {
                        let message = result
                            .get("error")
                            .and_then(Value::as_str)
                            .filter(|message| !message.is_empty())
                            .unwrap_or("AGY returned an unsuccessful result")
                            .chars()
                            .take(1024)
                            .collect::<String>();
                        return Err(io::Error::new(io::ErrorKind::InvalidData, message));
                    }
                    match result.get("denied_actions") {
                        None | Some(Value::Null) => {}
                        Some(Value::Array(actions)) if actions.is_empty() => {}
                        _ => return Err(invalid("AGY denied an action")),
                    }
                    self.finished = true;
                    events.push(CliEvent::Finished);
                }
                _ => return Err(invalid("unknown AGY stream event")),
            }
        }
        Ok(events)
    }

    pub fn finish(&self) -> io::Result<()> {
        if !self.finished || !self.buffer.is_empty() {
            return Err(invalid("AGY stream ended before its result"));
        }
        Ok(())
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
