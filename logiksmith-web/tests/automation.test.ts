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
    behaviors: {
      timed_bool: { input: 'wall_switch', output: 'test_light', off_delay_ms: 5_000 },
      percentage_forward: { input: 'dimmer_level', output: 'dimmer_output' }
    }
  };
}

describe('automation editor model', () => {
  it('decodes the complete document and revision', () => {
    expect(decodeAutomation({ document: document(), revision: 7 })).toEqual({ document: document(), revision: 7 });
  });

  it('propagates endpoint renames to bindings and both behavior references', () => {
    const renamed = renameEndpoint(document(), 'wall_switch', 'hall_switch');
    expect(renamed.inputs[0].name).toBe('hall_switch');
    expect(renamed.knx_bindings[0].endpoint).toBe('hall_switch');
    expect(renamed.behaviors.timed_bool.input).toBe('hall_switch');
  });

  it('leaves references invalid when a referenced endpoint is deleted', () => {
    const errors = validateAutomation(removeEndpoint(document(), 'test_light'));
    expect(errors.some((error) => error.path === 'knx_bindings[2].endpoint')).toBe(true);
    expect(errors.some((error) => error.path === 'behaviors.timed_bool.output')).toBe(true);
  });

  it('only offers endpoints with matching direction and DPT', () => {
    expect(compatibleEndpoints(document(), 'input', '1.001').map((endpoint) => endpoint.name)).toEqual(['wall_switch']);
    expect(compatibleEndpoints(document(), 'output', '5.001').map((endpoint) => endpoint.name)).toEqual(['dimmer_output']);
  });

  it('reports stable client field paths for invalid documents', () => {
    const invalid = document();
    invalid.outputs[0].name = 'Bad Name';
    invalid.knx_bindings[0].group_address = '17/99/999';
    invalid.behaviors.timed_bool.input = 'dimmer_level';
    const paths = validateAutomation(invalid).map((error) => error.path);
    expect(paths).toContain('outputs[0].name');
    expect(paths).toContain('knx_bindings[0].group_address');
    expect(paths).toContain('behaviors.timed_bool.input');
  });

  it('saves the whole document with the revision being replaced', async () => {
    let request: RequestInit | undefined;
    const result = await saveAutomation(document(), 7, async (_url, init) => {
      request = init;
      return new Response(JSON.stringify({ revision: 8, restart_required: true }), { status: 200 });
    });
    expect(result).toEqual({ revision: 8, restartRequired: true });
    expect(request?.method).toBe('PUT');
    expect(JSON.parse(String(request?.body))).toEqual({ document: document(), revision: 7 });
  });

  it('retains the latest document from a stale-save conflict', async () => {
    await expect(saveAutomation(document(), 7, async () => new Response(JSON.stringify({ document: document(), revision: 8 }), { status: 409 }))).rejects.toMatchObject({ status: 409, latest: { revision: 8 } });
  });

  it('marks a saved revision pending until the active snapshot catches up', () => {
    expect(hasPendingRestart(3, 4)).toBe(true);
    expect(hasPendingRestart(4, 4)).toBe(false);
    expect(hasPendingRestart(null, 4, true)).toBe(true);
  });

  it('decodes endpoint-oriented active snapshots and named values', () => {
    const snapshot = decodeSnapshot({
      revision: 9,
      connection: { state: 'connected' },
      config: {
        inputs: document().inputs,
        outputs: document().outputs,
        knx_bindings: document().knx_bindings,
        behaviors: {
          timed_bool: document().behaviors.timed_bool,
          percentage_forward: document().behaviors.percentage_forward
        }
      },
      values: { endpoints: { wall_switch: { observed: true }, test_light: { observed: false, requested: true } } },
      active_automation_revision: 6,
      saved_automation_revision: 7,
      timer: { state: 'idle' },
      telegrams: [],
      logs: []
    });
    expect(snapshot.automation?.inputs[0]).toMatchObject({ name: 'wall_switch', address: '17/4/21', observed: true });
    expect(snapshot.automation?.outputs[0]).toMatchObject({ name: 'test_light', requested: true });
    expect(snapshot.activeAutomationRevision).toBe(6);
    expect(snapshot.savedAutomationRevision).toBe(7);
  });

  it('provides an empty draft with both behavior sections', () => {
    expect(emptyAutomation().behaviors).toHaveProperty('timed_bool');
    expect(emptyAutomation().behaviors).toHaveProperty('percentage_forward');
  });
});
