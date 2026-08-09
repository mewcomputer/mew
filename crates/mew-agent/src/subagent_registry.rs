//! Durable subagent task registry.
//!
//! Outstanding subagent tasks (running or finished but uncollected) are
//! persisted to `<session>/subagent_tasks.json` so a resumed session can see
//! what the previous run left behind. Unlike todos, tasks cannot be re-attached
//! after the process dies — on resume every record is orphaned and surfaced to
//! the model once, with a best-effort recovery of the child's final text from
//! its transcript.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// One outstanding subagent task, persisted on spawn and removed on collect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentTaskRecord {
    pub task_id: String,
    pub name: String,
    #[serde(default)]
    pub todo_id: Option<usize>,
    /// Child session id, filled in once the runner reports `Started`. May be
    /// absent if the session ended before the child registered.
    #[serde(default)]
    pub child_session_id: Option<String>,
    pub started_at: i64,
}

/// Persist the registry (creates parent dirs). Best-effort: callers log and
/// continue on failure.
pub(crate) async fn save(path: &Path, records: &[SubagentTaskRecord]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let data = serde_json::to_string_pretty(records).map_err(|e| e.to_string())?;
    tokio::fs::write(path, data)
        .await
        .map_err(|e| e.to_string())
}

/// Load the registry. Missing file → empty.
pub(crate) async fn load(path: &Path) -> Result<Vec<SubagentTaskRecord>, String> {
    match tokio::fs::read_to_string(path).await {
        Ok(data) => serde_json::from_str(&data).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// Best-effort recovery of a child subagent's final assistant text from its
/// transcript. `session_dir` is the parent session's directory; child
/// transcripts live under `<session_dir>/subagents/<child_id>`.
pub(crate) async fn recover_child_text(
    session_dir: &Path,
    child_session_id: &str,
) -> Option<String> {
    let subagents_root = session_dir.join("subagents");
    let messages = mew_session::Reader::load_from(&subagents_root, child_session_id)
        .await
        .ok()?;
    let last_assistant = messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, mew_message::Role::Assistant))?;
    let text: String = last_assistant
        .parts
        .iter()
        .filter_map(|p| match p {
            mew_message::Part::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(task_id: &str) -> SubagentTaskRecord {
        SubagentTaskRecord {
            task_id: task_id.into(),
            name: "researcher".into(),
            todo_id: None,
            child_session_id: None,
            started_at: 1_700_000_000_000,
        }
    }

    #[tokio::test]
    async fn test_registry_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("sess").join("subagent_tasks.json");

        let mut a = record("sa_a");
        a.todo_id = Some(3);
        a.child_session_id = Some("child_1".into());
        let records = vec![a.clone(), record("sa_b")];
        save(&path, &records).await.unwrap();

        let loaded = load(&path).await.unwrap();
        assert_eq!(loaded, records);
    }

    #[tokio::test]
    async fn test_registry_load_missing_is_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("nope").join("subagent_tasks.json");
        assert!(load(&path).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_recover_child_text_reads_last_assistant_message() {
        // Build a parent session with a subagent child under
        // <root>/<parent>/subagents/<child>.
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let parent_id = "sess_parent";
        let child_id = "sess_child";
        let mut writer = mew_session::Writer::open_subagent_at(root, parent_id, child_id, "stub")
            .await
            .expect("open child");
        let msg = mew_message::Message {
            id: mew_message::MessageId::new(),
            session_id: mew_message::SessionId::new(),
            role: mew_message::Role::Assistant,
            parts: vec![mew_message::Part::Text(mew_message::TextPart {
                base: mew_message::PartBase {
                    id: mew_message::PartId::new(),
                    message_id: mew_message::MessageId::new(),
                    session_id: mew_message::SessionId::new(),
                },
                text: "child final answer".into(),
                synthetic: false,
            })],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };
        writer.write_message(&msg).await.expect("write");

        let session_dir = root.join(parent_id);
        let recovered = recover_child_text(&session_dir, child_id).await;
        assert_eq!(recovered.as_deref(), Some("child final answer"));

        // Unknown child → None (marked lost by the caller).
        assert!(recover_child_text(&session_dir, "sess_missing")
            .await
            .is_none());
    }
}
