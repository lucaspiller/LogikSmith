import type { DashboardAction, DashboardEvent, DashboardState, DisplaySnapshot, StreamStatus } from './dashboard-types';
import type { RevisionToken } from './revision';

export const initialDashboardState: DashboardState = { snapshot: null, revision: 0, streamStatus: 'connecting', stale: true, staleAtMs: null, error: null, needsResync: false, nowMs: 0, selectedBlockId: null, selectedExecutionId: null, selectionPinned: false, selectionNotice: null };
function compareRevision(left: RevisionToken, right: RevisionToken): number {
  const a = BigInt(String(left)); const b = BigInt(String(right));
  return a < b ? -1 : a > b ? 1 : 0;
}
function staleState(state: DashboardState, error: string | null = state.error): DashboardState { return { ...state, streamStatus: 'stale', stale: true, staleAtMs: state.staleAtMs ?? state.nowMs, error }; }
function executionsForBlock(snapshot: DisplaySnapshot, blockId: string | null | undefined) {
  const local = snapshot.blocks.find((item) => item.id === blockId)?.executions ?? [];
  return local.length ? local : blockId ? snapshot.executions.filter((execution) => execution.blockId === blockId) : snapshot.executions;
}
function reconcileSelection(state: DashboardState, snapshot: DisplaySnapshot): Pick<DashboardState, 'selectedExecutionId' | 'selectionPinned' | 'selectionNotice'> {
  const block = snapshot.blocks.find((item) => item.id === state.selectedBlockId) ?? snapshot.blocks[0];
  const executions = executionsForBlock(snapshot, block?.id);
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
      if (compareRevision(event.revision, state.revision) <= 0) return state;
      // Polling returns the latest cursor and deliberately coalesces revisions;
      // an SSE event, when explicitly enabled for legacy desktop tests, still
      // requires contiguous delivery.
      if (event.source !== 'poll' && (typeof event.revision !== 'number' || typeof state.revision !== 'number' || event.revision !== state.revision + 1)) return { ...staleState(state, 'The event stream skipped a revision.'), needsResync: true };
      const selectedBlockId = event.snapshot.blocks.some((item) => item.id === state.selectedBlockId) ? state.selectedBlockId : event.snapshot.blocks[0]?.id ?? null; const next = { ...state, snapshot: event.snapshot, revision: event.revision, error: null, needsResync: false, streamStatus: 'connected' as StreamStatus, stale: false, staleAtMs: null, selectedBlockId }; return { ...next, ...reconcileSelection(next, event.snapshot) };
    }
    case 'select_execution': {
      if (!state.snapshot) return state;
      const execution = state.snapshot.executions.find((item) => item.executionId === action.executionId)
        ?? state.snapshot.blocks.flatMap((blockValue) => blockValue.executions).find((item) => item.executionId === action.executionId);
      if (!execution) return state;
      const owner = state.snapshot.blocks.find((blockValue) => blockValue.executions.some((item) => item.executionId === action.executionId))
        ?? (execution.blockId ? state.snapshot.blocks.find((blockValue) => blockValue.id === execution.blockId) : undefined);
      const newest = executionsForBlock(state.snapshot, owner?.id ?? state.selectedBlockId)[0]?.executionId; return { ...state, selectedBlockId: owner?.id ?? state.selectedBlockId, selectedExecutionId: execution.executionId, selectionPinned: execution.executionId !== newest, selectionNotice: null };
    }
    case 'select_block': { if (!state.snapshot?.blocks.some((item) => item.id === action.blockId)) return state; const newest = executionsForBlock(state.snapshot, action.blockId)[0]?.executionId ?? null; return { ...state, selectedBlockId: action.blockId, selectedExecutionId: newest, selectionPinned: false, selectionNotice: null }; }
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
/** Countdown to a schedule's next occurrence, measured from the one captured server sample. */
export function scheduleCountdownMs(snapshot: DisplaySnapshot | null, blockId: string, scheduleName: string, nowMs: number): number | null {
  if (!snapshot) return null;
  const schedule = snapshot.blocks.find((block) => block.id === blockId)?.schedules.find((item) => item.name === scheduleName);
  if (!schedule || schedule.relativeMs === null) return null;
  return Math.max(0, schedule.relativeMs - (nowMs - snapshot.capturedAtMs - (snapshot.clockOffsetMs ?? 0)));
}
export function displayedScheduleCountdownMs(state: DashboardState, blockId: string, scheduleName: string): number | null {
  return scheduleCountdownMs(state.snapshot, blockId, scheduleName, state.stale ? (state.staleAtMs ?? state.nowMs) : state.nowMs);
}
