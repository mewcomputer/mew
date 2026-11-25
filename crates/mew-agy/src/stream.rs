use std::io;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AcpFrame {
    Response {
        id: u64,
        result: Option<Value>,
        error: Option<RpcError>,
    },
    Notification {
        method: String,
        params: Value,
    },
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

/// Parses bounded newline-delimited JSON-RPC 2.0 ACP frames without losing split UTF-8.
pub struct AcpStream {
    buffer: Vec<u8>,
    max_frame_bytes: usize,
}

impl AcpStream {
    pub fn new(max_frame_bytes: usize) -> Self {
        Self {
            buffer: Vec::new(),
            max_frame_bytes,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> io::Result<Vec<AcpFrame>> {
        let mut frames = Vec::new();
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
            let value: Value =
                serde_json::from_slice(&frame).map_err(|_| invalid("invalid AGY stream frame"))?;
            let obj = value
                .as_object()
                .ok_or_else(|| invalid("invalid AGY stream frame"))?;
            if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
                return Err(invalid("invalid AGY JSON-RPC version"));
            }
            if let Some(method) = obj.get("method").and_then(Value::as_str) {
                let params = obj.get("params").cloned().unwrap_or(Value::Null);
                if let Some(id) = obj.get("id") {
                    if !(id.is_string() || id.is_u64() || id.is_i64()) {
                        return Err(invalid("invalid AGY JSON-RPC request id"));
                    }
                    frames.push(AcpFrame::Request {
                        id: id.clone(),
                        method: method.to_owned(),
                        params,
                    });
                } else {
                    frames.push(AcpFrame::Notification {
                        method: method.to_owned(),
                        params,
                    });
                }
            } else if let Some(id) = obj.get("id").and_then(Value::as_u64) {
                let error = match obj.get("error") {
                    Some(err) => {
                        let code = err
                            .get("code")
                            .and_then(Value::as_i64)
                            .ok_or_else(|| invalid("invalid AGY JSON-RPC error"))?;
                        let message = err
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("AGY request failed")
                            .chars()
                            .take(1024)
                            .collect();
                        Some(RpcError { code, message })
                    }
                    None => None,
                };
                let result = obj.get("result").cloned();
                if result.is_none() && error.is_none() {
                    return Err(invalid("invalid AGY JSON-RPC response"));
                }
                frames.push(AcpFrame::Response { id, result, error });
            } else {
                return Err(invalid("unknown AGY JSON-RPC frame"));
            }
        }
        Ok(frames)
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
