use std::time::{Duration, Instant};

/// How long a provider may retain the cacheable prompt prefix.
///
/// `Unknown` is deliberately conservative: the agent must not assume that a
/// prompt-cache entry has expired merely because some time has passed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PromptCacheRetention {
    #[default]
    Unknown,
    Known(Duration),
}

impl PromptCacheRetention {
    pub fn from_secs(secs: Option<u64>) -> Self {
        secs.map_or(Self::Unknown, |secs| Self::Known(Duration::from_secs(secs)))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PromptCacheState {
    retention: PromptCacheRetention,
    last_request_at: Option<Instant>,
    rebuild_pending: bool,
    generation: u64,
    applied_system: Option<String>,
}

impl Default for PromptCacheState {
    fn default() -> Self {
        Self {
            retention: PromptCacheRetention::Unknown,
            last_request_at: None,
            rebuild_pending: false,
            generation: 0,
            applied_system: None,
        }
    }
}

impl PromptCacheState {
    pub(crate) fn retention(&self) -> PromptCacheRetention {
        self.retention
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn rebuild_pending(&self) -> bool {
        self.rebuild_pending
    }

    pub(crate) fn applied_system(&self) -> Option<String> {
        self.applied_system.clone()
    }

    pub(crate) fn mark_rebuilt(&mut self, system: String) -> u64 {
        self.applied_system = Some(system);
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    pub(crate) fn set_retention(&mut self, retention: PromptCacheRetention) {
        self.retention = retention;
    }

    pub(crate) fn request_rebuild(&mut self, now: Instant) -> bool {
        if self.refresh_is_safe(now) {
            self.rebuild_pending = false;
            true
        } else {
            self.rebuild_pending = true;
            false
        }
    }

    pub(crate) fn apply_pending_if_safe(&mut self, now: Instant) -> bool {
        if self.rebuild_pending && self.refresh_is_safe(now) {
            self.rebuild_pending = false;
            true
        } else {
            false
        }
    }

    pub(crate) fn apply_pending_after_compaction(&mut self) -> bool {
        if self.rebuild_pending {
            self.rebuild_pending = false;
            true
        } else {
            false
        }
    }

    pub(crate) fn mark_request_sent(&mut self, now: Instant) {
        self.last_request_at = Some(now);
    }

    fn refresh_is_safe(&self, now: Instant) -> bool {
        let Some(last_request_at) = self.last_request_at else {
            return true;
        };

        match self.retention {
            PromptCacheRetention::Unknown => false,
            PromptCacheRetention::Known(retention) => {
                now.saturating_duration_since(last_request_at) >= retention
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_retention_keeps_refresh_pending_until_compaction() {
        let started = Instant::now();
        let mut state = PromptCacheState::default();

        assert!(state.request_rebuild(started));
        state.mark_request_sent(started);

        assert!(!state.request_rebuild(started + Duration::from_secs(24 * 60 * 60)));
        assert!(!state.apply_pending_if_safe(started + Duration::from_secs(24 * 60 * 60)));
        assert!(state.apply_pending_after_compaction());
        assert!(!state.apply_pending_after_compaction());
    }

    #[test]
    fn known_retention_releases_refresh_after_window() {
        let started = Instant::now();
        let mut state = PromptCacheState::default();
        state.set_retention(PromptCacheRetention::Known(Duration::from_secs(
            4 * 60 * 60,
        )));

        assert!(state.request_rebuild(started));
        state.mark_request_sent(started);

        assert!(!state.request_rebuild(started + Duration::from_secs(4 * 60 * 60 - 1)));
        assert!(state.apply_pending_if_safe(started + Duration::from_secs(4 * 60 * 60)));
    }
}
