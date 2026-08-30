import { describe, expect, it } from 'vitest';
import { decodeSnapshot } from '../src/lib/api';

function baseBlock(overrides: Record<string, unknown> = {}) {
  return {
    id: 'weather_forecast',
    active_enabled: true,
    saved_enabled: true,
    active_revision: '7',
    saved_revision: '7',
    source: 'return nil',
    inputs: [
      { name: 'today_temperature_max', dpt: { major: 9, subtype: 1 }, bindingKind: 'http', source: 'today_temperature_max', observed: { kind: 'temperature', value: 21.5 } },
      { name: 'external_override', dpt: { major: 1, subtype: 1 }, bindingKind: 'webhook', source: 'external_override', observed: { kind: 'bool', value: true } }
    ],
    outputs: [],
    http_bindings: [{ endpoint: 'today_temperature_max', source: 'today_temperature_max' }],
    webhook_bindings: [{ endpoint: 'external_override', source: 'external_override' }],
    state: {},
    pending_timers: [],
    schedules: [],
    executions: [],
    ...overrides
  };
}

function snapshot(overrides: Record<string, unknown> = {}) {
  return {
    revision: 4,
    captured_at_ms: 10_000,
    connection: { state: 'connected' },
    blocks: [baseBlock()],
    telegrams: [],
    logs: [],
    external_inputs: {
      http_polls: [{
        name: 'berlin_today_forecast',
        url: 'https://api.open-meteo.com/v1/forecast?latitude=52.52&token=secret',
        interval_ms: 21_600_000,
        status: 'healthy',
        last_attempt_at_ms: 9_900,
        next_attempt_at_ms: 1_756_521_600_000,
        last_success_at_ms: 9_800,
        freshness_deadline_ms: 43_200_000,
        consecutive_failures: 0,
        last_error: null,
        consumers: [{ block_id: 'weather_forecast', endpoint: 'today_temperature_max' }],
        values: [{ name: 'today_temperature_max', dpt: { major: 9, subtype: 1 }, json_pointer: '/daily/temperature_2m_max/0', value: { kind: 'temperature', value: 21.5 }, valid: true, age_ms: 200 }]
      }],
      webhook_inputs: [{
        name: 'external_override',
        route: '/api/webhooks/external_override',
        dpt: { major: 1, subtype: 1 },
        json_pointer: '/enabled',
        status: 'healthy',
        authentication_required: true,
        authentication_configured: true,
        last_accepted_at_ms: 9_700,
        accepted_count: 3,
        rejected_count: 1,
        value: { kind: 'bool', value: true },
        valid: true,
        age_ms: 300,
        consumers: [{ block_id: 'weather_forecast', endpoint: 'external_override' }]
      }]
    },
    ...overrides
  };
}

describe('External input diagnostics', () => {
  it('decodes source health, extracted values, consumers, and external endpoint bindings', () => {
    const decoded = decodeSnapshot(snapshot(), 10_500);
    expect(decoded.externalInputs.httpPolls[0]).toMatchObject({ name: 'berlin_today_forecast', url: 'https://api.open-meteo.com/v1/forecast', status: 'healthy', intervalMs: 21_600_000, nextAttemptAtMs: 1_756_521_600_000, staleAtMs: 43_200_000 });
    expect(decoded.externalInputs.httpPolls[0].values[0]).toMatchObject({ name: 'today_temperature_max', dpt: '9.001', jsonPointer: '/daily/temperature_2m_max/0', value: 21.5, valid: true, ageMs: 200 });
    expect(decoded.externalInputs.webhooks[0]).toMatchObject({ route: '/api/webhooks/external_override', authenticationRequired: true, authenticationConfigured: true, acceptedCount: 3, rejectedCount: 1, value: true });
    expect(decoded.blocks[0].inputs[0]).toMatchObject({ bindingKind: 'http', source: 'today_temperature_max', observed: 21.5 });
    expect(decoded.blocks[0].inputs[1]).toMatchObject({ bindingKind: 'webhook', source: 'external_override', observed: true });
    expect(decoded.blocks[0].bindings).toEqual([
      { endpoint: 'today_temperature_max', kind: 'http', source: 'today_temperature_max' },
      { endpoint: 'external_override', kind: 'webhook', source: 'external_override' }
    ]);
  });

  it('decodes HTTP and webhook execution origins while retaining legacy origin absence', () => {
    const execution = {
      execution_id: 2,
      time_ms: 10_100,
      duration_us: 10,
      logic_revision: '7',
      status: 'succeeded',
      trigger: { type: 'input', endpoint: 'today_temperature_max', dpt: { major: 9, subtype: 1 }, value: { kind: 'temperature', value: 21.5 }, previous: null, changed: true, rising: false, falling: false },
      origin: { kind: 'http', poll: 'berlin_today_forecast', value: 'today_temperature_max' },
      inputs: [],
      state_before: {},
      state_after: {},
      effects: [],
      timer_effects: [],
      error: null
    };
    const decoded = decodeSnapshot(snapshot({ blocks: [baseBlock({ executions: [execution] })] }));
    expect(decoded.blocks[0].executions[0].origin).toEqual({ kind: 'http', poll: 'berlin_today_forecast', value: 'today_temperature_max' });
    const trigger = decoded.blocks[0].executions[0].trigger;
    expect(trigger.type).toBe('input');
    if (trigger.type === 'input') expect(trigger.value).toBe(21.5);
    expect(decodeSnapshot(snapshot()).blocks[0].executions).toEqual([]);
  });

  it('rejects unknown source health and invalid value validity', () => {
    expect(() => decodeSnapshot(snapshot({ external_inputs: { http_polls: [{ name: 'x', url: 'https://example.test', interval_ms: 1000, status: 'broken', values: [] }], webhook_inputs: [] } }))).toThrow(/externalInputs\.httpPolls\[0\]\.status/);
    expect(() => decodeSnapshot(snapshot({ external_inputs: { http_polls: [{ name: 'x', url: 'https://example.test', interval_ms: 1000, status: 'healthy', values: [{ name: 'v', dpt: '9.001', json_pointer: '/v', value: null, valid: true, age_ms: 1 }] }], webhook_inputs: [] } }))).toThrow(/valid external values/);
  });
});
