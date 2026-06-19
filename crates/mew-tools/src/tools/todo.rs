//! Todo tools. All five are agent-intercepted (their `execute()` errors);
//! `Agent::execute_todo` in `mew-agent/src/tools.rs` owns the state mutation,
//! dependency enforcement, and persistence. See `mew_agent::TodoList` for the
//! rules.

use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

/// Create one or more todos. Batchable: pass multiple items in one call.
pub struct TodoCreate;

#[async_trait]
impl Tool for TodoCreate {
    fn name(&self) -> &str {
        "todo_create"
    }
    fn description(&self) -> &str {
        "Create one or more todos. Each todo tracks a step of the work in \
         progress and survives context compaction, so neither you nor the user \
         loses track of what's left. Pass multiple items to batch. \
         `depends_on` lists ids that must be done before this one can be \
         completed; references to nonexistent ids are dropped."
    }
    fn schema(&self) -> &Value {
        static S: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        S.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": { "type": "string", "description": "What needs doing." },
                                "depends_on": {
                                    "type": "array",
                                    "items": { "type": "integer" },
                                    "description": "Ids of todos that must be done before this one."
                                }
                            },
                            "required": ["content"]
                        }
                    }
                },
                "required": ["todos"]
            })
        })
    }
    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }
    async fn execute(&self, _ctx: ToolCtx, _input: Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Execution(
            "todo_create execution must be handled by the agent core".into(),
        ))
    }
}

/// Update a todo's content and/or status. Moving to `done` enforces
/// dependencies (same as `todo_complete`).
pub struct TodoUpdate;

#[async_trait]
impl Tool for TodoUpdate {
    fn name(&self) -> &str {
        "todo_update"
    }
    fn description(&self) -> &str {
        "Update a todo's content and/or status. Use `in_progress` when you \
         start it and `done` when you finish (completion is blocked until its \
         dependencies are done)."
    }
    fn schema(&self) -> &Value {
        static S: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        S.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "The todo id." },
                    "content": { "type": "string", "description": "New content, if changing it." },
                    "status": {
                        "type": "string",
                        "enum": ["pending", "in_progress", "done", "blocked"],
                        "description": "New status, if changing it."
                    }
                },
                "required": ["id"]
            })
        })
    }
    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }
    async fn execute(&self, _ctx: ToolCtx, _input: Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Execution(
            "todo_update execution must be handled by the agent core".into(),
        ))
    }
}

/// Mark a todo done. Blocked if any dependency isn't done yet.
pub struct TodoComplete;

#[async_trait]
impl Tool for TodoComplete {
    fn name(&self) -> &str {
        "todo_complete"
    }
    fn description(&self) -> &str {
        "Mark a todo done. This is refused if any of its dependencies aren't \
         done yet — finish those first."
    }
    fn schema(&self) -> &Value {
        static S: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        S.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "The todo id to complete." }
                },
                "required": ["id"]
            })
        })
    }
    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }
    async fn execute(&self, _ctx: ToolCtx, _input: Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Execution(
            "todo_complete execution must be handled by the agent core".into(),
        ))
    }
}

/// Delete a todo. Refused if another todo depends on it.
pub struct TodoDelete;

#[async_trait]
impl Tool for TodoDelete {
    fn name(&self) -> &str {
        "todo_delete"
    }
    fn description(&self) -> &str {
        "Delete a todo. Refused if another todo depends on it — clear or \
         re-point those first. Ids of remaining todos don't shift."
    }
    fn schema(&self) -> &Value {
        static S: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        S.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "The todo id to delete." }
                },
                "required": ["id"]
            })
        })
    }
    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }
    async fn execute(&self, _ctx: ToolCtx, _input: Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Execution(
            "todo_delete execution must be handled by the agent core".into(),
        ))
    }
}

/// List all todos with their statuses and dependencies.
pub struct TodoListTool;

#[async_trait]
impl Tool for TodoListTool {
    fn name(&self) -> &str {
        "todo_list"
    }
    fn description(&self) -> &str {
        "List all todos in the session with their ids, statuses, and \
         dependencies. Call this before planning next steps so you work from \
         current state, not memory."
    }
    fn schema(&self) -> &Value {
        static S: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        S.get_or_init(|| serde_json::json!({ "type": "object", "properties": {} }))
    }
    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }
    async fn execute(&self, _ctx: ToolCtx, _input: Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Execution(
            "todo_list execution must be handled by the agent core".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_names() {
        assert_eq!(TodoCreate.name(), "todo_create");
        assert_eq!(TodoUpdate.name(), "todo_update");
        assert_eq!(TodoComplete.name(), "todo_complete");
        assert_eq!(TodoDelete.name(), "todo_delete");
        assert_eq!(TodoListTool.name(), "todo_list");
    }

    #[test]
    fn test_all_readonly() {
        assert_eq!(TodoCreate.sensitivity(), Sensitivity::ReadOnly);
        assert_eq!(TodoUpdate.sensitivity(), Sensitivity::ReadOnly);
        assert_eq!(TodoComplete.sensitivity(), Sensitivity::ReadOnly);
        assert_eq!(TodoDelete.sensitivity(), Sensitivity::ReadOnly);
        assert_eq!(TodoListTool.sensitivity(), Sensitivity::ReadOnly);
    }
}
