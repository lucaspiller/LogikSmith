import { describe, expect, it } from 'vitest';
import { decodeSimulation, decodeSnapshot } from '../src/lib/api';

const typed = (value: boolean) => ({ kind: 'bool', value });
const trigger = { type: 'input', endpoint: 'occupied', dpt: '1.001', value: typed(true), previous: null, changed: true, rising: true, falling: false };
const signal = (name: string, value: unknown, overrides: Record<string, unknown> = {}) => ({ name, dpt: '1.001', value, status: value === null ? 'unknown' : 'valid', observedAtMs: value === null ? null : 1200, changedAtMs: value === null ? null : 1200, producer: null, producingExecutionId: null, consumers: [], recentChanges: [], structuralRevision: '7', ...overrides });

function snapshot() {
  const effect = { endpoint: 'occupied', signal: 'house_occupied', dpt: '1.001', value: typed(true), changed: true, producer: { blockId: 'occupancy_source', endpoint: 'occupied' }, producingExecutionId: 101, consumers: [{ blockId: 'lighting_policy', endpoint: 'occupied' }] };
  const execution = { id: 101, timeMs: 1200, durationUs: 42, logicRevision: '3', status: 'succeeded', trigger, inputs: [], stateBefore: {}, stateAfter: {}, transition: { state: {}, effects: [], signalEffects: [effect], timers: [] }, signalEffects: [effect], causalProducerExecutionId: null, error: null };
  const block = (id: string, input: string, output: string, executionValue: Record<string, unknown> | null = null) => ({ id, activeEnabled: true, savedEnabled: true, activeRevision: '3', savedRevision: '3', activeLogicRevision: '3', savedLogicRevision: '3', source: 'return nil', inputs: [{ name: input, dpt: '1.001', bindingKind: 'signal', signal: 'house_occupied' }], outputs: [{ name: output, dpt: '1.001', bindingKind: 'signal', signal: 'lighting_allowed' }], signalBindings: [{ endpoint: input, signal: 'house_occupied' }, { endpoint: output, signal: 'lighting_allowed' }], knxBindings: [], state: {}, pendingTimers: [], schedules: [], executions: executionValue ? [executionValue] : [], lastResult: null });
  return { revision: 2, capturedAtMs: 1000, connection: { state: 'connected' }, activeStructuralRevision: '7', savedStructuralRevision: '7', restartRequired: false, signals: [signal('house_occupied', null, { consumers: [{ blockId: 'lighting_policy', endpoint: 'occupied' }] }), signal('lighting_allowed', typed(true), { producer: { blockId: 'lighting_policy', endpoint: 'allowed', executionId: 102 }, producingExecutionId: 102, recentChanges: [{ value: typed(true), observedAtMs: 1300, changedAtMs: 1300, executionId: 102 }] })], blocks: [block('occupancy_source', 'occupied', 'house_occupied', execution), block('lighting_policy', 'occupied', 'allowed', { ...execution, id: 102, causalProducerExecutionId: 101, signalEffects: [], transition: { state: {}, effects: [], signalEffects: [], timers: [] } })], telegrams: [], logs: [] };
}

describe('Internal signal diagnostics', () => {
  it('decodes unknown and known signals with producers, consumers, changes, and bindings', () => {
    const decoded = decodeSnapshot(snapshot());
    expect(decoded.signals[0]).toMatchObject({ name: 'house_occupied', value: null, status: 'unknown', observedAtMs: null, producer: null, consumers: [{ blockId: 'lighting_policy', endpoint: 'occupied' }] });
    expect(decoded.signals[1]).toMatchObject({ name: 'lighting_allowed', value: true, status: 'valid', producingExecutionId: 102, producer: { blockId: 'lighting_policy', endpoint: 'allowed' }, recentChanges: [{ value: true, executionId: 102 }] });
    expect(decoded.blocks[0].inputs[0]).toMatchObject({ bindingKind: 'signal', signal: 'house_occupied' });
    expect(decoded.blocks[0].signalBindings).toEqual([{ endpoint: 'occupied', signal: 'house_occupied' }, { endpoint: 'house_occupied', signal: 'lighting_allowed' }]);
    expect(decoded.blocks[1].executions[0]).toMatchObject({ executionId: 102, causalProducerExecutionId: 101 });
    expect(decoded.blocks[0].executions[0].signalEffects[0]).toMatchObject({ signal: 'house_occupied', value: true, changed: true, consumers: [{ blockId: 'lighting_policy', endpoint: 'occupied' }] });
  });

  it('decodes simulation signal effects and eligible consumers as proposed only', () => {
    const result = decodeSimulation({ logicRevision: '3', durationUs: 4, status: 'succeeded', trigger, inputs: [], stateBefore: {}, stateAfter: {}, signalEffects: [{ endpoint: 'occupied', signal: 'house_occupied', dpt: '1.001', value: typed(true), changed: true }], eligibleConsumers: [{ blockId: 'lighting_policy', endpoint: 'occupied' }], effects: [], timerEffects: [], pendingTimers: [], error: null });
    expect(result.signalEffects).toEqual([{ endpoint: 'occupied', signal: 'house_occupied', dpt: '1.001', value: true, changed: true }]);
    expect(result.eligibleConsumers).toEqual([{ blockId: 'lighting_policy', endpoint: 'occupied' }]);
  });
});
