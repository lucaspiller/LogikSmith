import { describe, expect, it } from 'vitest';
import { DashboardClient, decodeEvent, decodeSnapshot, loadSnapshot } from '../src/lib/api';
import {
  countdownMs,
  displayedCountdownMs,
  formatAge,
  formatCountdown,
  formatDuration,
  initialDashboardState,
  reduceDashboardState
} from '../src/lib/state';

function snapshot(revision = 1, overrides: Record<string, unknown> = {}) {
  return {
    revision,
    connection: { state: 'connected' },
    config: {
      input: { address: '1/2/3', dpt: '1.001' },
      output: { address: '1/2/4', dpt: '1.001' },
      off_delay_ms: 5_000
    },
    values: {
      input: { observed: true },
      output: { observed: false, requested: true }
    },
    timer: { state: 'pending', remaining_ms: 4_000 },
    telegrams: [{ time_ms: 12, source: '1.1.1', destination: '1/2/3', service: 'group_value_write', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true } }],
    logs: [{ time_ms: 13, level: 'info', target: 'logiksmith', message: 'ready', fields: { phase: 'startup' } }],
    ...overrides
  };
}

function execution(executionId: number, overrides: Record<string, unknown> = {}) {
  return {
    execution_id: executionId,
    time_ms: executionId * 100,
    duration_us: 417,
    logic_revision: 12,
    status: 'succeeded',
    trigger: {
      endpoint: 'wall_switch',
      dpt: { major: 1, subtype: 1 },
      value: { kind: 'bool', value: true },
      previous: null,
      changed: false,
      rising: false,
      falling: false
    },
    inputs: [
      { endpoint: 'wall_switch', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true }, valid: true, age_ms: 0 },
      { endpoint: 'enabled', dpt: { major: 1, subtype: 1 }, value: null, valid: false, age_ms: null }
    ],
    effects: [],
    error: null,
    ...overrides
  };
}

describe('dashboard API and state', () => {
  it('loads and maps a complete snapshot, retaining observed/requested distinction', () => {
    const mapped = decodeSnapshot(snapshot(), 100);
    expect(mapped.revision).toBe(1);
    expect(mapped.values.output.observed).toBe(false);
    expect(mapped.values.output.requested).toBe(true);
    expect(mapped.config.offDelayMs).toBe(5_000);
    expect(mapped.telegrams[0].time).toBe('12 ms');
    expect(mapped.logs[0].fields.phase).toBe('startup');
  });

  it('loads a snapshot through the HTTP client', async () => {
    const fetchImpl: typeof fetch = async () => new Response(JSON.stringify(snapshot()), {
      status: 200,
      headers: { 'content-type': 'application/json' }
    });
    await expect(loadSnapshot(fetchImpl)).resolves.toMatchObject({ revision: 1 });
  });

  it('fails visibly for malformed required data', () => {
    expect(() => decodeSnapshot({ revision: 1 })).toThrow(/config/);
  });

  it('applies ordered updates and ignores duplicate events', () => {
    const first = decodeSnapshot(snapshot(1), 100);
    const second = decodeSnapshot(snapshot(2, { values: { input: { observed: false }, output: { observed: false, requested: false } } }), 100);
    let state = reduceDashboardState(initialDashboardState, { type: 'snapshot_loaded', snapshot: first, nowMs: 100 });
    expect(state.stale).toBe(false);
    state = reduceDashboardState(state, { type: 'stream_open' });
    state = reduceDashboardState(state, { type: 'event_received', event: { kind: 'update', revision: 2, snapshot: second } });
    expect(state.revision).toBe(2);
    expect(state.snapshot?.values.input.observed).toBe(false);
    const duplicate = reduceDashboardState(state, { type: 'event_received', event: { kind: 'update', revision: 2, snapshot: first } });
    expect(duplicate).toEqual(state);
    expect(reduceDashboardState(state, { type: 'stream_lost' }).stale).toBe(true);
  });

  it('marks a resynchronisation and a revision gap stale', () => {
    const first = decodeSnapshot(snapshot(4), 100);
    let state = reduceDashboardState(initialDashboardState, { type: 'snapshot_loaded', snapshot: first, nowMs: 100 });
    state = reduceDashboardState(state, { type: 'stream_open' });
    state = reduceDashboardState(state, { type: 'event_received', event: { kind: 'resync', revision: 8 } });
    expect(state.needsResync).toBe(true);
    expect(state.stale).toBe(true);
    expect(reduceDashboardState(state, { type: 'event_received', event: { kind: 'update', revision: 6, snapshot: decodeSnapshot(snapshot(6), 100) } }).needsResync).toBe(true);
  });

  it('counts down from the server sample and formats the display', () => {
    const mapped = decodeSnapshot(snapshot(), 1_000);
    expect(countdownMs(mapped, 2_250)).toBe(2_750);
    expect(formatCountdown(2_750)).toBe('2.8 s');
    expect(formatCountdown(65_000)).toBe('1:05');
    expect(formatDuration(417)).toBe('417 μs');
    expect(formatDuration(1_250)).toBe('1.25 ms');
    expect(formatAge(null)).toBe('—');
    expect(formatAge(2_400)).toBe('2.4 s');
  });

  it('freezes a named timer countdown at the moment the stream goes stale', () => {
    const mapped = decodeSnapshot(snapshot(1, {
      captured_at_ms: 1_000,
      pending_timers: [{ name: 'off', scheduled_at_ms: 800, due_at_ms: 5_800, logic_revision: 12 }]
    }), 1_000);
    let dashboard = reduceDashboardState(initialDashboardState, { type: 'snapshot_loaded', snapshot: mapped, nowMs: 1_000 });
    dashboard = reduceDashboardState(dashboard, { type: 'stream_open' });
    dashboard = reduceDashboardState(dashboard, { type: 'tick', nowMs: 2_000 });
    expect(displayedCountdownMs(dashboard, 'off')).toBe(4_800);
    dashboard = reduceDashboardState(dashboard, { type: 'stream_lost' });
    expect(dashboard.staleAtMs).toBe(2_000);
    dashboard = reduceDashboardState(dashboard, { type: 'tick', nowMs: 9_000 });
    expect(displayedCountdownMs(dashboard, 'off')).toBe(4_800);
  });

  it('decodes successful, zero-effect, and unknown-input execution records', () => {
    const mapped = decodeSnapshot(snapshot(1, { logic: { executions: [execution(5, { effects: [{ endpoint: 'test_light', destination: '2/4/52', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true } }] }), execution(3)] } }));
    expect(mapped.executions).toHaveLength(2);
    expect(mapped.executions[1]).toMatchObject({ executionId: 3, status: 'succeeded', effects: [], trigger: { previous: null, rising: false } });
    expect(mapped.executions[1].inputs[1]).toMatchObject({ value: null, valid: false, ageMs: null });
    expect(mapped.executions[0].effects[0]).toMatchObject({ endpoint: 'test_light', destination: '2/4/52', value: true });
  });

  it('decodes failed executions with effects and bounded error details', () => {
    const mapped = decodeSnapshot(snapshot(1, {
      logic: {
        executions: [execution(4, {
          status: 'failed',
          effects: [],
          error: { category: 'runtime', message: 'attempt to index nil', line: 7 }
        })]
      }
    }));
    expect(mapped.executions[0].effects).toHaveLength(0);
    expect(mapped.executions[0].error).toMatchObject({ category: 'runtime', line: 7 });
  });

  it('follows the newest execution until an older selection is pinned, then reports eviction', () => {
    const first = decodeSnapshot(snapshot(1, { logic: { executions: [execution(3), execution(2)] } }));
    let state = reduceDashboardState(initialDashboardState, { type: 'snapshot_loaded', snapshot: first });
    expect(state.selectedExecutionId).toBe(3);
    state = reduceDashboardState(state, { type: 'select_execution', executionId: 2 });
    expect(state.selectionPinned).toBe(true);
    state = reduceDashboardState(state, { type: 'event_received', event: { kind: 'update', revision: 2, snapshot: decodeSnapshot(snapshot(2, { logic: { executions: [execution(4), execution(3), execution(2)] } })) } });
    expect(state.selectedExecutionId).toBe(2);
    state = reduceDashboardState(state, { type: 'event_received', event: { kind: 'update', revision: 3, snapshot: decodeSnapshot(snapshot(3, { logic: { executions: [execution(5), execution(4), execution(3)] } })) } });
    expect(state.selectedExecutionId).toBe(5);
    expect(state.selectionPinned).toBe(false);
    expect(state.selectionNotice).toMatch(/expired/);
  });

  it('decodes the preferred update and resync SSE payloads', () => {
    const update = decodeEvent(JSON.stringify({ revision: 2, snapshot: snapshot(2) }), 'update', '2');
    expect(update.kind).toBe('update');
    expect(decodeEvent('{"revision":3}', 'resync', '3')).toEqual({ kind: 'resync', revision: 3 });
  });

  it('reconnects from the latest revision without reloading a healthy snapshot', async () => {
    class FakeEventSource {
      static instances: FakeEventSource[] = [];
      readonly url: string;
      onopen: (() => void) | null = null;
      onerror: (() => void) | null = null;
      private readonly listeners = new Map<string, (event: MessageEvent<string>) => void>();

      constructor(url: string) {
        this.url = url;
        FakeEventSource.instances.push(this);
      }

      addEventListener(type: string, listener: (event: MessageEvent<string>) => void): void {
        this.listeners.set(type, listener);
      }

      close(): void {}

      emit(type: string, data: string, lastEventId = ''): void {
        this.listeners.get(type)?.({ data, lastEventId } as MessageEvent<string>);
      }
    }

    let fetchCount = 0;
    const fetchImpl: typeof fetch = async () => {
      fetchCount += 1;
      return new Response(JSON.stringify(snapshot(10)), { status: 200 });
    };
    const client = new DashboardClient({
      fetchImpl,
      eventSource: FakeEventSource,
      reconnectDelayMs: 1,
      handlers: { onSnapshot: () => {}, onEvent: () => {}, onStreamOpen: () => {}, onStreamLost: () => {}, onError: () => {} }
    });

    await client.start();
    FakeEventSource.instances[0].emit('update', JSON.stringify({ revision: 11, snapshot: snapshot(11) }), '11');
    FakeEventSource.instances[0].onerror?.();
    await new Promise((resolve) => setTimeout(resolve, 5));
    expect(FakeEventSource.instances[1].url).toBe('/api/events?since=11');
    expect(fetchCount).toBe(1);
    client.stop();
  });
});
