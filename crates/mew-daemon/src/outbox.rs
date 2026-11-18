use std::{sync::Arc, time::Duration};

use anyhow::{ensure, Result};
use mew_protocol::ServerMessage;
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

const MAX_QUEUED_FRAMES: usize = 1024;
const MAX_QUEUED_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct ClientSender {
    sender: mpsc::Sender<ClientFrame>,
    bytes: Arc<Semaphore>,
    history_slot: Arc<Semaphore>,
    lifetime: CancellationToken,
}

pub struct ClientFrames(pub(crate) mpsc::Receiver<ClientFrame>);

pub(crate) struct ClientFrame {
    pub(crate) json: String,
    // Keep bytes charged until the socket finishes writing this frame.
    pub(crate) budget: OwnedSemaphorePermit,
}

impl ClientSender {
    pub fn channel(lifetime: CancellationToken) -> (Self, ClientFrames) {
        let (sender, receiver) = mpsc::channel(MAX_QUEUED_FRAMES);
        (
            Self {
                sender,
                bytes: Arc::new(Semaphore::new(MAX_QUEUED_BYTES)),
                history_slot: Arc::new(Semaphore::new(1)),
                lifetime,
            },
            ClientFrames(receiver),
        )
    }

    pub(crate) fn send(&self, message: ServerMessage) -> Result<()> {
        self.send_ref(&message)
    }

    pub(crate) fn send_ref(&self, message: &ServerMessage) -> Result<()> {
        let result = (|| {
            ensure!(!self.lifetime.is_cancelled(), "connection closed");
            let json = mew_protocol::encode_json(message)?;
            let bytes = u32::try_from(json.capacity())?;
            let budget = self.bytes.clone().try_acquire_many_owned(bytes)?;
            self.sender
                .try_send(ClientFrame { json, budget })
                .map_err(|_| anyhow::anyhow!("client queue full or closed"))
        })();
        if result.is_err() {
            self.lifetime.cancel();
        }
        result
    }

    pub(crate) async fn send_history_page(&self, message: ServerMessage) -> Result<()> {
        // Bulk history can be larger than the entire queue. Feed one page at a
        // time so a healthy client can drain it without an arbitrary page limit.
        let result = tokio::select! {
            _ = self.lifetime.cancelled() => Err(anyhow::anyhow!("connection closed")),
            result = tokio::time::timeout(WRITE_TIMEOUT, async {
                let budget = self.history_slot.clone().acquire_owned().await?;
                let json = mew_protocol::encode_json(&message)?;
                ensure!(json.capacity() <= MAX_QUEUED_BYTES, "history frame exceeds client queue");
                self.sender.send(ClientFrame { json, budget }).await
                    .map_err(|_| anyhow::anyhow!("connection closed"))
            }) => result.unwrap_or_else(|error| Err(error.into())),
        };
        if result.is_err() {
            self.lifetime.cancel();
        }
        result
    }
}

#[cfg(test)]
impl ClientFrames {
    pub(crate) async fn recv(&mut self) -> Option<ServerMessage> {
        let frame = self.0.recv().await?;
        Some(mew_protocol::decode_json(&frame.json).unwrap())
    }

    pub(crate) fn try_recv(&mut self) -> Result<ServerMessage> {
        let frame = self.0.try_recv()?;
        Ok(mew_protocol::decode_json(&frame.json)?)
    }
}

#[cfg(test)]
#[tokio::test]
async fn live_events_queue_while_the_next_history_page_waits_for_its_slot() {
    let lifetime = CancellationToken::new();
    let (sender, mut receiver) = ClientSender::channel(lifetime.clone());
    let page = |cursor| ServerMessage::SessionHistoryPage {
        session_id: "test".into(),
        messages: Vec::new(),
        next_cursor: Some(cursor),
        replace: false,
    };
    sender.send_history_page(page(2)).await.unwrap();
    let next = sender.send_history_page(page(1));
    tokio::pin!(next);
    assert!(futures::poll!(&mut next).is_pending());

    sender
        .send(ServerMessage::ErrorEvent {
            message: "live".into(),
        })
        .unwrap();
    assert!(matches!(
        receiver.recv().await.unwrap(),
        ServerMessage::SessionHistoryPage {
            next_cursor: Some(2),
            ..
        }
    ));
    next.await.unwrap();
    assert!(
        matches!(receiver.recv().await.unwrap(), ServerMessage::ErrorEvent { message } if message == "live")
    );
    assert!(matches!(
        receiver.recv().await.unwrap(),
        ServerMessage::SessionHistoryPage {
            next_cursor: Some(1),
            ..
        }
    ));
    assert!(!lifetime.is_cancelled());
}
