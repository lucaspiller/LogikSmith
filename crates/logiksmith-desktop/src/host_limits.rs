//! Desktop-owned runtime capacity and process-health contracts.
//!
//! The core owns semantic limits (source size, state, timers, and execution
//! work).  This module owns resources which only exist on the desktop host:
//! transport/control queues and the in-flight KNX command table.  Keeping the
//! host half explicit makes the embedded-baseline profile executable on a
//! desktop without leaking Tokio or process details into `logiksmith-core`.

use logiksmith_core::RuntimeProfile;
use std::{
    fmt,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::watch;

/// Parse the profile selected by `LOGIKSMITH_RUNTIME_PROFILE`.
pub fn parse_runtime_profile(value: &str) -> Result<RuntimeProfile, RuntimeProfileError> {
    match value.trim() {
        "desktop" => Ok(RuntimeProfile::Desktop),
        "embedded-baseline" => Ok(RuntimeProfile::EmbeddedBaseline),
        value => Err(RuntimeProfileError(value.to_owned())),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeProfileError(String);

impl fmt::Display for RuntimeProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unknown runtime profile `{}` (expected desktop or embedded-baseline)",
            self.0
        )
    }
}

impl std::error::Error for RuntimeProfileError {}

/// Bounded resources owned by the Tokio desktop host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostLimits {
    pub profile: RuntimeProfile,
    pub knx_ingress_queue: usize,
    pub external_input_queue: usize,
    pub activation_queue: usize,
    pub simulation_queue: usize,
    pub pending_knx_writes: usize,
    pub execution_history_per_block: usize,
    pub recent_telegrams: usize,
    pub runtime_logs: usize,
    pub diagnostic_journal: usize,
    /// Maximum number of due timers/schedules handled before the event loop
    /// yields to control and connection work.
    pub due_work_batch: usize,
    pub pending_write_timeout_ms: u64,
}

impl HostLimits {
    pub const fn desktop() -> Self {
        Self {
            profile: RuntimeProfile::Desktop,
            knx_ingress_queue: 256,
            external_input_queue: 256,
            activation_queue: 8,
            simulation_queue: 8,
            pending_knx_writes: 256,
            execution_history_per_block: 50,
            recent_telegrams: 200,
            runtime_logs: 500,
            diagnostic_journal: 512,
            due_work_batch: 32,
            pending_write_timeout_ms: 5_000,
        }
    }

    pub const fn embedded_baseline() -> Self {
        Self {
            profile: RuntimeProfile::EmbeddedBaseline,
            knx_ingress_queue: 64,
            external_input_queue: 64,
            activation_queue: 2,
            simulation_queue: 2,
            pending_knx_writes: 64,
            execution_history_per_block: 8,
            recent_telegrams: 64,
            runtime_logs: 128,
            diagnostic_journal: 128,
            due_work_batch: 8,
            pending_write_timeout_ms: 5_000,
        }
    }

    pub fn from_environment() -> Result<Self, RuntimeProfileError> {
        let profile = std::env::var("LOGIKSMITH_RUNTIME_PROFILE")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map_or(Ok(RuntimeProfile::Desktop), |value| {
                parse_runtime_profile(&value)
            })?;
        Ok(match profile {
            RuntimeProfile::Desktop => Self::desktop(),
            RuntimeProfile::EmbeddedBaseline => Self::embedded_baseline(),
        })
    }
}

/// A bounded in-flight KNX command table.  Inserting at capacity is an error;
/// old requests are never silently evicted because that would make a later
/// command result impossible to attribute.
#[derive(Debug)]
pub struct PendingWrites {
    limit: usize,
    entries: std::collections::BTreeMap<u64, Instant>,
}

impl PendingWrites {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            entries: std::collections::BTreeMap::new(),
        }
    }

    pub fn insert(
        &mut self,
        request_id: u64,
        sent_at: Instant,
    ) -> Result<(), PendingWriteCapacity> {
        if self.entries.len() >= self.limit && !self.entries.contains_key(&request_id) {
            return Err(PendingWriteCapacity {
                limit: self.limit,
                depth: self.entries.len(),
            });
        }
        self.entries.insert(request_id, sent_at);
        Ok(())
    }

    pub fn remove(&mut self, request_id: u64) -> bool {
        self.entries.remove(&request_id).is_some()
    }

    pub fn contains(&self, request_id: u64) -> bool {
        self.entries.contains_key(&request_id)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn expired(&self, now: Instant, timeout: std::time::Duration) -> Option<u64> {
        self.entries.iter().find_map(|(request_id, sent_at)| {
            (now.saturating_duration_since(*sent_at) >= timeout).then_some(*request_id)
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingWriteCapacity {
    pub limit: usize,
    pub depth: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostHealthSnapshot {
    pub profile: RuntimeProfile,
    pub ready: bool,
    pub fatal: Option<String>,
}

#[derive(Clone)]
pub struct HostHealth {
    inner: Arc<HealthInner>,
}

struct HealthInner {
    profile: RuntimeProfile,
    state: Mutex<HealthState>,
    fatal: watch::Sender<Option<String>>,
}

#[derive(Clone, Debug)]
struct HealthState {
    ready: bool,
    fatal: Option<String>,
}

impl HostHealth {
    pub fn new(limits: HostLimits) -> Self {
        let (fatal, _) = watch::channel(None);
        Self {
            inner: Arc::new(HealthInner {
                profile: limits.profile,
                state: Mutex::new(HealthState {
                    ready: false,
                    fatal: None,
                }),
                fatal,
            }),
        }
    }

    pub fn mark_ready(&self) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.fatal.is_none() {
            state.ready = true;
        }
    }

    pub fn mark_not_ready(&self) {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .ready = false;
    }

    pub fn fail(&self, reason: impl Into<String>) {
        let reason = reason.into();
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.fatal.is_none() {
            state.ready = false;
            state.fatal = Some(reason.clone());
            let _ = self.inner.fatal.send(Some(reason));
        }
    }

    pub fn snapshot(&self) -> HostHealthSnapshot {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        HostHealthSnapshot {
            profile: self.inner.profile,
            ready: state.ready,
            fatal: state.fatal.clone(),
        }
    }

    pub fn subscribe_fatal(&self) -> watch::Receiver<Option<String>> {
        self.inner.fatal.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn profile_limits_are_bounded_and_distinct() {
        assert!(
            HostLimits::embedded_baseline().knx_ingress_queue
                < HostLimits::desktop().knx_ingress_queue
        );
        assert_eq!(HostLimits::embedded_baseline().pending_knx_writes, 64);
        assert_eq!(HostLimits::desktop().pending_knx_writes, 256);
        assert_eq!(
            HostLimits::embedded_baseline().execution_history_per_block,
            8
        );
        assert_eq!(HostLimits::embedded_baseline().recent_telegrams, 64);
        assert_eq!(HostLimits::embedded_baseline().runtime_logs, 128);
        assert_eq!(HostLimits::embedded_baseline().diagnostic_journal, 128);
    }

    #[test]
    fn pending_writes_reject_capacity_and_expire_without_eviction() {
        let mut pending = PendingWrites::new(1);
        let start = Instant::now();
        pending.insert(1, start).unwrap();
        assert!(pending.insert(2, start).is_err());
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending.expired(start + Duration::from_secs(5), Duration::from_secs(5)),
            Some(1)
        );
        assert!(pending.remove(1));
        assert!(!pending.remove(1));
    }

    #[test]
    fn health_fatal_clears_readiness_and_notifies() {
        let health = HostHealth::new(HostLimits::desktop());
        let receiver = health.subscribe_fatal();
        health.mark_ready();
        health.fail("queue full");
        assert!(!health.snapshot().ready);
        assert_eq!(health.snapshot().fatal.as_deref(), Some("queue full"));
        assert_eq!(receiver.borrow().as_deref(), Some("queue full"));
    }
}
