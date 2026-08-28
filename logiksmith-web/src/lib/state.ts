export type ConnectionState = 'starting' | 'connecting' | 'connected' | 'disconnected' | 'failed';
export type TimerState = 'idle' | 'pending';
export type StreamStatus = 'connecting' | 'connected' | 'stale' | 'error';
export type WriteStatus = 'idle' | 'pending' | 'succeeded' | 'failed';

export interface DisplayEndpoint { name?: string; address: string; dpt: string; direction?: 'input' | 'output'; observed?: boolean | number | null; requested?: boolean | number | null; }
export interface DisplayBinding { endpoint: string; groupAddress: string; }
export interface DisplayAutomation { inputs: DisplayEndpoint[]; outputs: DisplayEndpoint[]; bindings: DisplayBinding[]; source: string; }
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
export type DisplayExecutionTrigger = DisplayInputExecutionTrigger | DisplayTimerExecutionTrigger;

export interface DisplayExecutionInput { endpoint: string; dpt: string; value: boolean | number | null; valid: boolean; ageMs: number | null; }
export interface DisplayExecutionEffect { endpoint: string; destination: string; dpt: string; value: boolean | number; }
export type DisplayTimerEffectAction = 'scheduled' | 'replaced' | 'cancelled' | 'cancel_noop';
export interface DisplayTimerEffect { name: string; action: DisplayTimerEffectAction; afterMs: number | null; dueAtMs: number | null; previousDueAtMs: number | null; }
export interface DisplayTransition { state: DisplayState; effects: DisplayExecutionEffect[]; timers: DisplayTimerEffect[]; }

export interface DisplayExecution {
  blockId: string | null;
  executionId: number;
  timeMs: number;
  durationUs: number;
  logicRevision: RevisionToken | null;
  status: 'succeeded' | 'failed';
  trigger: DisplayExecutionTrigger;
  inputs: DisplayExecutionInput[];
  transition: DisplayTransition | null;
  stateBefore: DisplayState;
  stateAfter: DisplayState;
  effects: DisplayExecutionEffect[];
  timerEffects: DisplayTimerEffect[];
  error: DisplayLogicError | null;
}

export interface DisplayPendingTimer { name: string; scheduledAtMs: number; dueAtMs: number; logicRevision: RevisionToken; }
export interface DisplayBlock {
  id: string;
  activeEnabled: boolean;
  savedEnabled: boolean;
  source: string;
  inputs: DisplayEndpoint[];
  outputs: DisplayEndpoint[];
  bindings: DisplayBinding[];
  state: DisplayState;
  pendingTimers: DisplayPendingTimer[];
  executions: DisplayExecution[];
  activeLogicRevision: RevisionToken | null;
  savedLogicRevision: RevisionToken | null;
  lastResult: DisplayLastResult;
  lastError: DisplayLogicError | null;
}

export type SimulationTypedValue = { kind: 'bool'; value: boolean } | { kind: 'percent'; value: number };
export interface SimulationInputTriggerRequest { type?: 'input'; endpoint: string; value: SimulationTypedValue; previous: SimulationTypedValue | null; }
export interface SimulationTimerTriggerRequest { type: 'timer'; name?: string; timer?: string; firedAtMs: number; }
export type SimulationTriggerRequest = SimulationInputTriggerRequest | SimulationTimerTriggerRequest;
export interface SimulationInputRequest { endpoint: string; value: SimulationTypedValue | null; valid: boolean; ageMs: number | null; }
export interface SimulationScenario { blockId?: string; expectedLogicRevision: RevisionToken; trigger: SimulationTriggerRequest; inputs: SimulationInputRequest[]; state?: DisplayState; pendingTimers?: DisplayPendingTimer[]; }

export interface DisplaySimulation {
  logicRevision: RevisionToken;
  durationUs: number;
  status: 'succeeded' | 'failed';
  trigger: DisplayExecutionTrigger;
  inputs: DisplayExecutionInput[];
  transition: DisplayTransition | null;
  stateBefore: DisplayState;
  stateAfter: DisplayState;
  effects: DisplayExecutionEffect[];
  timerEffects: DisplayTimerEffect[];
  pendingTimers: DisplayPendingTimer[];
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
  activeStructuralRevision: number | null;
  savedStructuralRevision: number | null;
  activeLogicRevision: RevisionToken | null;
  savedLogicRevision: RevisionToken | null;
  restartRequired: boolean;
  capturedAtMs: number;
  /** Difference between the desktop monotonic clock and the browser clock when known. */
  clockOffsetMs?: number;
  state: DisplayState;
  pendingTimers: DisplayPendingTimer[];
  executions: DisplayExecution[];
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

export const initialDashboardState: DashboardState = { snapshot: null, revision: 0, streamStatus: 'connecting', stale: true, staleAtMs: null, error: null, needsResync: false, nowMs: 0, selectedBlockId: null, selectedExecutionId: null, selectionPinned: false, selectionNotice: null };
function staleState(state: DashboardState, error: string | null = state.error): DashboardState { return { ...state, streamStatus: 'stale', stale: true, staleAtMs: state.staleAtMs ?? state.nowMs, error }; }
function reconcileSelection(state: DashboardState, snapshot: DisplaySnapshot): Pick<DashboardState, 'selectedExecutionId' | 'selectionPinned' | 'selectionNotice'> {
  const block = snapshot.blocks.find((item) => item.id === state.selectedBlockId) ?? snapshot.blocks[0];
  const executions = block?.executions ?? snapshot.executions;
  const newest = executions[0]?.executionId ?? null;
  if (!state.selectionPinned) return { selectedExecutionId: newest, selectionPinned: false, selectionNotice: null };
  if (state.selectedExecutionId !== null && executions.some((execution) => execution.executionId === state.selectedExecutionId)) return { selectedExecutionId: state.selectedExecutionId, selectionPinned: true, selectionNotice: null };
  return { selectedExecutionId: newest, selectionPinned: false, selectionNotice: state.selectedExecutionId === null ? null : 'The selected execution expired from memory.' };
}
export function reduceDashboardState(state: DashboardState, action: DashboardAction): DashboardState {
  switch (action.type) {
    case 'snapshot_loaded': { const stale = state.snapshot !== null && state.stale; const selectedBlockId = action.snapshot.blocks.some((item) => item.id === state.selectedBlockId) ? state.selectedBlockId : action.snapshot.blocks[0]?.id ?? null; const next = { ...state, snapshot: action.snapshot, revision: action.snapshot.revision, streamStatus: 'connecting' as StreamStatus, stale, staleAtMs: stale ? state.staleAtMs : null, error: null, needsResync: false, nowMs: action.nowMs ?? state.nowMs, selectedBlockId }; return { ...next, ...reconcileSelection(next, action.snapshot) }; }
    case 'stream_open': return state.needsResync ? staleState(state) : { ...state, streamStatus: 'connected', stale: false, staleAtMs: null, error: null };
    case 'stream_lost': return staleState(state, action.error ?? null);
    case 'stream_error': return { ...staleState(state, action.error), streamStatus: 'error' };
    case 'event_received': {
      if (action.event.kind === 'resync') return { ...staleState(state), needsResync: true };
      const event = action.event;
      if (event.revision <= state.revision) return state;
      if (event.revision !== state.revision + 1) return { ...staleState(state, 'The event stream skipped a revision.'), needsResync: true };
      const selectedBlockId = event.snapshot.blocks.some((item) => item.id === state.selectedBlockId) ? state.selectedBlockId : event.snapshot.blocks[0]?.id ?? null; const next = { ...state, snapshot: event.snapshot, revision: event.revision, error: null, needsResync: false, streamStatus: 'connected' as StreamStatus, stale: false, staleAtMs: null, selectedBlockId }; return { ...next, ...reconcileSelection(next, event.snapshot) };
    }
    case 'select_execution': {
      if (!state.snapshot) return state;
      const owner = state.snapshot.blocks.find((blockValue) => blockValue.executions.some((item) => item.executionId === action.executionId));
      const execution = owner?.executions.find((item) => item.executionId === action.executionId) ?? state.snapshot.executions.find((item) => item.executionId === action.executionId);
      if (!execution) return state;
      const newest = owner?.executions[0]?.executionId ?? state.snapshot.executions[0]?.executionId; return { ...state, selectedBlockId: owner?.id ?? state.selectedBlockId, selectedExecutionId: execution.executionId, selectionPinned: execution.executionId !== newest, selectionNotice: null };
    }
    case 'select_block': { if (!state.snapshot?.blocks.some((item) => item.id === action.blockId)) return state; const selected = state.snapshot.blocks.find((item) => item.id === action.blockId); return { ...state, selectedBlockId: action.blockId, selectedExecutionId: selected?.executions[0]?.executionId ?? null, selectionPinned: false, selectionNotice: null }; }
    case 'tick': return { ...state, nowMs: action.nowMs };
  }
}

export function countdownMs(snapshot: DisplaySnapshot | null, nowMs: number, timerName?: string): number | null {
  if (!snapshot) return null;
  const pending = timerName ? snapshot.pendingTimers.find((timer) => timer.name === timerName) : snapshot.pendingTimers[0];
  if (pending) return Math.max(0, pending.dueAtMs - (nowMs - snapshot.capturedAtMs - (snapshot.clockOffsetMs ?? 0)));
  if (timerName || snapshot.pendingTimers.length > 0 || snapshot.timer.state !== 'pending') return null;
  if (snapshot.timer.remainingMs !== null) return Math.max(0, snapshot.timer.remainingMs - Math.max(0, nowMs - snapshot.timer.sampledAtMs));
  if (snapshot.timer.deadlineMs !== null && snapshot.timer.sampledAtMs !== 0) return Math.max(0, snapshot.timer.deadlineMs - (nowMs - snapshot.timer.sampledAtMs));
  return null;
}
export function displayedCountdownMs(state: DashboardState, timerName?: string): number | null {
  return countdownMs(state.snapshot, state.stale ? (state.staleAtMs ?? state.nowMs) : state.nowMs, timerName);
}
export function formatCountdown(milliseconds: number | null): string { if (milliseconds === null || !Number.isFinite(milliseconds)) return '—'; const value = Math.max(0, milliseconds); if (value >= 60_000) return `${Math.floor(value / 60_000)}:${Math.floor((value % 60_000) / 1_000).toString().padStart(2, '0')}`; return `${(value / 1_000).toFixed(1)} s`; }
export function formatValue(value: boolean | number | string | null): string { return value === null ? 'unknown' : String(value); }
export function formatStateValue(value: DisplayStateValue | null | undefined): string { if (!value) return 'unknown'; return value.kind === 'string' ? JSON.stringify(value.value) : String(value.value); }
export function formatStateEntry(value: DisplayStateValue | null | undefined): string { return value ? `${formatStateValue(value)} (${value.kind})` : 'unknown'; }
function trimNumber(value: number, digits: number): string { return value.toFixed(digits).replace(/(\.\d*?)0+$/, '$1').replace(/\.$/, ''); }
export function formatDuration(microseconds: number): string { if (!Number.isFinite(microseconds) || microseconds < 0) return '—'; if (microseconds < 1_000) return `${microseconds} μs`; if (microseconds < 1_000_000) return `${trimNumber(microseconds / 1_000, 2)} ms`; return `${trimNumber(microseconds / 1_000_000, 2)} s`; }
export function formatAge(milliseconds: number | null): string { if (milliseconds === null || !Number.isFinite(milliseconds) || milliseconds < 0) return '—'; if (milliseconds < 1_000) return `${milliseconds} ms`; return `${trimNumber(milliseconds / 1_000, 1)} s`; }
export function hasPendingRestart(activeRevision: number | null, savedRevision: number | null, restartRequired = false): boolean { return restartRequired || (activeRevision !== null && savedRevision !== null && activeRevision !== savedRevision); }
import type { RevisionToken } from './revision';
