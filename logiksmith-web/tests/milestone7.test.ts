import { describe, expect, it } from 'vitest';
import { decodeSimulation, decodeSnapshot } from '../src/lib/api';
import { applySimulationResult, createSimulationDraft, toSimulationScenario } from '../src/lib/simulation';
import { countdownMs, formatStateEntry } from '../src/lib/state';

const state = {
  active: { kind: 'bool', value: true },
  count: { kind: 'integer', value: 2 },
  ratio: { kind: 'number', value: 1.5 },
  phase: { kind: 'string', value: 'waiting_to_turn_off' }
};

function snapshot() {
  return {
    revision: 3,
    captured_at_ms: 1000,
    connection: { state: 'connected' },
    config: { active: { inputs: [{ name: 'wall_switch', dpt: { major: 1, subtype: 1 } }], outputs: [{ name: 'staircase_light', dpt: { major: 1, subtype: 1 } }], knx_bindings: [{ endpoint: 'wall_switch', group_address: '1/2/3' }, { endpoint: 'staircase_light', group_address: '1/2/4' }], logic: { source: 'function handle(event, input, meta, state) return nil end' } } },
    values: { endpoints: [] },
    write: { status: 'idle', request_id: null, value: null, error: null },
    state,
    pending_timers: [{ name: 'off', scheduled_at_ms: 800, due_at_ms: 5800, logic_revision: 12 }],
    logic: {
      active_logic_revision: 12, saved_logic_revision: 12, active_structural_revision: 1, saved_structural_revision: 1, restart_required: false, state,
      pending_timers: [{ name: 'off', scheduled_at_ms: 800, due_at_ms: 5800, logic_revision: 12 }],
      executions: [{
        execution_id: 7, time_ms: 1000, duration_us: 20, logic_revision: 12, status: 'succeeded',
        trigger: { type: 'timer', endpoint: '', dpt: { major: 0, subtype: 0 }, value: { kind: 'bool', value: false }, previous: null, changed: false, rising: false, falling: false, name: 'dim', scheduled_at_ms: 500, due_at_ms: 900, fired_at_ms: 1000, late_by_ms: 100, scheduled_logic_revision: 12 },
        inputs: [], state_before: { phase: { kind: 'string', value: 'waiting_to_turn_off' } }, state_after: state,
        transition: { state: { phase: { kind: 'string', value: 'dimmed' } }, effects: [], timers: [{ name: 'off', action: 'replaced', previous_due_at_ms: 9000, after_ms: 5000, due_at_ms: 6000 }] },
        effects: [], timer_effects: [], error: null
      }]
    },
    telegrams: [], logs: []
  };
}

describe('Milestone 7 browser contract', () => {
  it('decodes tagged state, pending timer projection, and timer execution facts', () => {
    const mapped = decodeSnapshot(snapshot(), 0);
    expect(mapped.state).toEqual(state);
    expect(mapped.pendingTimers[0]).toMatchObject({ name: 'off', dueAtMs: 5800, logicRevision: 12 });
    expect(mapped.executions[0].trigger).toMatchObject({ type: 'timer', name: 'dim', timer: 'dim', lateByMs: 100 });
    expect(mapped.executions[0].transition?.state.phase).toEqual({ kind: 'string', value: 'dimmed' });
    expect(mapped.executions[0].timerEffects[0]).toMatchObject({ action: 'replaced', previousDueAtMs: 9000 });
    expect(formatStateEntry(mapped.state.count)).toBe('2 (integer)');
  });

  it('moves a pending timer countdown locally and preserves a timer simulation selection', () => {
    const mapped = decodeSnapshot(snapshot(), 0);
    expect(countdownMs(mapped, 1800, 'off')).toBe(5000);
    const browserClock = decodeSnapshot(snapshot(), 1_000_000_000);
    expect(browserClock.clockOffsetMs).toBe(999_999_000);
    expect(countdownMs(browserClock, 1_000_000_500, 'off')).toBe(5_300);
    const draft = createSimulationDraft(mapped);
    expect(draft.pendingTimers).toHaveLength(1);
    draft.triggerType = 'timer'; draft.triggerTimerName = 'off'; draft.timerFiredAtMs = 5900;
    const scenario = toSimulationScenario(draft, 12);
    expect(scenario?.trigger).toEqual({ type: 'timer', name: 'off', firedAtMs: 5900 });
    expect(scenario?.pendingTimers?.[0]).toMatchObject({ name: 'off', dueAtMs: 5800 });
  });

  it('decodes a complete simulation transition without implying a live write', () => {
    const result = decodeSimulation({
      logic_revision: 12, duration_us: 30, status: 'succeeded',
      trigger: { type: 'timer', name: 'off', endpoint: '', dpt: { major: 0, subtype: 0 }, value: { kind: 'bool', value: false }, previous: null, changed: false, rising: false, falling: false, scheduled_at_ms: 800, due_at_ms: 5800, fired_at_ms: 5900, late_by_ms: 100, scheduled_logic_revision: 12 },
      inputs: [], state_before: state, state_after: { phase: { kind: 'string', value: 'idle' } },
      transition: { state: { phase: { kind: 'string', value: 'idle' } }, effects: [{ endpoint: 'staircase_light', destination: '1/2/4', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: false } }], timers: [{ name: 'off', action: 'cancelled', previous_due_at_ms: 5800 }] },
      pending_timers: [], effects: [], timer_effects: [], error: null
    });
    expect(result.transition?.effects[0].value).toBe(false);
    expect(result.timerEffects[0].action).toBe('cancelled');
    expect(result.pendingTimers).toEqual([]);
  });

  it('carries returned simulated timers into the next simulation', () => {
    const draft = createSimulationDraft(decodeSnapshot(snapshot(), 0));
    const result = decodeSimulation({
      logic_revision: 12, duration_us: 30, status: 'succeeded',
      trigger: { type: 'input', endpoint: 'wall_switch', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true }, previous: { kind: 'bool', value: false }, changed: true, rising: true, falling: false },
      inputs: [{ endpoint: 'wall_switch', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true }, valid: true, age_ms: 0 }],
      state_before: state, state_after: { phase: { kind: 'string', value: 'waiting_to_turn_off' } }, transition: { state: {}, effects: [], timers: [{ name: 'off', action: 'scheduled', after_ms: 5000, due_at_ms: 6000 }] },
      pending_timers: [{ name: 'off', scheduled_at_ms: 1000, due_at_ms: 6000, logic_revision: 12 }], effects: [], timer_effects: [], error: null
    });
    const next = applySimulationResult(draft, result);
    expect(next.state).toEqual({ phase: { kind: 'string', value: 'waiting_to_turn_off' } });
    expect(next.pendingTimers).toEqual([{ name: 'off', scheduledAtMs: 1000, dueAtMs: 6000, logicRevision: 12 }]);
    next.triggerType = 'timer';
    expect(toSimulationScenario(next, 12)?.trigger).toEqual({ type: 'timer', name: 'off', firedAtMs: 6000 });
  });
});
