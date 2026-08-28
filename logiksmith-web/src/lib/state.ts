export type ConnectionState = 'starting' | 'connecting' | 'connected' | 'disconnected' | 'failed';
export type TimerState = 'idle' | 'pending';
export type StreamStatus = 'connecting' | 'connected' | 'stale' | 'error';
export type WriteStatus = 'idle' | 'pending' | 'succeeded' | 'failed';

export interface DisplayEndpoint {
  name?: string;
  address: string;
  dpt: string;
  direction?: 'input' | 'output';
  observed?: boolean | number | null;
  requested?: boolean | number | null;
}

export interface DisplayBinding {
  endpoint: string;
  groupAddress: string;
}

export interface DisplayAutomation {
  inputs: DisplayEndpoint[];
  outputs: DisplayEndpoint[];
  bindings: DisplayBinding[];
  source: string;
}

export interface DisplayLogicError {
  category: string;
  message: string;
  line: number | null;
}

export interface DisplayExecutionTrigger {
  endpoint: string;
  dpt: string;
  value: boolean | number;
  previous: boolean | number | null;
  changed: boolean;
  rising: boolean;
  falling: boolean;
}

export interface DisplayExecutionInput {
  endpoint: string;
  dpt: string;
  value: boolean | number | null;
  valid: boolean;
  ageMs: number | null;
}

export interface DisplayExecutionEffect {
  endpoint: string;
  destination: string;
  dpt: string;
  value: boolean | number;
}

export interface DisplayExecution {
  executionId: number;
  timeMs: number;
  durationUs: number;
  logicRevision: number | null;
  status: 'succeeded' | 'failed';
  trigger: DisplayExecutionTrigger;
  inputs: DisplayExecutionInput[];
  effects: DisplayExecutionEffect[];
  error: DisplayLogicError | null;
}

export type SimulationTypedValue =
  | { kind: 'bool'; value: boolean }
  | { kind: 'percent'; value: number };

export interface SimulationTriggerRequest {
  endpoint: string;
  value: SimulationTypedValue;
  previous: SimulationTypedValue | null;
}

export interface SimulationInputRequest {
  endpoint: string;
  value: SimulationTypedValue | null;
  valid: boolean;
  ageMs: number | null;
}

export interface SimulationScenario {
  expectedLogicRevision: number;
  trigger: SimulationTriggerRequest;
  inputs: SimulationInputRequest[];
}

export interface DisplaySimulation {
  logicRevision: number;
  durationUs: number;
  status: 'succeeded' | 'failed';
  trigger: DisplayExecutionTrigger;
  inputs: DisplayExecutionInput[];
  effects: DisplayExecutionEffect[];
  error: DisplayLogicError | null;
}

export interface DisplayTelegram {
  time: string;
  source: string | null;
  destination: string;
  service: string;
  dpt: string;
  value: boolean | number | null;
}

export interface DisplayLog {
  time: string;
  level: string;
  target: string;
  message: string;
  fields: Record<string, string | number | boolean | null>;
}

export interface DisplayTimer {
  state: TimerState;
  deadlineMs: number | null;
  remainingMs: number | null;
  sampledAtMs: number;
}

export interface DisplayWrite {
  status: WriteStatus;
  requestId: number | null;
  value: boolean | number | null;
  error: string | null;
}

export interface DisplaySnapshot {
  revision: number;
  connection: { state: ConnectionState };
  config: {
    input: DisplayEndpoint;
    output: DisplayEndpoint;
    offDelayMs: number;
  };
  values: {
    input: { observed: boolean | number | null };
    output: { observed: boolean | number | null; requested: boolean | number | null };
  };
  automation?: DisplayAutomation;
  activeAutomationRevision?: number | null;
  savedAutomationRevision?: number | null;
  activeStructuralRevision: number | null;
  savedStructuralRevision: number | null;
  activeLogicRevision: number | null;
  savedLogicRevision: number | null;
  restartRequired: boolean;
  executions: DisplayExecution[];
  write: DisplayWrite;
  timer: DisplayTimer;
  telegrams: DisplayTelegram[];
  logs: DisplayLog[];
}

export interface DashboardState {
  snapshot: DisplaySnapshot | null;
  revision: number;
  streamStatus: StreamStatus;
  stale: boolean;
  error: string | null;
  needsResync: boolean;
  nowMs: number;
  selectedExecutionId: number | null;
  selectionPinned: boolean;
  selectionNotice: string | null;
}

export type DashboardEvent =
  | { kind: 'update'; revision: number; snapshot: DisplaySnapshot }
  | { kind: 'resync'; revision: number };

export type DashboardAction =
  | { type: 'snapshot_loaded'; snapshot: DisplaySnapshot; nowMs?: number }
  | { type: 'stream_open' }
  | { type: 'stream_lost'; error?: string }
  | { type: 'stream_error'; error: string }
  | { type: 'event_received'; event: DashboardEvent }
  | { type: 'select_execution'; executionId: number }
  | { type: 'tick'; nowMs: number };

export const initialDashboardState: DashboardState = {
  snapshot: null,
  revision: 0,
  streamStatus: 'connecting',
  stale: true,
  error: null,
  needsResync: false,
  nowMs: 0,
  selectedExecutionId: null,
  selectionPinned: false,
  selectionNotice: null
};

function staleState(state: DashboardState, error: string | null = state.error): DashboardState {
  return { ...state, streamStatus: 'stale', stale: true, error };
}

function reconcileSelection(state: DashboardState, snapshot: DisplaySnapshot): Pick<DashboardState, 'selectedExecutionId' | 'selectionPinned' | 'selectionNotice'> {
  const newest = snapshot.executions[0]?.executionId ?? null;
  if (!state.selectionPinned) return { selectedExecutionId: newest, selectionPinned: false, selectionNotice: null };
  if (state.selectedExecutionId !== null && snapshot.executions.some((execution) => execution.executionId === state.selectedExecutionId)) {
    return { selectedExecutionId: state.selectedExecutionId, selectionPinned: true, selectionNotice: null };
  }
  return {
    selectedExecutionId: newest,
    selectionPinned: false,
    selectionNotice: state.selectedExecutionId === null ? null : 'The selected execution expired from memory.'
  };
}

export function reduceDashboardState(state: DashboardState, action: DashboardAction): DashboardState {
  switch (action.type) {
    case 'snapshot_loaded': {
      // A first snapshot is current even while the first stream is opening;
      // a refresh after a lost stream keeps the old view explicitly stale.
      const stale = state.snapshot !== null && state.stale;
      return {
        ...state,
        snapshot: action.snapshot,
        revision: action.snapshot.revision,
        streamStatus: 'connecting',
        stale,
        error: null,
        needsResync: false,
        nowMs: action.nowMs ?? state.nowMs,
        ...reconcileSelection(state, action.snapshot)
      };
    }

    case 'stream_open':
      return state.needsResync
        ? staleState(state)
        : { ...state, streamStatus: 'connected', stale: false, error: null };

    case 'stream_lost':
      return staleState(state, action.error ?? null);

    case 'stream_error':
      return { ...staleState(state, action.error), streamStatus: 'error' };

    case 'event_received': {
      if (action.event.kind === 'resync') {
        return { ...staleState(state), needsResync: true };
      }

      const event = action.event;
      if (event.revision <= state.revision) return state;
      if (event.revision !== state.revision + 1) {
        return { ...staleState(state, 'The event stream skipped a revision.'), needsResync: true };
      }

      return {
        ...state,
        snapshot: event.snapshot,
        revision: event.revision,
        error: null,
        needsResync: false,
        streamStatus: 'connected',
        stale: false,
        ...reconcileSelection(state, event.snapshot)
      };
    }

    case 'select_execution': {
      if (!state.snapshot) return state;
      const execution = state.snapshot.executions.find((item) => item.executionId === action.executionId);
      if (!execution) return state;
      const newest = state.snapshot.executions[0]?.executionId;
      return {
        ...state,
        selectedExecutionId: execution.executionId,
        selectionPinned: execution.executionId !== newest,
        selectionNotice: null
      };
    }

    case 'tick':
      return { ...state, nowMs: action.nowMs };
  }
}

export function countdownMs(snapshot: DisplaySnapshot | null, nowMs: number): number | null {
  if (!snapshot || snapshot.timer.state !== 'pending') return null;
  const { timer } = snapshot;
  if (timer.remainingMs !== null) {
    return Math.max(0, timer.remainingMs - Math.max(0, nowMs - timer.sampledAtMs));
  }
  if (timer.deadlineMs !== null && timer.sampledAtMs !== 0) {
    return Math.max(0, timer.deadlineMs - (nowMs - timer.sampledAtMs));
  }
  return null;
}

export function formatCountdown(milliseconds: number | null): string {
  if (milliseconds === null || !Number.isFinite(milliseconds)) return '—';
  const value = Math.max(0, milliseconds);
  if (value >= 60_000) {
    const minutes = Math.floor(value / 60_000);
    const seconds = Math.floor((value % 60_000) / 1_000);
    return `${minutes}:${seconds.toString().padStart(2, '0')}`;
  }
  return `${(value / 1_000).toFixed(1)} s`;
}

export function formatValue(value: boolean | number | null): string {
  return value === null ? 'unknown' : String(value);
}

function trimNumber(value: number, digits: number): string {
  return value.toFixed(digits).replace(/(\.\d*?)0+$/, '$1').replace(/\.$/, '');
}

export function formatDuration(microseconds: number): string {
  if (!Number.isFinite(microseconds) || microseconds < 0) return '—';
  if (microseconds < 1_000) return `${microseconds} μs`;
  if (microseconds < 1_000_000) return `${trimNumber(microseconds / 1_000, 2)} ms`;
  return `${trimNumber(microseconds / 1_000_000, 2)} s`;
}

export function formatAge(milliseconds: number | null): string {
  if (milliseconds === null || !Number.isFinite(milliseconds) || milliseconds < 0) return '—';
  if (milliseconds < 1_000) return `${milliseconds} ms`;
  return `${trimNumber(milliseconds / 1_000, 1)} s`;
}

export function hasPendingRestart(activeRevision: number | null, savedRevision: number | null, restartRequired = false): boolean {
  return restartRequired || (activeRevision !== null && savedRevision !== null && activeRevision !== savedRevision);
}
