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
  behaviors: {
    timedBool: { input: string; output: string; offDelayMs: number };
    percentageForward: { input: string; output: string };
  };
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
  | { type: 'tick'; nowMs: number };

export const initialDashboardState: DashboardState = {
  snapshot: null,
  revision: 0,
  streamStatus: 'connecting',
  stale: true,
  error: null,
  needsResync: false,
  nowMs: 0
};

function staleState(state: DashboardState, error: string | null = state.error): DashboardState {
  return { ...state, streamStatus: 'stale', stale: true, error };
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
        nowMs: action.nowMs ?? state.nowMs
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
        stale: false
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

export function hasPendingRestart(activeRevision: number | null, savedRevision: number | null, restartRequired = false): boolean {
  return restartRequired || (activeRevision !== null && savedRevision !== null && activeRevision !== savedRevision);
}
