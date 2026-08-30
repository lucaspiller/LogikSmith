/// <reference types="node" />
import { spawn, type ChildProcess } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { afterAll, beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { decodeSnapshot } from '../src/lib/api';
import { activateBlockSource, setBlockEnabled, simulateBlockDraft, validateBlockSource } from '../src/lib/block-api';
import { createBlockDraft, reconcileBlockDraft, sourceFingerprint, updateDraftSource } from '../src/lib/block-workbench';
import { displayedCountdownMs, initialDashboardState, reduceDashboardState } from '../src/lib/dashboard-state';

type FixtureResponse = { status: number; value: Record<string, any> };

let fixture: ChildProcess | undefined;
let fixturePort = 0;
const fixturePath = fileURLToPath(new URL('./fixture-server.mjs', import.meta.url));

function startFixture(): Promise<number> {
  return new Promise((resolve, reject) => {
    fixture = spawn(process.execPath, [fixturePath, '0'], { cwd: fileURLToPath(new URL('..', import.meta.url)) });
    let output = '';
    const onOutput = (chunk: Buffer | string): void => {
      output += String(chunk);
      const match = output.match(/fixture listening on http:\/\/127\.0\.0\.1:(\d+)/);
      if (match) {
        fixturePort = Number(match[1]);
        resolve(fixturePort);
      }
    };
    fixture.stdout?.on('data', onOutput);
    fixture.stderr?.on('data', onOutput);
    fixture.once('error', reject);
    fixture.once('exit', (code) => reject(new Error(`fixture exited before startup (${code}): ${output}`)));
  });
}

async function call(path: string, body?: unknown, method = 'POST'): Promise<FixtureResponse> {
  const response = await fetch(`http://127.0.0.1:${fixturePort}${path}`, {
    method,
    headers: body === undefined ? { accept: 'application/json' } : { accept: 'application/json', 'content-type': 'application/json' },
    ...(body === undefined ? {} : { body: JSON.stringify(body) })
  });
  return { status: response.status, value: await response.json() as Record<string, any> };
}

const fixtureFetch: typeof fetch = (input, init) => fetch(`http://127.0.0.1:${fixturePort}${String(input)}`, init);

async function snapshot(): Promise<Record<string, any>> {
  return (await call('/api/snapshot', undefined, 'GET')).value;
}

async function resetFixture(): Promise<void> {
  const response = await call('/api/test/reset');
  expect(response.status).toBe(200);
}

function expectDecimal(value: unknown): void {
  expect(typeof value).toBe('string');
  expect(value).toMatch(/^(0|[1-9]\d*)$/);
}

function inputScenario() {
  const source = 'function handle(event, input, meta, state, ctx) return { outputs = {} } end';
  return {
    block_id: 'lighting_policy',
    source,
    source_fingerprint: sourceFingerprint(source),
    expected_revision: '4',
    expected_structural_revision: '1',
    trigger: { type: 'input', endpoint: 'occupied', value: { kind: 'bool', value: true }, previous: null },
    inputs: [{ endpoint: 'occupied', value: { kind: 'bool', value: true }, valid: true, age_ms: 0 }],
    state: {},
    pending_timers: []
  };
}

describe('Block workbench fixture contract', () => {
  beforeAll(async () => { await startFixture(); }, 10_000);
  afterAll(() => { fixture?.kill('SIGTERM'); });
  beforeEach(async () => { await resetFixture(); });

  it('validates a draft without persistence and returns line errors plus a fingerprint', async () => {
    const before = await snapshot();
    const scheduled = before.blocks.find((block: any) => block.id === 'scheduled_light_test');
    expectDecimal(before.active_structural_revision);
    expectDecimal(before.saved_structural_revision);
    expectDecimal(before.active_logic_revision);
    expectDecimal(before.saved_logic_revision);
    expectDecimal(scheduled.active_revision);
    expectDecimal(scheduled.saved_revision);
    expectDecimal(scheduled.pending_timers[0].logic_revision);
    expectDecimal(scheduled.executions[0].logic_revision);
    expectDecimal(scheduled.executions[0].trigger.structural_revision);
    expectDecimal(scheduled.executions[1].trigger.scheduled_logic_revision);
    const invalidSource = '-- syntax error';
    const invalid = await call('/api/blocks/lighting_policy/validate', { source: invalidSource, source_fingerprint: sourceFingerprint(invalidSource), expected_revision: '4', expected_structural_revision: '1' });
    expect(invalid.status).toBe(200);
    expect(invalid.value).toMatchObject({ status: 'invalid', block_id: 'lighting_policy', errors: [{ path: 'source', category: 'syntax', line: 1 }] });
    expectDecimal(invalid.value.active_revision);
    expectDecimal(invalid.value.structural_revision);
    expect(invalid.value.source_fingerprint).toMatch(/^fnv1a-[0-9a-f]{8}$/);

    const validScenario = inputScenario();
    const valid = await call('/api/blocks/lighting_policy/validate', { source: validScenario.source, source_fingerprint: validScenario.source_fingerprint, expected_revision: validScenario.expected_revision, expected_structural_revision: validScenario.expected_structural_revision });
    expect(valid.status).toBe(200);
    expect(valid.value).toMatchObject({ status: 'valid', errors: [] });
    expect(valid.value.source_fingerprint).not.toBe(invalid.value.source_fingerprint);
    expect(await snapshot()).toEqual(before);
  });

  it('runs the production block API through the fixture for validate, simulate, activate, and enable', async () => {
    const source = inputScenario().source;
    const validation = await validateBlockSource({ blockId: 'lighting_policy', source, expectedRevision: '4', expectedStructuralRevision: '1', fetchImpl: fixtureFetch });
    expect(validation).toMatchObject({ status: 'valid', blockId: 'lighting_policy', blockRevision: '4', structuralRevision: '1', sourceFingerprint: sourceFingerprint(source), errors: [] });

    const scenario = { blockId: 'lighting_policy', expectedLogicRevision: '4' as const, expectedStructuralRevision: '1' as const, trigger: { type: 'input' as const, endpoint: 'occupied', value: { kind: 'bool' as const, value: true }, previous: null }, inputs: [] };
    const simulation = await simulateBlockDraft('lighting_policy', source, scenario, fixtureFetch);
    expect(simulation).toMatchObject({ blockId: 'lighting_policy', logicRevision: '4', status: 'succeeded', signalEffects: [{ signal: 'lighting_allowed' }], eligibleConsumers: [{ blockId: 'hall_light', endpoint: 'allowed' }] });

    const activated = await activateBlockSource({ blockId: 'lighting_policy', source, expectedRevision: '4', expectedStructuralRevision: '1', fetchImpl: fixtureFetch });
    expect(activated).toMatchObject({ blockId: 'lighting_policy', activeRevision: '5', savedRevision: '5', activeEnabled: true, savedEnabled: true, structuralRevision: '1', cancelledTimers: [] });
    await expect(setBlockEnabled({ blockId: 'lighting_policy', enabled: false, expectedRevision: '5', expectedStructuralRevision: '1', fetchImpl: fixtureFetch })).resolves.toMatchObject({ blockId: 'lighting_policy', activeRevision: '6', activeEnabled: false, savedEnabled: false });
    await expect(activateBlockSource({ blockId: 'lighting_policy', source: 'function handle() return nil end', expectedRevision: '5', expectedStructuralRevision: '1', fetchImpl: fixtureFetch })).rejects.toMatchObject({ status: 409, currentRevision: '6', currentStructuralRevision: '1' });
  });

  it('simulates the exact draft with proposed signal effects while preserving live state', async () => {
    const before = await snapshot();
    const scenario = inputScenario();
    scenario.source = '-- state_change\n' + scenario.source;
    scenario.source_fingerprint = sourceFingerprint(scenario.source);
    const result = await call('/api/blocks/lighting_policy/simulate', scenario);
    expect(result.status).toBe(200);
    expect(result.value).toMatchObject({ block_id: 'lighting_policy', block_revision: '4', structural_revision: '1', logic_revision: '4', status: 'succeeded', state_before: {}, state_after: { policy_seen: { kind: 'bool', value: true } } });
    expect(result.value.signalEffects).toEqual([expect.objectContaining({ endpoint: 'allowed', signal: 'lighting_allowed', changed: true })]);
    expect(result.value.eligibleConsumers).toEqual([{ blockId: 'hall_light', endpoint: 'allowed' }]);
    expect(result.value.effects).toEqual([]);
    expect(result.value.source_fingerprint).toMatch(/^fnv1a-[0-9a-f]{8}$/);
    expectDecimal(result.value.logic_revision);
    expectDecimal(result.value.block_revision);
    expectDecimal(result.value.structural_revision);
    expect(await snapshot()).toEqual(before);
  });

  it('contains simulation failures and rejects numeric revision tokens', async () => {
    const scenario = inputScenario();
    scenario.source = '-- runtime_error\n' + scenario.source;
    scenario.source_fingerprint = sourceFingerprint(scenario.source);
    const failed = await call('/api/blocks/lighting_policy/simulate', scenario);
    expect(failed.status).toBe(200);
    expect(failed.value).toMatchObject({ status: 'failed', error: { category: 'runtime', line: 3 }, effects: [], signalEffects: [], timer_effects: [] });

    const numeric = await call('/api/blocks/lighting_policy/simulate', { ...inputScenario(), expected_revision: 4 });
    expect(numeric.status).toBe(422);
    expect(numeric.value.errors).toEqual(expect.arrayContaining([{ path: 'expected_revision', message: 'must be a decimal string' }]));
  });

  it('activates atomically with compare-and-swap, preserves unrelated revisions, and never creates an execution', async () => {
    const invalidSource = '-- invalid source';
    const invalid = await call('/api/blocks/scheduled_light_test/source', { source: invalidSource, source_fingerprint: sourceFingerprint(invalidSource), expected_revision: '12', expected_structural_revision: '1' }, 'PUT');
    expect(invalid.status).toBe(422);
    const persistenceSource = '-- persist_failure';
    const persistenceFailure = await call('/api/blocks/scheduled_light_test/source', { source: persistenceSource, source_fingerprint: sourceFingerprint(persistenceSource), expected_revision: '12', expected_structural_revision: '1' }, 'PUT');
    expect(persistenceFailure.status).toBe(500);
    const activationSource = '-- activation_failure';
    const activationFailure = await call('/api/blocks/scheduled_light_test/source', { source: activationSource, source_fingerprint: sourceFingerprint(activationSource), expected_revision: '12', expected_structural_revision: '1' }, 'PUT');
    expect(activationFailure.status).toBe(503);
    const staleSource = inputScenario().source;
    const stale = await call('/api/blocks/scheduled_light_test/source', { source: staleSource, source_fingerprint: sourceFingerprint(staleSource), expected_revision: '11', expected_structural_revision: '1' }, 'PUT');
    expect(stale.status).toBe(409);
    expect(stale.value).toMatchObject({ current_revision: '12', current_structural_revision: '1' });
    expectDecimal(stale.value.current_revision);
    expectDecimal(stale.value.current_structural_revision);

    const before = await snapshot();
    const source = 'function handle(event, input, meta, state, ctx) return { outputs = { scheduled_light = true } } end';
    const activated = await call('/api/blocks/scheduled_light_test/source', { source, source_fingerprint: sourceFingerprint(source), expected_revision: '12', expected_structural_revision: '1' }, 'PUT');
    expect(activated.status).toBe(200);
    expect(activated.value).toMatchObject({ block_id: 'scheduled_light_test', active_revision: '13', saved_revision: '13', cancelled_timers: ['dim', 'off'] });
    for (const field of ['active_revision', 'saved_revision', 'active_logic_revision', 'saved_logic_revision', 'active_structural_revision', 'saved_structural_revision']) expectDecimal(activated.value[field]);
    const after = await snapshot();
    const changed = after.blocks.find((block: any) => block.id === 'scheduled_light_test');
    const unrelated = after.blocks.find((block: any) => block.id === 'occupancy_source');
    expect(changed).toMatchObject({ source, active_revision: '13', saved_revision: '13', pending_timers: [] });
    expect(unrelated).toMatchObject({ active_revision: '3', saved_revision: '3' });
    expect(changed.executions).toHaveLength(before.blocks.find((block: any) => block.id === 'scheduled_light_test').executions.length);
  });

  it('keeps a stale second-tab draft from overwriting the first activation', async () => {
    const firstSource = 'function handle(event, input, meta, state, ctx) return { outputs = {} } end';
    const secondSource = 'function handle(event, input, meta, state, ctx) return nil end';
    const first = await call('/api/blocks/lighting_policy/source', { source: firstSource, source_fingerprint: sourceFingerprint(firstSource), expected_revision: '4', expected_structural_revision: '1' }, 'PUT');
    expect(first.status).toBe(200);
    const second = await call('/api/blocks/lighting_policy/source', { source: secondSource, source_fingerprint: sourceFingerprint(secondSource), expected_revision: '4', expected_structural_revision: '1' }, 'PUT');
    expect(second.status).toBe(409);
    expectDecimal(second.value.current_revision);
    expect(await snapshot()).toMatchObject({ blocks: expect.arrayContaining([expect.objectContaining({ id: 'lighting_policy', source: firstSource, active_revision: '5' })]) });
  });

  it('disables and re-enables a block with timer cancellation and quiet execution semantics', async () => {
    const before = await snapshot();
    const disabled = await call('/api/blocks/scheduled_light_test/enabled', { enabled: false, expected_revision: '12', expected_structural_revision: '1' }, 'PUT');
    expect(disabled.status).toBe(200);
    expect(disabled.value).toMatchObject({ enabled: false, active_enabled: false, saved_enabled: false, active_revision: '13', cancelled_timers: ['dim', 'off'] });
    const afterDisable = await snapshot();
    expect(afterDisable.blocks.find((block: any) => block.id === 'scheduled_light_test')).toMatchObject({ active_enabled: false, pending_timers: [] });
    expect(afterDisable.blocks.find((block: any) => block.id === 'scheduled_light_test').executions).toHaveLength(before.blocks.find((block: any) => block.id === 'scheduled_light_test').executions.length);

    const enabled = await call('/api/blocks/scheduled_light_test/enabled', { enabled: true, expected_revision: '13', expected_structural_revision: '1' }, 'PUT');
    expect(enabled.status).toBe(200);
    expect(enabled.value).toMatchObject({ enabled: true, active_enabled: true, saved_enabled: true, active_revision: '14', cancelled_timers: [] });
    const afterEnable = await snapshot();
    expect(afterEnable.blocks.find((block: any) => block.id === 'scheduled_light_test')).toMatchObject({ active_enabled: true, saved_enabled: true, active_revision: '14' });
    expect(afterEnable.blocks.find((block: any) => block.id === 'scheduled_light_test').executions).toHaveLength(before.blocks.find((block: any) => block.id === 'scheduled_light_test').executions.length);
    const stale = await call('/api/blocks/scheduled_light_test/enabled', { enabled: false, expected_revision: '13', expected_structural_revision: '1' }, 'PUT');
    expect(stale.status).toBe(409);
  });

  it('freezes live-derived timer state when the event stream goes stale', async () => {
    const raw = await snapshot();
    const decoded = decodeSnapshot(raw, 1_000);
    let dashboard = reduceDashboardState(initialDashboardState, { type: 'snapshot_loaded', snapshot: decoded, nowMs: 1_000 });
    dashboard = reduceDashboardState(dashboard, { type: 'stream_open' });
    dashboard = reduceDashboardState(dashboard, { type: 'tick', nowMs: 2_400 });
    dashboard = reduceDashboardState(dashboard, { type: 'stream_lost', error: 'fixture stream stopped' });
    const frozen = displayedCountdownMs(dashboard, 'dim');
    dashboard = reduceDashboardState(dashboard, { type: 'tick', nowMs: 9_900 });
    expect(dashboard.stale).toBe(true);
    expect(displayedCountdownMs(dashboard, 'dim')).toBe(frozen);
    expect(frozen).toBe(3_900);
    expect(dashboard.snapshot).toBe(decoded);
  });

  it('preserves a dirty draft across live revision changes and marks it conflicted', async () => {
    const raw = await snapshot();
    const decoded = decodeSnapshot(raw, 1_000);
    const block = decoded.blocks.find((item) => item.id === 'lighting_policy');
    expect(block).toBeDefined();
    const original = createBlockDraft(block!, decoded.activeStructuralRevision);
    const source = `${original.source}\n-- browser draft`;
    const dirty = updateDraftSource(original, source);
    expect(dirty).toMatchObject({ source, dirty: true, conflict: false, baseActiveRevision: '4', baseStructuralRevision: '1' });
    const live = { ...block!, source: 'function handle(event) return nil end', activeRevision: '5', activeLogicRevision: '5' };
    const conflicted = reconcileBlockDraft(dirty, live, '1');
    expect(conflicted).toMatchObject({ source, dirty: true, conflict: true, conflictRevision: '5' });
    const clean = reconcileBlockDraft(original, live, '1');
    expect(clean).toMatchObject({ source: live.source, dirty: false, conflict: false, baseActiveRevision: '5' });
  });
});
