import type { RevisionToken } from './revision';

export type ConnectionState = 'starting' | 'connecting' | 'connected' | 'disconnected' | 'failed';
export type TimerState = 'idle' | 'pending';
export type StreamStatus = 'connecting' | 'connected' | 'stale' | 'error';
export type WriteStatus = 'idle' | 'pending' | 'succeeded' | 'failed';
export type DisplayEndpointBindingKind = 'knx' | 'signal' | 'http' | 'webhook' | 'unbound';

export interface DisplayEndpoint { name?: string; address: string; dpt: string; direction?: 'input' | 'output'; bindingKind?: DisplayEndpointBindingKind; signal?: string | null; /** External source name for HTTP/webhook bindings. */ source?: string | null; observed?: boolean | number | null; requested?: boolean | number | null; }
export interface DisplayBinding { endpoint: string; groupAddress?: string; kind?: DisplayEndpointBindingKind; signal?: string; source?: string; poll?: string; value?: string; }
export interface DisplaySignalBinding { endpoint: string; signal: string; dpt?: string; }
export interface DisplayAutomation { inputs: DisplayEndpoint[]; outputs: DisplayEndpoint[]; bindings: DisplayBinding[]; signalBindings?: DisplaySignalBinding[]; source: string; }
export interface DisplayLogicError { category: string; message: string; line: number | null; }
export interface DisplayLastResult { status: 'none' | 'succeeded' | 'failed'; executionId: number | null; timeMs: number | null; error: DisplayLogicError | null; }

export type DisplayStateValue =
  | { kind: 'bool'; value: boolean }
  | { kind: 'integer'; value: number }
  | { kind: 'number'; value: number }
  | { kind: 'string'; value: string };
export type DisplayState = Record<string, DisplayStateValue>;

export interface DisplayInputExecutionTrigger { type: 'input'; endpoint: string; dpt: string; value: boolean | number; previous: boolean | number | null; changed: boolean; rising: boolean; falling: boolean; }
export interface DisplayTimerExecutionTrigger { type: 'timer'; name: string; timer: string; scheduledAtMs: number; dueAtMs: number; firedAtMs: number; lateByMs: number; scheduledLogicRevision: RevisionToken; }
export type DisplayScheduleKind = 'fixed' | 'interval' | 'astronomical';
export interface DisplayScheduleExecutionTrigger {
  type: 'schedule';
  name: string;
  kind: DisplayScheduleKind;
  blockId: string | null;
  /** Intended occurrence instant in UTC epoch milliseconds. */
  scheduledForUtcMs: number;
  /** Instant the scheduler detected the due occurrence, UTC epoch milliseconds. */
  detectedAtUtcMs: number;
  /** Instant the block dispatcher started handling the trigger, UTC epoch milliseconds. */
  handledAtUtcMs: number;
  /** Clock lateness: detection minus intended occurrence, milliseconds. */
  lateByMs: number;
  /** Dispatcher pressure: handling start minus detection, milliseconds. */
  queueDelayMs: number;
  /** Occurrences skipped while the schedule was due but undelivered. */
  coalescedCount: number;
  /** Structural revision that created the trigger. */
  structuralRevision: RevisionToken;
}
export type DisplayExecutionTrigger = DisplayInputExecutionTrigger | DisplayTimerExecutionTrigger | DisplayScheduleExecutionTrigger;

export interface DisplayExecutionInput { endpoint: string; dpt: string; value: boolean | number | null; valid: boolean; ageMs: number | null; }
export interface DisplayExecutionEffect { endpoint: string; destination: string; dpt: string; value: boolean | number; }
export interface DisplaySignalEffect { endpoint: string; signal: string; dpt: string; value: boolean | number; changed?: boolean; producer?: DisplaySignalProducer | null; producingExecutionId?: number | null; consumers?: DisplaySignalConsumer[]; }
export interface DisplayCausalLink { producerExecutionId: number; consumerExecutionId: number; signal: string | null; producerBlockId?: string | null; consumerBlockId?: string | null; }
export type DisplayTimerEffectAction = 'scheduled' | 'replaced' | 'cancelled' | 'cancel_noop';
export interface DisplayTimerEffect { name: string; action: DisplayTimerEffectAction; afterMs: number | null; dueAtMs: number | null; previousDueAtMs: number | null; }
export interface DisplayTransition { state: DisplayState; effects: DisplayExecutionEffect[]; signalEffects: DisplaySignalEffect[]; timers: DisplayTimerEffect[]; }

export interface DisplaySignalProducer { blockId: string; endpoint: string; executionId: number | null; }
export interface DisplaySignalConsumer { blockId: string; endpoint: string; }
export interface DisplaySignalChange { value: boolean | number | null; observedAtMs: number | null; changedAtMs: number | null; executionId: number | null; }
export interface DisplaySignal {
  name: string;
  dpt: string;
  value: boolean | number | null;
  status: string;
  observedAtMs: number | null;
  changedAtMs: number | null;
  producer: DisplaySignalProducer | null;
  producingExecutionId: number | null;
  consumers: DisplaySignalConsumer[];
  recentChanges: DisplaySignalChange[];
  structuralRevision: RevisionToken | null;
}

/** A block endpoint reached through a host-managed external input. */
export interface DisplayExternalConsumer { blockId: string; endpoint: string; }
export interface DisplayExternalValue {
  name: string;
  dpt: string;
  jsonPointer: string;
  value: boolean | number | null;
  valid: boolean;
  ageMs: number | null;
  consumers: DisplayExternalConsumer[];
}
export type DisplayExternalHealth = 'starting' | 'healthy' | 'failing' | 'stale';
export interface DisplayHttpPoll {
  kind: 'http';
  name: string;
  url: string;
  intervalMs: number;
  status: DisplayExternalHealth;
  lastAttemptAtMs: number | null;
  nextAttemptAtMs: number | null;
  lastSuccessAtMs: number | null;
  staleAtMs: number | null;
  consecutiveFailures: number;
  lastError: string | null;
  values: DisplayExternalValue[];
}
export interface DisplayWebhookInput {
  kind: 'webhook';
  name: string;
  route: string;
  dpt: string;
  jsonPointer: string;
  status: DisplayExternalHealth;
  authenticationRequired: boolean;
  authenticationConfigured: boolean;
  lastAcceptedAtMs: number | null;
  acceptedCount: number;
  rejectedCount: number;
  value: boolean | number | null;
  valid: boolean;
  ageMs: number | null;
  consumers: DisplayExternalConsumer[];
}
export interface DisplayExternalInputs {
  httpPolls: DisplayHttpPoll[];
  webhooks: DisplayWebhookInput[];
}

/** A captured civil calendar value. Unavailable values expose null fields. */
export interface DisplayDateTimeValue {
  available: boolean;
  year: number | null;
  month: number | null;
  day: number | null;
  hour: number | null;
  minute: number | null;
  second: number | null;
  /** Weekday name like "Friday" when available. */
  weekday: string | null;
}
export interface DisplaySunContext {
  dawn: DisplayDateTimeValue;
  sunrise: DisplayDateTimeValue;
  sunset: DisplayDateTimeValue;
  dusk: DisplayDateTimeValue;
  /** Degrees above the horizon at the context instant; null without coordinates. */
  elevationDegrees: number | null;
  /** Degrees clockwise from true north at the context instant; null without coordinates. */
  azimuthDegrees: number | null;
}
export interface DisplayTimeContext {
  now: DisplayDateTimeValue;
  sun: DisplaySunContext;
}

/** Per-block schedule row. Wire status values: active, paused, unavailable, clock_error. */
export interface DisplayBlockSchedule {
  name: string;
  /** Configured enabled flag (schedule-level). */
  enabled: boolean;
  status: 'active' | 'paused' | 'unavailable' | 'clock_error';
  kind: DisplayScheduleKind;
  /** Server-rendered rule summary, e.g. "sunrise - 1h30m, Mon-Fri". */
  ruleSummary: string;
  /** Next occurrence local civil rendering "YYYY-MM-DD HH:MM:SS"; null when none. */
  nextOccurrenceLocal: string | null;
  /** Next occurrence UTC epoch milliseconds; null when none. */
  nextOccurrenceUtcMs: number | null;
  /** Milliseconds until the next occurrence measured from the snapshot capture. */
  relativeMs: number | null;
  /** UTC offset at the next occurrence in seconds; null when unavailable. */
  utcOffsetSeconds: number | null;
  /** Unavailable or clock-error explanation. */
  reason: string | null;
  lastOutcome: { status: 'none' | 'delivered' | 'failed'; executionId: number | null; timeMs: number | null };
  /** Occurrence previews fetched for the detail view. */
  occurrences: DisplayScheduleOccurrence[];
}
export interface DisplayScheduleOccurrence {
  /** UTC epoch milliseconds of the occurrence. */
  utcMs: number;
  /** Local civil rendering "YYYY-MM-DD HH:MM:SS"; null when unavailable. */
  local: string | null;
  /** UTC offset at the occurrence in seconds; null when unavailable. */
  utcOffsetSeconds: number | null;
  /** Weekday name like "Saturday"; present in occurrence previews. */
  weekday: string | null;
}
export interface DisplaySchedulePreview {
  blockId: string;
  schedule: string;
  kind: DisplayScheduleKind;
  ruleSummary: string;
  occurrences: DisplayScheduleOccurrence[];
}
export interface DisplaySiteTime {
  /** IANA time-zone identifier. */
  timezone: string;
  /** Wall time at capture "YYYY-MM-DD HH:MM:SS"; null when the clock is invalid. */
  localTime: string | null;
  /** UTC offset at capture in seconds east of UTC; null when the clock is invalid. */
  utcOffsetSeconds: number | null;
  /** Configured coordinates; null when the site has none. */
  coordinates: { latitude: number; longitude: number } | null;
  astronomy: 'available' | 'unavailable';
  astronomyReason: string | null;
  /** Today's solar events in local civil time; null when unavailable or without a crossing. */
  sun: { dawn: string | null; sunrise: string | null; sunset: string | null; dusk: string | null };
  clockOk: boolean;
  schedulerOk: boolean;
}

export interface DisplayExecution {
  blockId: string | null;
  executionId: number;
  timeMs: number;
  durationUs: number;
  logicRevision: RevisionToken | null;
  status: 'succeeded' | 'failed';
  trigger: DisplayExecutionTrigger;
  /** Host transport provenance; absent for legacy execution records. */
  origin: DisplayExecutionOrigin | null;
  inputs: DisplayExecutionInput[];
  transition: DisplayTransition | null;
  stateBefore: DisplayState;
  stateAfter: DisplayState;
  effects: DisplayExecutionEffect[];
  signalEffects: DisplaySignalEffect[];
  causalProducerExecutionId: number | null;
  causalProducerBlockId: string | null;
  causalSignal: string | null;
  causalLinks: DisplayCausalLink[];
  timerEffects: DisplayTimerEffect[];
  /** Captured time context; null only for legacy records without one. */
  timeContext: DisplayTimeContext | null;
  error: DisplayLogicError | null;
}

export type DisplayExecutionOrigin =
  | { kind: 'knx'; groupAddress: string | null }
  | { kind: 'signal'; signal: string }
  | { kind: 'http'; poll: string; value: string }
  | { kind: 'webhook'; source: string };

export interface DisplayPendingTimer { name: string; scheduledAtMs: number; dueAtMs: number; logicRevision: RevisionToken; }
export interface DisplayBlock {
  id: string;
  activeEnabled: boolean;
  savedEnabled: boolean;
  source: string;
  inputs: DisplayEndpoint[];
  outputs: DisplayEndpoint[];
  bindings: DisplayBinding[];
  signalBindings: DisplaySignalBinding[];
  state: DisplayState;
  pendingTimers: DisplayPendingTimer[];
  schedules: DisplayBlockSchedule[];
  executions: DisplayExecution[];
  /** Persisted per-block revision, kept opaque at the JSON boundary. */
  activeRevision: RevisionToken | null;
  savedRevision: RevisionToken | null;
  activeLogicRevision: RevisionToken | null;
  savedLogicRevision: RevisionToken | null;
  lastResult: DisplayLastResult;
  lastError: DisplayLogicError | null;
}

export type SimulationTypedValue = { kind: 'bool'; value: boolean } | { kind: 'percent'; value: number } | { kind: 'temperature'; value: number };
export interface SimulationInputTriggerRequest { type?: 'input'; endpoint: string; value: SimulationTypedValue; previous: SimulationTypedValue | null; }
export interface SimulationTimerTriggerRequest { type: 'timer'; name?: string; timer?: string; firedAtMs: number; }
export interface SimulationScheduleTriggerRequest { type: 'schedule'; schedule: string; occurrenceAtMs: number | null; }
export type SimulationTriggerRequest = SimulationInputTriggerRequest | SimulationTimerTriggerRequest | SimulationScheduleTriggerRequest;
export interface SimulationInputRequest { endpoint: string; value: SimulationTypedValue | null; valid: boolean; ageMs: number | null; }
export interface SimulationScenario { blockId?: string; expectedLogicRevision: RevisionToken; expectedStructuralRevision?: RevisionToken | null; trigger: SimulationTriggerRequest; inputs: SimulationInputRequest[]; state?: DisplayState; pendingTimers?: DisplayPendingTimer[]; }

export interface DisplaySimulation {
  /** Block that was simulated; null only for legacy generic responses. */
  blockId: string | null;
  logicRevision: RevisionToken;
  durationUs: number;
  status: 'succeeded' | 'failed';
  trigger: DisplayExecutionTrigger;
  inputs: DisplayExecutionInput[];
  transition: DisplayTransition | null;
  stateBefore: DisplayState;
  stateAfter: DisplayState;
  effects: DisplayExecutionEffect[];
  signalEffects: DisplaySignalEffect[];
  eligibleConsumers: DisplaySignalConsumer[];
  timerEffects: DisplayTimerEffect[];
  pendingTimers: DisplayPendingTimer[];
  timeContext: DisplayTimeContext | null;
  error: DisplayLogicError | null;
}

export interface DisplayTelegram { time: string; source: string | null; destination: string; service: string; dpt: string; value: boolean | number | null; }
export interface DisplayLog { time: string; level: string; target: string; message: string; fields: Record<string, string | number | boolean | null>; }
/** Legacy single-timer projection retained only for old desktop snapshots. */
export interface DisplayTimer { state: TimerState; deadlineMs: number | null; remainingMs: number | null; sampledAtMs: number; }
export interface DisplayWrite { status: WriteStatus; requestId: number | null; blockId?: string | null; executionId?: number | null; value: boolean | number | null; error: string | null; }

export interface DisplaySnapshot {
  revision: number;
  connection: { state: ConnectionState };
  config: { input: DisplayEndpoint; output: DisplayEndpoint; offDelayMs: number };
  values: { input: { observed: boolean | number | null }; output: { observed: boolean | number | null; requested: boolean | number | null } };
  automation?: DisplayAutomation;
  activeAutomationRevision?: number | null;
  savedAutomationRevision?: number | null;
  activeStructuralRevision: RevisionToken | null;
  savedStructuralRevision: RevisionToken | null;
  activeLogicRevision: RevisionToken | null;
  savedLogicRevision: RevisionToken | null;
  restartRequired: boolean;
  capturedAtMs: number;
  /** Difference between the desktop monotonic clock and the browser clock when known. */
  clockOffsetMs?: number;
  /** Browser wall-clock time at which this snapshot was decoded, for local countdown anchors. */
  receivedAtMs: number;
  state: DisplayState;
  pendingTimers: DisplayPendingTimer[];
  executions: DisplayExecution[];
  /** Site-time card; null only for legacy snapshots without one. */
  siteTime: DisplaySiteTime | null;
  signals: DisplaySignal[];
  /** Host-managed HTTP poll and webhook diagnostics. */
  externalInputs: DisplayExternalInputs;
  write: DisplayWrite;
  timer: DisplayTimer;
  telegrams: DisplayTelegram[];
  logs: DisplayLog[];
  blocks: DisplayBlock[];
}

export interface DashboardState { snapshot: DisplaySnapshot | null; revision: number; streamStatus: StreamStatus; stale: boolean; staleAtMs: number | null; error: string | null; needsResync: boolean; nowMs: number; selectedBlockId: string | null; selectedExecutionId: number | null; selectionPinned: boolean; selectionNotice: string | null; }
export type DashboardEvent = { kind: 'update'; revision: number; snapshot: DisplaySnapshot } | { kind: 'resync'; revision: number };
export type DashboardAction =
  | { type: 'snapshot_loaded'; snapshot: DisplaySnapshot; nowMs?: number }
  | { type: 'stream_open' }
  | { type: 'stream_lost'; error?: string }
  | { type: 'stream_error'; error: string }
  | { type: 'event_received'; event: DashboardEvent }
  | { type: 'select_execution'; executionId: number }
  | { type: 'select_block'; blockId: string }
  | { type: 'tick'; nowMs: number };
