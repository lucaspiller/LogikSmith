//! Bounded, read-only diagnostic state for the desktop dashboard.

use crate::{AutomationRuntime, DptMessage, GroupAddress, KnxEvent, ValueMessage};
use logiksmith_core::{Dpt, Effect, EndpointDirection, EndpointName, InputEvent, TypedValue};
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
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
pub const MAX_LOGICAL_EFFECTS: usize = 200;
pub const JOURNAL_CAPACITY: usize = 512;
const MAX_PENDING_WRITES: usize = 200;
const MAX_LOGIC_ERROR: usize = 2_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionState {
    Starting,
    Connecting,
    Connected,
    Disconnected,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Snapshot {
    pub revision: u64,
    pub connection: ConnectionSnapshot,
    pub config: ConfigSnapshot,
    pub automation: AutomationSnapshot,
    pub active_automation_revision: u64,
    pub saved_automation_revision: u64,
    pub values: ValuesSnapshot,
    pub write: WriteSnapshot,
    pub logic: LogicStatusSnapshot,
    pub telegrams: Vec<TelegramRecord>,
    pub logs: Vec<LogRecord>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ConnectionSnapshot {
    pub state: ConnectionState,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConfigSnapshot {
    pub active: AutomationSnapshot,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AutomationSnapshot {
    pub inputs: Vec<EndpointSnapshot>,
    pub outputs: Vec<EndpointSnapshot>,
    pub knx_bindings: Vec<BindingSnapshot>,
    pub logic: LogicSourceSnapshot,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LogicSourceSnapshot {
    pub source: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EndpointSnapshot {
    pub name: String,
    pub dpt: DptMessage,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BindingSnapshot {
    pub endpoint: String,
    pub group_address: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ValuesSnapshot {
    pub endpoints: Vec<EndpointValueSnapshot>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EndpointValueSnapshot {
    pub name: String,
    pub direction: String,
    pub dpt: DptMessage,
    pub observed: Option<ValueMessage>,
    pub requested: Option<ValueMessage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LogicStatusSnapshot {
    pub active_logic_revision: u64,
    pub saved_logic_revision: u64,
    pub active_structural_revision: u64,
    pub saved_structural_revision: u64,
    pub restart_required: bool,
    pub last_execution: LastExecutionSnapshot,
    pub recent_effects: Vec<LogicalEffectRecord>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LastExecutionSnapshot {
    pub status: LogicExecutionStatus,
    pub trigger: Option<LogicalInputRecord>,
    pub logic_revision: Option<u64>,
    pub effect_count: usize,
    pub error: Option<LogicErrorRecord>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogicExecutionStatus {
    Idle,
    Succeeded,
    Failed,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LogicalInputRecord {
    pub endpoint: String,
    pub dpt: DptMessage,
    pub value: ValueMessage,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LogicErrorRecord {
    pub category: String,
    pub message: String,
    pub line: Option<u32>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LogicalEffectRecord {
    pub time_ms: u64,
    pub endpoint: String,
    pub destination: String,
    pub dpt: DptMessage,
    pub value: ValueMessage,
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
    pub value: Option<ValueMessage>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TelegramRecord {
    pub time_ms: u64,
    pub source: Option<String>,
    pub destination: String,
    pub endpoint: Option<String>,
    pub service: String,
    pub dpt: DptMessage,
    pub value: Option<ValueMessage>,
}
impl TelegramRecord {
    pub fn from_event(event: &KnxEvent, endpoint: Option<&EndpointName>) -> Self {
        Self {
            time_ms: 0,
            source: event.source.clone(),
            destination: event.destination.clone(),
            endpoint: endpoint.map(ToString::to_string),
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
    automation_path: PathBuf,
    automation: AutomationSnapshot,
    active_automation_revision: u64,
    saved_automation_revision: u64,
    endpoint_values: BTreeMap<EndpointName, EndpointValueState>,
    last_write: WriteSnapshot,
    active_logic_revision: u64,
    saved_logic_revision: u64,
    active_structural_revision: u64,
    saved_structural_revision: u64,
    restart_required: bool,
    last_execution: LastExecutionSnapshot,
    logical_effects: VecDeque<LogicalEffectRecord>,
    telegrams: VecDeque<TelegramRecord>,
    logs: VecDeque<LogRecord>,
    journal: VecDeque<DiagnosticUpdate>,
    pending_writes: BTreeMap<u64, WriteState>,
}
#[derive(Clone, Debug)]
struct EndpointValueState {
    direction: EndpointDirection,
    dpt: Dpt,
    address: GroupAddress,
    observed: Option<ValueMessage>,
    requested: Option<ValueMessage>,
}
#[derive(Clone, Debug)]
struct WriteState;

impl DiagnosticStore {
    pub fn new(runtime: &AutomationRuntime, automation_path: PathBuf, revision: u64) -> Self {
        let (events, _) = broadcast::channel(JOURNAL_CAPACITY);
        let mut endpoint_values = BTreeMap::new();
        for endpoint in runtime.engine_config.endpoints.iter() {
            let address = runtime.endpoint_to_address[&endpoint.name];
            endpoint_values.insert(
                endpoint.name.clone(),
                EndpointValueState {
                    direction: endpoint.direction,
                    dpt: endpoint.dpt,
                    address,
                    observed: None,
                    requested: None,
                },
            );
        }
        let automation = automation_snapshot(runtime);
        let idle = LastExecutionSnapshot {
            status: LogicExecutionStatus::Idle,
            trigger: None,
            logic_revision: None,
            effect_count: 0,
            error: None,
        };
        Self {
            inner: Arc::new(Mutex::new(Inner {
                revision: 0,
                connection: ConnectionState::Starting,
                automation_path,
                automation,
                active_automation_revision: revision,
                saved_automation_revision: revision,
                endpoint_values,
                last_write: WriteSnapshot {
                    status: WriteStatus::Idle,
                    request_id: None,
                    value: None,
                    error: None,
                },
                active_logic_revision: runtime.logic_revision,
                saved_logic_revision: runtime.logic_revision,
                active_structural_revision: runtime.structural_revision,
                saved_structural_revision: runtime.structural_revision,
                restart_required: false,
                last_execution: idle,
                logical_effects: VecDeque::new(),
                telegrams: VecDeque::new(),
                logs: VecDeque::new(),
                journal: VecDeque::new(),
                pending_writes: BTreeMap::new(),
            })),
            events,
            origin: Instant::now(),
        }
    }
    pub fn now(&self) -> logiksmith_core::MonotonicMs {
        logiksmith_core::MonotonicMs(
            u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX),
        )
    }
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot_at(self.now())
    }
    pub fn snapshot_at(&self, now: logiksmith_core::MonotonicMs) -> Snapshot {
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
    pub fn automation_path(&self) -> PathBuf {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .automation_path
            .clone()
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
        self.publish_locked(&mut inner);
    }
    pub fn set_saved_automation_revision(&self, revision: u64) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.saved_automation_revision == revision {
            return;
        }
        inner.saved_automation_revision = revision;
        self.publish_locked(&mut inner);
    }
    pub fn set_saved_logic_state(
        &self,
        automation_revision: u64,
        logic_revision: u64,
        structural_revision: u64,
        restart_required: bool,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.saved_automation_revision = automation_revision;
        inner.saved_logic_revision = logic_revision;
        inner.saved_structural_revision = structural_revision;
        inner.restart_required = restart_required;
        self.publish_locked(&mut inner);
    }
    pub fn set_active_logic(&self, logic_revision: u64, source: impl Into<String>) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.active_logic_revision = logic_revision;
        inner.automation.logic.source = source.into();
        inner.restart_required =
            inner.saved_structural_revision != inner.active_structural_revision;
        self.publish_locked(&mut inner);
    }
    pub fn record_logic_success(
        &self,
        event: &InputEvent,
        revision: u64,
        effects: &[Effect],
        automation: &AutomationRuntime,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.last_execution = LastExecutionSnapshot {
            status: LogicExecutionStatus::Succeeded,
            trigger: Some(input_record(event)),
            logic_revision: Some(revision),
            effect_count: effects.len(),
            error: None,
        };
        for effect in effects {
            let Effect::SetOutput { endpoint, value } = effect;
            if let Some(address) = automation.endpoint_to_address.get(endpoint) {
                inner.logical_effects.push_back(LogicalEffectRecord {
                    time_ms: self.now().0,
                    endpoint: endpoint.to_string(),
                    destination: address.to_string(),
                    dpt: DptMessage::from_core(value.dpt),
                    value: ValueMessage::from_core(*value),
                });
            }
        }
        while inner.logical_effects.len() > MAX_LOGICAL_EFFECTS {
            inner.logical_effects.pop_front();
        }
        self.publish_locked(&mut inner);
    }
    pub fn record_logic_failure(
        &self,
        event: &InputEvent,
        revision: u64,
        category: impl Into<String>,
        message: impl Into<String>,
        line: Option<u32>,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut message = message.into();
        message.truncate(MAX_LOGIC_ERROR);
        inner.last_execution = LastExecutionSnapshot {
            status: LogicExecutionStatus::Failed,
            trigger: Some(input_record(event)),
            logic_revision: Some(revision),
            effect_count: 0,
            error: Some(LogicErrorRecord {
                category: category.into(),
                message,
                line,
            }),
        };
        self.publish_locked(&mut inner);
    }
    pub fn record_telegram(&self, mut telegram: TelegramRecord) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if telegram.endpoint.is_none()
            && let Ok(address) = telegram.destination.parse::<GroupAddress>()
        {
            telegram.endpoint = inner
                .endpoint_values
                .iter()
                .find(|(_, state)| state.address == address)
                .map(|(name, _)| name.to_string());
        }
        if let (Some(endpoint), Some(value)) =
            (telegram.endpoint.as_deref(), telegram.value.as_ref())
            && let Ok(endpoint) = endpoint.parse::<EndpointName>()
            && let Some(state) = inner.endpoint_values.get_mut(&endpoint)
        {
            state.observed = Some(value.clone());
        }
        inner.telegrams.push_back(telegram);
        while inner.telegrams.len() > MAX_TELEGRAMS {
            inner.telegrams.pop_front();
        }
        self.publish_locked(&mut inner);
    }
    pub fn record_write_requested(
        &self,
        request_id: u64,
        endpoint: EndpointName,
        destination: GroupAddress,
        dpt: Dpt,
        value: TypedValue,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let value = ValueMessage::from_core(value);
        if let Some(state) = inner.endpoint_values.get_mut(&endpoint) {
            state.requested = Some(value.clone());
            if state.address == destination && state.direction == EndpointDirection::Output {
                inner.last_write = WriteSnapshot {
                    status: WriteStatus::Pending,
                    request_id: Some(request_id),
                    value: Some(value.clone()),
                    error: None,
                };
            }
        }
        let _ = dpt;
        if inner.pending_writes.len() >= MAX_PENDING_WRITES
            && let Some(oldest) = inner.pending_writes.keys().next().copied()
        {
            inner.pending_writes.remove(&oldest);
        }
        inner.pending_writes.insert(request_id, WriteState);
        self.publish_locked(&mut inner);
    }
    pub fn record_write_result(&self, request_id: u64, ok: bool, error: Option<String>) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.pending_writes.remove(&request_id).is_some()
            && inner.last_write.request_id == Some(request_id)
        {
            inner.last_write.status = if ok {
                WriteStatus::Succeeded
            } else {
                WriteStatus::Failed
            };
            inner.last_write.error = if ok { None } else { error };
        }
        self.publish_locked(&mut inner);
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
        self.publish_locked(&mut inner);
    }
    pub fn subscribe(&self, since: Option<u64>) -> EventSubscription {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    fn publish_locked(&self, inner: &mut Inner) {
        inner.revision = inner.revision.saturating_add(1);
        let update = DiagnosticUpdate {
            revision: inner.revision,
            snapshot: snapshot_locked(inner, self.now()),
        };
        inner.journal.push_back(update.clone());
        while inner.journal.len() > JOURNAL_CAPACITY {
            inner.journal.pop_front();
        }
        let _ = self.events.send(update);
    }
}

fn input_record(event: &InputEvent) -> LogicalInputRecord {
    LogicalInputRecord {
        endpoint: event.endpoint.to_string(),
        dpt: DptMessage::from_core(event.value.dpt),
        value: ValueMessage::from_core(event.value),
    }
}
fn automation_snapshot(runtime: &AutomationRuntime) -> AutomationSnapshot {
    let endpoint = |name: &str, dpt: Dpt| EndpointSnapshot {
        name: name.to_owned(),
        dpt: DptMessage::from_core(dpt),
    };
    let inputs = runtime
        .engine_config
        .endpoints
        .iter()
        .filter(|item| item.direction == EndpointDirection::Input)
        .map(|item| endpoint(item.name.as_str(), item.dpt))
        .collect();
    let outputs = runtime
        .engine_config
        .endpoints
        .iter()
        .filter(|item| item.direction == EndpointDirection::Output)
        .map(|item| endpoint(item.name.as_str(), item.dpt))
        .collect();
    let mut knx_bindings: Vec<_> = runtime
        .endpoint_to_address
        .iter()
        .map(|(endpoint, address)| BindingSnapshot {
            endpoint: endpoint.to_string(),
            group_address: address.to_string(),
        })
        .collect();
    knx_bindings.sort_by(|left, right| left.endpoint.cmp(&right.endpoint));
    AutomationSnapshot {
        inputs,
        outputs,
        knx_bindings,
        logic: LogicSourceSnapshot {
            source: runtime.document.logic.source.clone(),
        },
    }
}
fn snapshot_locked(inner: &Inner, _now: logiksmith_core::MonotonicMs) -> Snapshot {
    let values = ValuesSnapshot {
        endpoints: inner
            .endpoint_values
            .iter()
            .map(|(name, state)| EndpointValueSnapshot {
                name: name.to_string(),
                direction: state.direction.to_string(),
                dpt: DptMessage::from_core(state.dpt),
                observed: state.observed.clone(),
                requested: state.requested.clone(),
            })
            .collect(),
    };
    Snapshot {
        revision: inner.revision,
        connection: ConnectionSnapshot {
            state: inner.connection,
        },
        config: ConfigSnapshot {
            active: inner.automation.clone(),
        },
        automation: inner.automation.clone(),
        active_automation_revision: inner.active_automation_revision,
        saved_automation_revision: inner.saved_automation_revision,
        values,
        write: inner.last_write.clone(),
        logic: LogicStatusSnapshot {
            active_logic_revision: inner.active_logic_revision,
            saved_logic_revision: inner.saved_logic_revision,
            active_structural_revision: inner.active_structural_revision,
            saved_structural_revision: inner.saved_structural_revision,
            restart_required: inner.restart_required,
            last_execution: inner.last_execution.clone(),
            recent_effects: inner.logical_effects.iter().cloned().collect(),
        },
        telegrams: inner.telegrams.iter().cloned().collect(),
        logs: inner.logs.iter().cloned().collect(),
    }
}

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
        store.record_log(
            event.metadata().level().to_string().to_lowercase(),
            event.metadata().target().to_owned(),
            visitor.message.unwrap_or_default(),
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
