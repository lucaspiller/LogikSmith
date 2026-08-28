import { describe, expect, it } from 'vitest';
import { decodeSnapshot } from '../src/lib/api';
import {
  compatibleEndpoints,
  decodeAutomation,
  emptyAutomation,
  removeEndpoint,
  renameEndpoint,
  saveAutomation,
  validateAutomation,
  type AutomationDocument
} from '../src/lib/automation';
import { hasPendingRestart } from '../src/lib/state';

function document(): AutomationDocument {
  return {
    inputs: [{ name: 'wall_switch', dpt: '1.001' }, { name: 'dimmer_level', dpt: '5.001' }],
    outputs: [{ name: 'test_light', dpt: '1.001' }, { name: 'dimmer_output', dpt: '5.001' }],
    knx_bindings: [
      { endpoint: 'wall_switch', group_address: '17/4/21' },
      { endpoint: 'dimmer_level', group_address: '1/2/3' },
      { endpoint: 'test_light', group_address: '13/0/1' },
      { endpoint: 'dimmer_output', group_address: '1/2/4' }
    ],
    logic: { source: 'function handle(event, input) return { outputs = { test_light = event.value } } end' }
  };
}

describe('automation editor model', () => {
  it('decodes the complete M4 document and revision', () => {
    expect(decodeAutomation({ document: document(), revision: 7, active_logic_revision: 6 })).toMatchObject({ document: document(), revision: 7, activeLogicRevision: 6 });
  });

  it('loads, edits, and preserves source alongside endpoint and binding drafts', () => {
    const renamed = renameEndpoint(document(), 'wall_switch', 'hall_switch');
    expect(renamed.logic?.source).toBe(document().logic?.source);
    expect(renamed.knx_bindings[0].endpoint).toBe('hall_switch');
    const changed = { ...renamed, logic: { source: 'function handle(event, input) return nil end' } };
    expect(changed.inputs).toEqual(renamed.inputs);
    expect(changed.knx_bindings).toEqual(renamed.knx_bindings);
  });

  it('leaves references invalid when an endpoint is deleted', () => {
    const errors = validateAutomation(removeEndpoint(document(), 'test_light'));
    expect(errors.some((error) => error.path === 'knx_bindings[2].endpoint')).toBe(true);
  });

  it('only offers endpoints with matching direction and DPT', () => {
    expect(compatibleEndpoints(document(), 'input', '1.001').map((endpoint) => endpoint.name)).toEqual(['wall_switch']);
    expect(compatibleEndpoints(document(), 'output', '5.001').map((endpoint) => endpoint.name)).toEqual(['dimmer_output']);
  });

  it('reports structural and source validation paths', () => {
    const invalid = document();
    invalid.outputs[0].name = 'Bad Name';
    invalid.knx_bindings[0].group_address = '17/99/999';
    invalid.logic = { source: '' };
    const paths = validateAutomation(invalid).map((error) => error.path);
    expect(paths).toContain('outputs[0].name');
    expect(paths).toContain('knx_bindings[0].group_address');
    expect(paths).toContain('logic.source');
  });

  it('saves the complete document and reports source activation', async () => {
    let request: RequestInit | undefined;
    const result = await saveAutomation(document(), 7, async (_url, init) => {
      request = init;
      return new Response(JSON.stringify({ revision: 8, logic_activated: true, active_logic_revision: 8, restart_required: false }), { status: 200 });
    });
    expect(result).toEqual({ revision: 8, logicActivated: true, activeLogicRevision: 8, restartRequired: false });
    expect(request?.method).toBe('PUT');
    expect(JSON.parse(String(request?.body))).toEqual({ document: document(), revision: 7 });
  });

  it('retains the latest document from a stale-save conflict', async () => {
    await expect(saveAutomation(document(), 7, async () => new Response(JSON.stringify({ document: document(), revision: 8 }), { status: 409 }))).rejects.toMatchObject({ status: 409, latest: { revision: 8 } });
  });

  it('marks a structural revision pending while allowing source-only activation', () => {
    expect(hasPendingRestart(3, 4)).toBe(true);
    expect(hasPendingRestart(4, 4)).toBe(false);
    expect(hasPendingRestart(null, 4, true)).toBe(true);
  });

  it('provides an empty source draft', () => {
    expect(emptyAutomation().logic).toEqual({ source: '' });
  });

  it('decodes execution status, errors, and resolved effects', () => {
    const snapshot = decodeSnapshot({
      revision: 9,
      connection: { state: 'connected' },
      config: { active: { inputs: document().inputs, outputs: document().outputs, knx_bindings: document().knx_bindings, logic: document().logic } },
      values: { endpoints: [{ name: 'wall_switch', observed: { kind: 'bool', value: true } }, { name: 'test_light', observed: { kind: 'bool', value: false }, requested: { kind: 'bool', value: true } }] },
      logic: {
        active_structural_revision: 3,
        saved_structural_revision: 3,
        active_logic_revision: 8,
        saved_logic_revision: 8,
        restart_required: false,
        last_execution: { status: 'failed', trigger: { endpoint: 'wall_switch', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true } }, logic_revision: 8, effect_count: 0, error: { category: 'runtime', line: 4, message: 'attempt to index nil' } },
        recent_effects: [{ endpoint: 'test_light', destination: '13/0/1', dpt: '1.001', value: { kind: 'bool', value: true } }]
      },
      timer: { state: 'idle' }, telegrams: [], logs: []
    });
    expect(snapshot.logicExecution.error).toMatchObject({ category: 'runtime', line: 4 });
    expect(snapshot.logicEffects[0]).toMatchObject({ endpoint: 'test_light', address: '13/0/1', value: true });
    expect(snapshot.restartRequired).toBe(false);
  });
});
