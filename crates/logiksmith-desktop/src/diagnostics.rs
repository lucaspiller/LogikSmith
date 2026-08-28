//! Bounded, read-only diagnostic state for the desktop dashboard.

use crate::{
    AutomationRuntime, BoolValueMessage, DptMessage, GroupAddress, KnxEvent, ValueMessage,
};
use logiksmith_core::{
    BlockExecution, Dpt, EndpointDirection, EndpointName, Execution, OutputEffect, Runtime,
    StateValue, TimerAction, Trigger, TypedValue,
};
use serde::Serialize;
use serde_json::Value as JsonValue;
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
pub const MAX_EXECUTIONS: usize = 50;
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
    pub captured_at_ms: u64,
    pub state: BTreeMap<String, StateValueRecord>,
    pub pending_timers: Vec<PendingTimerRecord>,
    pub values: ValuesSnapshot,
    pub write: WriteSnapshot,
    pub logic: LogicStatusSnapshot,
    pub telegrams: Vec<TelegramRecord>,
    pub logs: Vec<LogRecord>,
    /// Ordered block-local diagnostics. The older global projections remain
    /// during the dashboard migration, but this is the authoritative M8 view.
    pub blocks: Vec<BlockSnapshot>,
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
pub struct BlockSnapshot {
    pub id: String,
    pub active_enabled: bool,
    pub saved_enabled: bool,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    pub active_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    pub saved_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    pub active_logic_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    pub saved_logic_revision: u64,
    pub source: String,
    pub inputs: Vec<EndpointSnapshot>,
    pub outputs: Vec<EndpointSnapshot>,
    pub knx_bindings: Vec<BindingSnapshot>,
    pub values: ValuesSnapshot,
    pub state: BTreeMap<String, StateValueRecord>,
    pub pending_timers: Vec<PendingTimerRecord>,
    pub executions: Vec<ExecutionRecord>,
    pub last_result: Option<LastResultSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LastResultSnapshot {
    pub status: LogicExecutionStatus,
    pub execution_id: u64,
    pub time_ms: u64,
    pub error: Option<LogicErrorRecord>,
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
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    pub active_logic_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    pub saved_logic_revision: u64,
    pub active_structural_revision: u64,
    pub saved_structural_revision: u64,
    pub restart_required: bool,
    /// Current transient state owned by the active logic block. Values are
    /// tagged so Lua integers remain distinguishable from Lua numbers.
    pub state: BTreeMap<String, StateValueRecord>,
    /// Pending named timers in deterministic name/deadline order.
    pub pending_timers: Vec<PendingTimerRecord>,
    pub executions: Vec<ExecutionRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StateValueRecord {
    pub kind: String,
    pub value: JsonValue,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PendingTimerRecord {
    pub name: String,
    pub scheduled_at_ms: u64,
    pub due_at_ms: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    pub logic_revision: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogicExecutionStatus {
    Succeeded,
    Failed,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExecutionRecord {
    pub block_id: String,
    pub execution_id: u64,
    pub time_ms: u64,
    pub duration_us: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    pub logic_revision: u64,
    pub status: LogicExecutionStatus,
    pub trigger: LogicalTriggerRecord,
    pub inputs: Vec<LogicalInputSnapshot>,
    pub state_before: BTreeMap<String, StateValueRecord>,
    pub state_after: BTreeMap<String, StateValueRecord>,
    pub transition: Option<LogicalTransitionRecord>,
    pub effects: Vec<LogicalEffectRecord>,
    pub timer_effects: Vec<LogicalTimerEffectRecord>,
    pub error: Option<LogicErrorRecord>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogicalTriggerRecord {
    pub trigger_type: String,
    pub endpoint: String,
    pub dpt: DptMessage,
    pub value: ValueMessage,
    pub previous: Option<ValueMessage>,
    pub changed: bool,
    pub rising: bool,
    pub falling: bool,
    pub name: Option<String>,
    pub scheduled_at_ms: Option<u64>,
    pub due_at_ms: Option<u64>,
    pub fired_at_ms: Option<u64>,
    pub late_by_ms: Option<u64>,
    pub scheduled_logic_revision: Option<u64>,
}

impl Serialize for LogicalTriggerRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("type", &self.trigger_type)?;
        if self.trigger_type == "timer" {
            if let Some(value) = &self.name {
                map.serialize_entry("name", value)?;
            }
            if let Some(value) = self.scheduled_at_ms {
                map.serialize_entry("scheduled_at_ms", &value)?;
            }
            if let Some(value) = self.due_at_ms {
                map.serialize_entry("due_at_ms", &value)?;
            }
            if let Some(value) = self.fired_at_ms {
                map.serialize_entry("fired_at_ms", &value)?;
            }
            if let Some(value) = self.late_by_ms {
                map.serialize_entry("late_by_ms", &value)?;
            }
            if let Some(value) = self.scheduled_logic_revision {
                map.serialize_entry(
                    "scheduled_logic_revision",
                    &crate::wire_revision::Value(value),
                )?;
            }
        } else {
            map.serialize_entry("endpoint", &self.endpoint)?;
            map.serialize_entry("dpt", &self.dpt)?;
            map.serialize_entry("value", &self.value)?;
            map.serialize_entry("previous", &self.previous)?;
            map.serialize_entry("changed", &self.changed)?;
            map.serialize_entry("rising", &self.rising)?;
            map.serialize_entry("falling", &self.falling)?;
        }
        map.end()
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LogicalInputSnapshot {
    pub endpoint: String,
    pub dpt: DptMessage,
    pub value: Option<ValueMessage>,
    pub valid: bool,
    pub age_ms: Option<u64>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LogicErrorRecord {
    pub category: String,
    pub message: String,
    pub line: Option<u32>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LogicalEffectRecord {
    pub block_id: String,
    pub endpoint: String,
    pub destination: String,
    pub dpt: DptMessage,
    pub value: ValueMessage,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LogicalTransitionRecord {
    pub state: BTreeMap<String, StateValueRecord>,
    pub effects: Vec<LogicalEffectRecord>,
    pub timers: Vec<LogicalTimerEffectRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LogicalTimerEffectRecord {
    pub name: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_due_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_at_ms: Option<u64>,
}

/// Browser-facing result for an immutable simulation. It intentionally omits
/// live execution IDs and timestamps because no live diagnostic record exists.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SimulationResponse {
    pub block_id: String,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    pub logic_revision: u64,
    pub duration_us: u64,
    pub status: LogicExecutionStatus,
    pub trigger: LogicalTriggerRecord,
    pub inputs: Vec<LogicalInputSnapshot>,
    pub state_before: BTreeMap<String, StateValueRecord>,
    pub state_after: BTreeMap<String, StateValueRecord>,
    pub transition: Option<LogicalTransitionRecord>,
    pub pending_timers: Vec<PendingTimerRecord>,
    pub effects: Vec<LogicalEffectRecord>,
    pub timer_effects: Vec<LogicalTimerEffectRecord>,
    pub error: Option<LogicErrorRecord>,
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
    pub block_id: Option<String>,
    pub execution_id: Option<u64>,
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
    captured_at_ms: u64,
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
    state: BTreeMap<String, StateValueRecord>,
    pending_timers: Vec<PendingTimerRecord>,
    executions: VecDeque<ExecutionRecord>,
    next_execution_id: u64,
    telegrams: VecDeque<TelegramRecord>,
    logs: VecDeque<LogRecord>,
    journal: VecDeque<DiagnosticUpdate>,
    pending_writes: BTreeMap<u64, WriteState>,
    blocks: BTreeMap<String, BlockDiagnosticState>,
    block_order: Vec<String>,
    block_automation: BTreeMap<String, AutomationSnapshot>,
    block_endpoint_values: BTreeMap<(String, EndpointName), EndpointValueState>,
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

#[derive(Clone, Debug)]
struct BlockDiagnosticState {
    active_enabled: bool,
    saved_enabled: bool,
    active_logic_revision: u64,
    saved_logic_revision: u64,
    source: String,
    state: BTreeMap<String, StateValueRecord>,
    pending_timers: Vec<PendingTimerRecord>,
    executions: VecDeque<ExecutionRecord>,
    last_result: Option<LastResultSnapshot>,
}

impl DiagnosticStore {
    pub fn new(runtime: &AutomationRuntime, automation_path: PathBuf, revision: u64) -> Self {
        let (events, _) = broadcast::channel(JOURNAL_CAPACITY);
        let mut endpoint_values = BTreeMap::new();
        let mut block_endpoint_values = BTreeMap::new();
        let mut blocks = BTreeMap::new();
        let mut block_automation = BTreeMap::new();
        for block in &runtime.blocks {
            let source = runtime
                .document
                .blocks
                .iter()
                .find(|candidate| candidate.id == block.id)
                .map(|candidate| candidate.source.clone())
                .unwrap_or_default();
            let revision = block.revision;
            blocks.insert(
                block.id.clone(),
                BlockDiagnosticState {
                    active_enabled: block.enabled,
                    saved_enabled: block.enabled,
                    active_logic_revision: revision,
                    saved_logic_revision: revision,
                    source,
                    state: BTreeMap::new(),
                    pending_timers: Vec::new(),
                    executions: VecDeque::new(),
                    last_result: None,
                },
            );
            block_automation.insert(block.id.clone(), block_automation_snapshot(runtime, block));
            for endpoint in block.engine_config.endpoints.iter() {
                let Some(address) = block.endpoint_to_address.get(&endpoint.name).copied() else {
                    continue;
                };
                // The legacy global value projection is retained for the
                // desktop shell while block snapshots carry the authoritative
                // identity. Repeated local names intentionally overwrite this
                // compatibility view; block-local diagnostics never do.
                let endpoint_state = EndpointValueState {
                    direction: endpoint.direction,
                    dpt: endpoint.dpt,
                    address,
                    observed: None,
                    requested: None,
                };
                block_endpoint_values.insert(
                    (block.id.clone(), endpoint.name.clone()),
                    endpoint_state.clone(),
                );
                endpoint_values.insert(endpoint.name.clone(), endpoint_state);
            }
        }
        let automation = automation_snapshot(runtime);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                revision: 0,
                captured_at_ms: 0,
                connection: ConnectionState::Starting,
                automation_path,
                automation,
                active_automation_revision: revision,
                saved_automation_revision: revision,
                endpoint_values,
                last_write: WriteSnapshot {
                    status: WriteStatus::Idle,
                    request_id: None,
                    block_id: None,
                    execution_id: None,
                    value: None,
                    error: None,
                },
                active_logic_revision: runtime
                    .blocks
                    .first()
                    .map(|block| block.revision)
                    .unwrap_or(1),
                saved_logic_revision: runtime
                    .blocks
                    .first()
                    .map(|block| block.revision)
                    .unwrap_or(1),
                active_structural_revision: runtime.structural_revision,
                saved_structural_revision: runtime.structural_revision,
                restart_required: false,
                state: BTreeMap::new(),
                pending_timers: Vec::new(),
                executions: VecDeque::new(),
                next_execution_id: 1,
                telegrams: VecDeque::new(),
                logs: VecDeque::new(),
                journal: VecDeque::new(),
                pending_writes: BTreeMap::new(),
                blocks,
                block_order: runtime
                    .blocks
                    .iter()
                    .map(|block| block.id.clone())
                    .collect(),
                block_automation,
                block_endpoint_values,
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

    pub fn set_saved_document_state(
        &self,
        automation_revision: u64,
        structural_revision: u64,
        restart_required: bool,
        document: &crate::AutomationDocument,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.saved_automation_revision = automation_revision;
        inner.saved_structural_revision = structural_revision;
        inner.restart_required = restart_required;
        for candidate in &document.blocks {
            if let Some(block) = inner.blocks.get_mut(&candidate.id) {
                block.saved_enabled = candidate.enabled;
                block.saved_logic_revision = candidate.revision.max(1);
                if !restart_required {
                    block.active_logic_revision = candidate.revision.max(1);
                    block.active_enabled = candidate.enabled;
                }
            }
        }
        self.publish_locked(&mut inner);
    }
    pub fn set_active_logic(&self, logic_revision: u64, source: impl Into<String>) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.active_logic_revision = logic_revision;
        inner.active_automation_revision = logic_revision;
        inner.automation.logic.source = source.into();
        inner.restart_required =
            inner.saved_structural_revision != inner.active_structural_revision;
        self.publish_locked(&mut inner);
    }

    /// Publishes a source activation together with its cancelled timer names
    /// and the new core projection in one SSE update.
    pub fn record_activation(
        &self,
        logic_revision: u64,
        source: impl Into<String>,
        cancelled_timers: &[String],
        snapshot: &logiksmith_core::EngineSnapshot,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.active_logic_revision = logic_revision;
        inner.active_automation_revision = logic_revision;
        inner.automation.logic.source = source.into();
        inner.restart_required =
            inner.saved_structural_revision != inner.active_structural_revision;
        inner.state = state_record(&snapshot.state);
        inner.pending_timers = snapshot
            .pending_timers
            .iter()
            .map(|timer| pending_timer_record(timer, logic_revision))
            .collect();
        if !cancelled_timers.is_empty() {
            let mut fields = BTreeMap::new();
            fields.insert("cancelled_timers".to_owned(), cancelled_timers.join(","));
            inner.logs.push_back(LogRecord {
                time_ms: self.now().0,
                level: "info".to_owned(),
                target: "logiksmith".to_owned(),
                message: "source activation cancelled pending timers".to_owned(),
                fields,
            });
            while inner.logs.len() > MAX_LOGS {
                inner.logs.pop_front();
            }
        }
        self.publish_locked(&mut inner);
    }

    /// Replaces the browser projection of core-owned transient state and
    /// pending timers. The session calls this after every serialized runtime
    /// operation, so snapshots and SSE updates share one coherent view.
    pub fn set_runtime_projection(
        &self,
        state: BTreeMap<String, StateValueRecord>,
        pending_timers: Vec<PendingTimerRecord>,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.state == state && inner.pending_timers == pending_timers {
            return;
        }
        inner.state = state;
        inner.pending_timers = pending_timers;
        self.publish_locked(&mut inner);
    }

    pub fn set_engine_snapshot(&self, snapshot: &logiksmith_core::EngineSnapshot) {
        let logic_revision = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_logic_revision;
        self.set_runtime_projection(
            state_record(&snapshot.state),
            snapshot
                .pending_timers
                .iter()
                .map(|timer| pending_timer_record(timer, logic_revision))
                .collect(),
        );
    }

    /// Projects all portable block state into the browser diagnostics view.
    /// Runtime state remains owned by the core; this method only copies its
    /// immutable snapshot into bounded dashboard data.
    pub fn set_runtime_projection_from_runtime(
        &self,
        runtime: &Runtime,
        now: logiksmith_core::MonotonicMs,
    ) {
        let snapshot = runtime.snapshot_at(now);
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for block in snapshot.blocks {
            let id = block.id.to_string();
            let revision = inner
                .blocks
                .get(&id)
                .map(|current| current.active_logic_revision)
                .unwrap_or(1);
            let entry = inner
                .blocks
                .entry(id.clone())
                .or_insert_with(|| BlockDiagnosticState {
                    active_enabled: block.enabled,
                    saved_enabled: block.enabled,
                    active_logic_revision: revision,
                    saved_logic_revision: revision,
                    source: String::new(),
                    state: BTreeMap::new(),
                    pending_timers: Vec::new(),
                    executions: VecDeque::new(),
                    last_result: None,
                });
            entry.active_enabled = block.enabled;
            entry.state = state_record(&block.state);
            entry.pending_timers = block
                .pending_timers
                .iter()
                .map(|timer| pending_timer_record(timer, revision))
                .collect();
            for input in block.inputs {
                if let Some(value) = input.value {
                    if let Some(state) = inner
                        .block_endpoint_values
                        .get_mut(&(id.clone(), input.endpoint.clone()))
                    {
                        state.observed = Some(ValueMessage::from_core(value));
                    }
                }
            }
        }
        let first_revision = inner
            .blocks
            .values()
            .next()
            .map(|block| block.active_logic_revision);
        if let Some(first_revision) = first_revision {
            inner.active_logic_revision = first_revision;
            inner.active_automation_revision = first_revision;
        }
        self.publish_locked(&mut inner);
    }

    /// Records one tagged execution in the owning block's bounded history.
    pub fn record_block_execution(
        &self,
        execution: &BlockExecution,
        now: logiksmith_core::MonotonicMs,
        duration_us: u64,
        automation: &AutomationRuntime,
    ) {
        let block_id = execution.block_id.to_string();
        let semantic = &execution.execution;
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let revision = crate::block_revision(&automation.document, &block_id);
        let (status, effects, transition, error) = match &semantic.outcome {
            Ok(effects) => (
                LogicExecutionStatus::Succeeded,
                effects
                    .outputs
                    .iter()
                    .filter_map(|effect| {
                        effect_record_for_block(effect, automation, &execution.block_id)
                    })
                    .collect(),
                Some(transition_record_for_block(
                    effects,
                    automation,
                    &execution.block_id,
                )),
                None,
            ),
            Err(error) => (
                LogicExecutionStatus::Failed,
                Vec::new(),
                None,
                Some(logic_error_record(error)),
            ),
        };
        let execution_id = inner.next_execution_id;
        inner.next_execution_id = inner.next_execution_id.saturating_add(1);
        let first_block_id = inner.blocks.keys().next().cloned();
        let record = ExecutionRecord {
            block_id: execution.block_id.to_string(),
            execution_id,
            time_ms: now.0,
            duration_us,
            logic_revision: revision,
            status,
            trigger: trigger_record(&semantic.trigger, revision),
            inputs: semantic.inputs.iter().map(input_snapshot_record).collect(),
            state_before: state_record(&semantic.state_before),
            state_after: state_record(&semantic.state_after),
            timer_effects: transition
                .as_ref()
                .map(|transition| transition.timers.clone())
                .unwrap_or_default(),
            transition,
            effects,
            error: error.clone(),
        };
        let (block_revision, block_state, block_pending, block_executions) = {
            let block =
                inner
                    .blocks
                    .entry(block_id.clone())
                    .or_insert_with(|| BlockDiagnosticState {
                        active_enabled: true,
                        saved_enabled: true,
                        active_logic_revision: revision,
                        saved_logic_revision: revision,
                        source: String::new(),
                        state: BTreeMap::new(),
                        pending_timers: Vec::new(),
                        executions: VecDeque::new(),
                        last_result: None,
                    });
            block.active_logic_revision = revision;
            block.state = record.state_after.clone();
            block.pending_timers = semantic
                .pending_timers
                .iter()
                .map(|timer| pending_timer_record(timer, revision))
                .collect();
            block.last_result = Some(LastResultSnapshot {
                status,
                execution_id,
                time_ms: now.0,
                error,
            });
            block.executions.push_back(record);
            while block.executions.len() > MAX_EXECUTIONS {
                block.executions.pop_front();
            }
            (
                block.active_logic_revision,
                block.state.clone(),
                block.pending_timers.clone(),
                block.executions.clone(),
            )
        };
        if first_block_id.as_deref() == Some(block_id.as_str()) {
            inner.active_logic_revision = block_revision;
            inner.active_automation_revision = block_revision;
            inner.state = block_state;
            inner.pending_timers = block_pending;
            inner.executions = block_executions;
        }
        self.publish_locked(&mut inner);
    }

    /// Applies the core's atomic source/enabled activation result to
    /// diagnostics. No block is updated if core activation failed.
    pub fn record_runtime_activation(
        &self,
        document_revision: u64,
        activation: &logiksmith_core::ActivationResult,
        runtime: &Runtime,
        automation: &AutomationRuntime,
    ) {
        let snapshot = runtime.snapshot();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for result in &activation.blocks {
            let id = result.block_id.to_string();
            let Some(block) = inner.blocks.get_mut(&id) else {
                continue;
            };
            block.active_enabled = result.enabled;
            block.active_logic_revision = crate::block_revision(&automation.document, &id);
            if let Ok(core_id) = id.parse::<logiksmith_core::BlockId>()
                && let Some(core_block) = runtime.block(&core_id)
            {
                block.source = core_block.logic_program().source().to_owned();
            } else if let Some(document_block) = automation
                .document
                .blocks
                .iter()
                .find(|candidate| candidate.id == id)
            {
                block.source = document_block.source.clone();
            }
            if !result.cancelled_timers.is_empty() {
                let mut fields = BTreeMap::new();
                fields.insert("block_id".to_owned(), id.clone());
                fields.insert(
                    "cancelled_timers".to_owned(),
                    result
                        .cancelled_timers
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(","),
                );
                inner.logs.push_back(LogRecord {
                    time_ms: self.now().0,
                    level: "info".to_owned(),
                    target: "logiksmith".to_owned(),
                    message: "block activation cancelled pending timers".to_owned(),
                    fields,
                });
                while inner.logs.len() > MAX_LOGS {
                    inner.logs.pop_front();
                }
            }
        }
        inner.saved_automation_revision = document_revision;
        drop(inner);
        self.set_runtime_projection_from_runtime(
            runtime,
            runtime.last_accepted_at().unwrap_or_default(),
        );
        let _ = snapshot;
    }
    /// Stores one immutable semantic core execution with host-only timing and
    /// resolved KNX destinations. The core outcome is intentionally handled
    /// here so zero-effect successes and contained Lua failures are retained.
    pub fn record_execution(
        &self,
        execution: &Execution,
        duration_us: u64,
        automation: &AutomationRuntime,
    ) {
        self.record_execution_at(execution, self.now(), duration_us, automation);
    }

    pub fn record_execution_at(
        &self,
        execution: &Execution,
        now: logiksmith_core::MonotonicMs,
        duration_us: u64,
        automation: &AutomationRuntime,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let execution_id = inner.next_execution_id;
        inner.next_execution_id = inner.next_execution_id.saturating_add(1);
        let document_revision = inner.active_logic_revision;
        let (status, effects, transition, error) = match &execution.outcome {
            Ok(effects) => (
                LogicExecutionStatus::Succeeded,
                effects
                    .outputs
                    .iter()
                    .filter_map(|effect| effect_record(effect, automation))
                    .collect(),
                Some(transition_record(effects, automation)),
                None,
            ),
            Err(error) => (
                LogicExecutionStatus::Failed,
                Vec::new(),
                None,
                Some(logic_error_record(error)),
            ),
        };
        inner.executions.push_back(ExecutionRecord {
            block_id: automation
                .blocks
                .first()
                .map(|block| block.id.clone())
                .unwrap_or_default(),
            execution_id,
            time_ms: now.0,
            duration_us,
            logic_revision: document_revision,
            status,
            trigger: trigger_record(&execution.trigger, document_revision),
            inputs: execution.inputs.iter().map(input_snapshot_record).collect(),
            state_before: state_record(&execution.state_before),
            state_after: state_record(&execution.state_after),
            timer_effects: transition
                .as_ref()
                .map(|transition| transition.timers.clone())
                .unwrap_or_default(),
            transition,
            effects,
            error,
        });
        inner.state = state_record(&execution.state_after);
        inner.pending_timers = execution
            .pending_timers
            .iter()
            .map(|timer| pending_timer_record(timer, document_revision))
            .collect();
        while inner.executions.len() > MAX_EXECUTIONS {
            inner.executions.pop_front();
        }
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
        if let Some(value) = telegram.value.as_ref()
            && let Ok(address) = telegram.destination.parse::<GroupAddress>()
        {
            for state in inner.block_endpoint_values.values_mut() {
                if state.address == address && state.direction == EndpointDirection::Input {
                    state.observed = Some(value.clone());
                }
            }
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
        block_id: &logiksmith_core::BlockId,
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
        let execution_id = inner
            .blocks
            .get(block_id.as_str())
            .and_then(|block| block.last_result.as_ref())
            .map(|result| result.execution_id);
        if let Some(block_state) = inner
            .block_endpoint_values
            .get_mut(&(block_id.to_string(), endpoint.clone()))
        {
            block_state.requested = Some(value.clone());
        }
        if let Some(state) = inner.endpoint_values.get_mut(&endpoint) {
            state.requested = Some(value.clone());
            if state.address == destination && state.direction == EndpointDirection::Output {
                inner.last_write = WriteSnapshot {
                    status: WriteStatus::Pending,
                    request_id: Some(request_id),
                    block_id: Some(block_id.to_string()),
                    execution_id,
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
        inner.captured_at_ms = self.now().0;
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

fn trigger_record(trigger: &Trigger, document_revision: u64) -> LogicalTriggerRecord {
    match trigger {
        Trigger::Input(trigger) => LogicalTriggerRecord {
            trigger_type: "input".to_owned(),
            endpoint: trigger.endpoint.to_string(),
            dpt: DptMessage::from_core(trigger.value.dpt),
            value: ValueMessage::from_core(trigger.value),
            previous: trigger.previous.map(ValueMessage::from_core),
            changed: trigger.changed,
            rising: trigger.rising,
            falling: trigger.falling,
            name: None,
            scheduled_at_ms: None,
            due_at_ms: None,
            fired_at_ms: None,
            late_by_ms: None,
            scheduled_logic_revision: None,
        },
        Trigger::Timer(trigger) => LogicalTriggerRecord {
            trigger_type: "timer".to_owned(),
            endpoint: String::new(),
            dpt: DptMessage {
                major: 0,
                subtype: 0,
            },
            value: ValueMessage::Bool(BoolValueMessage {
                kind: "bool".to_owned(),
                value: false,
            }),
            previous: None,
            changed: false,
            rising: false,
            falling: false,
            name: Some(trigger.name.to_string()),
            scheduled_at_ms: Some(trigger.scheduled_at.0),
            due_at_ms: Some(trigger.due_at.0),
            fired_at_ms: Some(trigger.fired_at.0),
            late_by_ms: Some(trigger.fired_at.0.saturating_sub(trigger.due_at.0)),
            scheduled_logic_revision: Some(document_revision),
        },
    }
}

pub fn simulation_response(
    execution: &Execution,
    duration_us: u64,
    logic_revision: u64,
    automation: &AutomationRuntime,
) -> SimulationResponse {
    let (status, effects, transition, error) = match &execution.outcome {
        Ok(effects) => (
            LogicExecutionStatus::Succeeded,
            effects
                .outputs
                .iter()
                .filter_map(|effect| effect_record(effect, automation))
                .collect(),
            Some(transition_record(effects, automation)),
            None,
        ),
        Err(error) => (
            LogicExecutionStatus::Failed,
            Vec::new(),
            None,
            Some(logic_error_record(error)),
        ),
    };
    SimulationResponse {
        block_id: automation
            .blocks
            .first()
            .map(|block| block.id.clone())
            .unwrap_or_default(),
        logic_revision,
        duration_us,
        status,
        trigger: trigger_record(&execution.trigger, logic_revision),
        inputs: execution.inputs.iter().map(input_snapshot_record).collect(),
        state_before: state_record(&execution.state_before),
        state_after: state_record(&execution.state_after),
        transition: transition.clone(),
        pending_timers: execution
            .pending_timers
            .iter()
            .map(|timer| pending_timer_record(timer, logic_revision))
            .collect(),
        effects,
        timer_effects: transition
            .as_ref()
            .map(|transition| transition.timers.clone())
            .unwrap_or_default(),
        error,
    }
}

pub fn simulation_response_for_block(
    tagged: &BlockExecution,
    duration_us: u64,
    logic_revision: u64,
    automation: &AutomationRuntime,
) -> SimulationResponse {
    let execution = &tagged.execution;
    let (status, effects, transition, error) = match &execution.outcome {
        Ok(effects) => (
            LogicExecutionStatus::Succeeded,
            effects
                .outputs
                .iter()
                .filter_map(|effect| effect_record_for_block(effect, automation, &tagged.block_id))
                .collect(),
            Some(transition_record_for_block(
                effects,
                automation,
                &tagged.block_id,
            )),
            None,
        ),
        Err(error) => (
            LogicExecutionStatus::Failed,
            Vec::new(),
            None,
            Some(logic_error_record(error)),
        ),
    };
    SimulationResponse {
        block_id: tagged.block_id.to_string(),
        logic_revision,
        duration_us,
        status,
        trigger: trigger_record(&execution.trigger, logic_revision),
        inputs: execution.inputs.iter().map(input_snapshot_record).collect(),
        state_before: state_record(&execution.state_before),
        state_after: state_record(&execution.state_after),
        transition: transition.clone(),
        pending_timers: execution
            .pending_timers
            .iter()
            .map(|timer| pending_timer_record(timer, logic_revision))
            .collect(),
        effects,
        timer_effects: transition
            .as_ref()
            .map(|transition| transition.timers.clone())
            .unwrap_or_default(),
        error,
    }
}

fn input_snapshot_record(input: &logiksmith_core::InputSnapshot) -> LogicalInputSnapshot {
    LogicalInputSnapshot {
        endpoint: input.endpoint.to_string(),
        dpt: DptMessage::from_core(input.dpt),
        value: input.value.map(ValueMessage::from_core),
        valid: input.valid,
        age_ms: input.age_ms,
    }
}

fn effect_record(
    effect: &OutputEffect,
    automation: &AutomationRuntime,
) -> Option<LogicalEffectRecord> {
    let block_id = automation.blocks.first().map(|block| block.id.as_str())?;
    effect_record_for_block(effect, automation, &block_id.parse().ok()?)
}

fn effect_record_for_block(
    effect: &OutputEffect,
    automation: &AutomationRuntime,
    block_id: &logiksmith_core::BlockId,
) -> Option<LogicalEffectRecord> {
    let endpoint = &effect.endpoint;
    let value = effect.value;
    Some(LogicalEffectRecord {
        block_id: block_id.to_string(),
        endpoint: endpoint.to_string(),
        destination: automation
            .output_to_address
            .get(&(block_id.to_string(), endpoint.clone()))?
            .to_string(),
        dpt: DptMessage::from_core(value.dpt),
        value: ValueMessage::from_core(value),
    })
}

fn state_value_record(value: &StateValue) -> StateValueRecord {
    let (kind, value) = match value {
        StateValue::Bool(value) => ("bool", JsonValue::Bool(*value)),
        StateValue::Integer(value) => ("integer", JsonValue::Number((*value).into())),
        StateValue::Number(value) => (
            "number",
            serde_json::Number::from_f64(*value)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null),
        ),
        StateValue::String(value) => ("string", JsonValue::String(value.clone())),
    };
    StateValueRecord {
        kind: kind.to_owned(),
        value,
    }
}

fn state_record(state: &logiksmith_core::TransientState) -> BTreeMap<String, StateValueRecord> {
    state
        .iter()
        .map(|(key, value)| (key.clone(), state_value_record(value)))
        .collect()
}

fn pending_timer_record(
    timer: &logiksmith_core::PendingTimer,
    document_revision: u64,
) -> PendingTimerRecord {
    PendingTimerRecord {
        name: timer.name.to_string(),
        scheduled_at_ms: timer.scheduled_at.0,
        due_at_ms: timer.due_at.0,
        logic_revision: document_revision,
    }
}

fn transition_record(
    transition: &logiksmith_core::Transition,
    automation: &AutomationRuntime,
) -> LogicalTransitionRecord {
    let Some(block_id) = automation
        .blocks
        .first()
        .and_then(|block| block.id.parse::<logiksmith_core::BlockId>().ok())
    else {
        return LogicalTransitionRecord {
            state: state_record(&transition.state),
            effects: Vec::new(),
            timers: transition_timer_records(transition),
        };
    };
    transition_record_for_block(transition, automation, &block_id)
}

fn transition_record_for_block(
    transition: &logiksmith_core::Transition,
    automation: &AutomationRuntime,
    block_id: &logiksmith_core::BlockId,
) -> LogicalTransitionRecord {
    LogicalTransitionRecord {
        state: state_record(&transition.state),
        effects: transition
            .outputs
            .iter()
            .filter_map(|effect| effect_record_for_block(effect, automation, block_id))
            .collect(),
        timers: transition_timer_records(transition),
    }
}

fn transition_timer_records(
    transition: &logiksmith_core::Transition,
) -> Vec<LogicalTimerEffectRecord> {
    transition
        .timers
        .iter()
        .map(|effect| {
            let (action, after_ms, previous_due_at_ms, due_at_ms) = match effect.action {
                TimerAction::Scheduled { after_ms, due_at } => {
                    ("scheduled", Some(after_ms), None, Some(due_at.0))
                }
                TimerAction::Replaced {
                    previous_due_at,
                    after_ms,
                    due_at,
                } => (
                    "replaced",
                    Some(after_ms),
                    Some(previous_due_at.0),
                    Some(due_at.0),
                ),
                TimerAction::Cancelled { previous_due_at } => {
                    ("cancelled", None, Some(previous_due_at.0), None)
                }
                TimerAction::CancelNoop => ("cancel_noop", None, None, None),
            };
            LogicalTimerEffectRecord {
                name: effect.name.to_string(),
                action: action.to_owned(),
                after_ms,
                previous_due_at_ms,
                due_at_ms,
            }
        })
        .collect()
}

fn logic_error_record(error: &logiksmith_core::LogicError) -> LogicErrorRecord {
    let mut message = error.message().to_owned();
    if message.len() > MAX_LOGIC_ERROR {
        let end = (0..=MAX_LOGIC_ERROR)
            .rev()
            .find(|index| message.is_char_boundary(*index))
            .unwrap_or(0);
        message.truncate(end);
    }
    LogicErrorRecord {
        category: error.category().to_owned(),
        message,
        line: error.line().and_then(|line| u32::try_from(line).ok()),
    }
}
fn automation_snapshot(runtime: &AutomationRuntime) -> AutomationSnapshot {
    let Some(block) = runtime.blocks.first() else {
        return AutomationSnapshot {
            inputs: Vec::new(),
            outputs: Vec::new(),
            knx_bindings: Vec::new(),
            logic: LogicSourceSnapshot {
                source: String::new(),
            },
        };
    };
    block_automation_snapshot(runtime, block)
}

fn block_automation_snapshot(
    runtime: &AutomationRuntime,
    block: &crate::BlockRuntime,
) -> AutomationSnapshot {
    let endpoint = |name: &str, dpt: Dpt| EndpointSnapshot {
        name: name.to_owned(),
        dpt: DptMessage::from_core(dpt),
    };
    let inputs = block
        .engine_config
        .endpoints
        .iter()
        .filter(|item| item.direction == EndpointDirection::Input)
        .map(|item| endpoint(item.name.as_str(), item.dpt))
        .collect();
    let outputs = block
        .engine_config
        .endpoints
        .iter()
        .filter(|item| item.direction == EndpointDirection::Output)
        .map(|item| endpoint(item.name.as_str(), item.dpt))
        .collect();
    let mut knx_bindings: Vec<_> = block
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
            source: runtime.document.blocks[0].source.clone(),
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
    let blocks = inner
        .block_order
        .iter()
        .filter_map(|id| inner.blocks.get(id).map(|state| (id, state)))
        .map(|(id, state)| {
            let automation = inner.block_automation.get(id);
            BlockSnapshot {
                id: id.clone(),
                active_enabled: state.active_enabled,
                saved_enabled: state.saved_enabled,
                active_revision: state.active_logic_revision,
                saved_revision: state.saved_logic_revision,
                active_logic_revision: state.active_logic_revision,
                saved_logic_revision: state.saved_logic_revision,
                source: if state.source.is_empty() {
                    automation
                        .map(|automation| automation.logic.source.clone())
                        .unwrap_or_default()
                } else {
                    state.source.clone()
                },
                inputs: automation
                    .map(|automation| automation.inputs.clone())
                    .unwrap_or_default(),
                outputs: automation
                    .map(|automation| automation.outputs.clone())
                    .unwrap_or_default(),
                knx_bindings: automation
                    .map(|automation| automation.knx_bindings.clone())
                    .unwrap_or_default(),
                values: ValuesSnapshot {
                    endpoints: inner
                        .block_endpoint_values
                        .iter()
                        .filter(|((block_id, _), _)| block_id == id)
                        .map(|((_, name), state)| EndpointValueSnapshot {
                            name: name.to_string(),
                            direction: state.direction.to_string(),
                            dpt: DptMessage::from_core(state.dpt),
                            observed: state.observed.clone(),
                            requested: state.requested.clone(),
                        })
                        .collect(),
                },
                state: state.state.clone(),
                pending_timers: state.pending_timers.clone(),
                executions: state.executions.iter().rev().cloned().collect(),
                last_result: state.last_result.clone(),
            }
        })
        .collect();
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
        captured_at_ms: inner.captured_at_ms,
        state: inner.state.clone(),
        pending_timers: inner.pending_timers.clone(),
        values,
        write: inner.last_write.clone(),
        logic: LogicStatusSnapshot {
            active_logic_revision: inner.active_logic_revision,
            saved_logic_revision: inner.saved_logic_revision,
            active_structural_revision: inner.active_structural_revision,
            saved_structural_revision: inner.saved_structural_revision,
            restart_required: inner.restart_required,
            state: inner.state.clone(),
            pending_timers: inner.pending_timers.clone(),
            executions: inner.executions.iter().rev().cloned().collect(),
        },
        telegrams: inner.telegrams.iter().cloned().collect(),
        logs: inner.logs.iter().cloned().collect(),
        blocks,
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
