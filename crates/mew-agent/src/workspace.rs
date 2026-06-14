use std::path::{Path, PathBuf};
use tokio::sync::{mpsc, oneshot};

use mew_hooks::PermissionDecision;

use super::{Agent, AgentEvent};

impl Agent {
    /// True if `path` is within any workspace root or session allowance.
    fn is_within_workspace(&self, path: &Path) -> bool {
        // First check static roots.
        if self
            .workspace_roots
            .iter()
            .any(|root| path.starts_with(root))
        {
            return true;
        }
        // Try session allowances (sync try_lock — these are uncontended, small scope).
        if let Ok(allowances) = self.workspace_allowances.try_lock() {
            if allowances.iter().any(|a| path.starts_with(a)) {
                return true;
            }
        }
        false
    }

    /// Check workspace containment for a tool path. If outside all roots
    /// and session allowances, request user approval. Returns `Err` with an
    /// error message if denied.
    pub(crate) async fn ensure_workspace_path(
        &self,
        tool_path: &Path,
        ev_tx: &mpsc::Sender<AgentEvent>,
    ) -> Result<(), String> {
        if self.is_within_workspace(tool_path) {
            return Ok(());
        }

        // Outside workspace — request permission for the containing directory.
        let dir = tool_path.parent().unwrap_or(tool_path).to_path_buf();

        let (tx, rx) = oneshot::channel();
        let _ = ev_tx
            .send(AgentEvent::WorkspacePermissionRequest {
                path: dir.clone(),
                tx,
            })
            .await;

        match rx.await {
            Ok(PermissionDecision::AllowOnce | PermissionDecision::AllowSession) => {
                self.workspace_allowances.lock().await.insert(dir);
                Ok(())
            }
            _ => Err("path outside workspace (permission denied)".into()),
        }
    }

    /// Extract the path argument from tool input, if this tool operates on files.
    pub(crate) fn workspace_path_for_tool(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> Option<PathBuf> {
        match tool_name {
            "read" | "write" | "edit" => input
                .get("path")
                .and_then(|v| v.as_str())
                .map(PathBuf::from),
            "glob" | "grep" => {
                // These operate on a directory; default to ".".
                let s = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                Some(PathBuf::from(s))
            }
            _ => None,
        }
    }
}
