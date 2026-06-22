//! Background shell job tools. These are placeholder structs — actual
//! execution is intercepted by the agent core (same pattern as
//! `subagent_start`). The agent has the shell job registry.

use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

/// Launch a shell command in the background. Returns a job ID immediately.
pub struct ShellBackground;

#[async_trait]
impl Tool for ShellBackground {
    fn name(&self) -> &str {
        "shell_background"
    }

    fn description(&self) -> &str {
        "Launch a shell command in the background and return a job ID \
         immediately. The command runs detached from the agent's turn loop \
         — use job_status or job_block to check on it later. Ideal for \
         long-running processes: builds, test suites, dev servers. The \
         job accumulates stdout+stderr; retrieve it with job_result or \
         job_block."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to run."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Working directory (default: current)."
                    }
                },
                "required": ["command"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::Dangerous
    }

    async fn execute(&self, _ctx: ToolCtx, _input: Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Execution(
            "shell_background execution must be handled by the agent core".into(),
        ))
    }
}

/// Check the status of a background job.
pub struct JobStatus;

#[async_trait]
impl Tool for JobStatus {
    fn name(&self) -> &str {
        "job_status"
    }

    fn description(&self) -> &str {
        "Check the status of a background job (shell or subagent). Returns \
         the current state (running/completed/failed/cancelled) and any \
         accumulated output so far."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "job_id": {
                        "type": "string",
                        "description": "The job ID returned by shell_background."
                    }
                },
                "required": ["job_id"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, _ctx: ToolCtx, _input: Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Execution(
            "job_status execution must be handled by the agent core".into(),
        ))
    }
}

/// Wait for a background job to finish (blocking, with timeout).
pub struct JobBlock;

#[async_trait]
impl Tool for JobBlock {
    fn name(&self) -> &str {
        "job_block"
    }

    fn description(&self) -> &str {
        "Wait for a background job to reach a terminal state (completed, \
         failed, or cancelled), up to an optional timeout. Returns the \
         final state and full output. If the timeout fires while still \
         running, returns the current partial output."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "job_id": {
                        "type": "string",
                        "description": "The job ID returned by shell_background."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Maximum seconds to wait (default: 300)."
                    }
                },
                "required": ["job_id"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, _ctx: ToolCtx, _input: Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Execution(
            "job_block execution must be handled by the agent core".into(),
        ))
    }
}

/// Cancel a running background job.
pub struct JobCancel;

#[async_trait]
impl Tool for JobCancel {
    fn name(&self) -> &str {
        "job_cancel"
    }

    fn description(&self) -> &str {
        "Cancel a running background job by killing its process. No-op if \
         the job already finished."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "job_id": {
                        "type": "string",
                        "description": "The job ID to cancel."
                    }
                },
                "required": ["job_id"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::Mutating
    }

    async fn execute(&self, _ctx: ToolCtx, _input: Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Execution(
            "job_cancel execution must be handled by the agent core".into(),
        ))
    }
}

/// Run a command and wait for it to exit successfully. Polls the job in a
/// loop (via shell_background + job_block) until it exits 0 or the
/// timeout fires. Returns the accumulated output and timing on success,
/// or the last output + failure reason on timeout. Ideal for readiness
/// checks: "wait for the dev server to be up".
pub struct ShellMonitor;

#[async_trait]
impl Tool for ShellMonitor {
    fn name(&self) -> &str {
        "shell_monitor"
    }

    fn description(&self) -> &str {
        "Run a command and wait for it to exit successfully. Launches \
         the command via shell_background, then blocks until it exits 0 \
         or the timeout (default 60s) fires. Returns the output and \
         elapsed time on success, or the failure reason on timeout. Use \
         for readiness checks: 'wait for the dev server to bind to \
         port 3000' or 'wait for the build to finish'."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command to run. Should exit 0 when 'ready'."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Maximum seconds to wait (default: 60)."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Working directory (default: current)."
                    }
                },
                "required": ["command"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::Dangerous
    }

    async fn execute(&self, _ctx: ToolCtx, _input: Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Execution(
            "shell_monitor execution must be handled by the agent core".into(),
        ))
    }
}
