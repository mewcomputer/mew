//! Mobile worker activity and background job reads.

use crate::{DaemonId, MobileCore};
use mew_protocol::ClientMessage;

#[derive(Debug, Clone, uniffi::Record)]
pub struct WorkerInfo {
    pub task_id: String,
    pub child_session_id: Option<String>,
    pub name: String,
    pub display_name: Option<String>,
    pub status: String,
    pub tool_name: Option<String>,
}

impl From<&mew_protocol::SubagentActivity> for WorkerInfo {
    fn from(worker: &mew_protocol::SubagentActivity) -> Self {
        Self {
            task_id: worker.task_id.clone(),
            child_session_id: worker.child_session_id.clone(),
            name: worker.name.clone(),
            display_name: worker.display_name.clone(),
            status: worker.status.clone(),
            tool_name: worker.tool_name.clone(),
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct BackgroundJob {
    pub job_id: String,
    pub command: String,
    pub state: String,
}

impl From<&mew_protocol::JobActivity> for BackgroundJob {
    fn from(job: &mew_protocol::JobActivity) -> Self {
        Self {
            job_id: job.job_id.clone(),
            command: job.command.clone(),
            state: job.state.clone(),
        }
    }
}

#[uniffi::export]
impl MobileCore {
    pub fn read_subagent_history(
        &self,
        id: DaemonId,
        session_id: String,
        child_session_id: String,
        request_id: String,
    ) -> bool {
        self.connections
            .lock()
            .unwrap()
            .get(&id.node_id)
            .is_some_and(|conn| {
                conn.send(ClientMessage::ReadSubagentHistory {
                    session_id,
                    child_session_id,
                    request_id,
                })
            })
    }

    pub fn read_job_output(
        &self,
        id: DaemonId,
        session_id: String,
        job_id: String,
        request_id: String,
        offset: u64,
    ) -> bool {
        self.connections
            .lock()
            .unwrap()
            .get(&id.node_id)
            .is_some_and(|conn| {
                conn.send(ClientMessage::ReadJobOutput {
                    session_id,
                    job_id,
                    request_id,
                    offset,
                })
            })
    }
}
