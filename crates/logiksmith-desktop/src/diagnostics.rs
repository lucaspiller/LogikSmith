//! Bounded, read-only diagnostic state for the desktop dashboard.

use crate::{BoolValueMessage, DptMessage};
use logiksmith_core::{Dpt, EngineConfig, GroupAddress, MonotonicMs};
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};
use tokio::sync::broadcast;
use tracing_subscriber::{
    layer::{Context, Layer},
    registry::LookupSpan,
};

pub const MAX_TELEGRAMS: usize = 200;
pub const MAX_LOGS: usize = 500;
pub const JOURNAL_CAPACITY: usize = 512;
const MAX_PENDING_WRITES: usize = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionState {
    Starting,
    Connecting,
    Connected,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Snapshot {
    pub revision: u64,
    pub connection: ConnectionSnapshot,
    pub config: ConfigSnapshot,
    pub values: ValuesSnapshot,
    pub write: WriteSnapshot,
    pub timer: TimerSnapshot,
    pub telegrams: Vec<TelegramRecord>,
    pub logs: Vec<LogRecord>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ConnectionSnapshot {
    pub state: ConnectionState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConfigSnapshot {
    pub input: EndpointSnapshot,
    pub output: EndpointSnapshot,
    pub off_delay_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EndpointSnapshot {
    pub address: String,
    pub dpt: DptMessage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ValuesSnapshot {
    pub input: ObservedValue,
    pub output: OutputValues,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ObservedValue {
    pub observed: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct OutputValues {
    pub observed: Option<bool>,
    pub requested: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteStatus {
    Idle,
    Pending,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WriteSnapshot {
    pub status: WriteStatus,
    pub request_id: Option<u64>,
    pub value: Option<bool>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TimerState {
    Idle,
    Pending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct TimerSnapshot {
    pub state: TimerState,
    pub deadline_ms: Option<u64>,
    pub remaining_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TelegramRecord {
    pub time_ms: u64,
    pub source: Option<String>,
    pub destination: String,
    pub service: String,
    pub dpt: DptMessage,
    pub value: Option<BoolValueMessage>,
}

impl From<&crate::KnxEvent> for TelegramRecord {
    fn from(event: &crate::KnxEvent) -> Self {
        Self {
            time_ms: 0,
            source: event.source.clone(),
            destination: event.destination.clone(),
            service: event.service.clone(),
            dpt: event.dpt.clone(),
            value: event.value.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LogRecord {
    pub time_ms: u64,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct DiagnosticUpdate {
    pub revision: u64,
    pub snapshot: Snapshot,
}

#[derive(Clone, Debug)]
pub enum Replay {
    Updates(Vec<DiagnosticUpdate>),
    Resync { revision: u64 },
}

pub struct EventSubscription {
    pub replay: Replay,
    pub receiver: broadcast::Receiver<DiagnosticUpdate>,
}

#[derive(Clone)]
pub struct DiagnosticStore {
    inner: Arc<Mutex<Inner>>,
    events: broadcast::Sender<DiagnosticUpdate>,
    origin: Instant,
}

struct Inner {
    revision: u64,
    connection: ConnectionState,
    config: ConfigSnapshot,
    input_observed: Option<bool>,
    output_observed: Option<bool>,
    output_requested: Option<bool>,
    last_write: WriteSnapshot,
    timer_deadline: Option<u64>,
    telegrams: VecDeque<TelegramRecord>,
    logs: VecDeque<LogRecord>,
    journal: VecDeque<DiagnosticUpdate>,
    pending_writes: BTreeMap<u64, WriteState>,
    write_results: VecDeque<WriteResult>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
struct WriteState {
    destination: String,
    dpt: DptMessage,
    value: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
struct WriteResult {
    request_id: u64,
    ok: bool,
    error: Option<String>,
}

impl DiagnosticStore {
    pub fn new(config: EngineConfig) -> Self {
        let (events, _) = broadcast::channel(JOURNAL_CAPACITY);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                revision: 0,
                connection: ConnectionState::Starting,
                config: ConfigSnapshot {
                    input: endpoint(config.input_group_address, config.input_dpt),
                    output: endpoint(config.output_group_address, config.output_dpt),
                    off_delay_ms: config.off_delay_ms,
                },
                input_observed: None,
                output_observed: None,
                output_requested: None,
                last_write: WriteSnapshot {
                    status: WriteStatus::Idle,
                    request_id: None,
                    value: None,
                    error: None,
                },
                timer_deadline: None,
                telegrams: VecDeque::new(),
                logs: VecDeque::new(),
                journal: VecDeque::new(),
                pending_writes: BTreeMap::new(),
                write_results: VecDeque::new(),
            })),
            events,
            origin: Instant::now(),
        }
    }

    pub fn now(&self) -> MonotonicMs {
        MonotonicMs(u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX))
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot_at(self.now())
    }

    pub fn snapshot_at(&self, now: MonotonicMs) -> Snapshot {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        snapshot_locked(&inner, now)
    }

    pub fn latest_revision(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .revision
    }

    pub fn set_connection(&self, state: ConnectionState) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.connection == state {
            return;
        }
        inner.connection = state;
        self.publish_locked(&mut inner, self.now());
    }

    /// Records a typed incoming telegram and advances only the corresponding
    /// observed value. Requested output values are intentionally separate.
    pub fn record_telegram(&self, telegram: TelegramRecord) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if telegram.destination == inner.config.input.address
            && let Some(value) = telegram.value.as_ref()
        {
            inner.input_observed = Some(value.value);
        }
        if telegram.destination == inner.config.output.address
            && let Some(value) = telegram.value.as_ref()
        {
            inner.output_observed = Some(value.value);
        }
        inner.telegrams.push_back(telegram);
        while inner.telegrams.len() > MAX_TELEGRAMS {
            inner.telegrams.pop_front();
        }
        self.publish_locked(&mut inner, self.now());
    }

    pub fn record_write_requested(
        &self,
        request_id: u64,
        destination: GroupAddress,
        dpt: Dpt,
        value: bool,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = WriteState {
            destination: destination.to_string(),
            dpt: dpt_message(dpt),
            value,
        };
        if destination.to_string() == inner.config.output.address {
            inner.output_requested = Some(value);
            inner.last_write = WriteSnapshot {
                status: WriteStatus::Pending,
                request_id: Some(request_id),
                value: Some(value),
                error: None,
            };
        }
        if inner.pending_writes.len() >= MAX_PENDING_WRITES
            && let Some(oldest) = inner.pending_writes.keys().next().copied()
        {
            inner.pending_writes.remove(&oldest);
        }
        inner.pending_writes.insert(request_id, state);
        self.publish_locked(&mut inner, self.now());
    }

    pub fn record_write_result(&self, request_id: u64, ok: bool, error: Option<String>) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Keep results associated with requests without treating an
        // acknowledgement as an observed actuator value. Runtime tracing
        // remains the source for the browser-visible result log.
        if let Some(write) = inner.pending_writes.remove(&request_id) {
            if write.destination == inner.config.output.address
                && inner.last_write.request_id == Some(request_id)
            {
                inner.last_write.status = if ok {
                    WriteStatus::Succeeded
                } else {
                    WriteStatus::Failed
                };
                inner.last_write.error = if ok { None } else { error.clone() };
            }
            inner.write_results.push_back(WriteResult {
                request_id,
                ok,
                error,
            });
            while inner.write_results.len() > MAX_TELEGRAMS {
                inner.write_results.pop_front();
            }
        }
        self.publish_locked(&mut inner, self.now());
    }

    pub fn set_timer_deadline(&self, deadline: Option<MonotonicMs>) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let deadline = deadline.map(|value| value.0);
        if inner.timer_deadline == deadline {
            return;
        }
        inner.timer_deadline = deadline;
        self.publish_locked(&mut inner, self.now());
    }

    pub fn record_log(
        &self,
        level: impl Into<String>,
        target: impl Into<String>,
        message: impl Into<String>,
        fields: BTreeMap<String, String>,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.logs.push_back(LogRecord {
            time_ms: self.now().0,
            level: level.into(),
            target: target.into(),
            message: message.into(),
            fields,
        });
        while inner.logs.len() > MAX_LOGS {
            inner.logs.pop_front();
        }
        self.publish_locked(&mut inner, self.now());
    }

    pub fn subscribe(&self, since: Option<u64>) -> EventSubscription {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Subscribe while holding the state lock. This prevents an update
        // from landing between replay selection and receiver creation.
        let receiver = self.events.subscribe();
        let since = since.unwrap_or(0);
        let replay = match inner.journal.front().map(|update| update.revision) {
            Some(first) if since.saturating_add(1) < first => Replay::Resync {
                revision: inner.revision,
            },
            _ => Replay::Updates(
                inner
                    .journal
                    .iter()
                    .filter(|update| update.revision > since)
                    .cloned()
                    .collect(),
            ),
        };
        EventSubscription { replay, receiver }
    }

    fn publish_locked(&self, inner: &mut Inner, now: MonotonicMs) {
        inner.revision = inner.revision.saturating_add(1);
        let snapshot = snapshot_locked(inner, now);
        let update = DiagnosticUpdate {
            revision: inner.revision,
            snapshot,
        };
        inner.journal.push_back(update.clone());
        while inner.journal.len() > JOURNAL_CAPACITY {
            inner.journal.pop_front();
        }
        let _ = self.events.send(update);
    }
}

fn endpoint(address: GroupAddress, dpt: Dpt) -> EndpointSnapshot {
    EndpointSnapshot {
        address: address.to_string(),
        dpt: dpt_message(dpt),
    }
}

fn dpt_message(dpt: Dpt) -> DptMessage {
    DptMessage {
        major: dpt.major,
        subtype: dpt.subtype,
    }
}

fn snapshot_locked(inner: &Inner, now: MonotonicMs) -> Snapshot {
    let remaining_ms = inner
        .timer_deadline
        .map(|deadline| deadline.saturating_sub(now.0));
    Snapshot {
        revision: inner.revision,
        connection: ConnectionSnapshot {
            state: inner.connection,
        },
        config: inner.config.clone(),
        values: ValuesSnapshot {
            input: ObservedValue {
                observed: inner.input_observed,
            },
            output: OutputValues {
                observed: inner.output_observed,
                requested: inner.output_requested,
            },
        },
        write: inner.last_write.clone(),
        timer: TimerSnapshot {
            state: if inner.timer_deadline.is_some() {
                TimerState::Pending
            } else {
                TimerState::Idle
            },
            deadline_ms: inner.timer_deadline,
            remaining_ms,
        },
        telegrams: inner.telegrams.iter().cloned().collect(),
        logs: inner.logs.iter().cloned().collect(),
    }
}

// The active store is swapped for each host run. The subscriber itself is
// installed once because tracing's global default is process-wide.
static ACTIVE_STORE: OnceLock<Arc<Mutex<Option<DiagnosticStore>>>> = OnceLock::new();

pub fn activate_tracing_store(store: DiagnosticStore) {
    let slot = ACTIVE_STORE.get_or_init(|| Arc::new(Mutex::new(None)));
    *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(store);
}

pub fn tracing_layer() -> DiagnosticLayer {
    DiagnosticLayer {
        slot: ACTIVE_STORE
            .get_or_init(|| Arc::new(Mutex::new(None)))
            .clone(),
    }
}

pub struct DiagnosticLayer {
    slot: Arc<Mutex<Option<DiagnosticStore>>>,
}

impl<S> Layer<S> for DiagnosticLayer
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let store = self
            .slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let Some(store) = store else { return };
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let message = visitor.message.unwrap_or_default();
        store.record_log(
            event.metadata().level().to_string().to_lowercase(),
            event.metadata().target().to_owned(),
            message,
            visitor.fields,
        );
    }
}

#[derive(Default)]
struct FieldVisitor {
    message: Option<String>,
    fields: BTreeMap<String, String>,
}

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let value = format!("{value:?}");
        if field.name() == "message" {
            self.message = Some(value.trim_matches('"').to_owned());
        } else {
            self.fields.insert(field.name().to_owned(), value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        } else {
            self.fields
                .insert(field.name().to_owned(), value.to_owned());
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.record_str(field, &value.to_string());
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.record_str(field, &value.to_string());
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.record_str(field, &value.to_string());
    }

    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        self.record_str(field, &value.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logiksmith_core::{Dpt, EngineConfig};

    fn config() -> EngineConfig {
        EngineConfig {
            input_group_address: "2/2/52".parse().unwrap(),
            input_dpt: Dpt::BOOL,
            output_group_address: "2/3/52".parse().unwrap(),
            output_dpt: Dpt::BOOL,
            off_delay_ms: 5_000,
        }
    }

    fn telegram(address: &str, value: bool) -> TelegramRecord {
        TelegramRecord {
            time_ms: 0,
            source: Some("1.1.42".to_owned()),
            destination: address.to_owned(),
            service: "group_value_write".to_owned(),
            dpt: DptMessage::bool(),
            value: Some(BoolValueMessage {
                kind: "bool".to_owned(),
                value,
            }),
        }
    }

    #[test]
    fn observed_and_requested_are_distinct_and_histories_are_bounded() {
        let store = DiagnosticStore::new(config());
        store.record_write_requested(1, "2/3/52".parse().unwrap(), Dpt::BOOL, true);
        assert_eq!(store.snapshot().values.output.requested, Some(true));
        assert_eq!(store.snapshot().values.output.observed, None);
        store.record_telegram(telegram("2/3/52", false));
        assert_eq!(store.snapshot().values.output.observed, Some(false));
        for _ in 0..MAX_TELEGRAMS + 1 {
            store.record_telegram(telegram("2/2/52", true));
        }
        assert_eq!(store.snapshot().telegrams.len(), MAX_TELEGRAMS);
    }

    #[test]
    fn latest_output_write_tracks_pending_and_result_without_changing_observed() {
        let store = DiagnosticStore::new(config());
        store.record_write_requested(1, "2/3/52".parse().unwrap(), Dpt::BOOL, true);
        let pending = store.snapshot();
        assert_eq!(pending.write.status, WriteStatus::Pending);
        assert_eq!(pending.write.request_id, Some(1));
        assert_eq!(pending.write.value, Some(true));
        assert_eq!(pending.values.output.requested, Some(true));
        assert_eq!(pending.values.output.observed, None);

        // A non-output request may be pending, but it must not replace the
        // latest output write shown to the dashboard.
        store.record_write_requested(2, "2/2/52".parse().unwrap(), Dpt::BOOL, false);
        store.record_write_result(2, false, Some("input write rejected".to_owned()));
        assert_eq!(store.snapshot().write.request_id, Some(1));
        assert_eq!(store.snapshot().write.status, WriteStatus::Pending);

        store.record_write_result(1, true, None);
        let succeeded = store.snapshot();
        assert_eq!(succeeded.write.status, WriteStatus::Succeeded);
        assert_eq!(succeeded.write.value, Some(true));
        assert_eq!(succeeded.write.error, None);
        assert_eq!(succeeded.values.output.observed, None);

        store.record_write_requested(3, "2/3/52".parse().unwrap(), Dpt::BOOL, false);
        store.record_write_result(3, false, Some("bus unavailable".to_owned()));
        let failed = store.snapshot();
        assert_eq!(failed.write.status, WriteStatus::Failed);
        assert_eq!(failed.write.value, Some(false));
        assert_eq!(failed.write.error.as_deref(), Some("bus unavailable"));
        assert_eq!(failed.values.output.observed, None);
    }

    #[test]
    fn revisions_and_resync_are_deterministic() {
        let store = DiagnosticStore::new(config());
        assert_eq!(store.latest_revision(), 0);
        store.set_connection(ConnectionState::Connecting);
        assert_eq!(store.latest_revision(), 1);
        let updates = match store.subscribe(Some(0)).replay {
            Replay::Updates(updates) => updates,
            Replay::Resync { .. } => panic!("unexpected resync"),
        };
        assert_eq!(updates[0].revision, 1);
        for _ in 0..JOURNAL_CAPACITY + 1 {
            store.set_connection(ConnectionState::Connected);
            store.record_log("info", "test", "event", BTreeMap::new());
        }
        assert!(matches!(
            store.subscribe(Some(0)).replay,
            Replay::Resync { .. }
        ));
    }
}
