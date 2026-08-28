import { describe, expect, it } from 'vitest';
import { decodeSimulation, decodeSnapshot, simulateScenario } from '../src/lib/api';
import { createSimulationDraft, forceTriggerInput, toSimulationScenario, validateSimulationDraft } from '../src/lib/simulation';
import { type DisplaySimulation, type SimulationScenario } from '../src/lib/state';

function simulationResponse(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    logic_revision: 12,
    duration_us: 417,
    status: 'succeeded',
    trigger: {
      endpoint: 'wall_switch',
      dpt: { major: 1, subtype: 1 },
      value: { kind: 'bool', value: true },
      previous: { kind: 'bool', value: false },
      changed: true,
      rising: true,
      falling: false
    },
    inputs: [
      { endpoint: 'wall_switch', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true }, valid: true, age_ms: 0 },
      { endpoint: 'enabled', dpt: { major: 1, subtype: 1 }, value: null, valid: false, age_ms: null }
    ],
    effects: [{ endpoint: 'test_light', destination: '2/4/52', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true } }],
    error: null,
    ...overrides
  };
}

function scenario(): SimulationScenario {
  return {
    expectedLogicRevision: 12,
    trigger: { endpoint: 'wall_switch', value: { kind: 'bool', value: true }, previous: null },
    inputs: [
      { endpoint: 'wall_switch', value: { kind: 'bool', value: true }, valid: true, ageMs: 0 },
      { endpoint: 'enabled', value: null, valid: false, ageMs: null }
    ]
  };
}

function snapshot() {
  return decodeSnapshot({
    revision: 1,
    connection: { state: 'connected' },
    config: { input: { address: '1/2/3', dpt: '1.001' }, output: { address: '1/2/4', dpt: '1.001' } },
    automation: {
      inputs: [{ name: 'wall_switch', dpt: '1.001', address: '1/2/3' }, { name: 'enabled', dpt: '1.001', address: '1/2/5' }],
      outputs: [],
      bindings: [],
      logic: { source: 'function handle(event, input) return nil end' }
    },
    values: { input: { observed: null }, output: { observed: null, requested: null } },
    telegrams: [],
    logs: [],
    logic: { active_logic_revision: 12, saved_logic_revision: 12, executions: [] }
  });
}

describe('simulation API and model', () => {
  it('decodes a successful result with resolved effects', () => {
    const result = decodeSimulation(simulationResponse());
    expect(result).toMatchObject<Partial<DisplaySimulation>>({ logicRevision: 12, durationUs: 417, status: 'succeeded' });
    expect(result.effects[0]).toMatchObject({ endpoint: 'test_light', destination: '2/4/52', value: true });
  });

  it('decodes contained Lua failures without effects', () => {
    const result = decodeSimulation(simulationResponse({ status: 'failed', effects: [], error: { category: 'runtime', message: 'attempt to index nil', line: 7 } }));
    expect(result).toMatchObject({ status: 'failed', effects: [], error: { category: 'runtime', line: 7 } });
  });

  it('posts the complete scenario using the backend field names', async () => {
    let request: RequestInit | undefined;
    const result = await simulateScenario(scenario(), async (_url, init) => {
      request = init;
      return new Response(JSON.stringify(simulationResponse()), { status: 200 });
    });
    expect(result.status).toBe('succeeded');
    expect(JSON.parse(String(request?.body))).toEqual({
      expected_logic_revision: '12',
      trigger: scenario().trigger,
      inputs: [
        { endpoint: 'wall_switch', value: { kind: 'bool', value: true }, valid: true, age_ms: 0 },
        { endpoint: 'enabled', value: null, valid: false, age_ms: null }
      ]
    });
  });

  it('surfaces revision conflicts and validation errors', async () => {
    await expect(simulateScenario(scenario(), async () => new Response(JSON.stringify({ error: 'stale' }), { status: 409 }))).rejects.toMatchObject({
      status: 409,
      message: 'The active logic source changed. Refresh the dashboard and re-run the simulation.'
    });
    await expect(simulateScenario(scenario(), async () => new Response(JSON.stringify({ errors: [{ path: 'inputs[1].value', message: 'required' }] }), { status: 422 }))).rejects.toMatchObject({
      status: 422,
      fieldErrors: [{ path: 'inputs[1].value', message: 'required' }]
    });
  });

  it('starts complete no-history scenarios unknown and forces the selected trigger entry', () => {
    const draft = createSimulationDraft(snapshot());
    expect(draft.inputs.map((input) => [input.value, input.valid, input.ageMs])).toEqual([[null, false, null], [null, false, null]]);
    const prepared = forceTriggerInput({ ...draft, triggerValue: true });
    expect(prepared.inputs[0]).toMatchObject({ value: true, valid: true, ageMs: 0 });
    expect(validateSimulationDraft(draft)).toContain('Choose a valid current trigger value.');
    expect(toSimulationScenario(prepared, 12)).toMatchObject({ expectedLogicRevision: 12, trigger: { endpoint: 'wall_switch', value: { kind: 'bool', value: true } } });
  });
});
