// Bounded, read-only diagnostic state for the desktop dashboard.

use crate::{
    AutomationRuntime, BoolValueMessage, DptMessage, GroupAddress, KnxEvent, ValueMessage,
};
use logiksmith_core::{
    BlockExecution, ClockSample, Dpt, EndpointDirection, EndpointName, Execution, OutputEffect,
    Runtime, ScheduleOccurrence, ScheduleStatus, StateValue, TimeContext, TimerAction, Trigger,
    TypedValue,
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

#[derive(Clone, Debug, PartialEq, Serialize)]
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
    /// Site wall-clock and astronomy card.
    pub site_time: SiteTimeSnapshot,
    /// Ordered block-local diagnostics. The older global projections remain
    /// during the dashboard migration, but this is the authoritative M8 view.
    pub blocks: Vec<BlockSnapshot>,
    pub signals: Vec<SignalSnapshot>,
}
/// Read-only site wall-clock and astronomy facts for the dashboard card.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SiteTimeSnapshot {
    pub timezone: String,
    /// Local wall time at capture, formatted `YYYY-MM-DD HH:MM:SS`.
    pub local_time: Option<String>,
    /// Seconds east of UTC at capture.
    pub utc_offset: Option<i64>,
    pub coordinates: Option<CoordinatesSnapshot>,
    /// `available` when solar events could be computed, else `unavailable`.
    pub astronomy: String,
    pub astronomy_reason: Option<String>,
    pub dawn: Option<String>,
    pub sunrise: Option<String>,
    pub sunset: Option<String>,
    pub dusk: Option<String>,
    pub clock_ok: bool,
    pub scheduler_ok: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct CoordinatesSnapshot {
    pub latitude: f64,
    pub longitude: f64,
}

/// Immutable comparable civil date-time values captured for one execution.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TimeContextRecord {
    pub now: DateTimeValueRecord,
    pub sun: SunContextRecord,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DateTimeValueRecord {
    pub available: bool,
    pub year: Option<i32>,
    pub month: Option<u8>,
    pub day: Option<u8>,
    pub hour: Option<u8>,
    pub minute: Option<u8>,
    pub second: Option<u8>,
    /// Full weekday name such as `Friday`; null when unavailable.
    pub weekday: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SunContextRecord {
    pub dawn: DateTimeValueRecord,
    pub sunrise: DateTimeValueRecord,
    pub sunset: DateTimeValueRecord,
    pub dusk: DateTimeValueRecord,
    pub elevation_degrees: Option<f64>,
    pub azimuth_degrees: Option<f64>,
}

/// One schedule row in a block's diagnostic section.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScheduleSnapshot {
    pub name: String,
    pub enabled: bool,
    /// `active`, `paused`, `unavailable`, or `clock_error`.
    pub status: String,
    pub rule: ScheduleRuleSnapshot,
    pub next_occurrence: Option<String>,
    pub next_occurrence_utc_ms: Option<i64>,
    /// Milliseconds until the next occurrence from the snapshot capture.
    pub relative_ms: Option<i64>,
    /// UTC offset in seconds at the next occurrence.
    pub utc_offset: Option<i64>,
    pub unavailable_reason: Option<String>,
    pub last_result: Option<ScheduleLastResultSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScheduleRuleSnapshot {
    pub kind: String,
    pub summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScheduleLastResultSnapshot {
    /// `delivered` or `failed`.
    pub status: String,
    pub execution_id: u64,
    pub time_ms: u64,
}
/// Read-only schedule occurrence preview returned for a schedule trigger
/// simulation without a selected instant. The preview itself mutates nothing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SchedulePreviewResponse {
    pub block_id: String,
    pub schedule: String,
    pub rule: ScheduleRuleSnapshot,
    pub occurrences: Vec<ScheduleOccurrenceSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScheduleOccurrenceSnapshot {
    pub utc_ms: i64,
    /// Local civil time at the occurrence, `YYYY-MM-DD HH:MM:SS`.
    pub local: String,
    pub utc_offset: i64,
    pub weekday: Option<String>,
}

/// Host-fed scheduler status for one block schedule. The session refreshes
/// this after every poll and structural restart.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleStatusFeed {
    pub block_id: String,
    pub name: String,
    pub clock_error: bool,
    pub unavailable_reason: Option<String>,
    pub next_occurrence_utc_ms: Option<i64>,
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
    #[serde(rename = "signalBindings")]
    pub signal_bindings: Vec<SignalBindingSnapshot>,
    pub logic: LogicSourceSnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
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
    #[serde(rename = "signalBindings")]
    pub signal_bindings: Vec<SignalBindingSnapshot>,
    pub values: ValuesSnapshot,
    pub state: BTreeMap<String, StateValueRecord>,
    pub pending_timers: Vec<PendingTimerRecord>,
    pub executions: Vec<ExecutionRecord>,
    pub schedules: Vec<ScheduleSnapshot>,
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
    #[serde(rename = "bindingKind")]
    pub binding_kind: String,
    pub signal: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BindingSnapshot {
    pub endpoint: String,
    pub group_address: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalBindingSnapshot {
    pub endpoint: String,
    pub signal: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalSnapshot {
    pub name: String,
    pub dpt: DptMessage,
    pub value: Option<ValueMessage>,
    pub status: String,
    pub producer: Option<SignalProducerSnapshot>,
    pub consumers: Vec<SignalConsumerSnapshot>,
    pub observed_at_ms: Option<u64>,
    pub changed_at_ms: Option<u64>,
    pub producing_execution_id: Option<u64>,
    pub recent_changes: Vec<SignalChangeSnapshot>,
    #[serde(serialize_with = "crate::wire_revision::serialize_option")]
    pub structural_revision: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalProducerSnapshot {
    pub block_id: String,
    pub endpoint: String,
    pub execution_id: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalConsumerSnapshot {
    pub block_id: String,
    pub endpoint: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalChangeSnapshot {
    pub value: Option<ValueMessage>,
    pub observed_at_ms: Option<u64>,
    pub changed_at_ms: Option<u64>,
    pub execution_id: Option<u64>,
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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LogicStatusSnapshot {
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    pub active_logic_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    pub saved_logic_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    pub active_structural_revision: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
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
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionRecord {
    pub block_id: String,
    pub execution_id: u64,
    pub time_ms: u64,
    pub duration_us: u64,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    pub logic_revision: u64,
    pub status: LogicExecutionStatus,
    pub trigger: LogicalTriggerRecord,
    /// Captured wall-clock context for this execution, resolved at the
    /// triggering instant. Stored values never change after the record lands.
    pub time_context: TimeContextRecord,
    pub inputs: Vec<LogicalInputSnapshot>,
    pub state_before: BTreeMap<String, StateValueRecord>,
    pub state_after: BTreeMap<String, StateValueRecord>,
    pub transition: Option<LogicalTransitionRecord>,
    pub effects: Vec<LogicalEffectRecord>,
    #[serde(rename = "signalEffects")]
    pub signal_effects: Vec<LogicalSignalEffectRecord>,
    #[serde(rename = "causalProducerExecutionId")]
    pub causal_producer_execution_id: Option<u64>,
    #[serde(rename = "causalProducerBlockId")]
    pub causal_producer_block_id: Option<String>,
    #[serde(rename = "causalSignal")]
    pub causal_signal: Option<String>,
    #[serde(rename = "causalLinks")]
    pub causal_links: Vec<CausalLinkSnapshot>,
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
    /// `fixed`, `interval`, or `astronomical`; absent for input/timer.
    pub kind: Option<String>,
    pub scheduled_for_utc_ms: Option<i64>,
    pub detected_at_utc_ms: Option<i64>,
    /// Host wall-clock instant when handling started; queue delay is measured
    /// from this against detection.
    pub handled_at_utc_ms: Option<i64>,
    pub queue_delay_ms: Option<u64>,
    pub coalesced_count: Option<u64>,
    pub structural_revision: Option<u64>,
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
        } else if self.trigger_type == "schedule" {
            if let Some(value) = &self.name {
                map.serialize_entry("name", value)?;
            }
            if let Some(value) = &self.kind {
                map.serialize_entry("kind", value)?;
            }
            if let Some(value) = self.scheduled_for_utc_ms {
                map.serialize_entry("scheduled_for_utc_ms", &value)?;
            }
            if let Some(value) = self.detected_at_utc_ms {
                map.serialize_entry("detected_at_utc_ms", &value)?;
            }
            if let Some(value) = self.handled_at_utc_ms {
                map.serialize_entry("handled_at_utc_ms", &value)?;
            }
            if let Some(value) = self.late_by_ms {
                map.serialize_entry("late_by_ms", &value)?;
            }
            if let Some(value) = self.queue_delay_ms {
                map.serialize_entry("queue_delay_ms", &value)?;
            }
            if let Some(value) = self.coalesced_count {
                map.serialize_entry("coalesced_count", &value)?;
            }
            if let Some(value) = self.structural_revision {
                map.serialize_entry("structural_revision", &crate::wire_revision::Value(value))?;
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
#[serde(rename_all = "camelCase")]
pub struct LogicalSignalEffectRecord {
    pub endpoint: String,
    pub signal: String,
    pub dpt: DptMessage,
    pub value: ValueMessage,
    pub changed: bool,
    pub producer: Option<SignalProducerSnapshot>,
    pub producing_execution_id: Option<u64>,
    pub consumers: Vec<SignalConsumerSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CausalLinkSnapshot {
    pub producer_execution_id: u64,
    pub consumer_execution_id: u64,
    pub signal: Option<String>,
    pub producer_block_id: Option<String>,
    pub consumer_block_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LogicalTransitionRecord {
    pub state: BTreeMap<String, StateValueRecord>,
    pub effects: Vec<LogicalEffectRecord>,
    #[serde(rename = "signalEffects")]
    pub signal_effects: Vec<LogicalSignalEffectRecord>,
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
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SimulationResponse {
    pub block_id: String,
    #[serde(serialize_with = "crate::wire_revision::serialize")]
    pub logic_revision: u64,
    pub duration_us: u64,
    pub status: LogicExecutionStatus,
    pub trigger: LogicalTriggerRecord,
    pub time_context: TimeContextRecord,
    pub inputs: Vec<LogicalInputSnapshot>,
    pub state_before: BTreeMap<String, StateValueRecord>,
    pub state_after: BTreeMap<String, StateValueRecord>,
    pub transition: Option<LogicalTransitionRecord>,
    pub pending_timers: Vec<PendingTimerRecord>,
    pub effects: Vec<LogicalEffectRecord>,
    #[serde(rename = "signalEffects")]
    pub signal_effects: Vec<LogicalSignalEffectRecord>,
    #[serde(rename = "eligibleConsumers")]
    pub eligible_consumers: Vec<SignalConsumerSnapshot>,
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
    /// The documents actually used by the running session. `automation` is
    /// the legacy browser projection; these documents are the authoritative
    /// active/saved lifecycle state after a save or activation.
    active_document: crate::AutomationDocument,
    saved_document: crate::AutomationDocument,
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
    signals: Vec<SignalSnapshot>,
    block_order: Vec<String>,
    block_automation: BTreeMap<String, AutomationSnapshot>,
    block_endpoint_values: BTreeMap<(String, EndpointName), EndpointValueState>,
    site_time: SiteTimeSnapshot,
    last_clock_sample: Option<ClockSample>,
    schedule_status: BTreeMap<(String, String), ScheduleStatusFeed>,
    /// Structural schedule definitions per block, read from the active
    /// runtime at store construction.
    block_schedules: BTreeMap<String, Vec<ScheduleConfigSnapshot>>,
}
#[derive(Clone, Debug)]
struct ScheduleConfigSnapshot {
    name: String,
    enabled: bool,
    rule: ScheduleRuleSnapshot,
}
#[derive(Clone, Debug)]
struct EndpointValueState {
    direction: EndpointDirection,
    dpt: Dpt,
    address: Option<GroupAddress>,
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
