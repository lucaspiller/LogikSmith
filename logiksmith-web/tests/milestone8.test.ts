import { describe, expect, it } from 'vitest';
import { decodeAutomationDocument, AutomationDecodeError, MAX_BLOCKS } from '../src/lib/automation';
import { decodeSnapshot, simulateScenario } from '../src/lib/api';
import { initialDashboardState, reduceDashboardState } from '../src/lib/state';

function block(id: string, executionId?: number) {
  return {
    id,
    active_enabled: id !== 'utility_light',
    saved_enabled: true,
    active_logic_revision: 10,
    saved_logic_revision: id === 'staircase' ? 10 : 9,
    source: 'function handle(event, input, meta, state) return nil end',
    inputs: [{ name: 'button', dpt: { major: 1, subtype: 1 }, address: '1/2/3', observed: { kind: 'bool', value: true } }],
    outputs: [{ name: 'light', dpt: { major: 1, subtype: 1 }, address: '1/2/4', observed: { kind: 'bool', value: false }, requested: null }],
    knx_bindings: [{ endpoint: 'button', group_address: '1/2/3' }, { endpoint: 'light', group_address: '1/2/4' }],
    state: { phase: { kind: 'string', value: 'idle' } },
    pending_timers: [{ name: 'off', scheduled_at_ms: 100, due_at_ms: 5_100, logic_revision: 10 }],
    executions: executionId === undefined ? [] : [{
      execution_id: executionId, time_ms: executionId * 100, duration_us: 12, logic_revision: 10, status: 'succeeded',
      trigger: { type: 'input', endpoint: 'button', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true }, previous: null, changed: true, rising: true, falling: false },
      inputs: [{ endpoint: 'button', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true }, valid: true, age_ms: 0 }],
      state_before: {}, state_after: {}, effects: [], timer_effects: [], error: null
    }],
    last_result: executionId === undefined ? null : { status: 'succeeded', execution_id: executionId, time_ms: executionId * 100, error: null }
  };
}

function snapshot(blocks: Array<Record<string, unknown>> = [block('staircase', 3), block('utility_light', 2)]) {
  return { revision: 4, captured_at_ms: 1_000, connection: { state: 'connected' }, blocks, telegrams: [], logs: [], active_structural_revision: 8, saved_structural_revision: 8, restart_required: false };
}

describe('Milestone 8 block dashboard contract', () => {
  it('decodes ordered block documents and rejects duplicate IDs or a malformed nested block', () => {
    const document = decodeAutomationDocument({ blocks: [{ id: 'staircase', enabled: true, source: 'return nil', inputs: [{ name: 'button', dpt: '1.001' }], outputs: [], knx_bindings: [{ endpoint: 'button', group_address: '1/2/3' }] }] });
    expect(document.blocks?.map((item) => item.id)).toEqual(['staircase']);
    expect(() => decodeAutomationDocument({ blocks: [{ ...document.blocks?.[0] }, { ...document.blocks?.[0] }] })).toThrow(/duplicate block id/);
    expect(() => decodeAutomationDocument({ blocks: [{ ...document.blocks?.[0], inputs: 'not an array' }] })).toThrow(/blocks\[0\]\.inputs/);
    expect(() => decodeAutomationDocument({ blocks: Array.from({ length: MAX_BLOCKS + 1 }, (_, index) => ({ ...document.blocks?.[0], id: `block_${index}` })) })).toThrow(/at most 64/);
  });

  it('decodes private state, timers, source revisions and repeated local endpoint names', () => {
    const mapped = decodeSnapshot(snapshot(), 1_000);
    expect(mapped.blocks.map((item) => item.id)).toEqual(['staircase', 'utility_light']);
    expect(mapped.blocks[0].inputs[0].name).toBe(mapped.blocks[1].inputs[0].name);
    expect(mapped.blocks[0].state.phase).toEqual({ kind: 'string', value: 'idle' });
    expect(mapped.blocks[0].pendingTimers[0]).toMatchObject({ name: 'off', dueAtMs: 5_100 });
    expect(mapped.blocks[1].activeEnabled).toBe(false);
    expect(mapped.blocks[0].lastResult.status).toBe('succeeded');
    expect(mapped.blocks[0].executions[0].blockId).toBe('staircase');
  });

  it('keeps the selected block and pinned execution when another block updates', () => {
    const first = decodeSnapshot(snapshot(), 1_000);
    const second = decodeSnapshot(snapshot([block('staircase', 5), block('utility_light', 6)]), 1_000);
    let state = reduceDashboardState(initialDashboardState, { type: 'snapshot_loaded', snapshot: first, nowMs: 1_000 });
    state = reduceDashboardState(state, { type: 'select_block', blockId: 'staircase' });
    state = reduceDashboardState(state, { type: 'select_execution', executionId: 3 });
    expect(state.selectionPinned).toBe(false);
    state = reduceDashboardState(state, { type: 'event_received', event: { kind: 'update', revision: 5, snapshot: second } });
    expect(state.selectedBlockId).toBe('staircase');
    expect(state.selectedExecutionId).toBe(5);
    expect(state.snapshot?.blocks.find((item) => item.id === 'utility_light')?.executions[0].executionId).toBe(6);
  });

  it('includes the selected block in simulation requests and preserves scoped errors', async () => {
    const response = { logic_revision: 10, duration_us: 1, status: 'failed', trigger: { type: 'timer', name: 'off', scheduled_at_ms: 0, due_at_ms: 1, fired_at_ms: 1, late_by_ms: 0, scheduled_logic_revision: 10 }, inputs: [], state_before: {}, state_after: {}, effects: [], timer_effects: [], pending_timers: [], error: { category: 'instruction_limit', message: 'bounded', line: null } };
    let request: RequestInit | undefined;
    await simulateScenario({ blockId: 'utility_light', expectedLogicRevision: 10, trigger: { type: 'timer', name: 'off', firedAtMs: 1 }, inputs: [] }, async (_url, init) => { request = init; return new Response(JSON.stringify(response), { status: 200 }); });
    expect(JSON.parse(String(request?.body)).block_id).toBe('utility_light');
    await expect(simulateScenario({ blockId: 'missing', expectedLogicRevision: 10, trigger: { type: 'timer', name: 'off', firedAtMs: 1 }, inputs: [] }, async () => new Response(JSON.stringify({ error: 'unknown logic block' }), { status: 404 }))).rejects.toMatchObject({ status: 404, message: 'The selected logic block no longer exists. Refresh the dashboard.' });
  });

  it('round-trips a 64-bit logic revision without converting it to a JS number', async () => {
    const token = '18446744073709551615';
    const source = block('staircase', 3) as Record<string, unknown>;
    const raw = snapshot([{ ...source, active_logic_revision: token, saved_logic_revision: token, pending_timers: [{ name: 'off', scheduled_at_ms: 100, due_at_ms: 5_100, logic_revision: token }], executions: [{ ...(source.executions as Array<Record<string, unknown>>)[0], logic_revision: token, trigger: { type: 'input', endpoint: 'button', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true }, previous: null, changed: true, rising: true, falling: false } }], last_result: { status: 'succeeded', execution_id: 3, time_ms: 300, error: null } }]);
    const decoded = decodeSnapshot(raw, 1_000);
    expect(decoded.blocks[0].activeLogicRevision).toBe(token);
    expect(decoded.blocks[0].pendingTimers[0].logicRevision).toBe(token);
    let request: RequestInit | undefined;
    const response = { logic_revision: token, duration_us: 1, status: 'failed', trigger: { type: 'timer', name: 'off', scheduled_at_ms: 0, due_at_ms: 1, fired_at_ms: 1, late_by_ms: 0, scheduled_logic_revision: token }, inputs: [], state_before: {}, state_after: {}, effects: [], timer_effects: [], pending_timers: [], error: { category: 'instruction_limit', message: 'bounded', line: null } };
    await simulateScenario({ blockId: 'staircase', expectedLogicRevision: token, trigger: { type: 'timer', name: 'off', firedAtMs: 1 }, inputs: [], pendingTimers: [{ name: 'off', scheduledAtMs: 0, dueAtMs: 1, logicRevision: token }] }, async (_url, init) => { request = init; return new Response(JSON.stringify(response), { status: 200 }); });
    expect(JSON.parse(String(request?.body)).expected_logic_revision).toBe(token);
    expect(JSON.parse(String(request?.body)).pending_timers[0].logic_revision).toBe(token);
    await expect(simulateScenario({ blockId: 'staircase', expectedLogicRevision: token, trigger: { type: 'timer', name: 'off', firedAtMs: 1 }, inputs: [] }, async () => new Response(JSON.stringify({ error: 'stale', current_logic_revision: token }), { status: 409 }))).rejects.toMatchObject({ status: 409, currentLogicRevision: token });
  });
});
