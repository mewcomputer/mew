use std::collections::BTreeMap;

use mew_protocol::{JobActivity, ServerMessage, SubagentActivity};
use tokio::sync::mpsc;

#[derive(Default)]
pub(crate) struct SessionActivity {
    subagents: BTreeMap<String, SubagentActivity>,
    jobs: BTreeMap<String, JobActivity>,
}

impl SessionActivity {
    pub(crate) fn apply(&mut self, message: &ServerMessage) {
        match message {
            ServerMessage::SubagentStart {
                parent_call_id,
                child_session_id,
                name,
                display_name,
            } => {
                self.subagents.insert(
                    parent_call_id.clone(),
                    SubagentActivity {
                        task_id: parent_call_id.clone(),
                        child_session_id: Some(child_session_id.clone()),
                        name: name.clone(),
                        display_name: display_name.clone(),
                        status: "running".into(),
                        tool_name: None,
                    },
                );
            }
            ServerMessage::SubagentStatus {
                parent_call_id,
                tool_name,
                message,
            } => {
                let entry = self
                    .subagents
                    .entry(parent_call_id.clone())
                    .or_insert_with(|| SubagentActivity {
                        task_id: parent_call_id.clone(),
                        child_session_id: None,
                        name: String::new(),
                        display_name: None,
                        status: String::new(),
                        tool_name: None,
                    });
                entry.status.clone_from(message);
                entry.tool_name = Some(tool_name.clone());
            }
            ServerMessage::SubagentEnd {
                parent_call_id,
                child_session_id,
                ..
            } => {
                if self.subagents.get(parent_call_id).is_some_and(|entry| {
                    entry
                        .child_session_id
                        .as_deref()
                        .is_none_or(|child| child == child_session_id)
                }) {
                    self.subagents.remove(parent_call_id);
                }
            }
            ServerMessage::JobUpdate {
                job_id,
                command,
                state,
            } => {
                self.jobs.insert(
                    job_id.clone(),
                    JobActivity {
                        job_id: job_id.clone(),
                        command: command.clone(),
                        state: state.clone(),
                    },
                );
            }
            _ => {}
        }
    }
}

impl crate::session::Session {
    pub(crate) async fn send_activity(&self, sender: &mpsc::UnboundedSender<ServerMessage>) {
        // Activity publications hold this lock until their event is queued.
        // The snapshot therefore precedes or includes every concurrent update.
        let activity = self.activity.lock().await;
        let _ = sender.send(ServerMessage::SessionActivitySnapshot {
            session_id: self.id.clone(),
            subagents: activity.subagents.values().cloned().collect(),
            jobs: activity.jobs.values().cloned().collect(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn session() -> Arc<crate::session::Session> {
        Arc::new(crate::session::Session::new(
            "session".into(),
            mew_agent::Agent::new(
                Arc::new(mew_provider_fake::FakeProvider::new(Vec::new())),
                Arc::new(mew_hooks::NopDispatcher),
                None,
                Vec::new(),
                None,
            ),
            None,
            None,
            std::path::PathBuf::new(),
        ))
    }

    #[tokio::test]
    async fn old_child_completion_cannot_remove_a_restarted_activity() {
        let session = session();
        for child in ["old", "new"] {
            session
                .broadcast(ServerMessage::SubagentStart {
                    parent_call_id: "task".into(),
                    child_session_id: child.into(),
                    name: "worker".into(),
                    display_name: None,
                })
                .await;
        }
        session
            .broadcast(ServerMessage::SubagentEnd {
                parent_call_id: "task".into(),
                child_session_id: "old".into(),
                outcome: mew_protocol::SubagentOutcome::Completed,
                manifests: Vec::new(),
            })
            .await;
        let (sender, mut receiver) = mpsc::unbounded_channel();
        session.send_activity(&sender).await;
        let ServerMessage::SessionActivitySnapshot { subagents, .. } =
            receiver.recv().await.unwrap()
        else {
            panic!("missing activity snapshot");
        };
        assert_eq!(subagents.len(), 1);
        assert_eq!(subagents[0].child_session_id.as_deref(), Some("new"));
        session
            .broadcast(ServerMessage::SubagentEnd {
                parent_call_id: "task".into(),
                child_session_id: "new".into(),
                outcome: mew_protocol::SubagentOutcome::Cancelled,
                manifests: Vec::new(),
            })
            .await;
        session.send_activity(&sender).await;
        assert!(
            matches!(receiver.recv().await.unwrap(),ServerMessage::SessionActivitySnapshot {subagents,..} if subagents.is_empty())
        );
    }

    #[tokio::test]
    async fn snapshot_cannot_overtake_an_activity_publication() {
        let session = session();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        session
            .attach_client(sender.clone(), mew_protocol::ClientKind::Tui)
            .await;
        let clients = session.clients.lock().await;
        let publisher = session.clone();
        let publish = tokio::spawn(async move {
            publisher
                .broadcast(ServerMessage::JobUpdate {
                    job_id: "job".into(),
                    command: "sleep 1".into(),
                    state: "running".into(),
                })
                .await;
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while session.activity.try_lock().is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let snapshots = session.clone();
        let snapshot = tokio::spawn(async move {
            snapshots.send_activity(&sender).await;
        });
        tokio::task::yield_now().await;
        assert!(receiver.try_recv().is_err());
        drop(clients);
        publish.await.unwrap();
        snapshot.await.unwrap();
        assert!(matches!(
            receiver.recv().await.unwrap(),
            ServerMessage::JobUpdate { .. }
        ));
        assert!(
            matches!(receiver.recv().await.unwrap(),ServerMessage::SessionActivitySnapshot {jobs,..} if jobs.len()==1 && jobs[0].job_id=="job")
        );
    }
}
