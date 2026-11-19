use std::collections::HashMap;
use std::sync::Mutex;

use mew_subagents::ChildGuidanceError;
use tokio::sync::watch;

const MAX_RECEIPTS: usize = 128;
pub(crate) const CHILD_ENDED: &str = "The child ended before it could use this guidance.";

#[derive(Default)]
pub(crate) struct ChildGuidance {
    state: Mutex<GuidanceState>,
}

#[derive(Default)]
struct GuidanceState {
    closed: bool,
    pending: Option<PendingGuidance>,
    settled: HashMap<String, Result<(), ChildGuidanceError>>,
}

struct PendingGuidance {
    request_id: String,
    text: Option<String>,
    receipt: watch::Sender<Option<Result<(), ChildGuidanceError>>>,
}

impl ChildGuidance {
    pub(crate) async fn guide(
        &self,
        request_id: &str,
        text: String,
    ) -> Result<(), ChildGuidanceError> {
        let mut receipt = {
            let mut state = self.state.lock().unwrap();
            if let Some(result) = state.settled.get(request_id) {
                return result.clone();
            }
            if state.closed {
                return Err(ChildGuidanceError::NotRunning);
            }
            if let Some(pending) = &state.pending {
                if pending.request_id != request_id {
                    return Err(ChildGuidanceError::Rejected(
                        "The child already has guidance waiting for its next request.".into(),
                    ));
                }
                pending.receipt.subscribe()
            } else {
                if state.settled.len() >= MAX_RECEIPTS {
                    return Err(ChildGuidanceError::Rejected(
                        "The child has reached its guidance limit.".into(),
                    ));
                }
                let (send, receive) = watch::channel(None);
                state.pending = Some(PendingGuidance {
                    request_id: request_id.to_owned(),
                    text: Some(text),
                    receipt: send,
                });
                receive
            }
        };
        let ready = receipt
            .wait_for(Option::is_some)
            .await
            .map_err(|_| ChildGuidanceError::Rejected(CHILD_ENDED.into()))?;
        ready.as_ref().expect("guidance receipt is ready").clone()
    }

    pub(crate) fn receipt(&self, request_id: &str) -> Option<Result<(), ChildGuidanceError>> {
        let state = self.state.lock().unwrap();
        state.settled.get(request_id).cloned().or_else(|| {
            state
                .pending
                .as_ref()
                .filter(|pending| pending.request_id == request_id)
                .map(|_| {
                    Err(ChildGuidanceError::Unknown(
                        "The child has not confirmed this guidance yet.".into(),
                    ))
                })
        })
    }

    pub(crate) fn has_waiting_guide(&self) -> bool {
        let state = self.state.lock().unwrap();
        !state.closed
            && state
                .pending
                .as_ref()
                .is_some_and(|pending| pending.text.is_some())
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.state.lock().unwrap().closed
    }

    pub(crate) fn claim(&self) -> Option<(GuidanceAdmission<'_>, String)> {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return None;
        }
        let pending = state.pending.as_mut()?;
        let text = pending.text.take()?;
        Some((
            GuidanceAdmission {
                queue: self,
                request_id: pending.request_id.clone(),
                finished: false,
            },
            text,
        ))
    }

    pub(crate) fn close(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        if state
            .pending
            .as_ref()
            .is_some_and(|pending| pending.text.is_some())
        {
            let pending = state.pending.take().expect("guidance is pending");
            pending
                .receipt
                .send_replace(Some(Err(ChildGuidanceError::Rejected(CHILD_ENDED.into()))));
        }
        state
            .settled
            .retain(|_, result| matches!(result, Err(ChildGuidanceError::Unknown(_))));
        state.pending.is_some() || !state.settled.is_empty()
    }
}

/// Keeps the slot reserved from request assembly through durable admission.
pub(crate) struct GuidanceAdmission<'a> {
    queue: &'a ChildGuidance,
    pub(crate) request_id: String,
    finished: bool,
}

impl GuidanceAdmission<'_> {
    pub(crate) fn finish(mut self, result: Result<(), ChildGuidanceError>) {
        self.settle(result);
    }

    fn settle(&mut self, result: Result<(), ChildGuidanceError>) {
        let mut state = self.queue.state.lock().unwrap();
        let pending = state
            .pending
            .take()
            .expect("claimed guidance remains pending");
        state.settled.insert(pending.request_id, result.clone());
        pending.receipt.send_replace(Some(result));
        self.finished = true;
    }
}

impl Drop for GuidanceAdmission<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.settle(Err(ChildGuidanceError::Rejected(CHILD_ENDED.into())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn uncertain_guidance_receipt_prevents_a_second_admission() {
        let queue = ChildGuidance::default();
        let mut first = Box::pin(queue.guide("id", "guide".into()));
        assert!(futures::poll!(&mut first).is_pending());
        assert!(matches!(
            queue.receipt("id"),
            Some(Err(ChildGuidanceError::Unknown(_)))
        ));
        let (admission, _) = queue.claim().unwrap();
        let unknown = Err(ChildGuidanceError::Unknown("commit unknown".into()));
        admission.finish(unknown.clone());

        assert_eq!(first.await, unknown);
        assert_eq!(queue.receipt("id"), Some(unknown.clone()));
        assert_eq!(queue.guide("id", "duplicate".into()).await, unknown);
        assert!(queue.claim().is_none());
    }
}
