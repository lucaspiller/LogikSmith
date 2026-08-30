import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(fileURLToPath(new URL('.', import.meta.url)), '..', 'dist');
const port = Number(process.argv[2] ?? 8090);
const m12InitialBlocks = [
  ['scheduled_light_test', { revision: '12', source: null, enabled: true }],
  ['occupancy_source', { revision: '3', source: null, enabled: true }],
  ['lighting_policy', { revision: '4', source: null, enabled: true }],
  ['hall_light', { revision: '5', source: null, enabled: false }]
];
const m12State = { structuralRevision: '1', blocks: new Map(m12InitialBlocks) };
function resetM12State() {
  m12State.structuralRevision = '1';
  m12State.blocks = new Map(m12InitialBlocks.map(([id, value]) => [id, { ...value }]));
}
const state = {
  phase: { kind: 'string', value: 'waiting_to_turn_off' },
  activation_count: { kind: 'integer', value: 2 }
};
const pendingTimers = [
  { name: 'dim', scheduled_at_ms: 800, due_at_ms: 5300, logic_revision: '12' },
  { name: 'off', scheduled_at_ms: 800, due_at_ms: 5800, logic_revision: '12' }
];
const siteTime = {
  timezone: 'Europe/Vilnius',
  local_time: '2026-08-29 05:30:00',
  utc_offset: 10800,
  coordinates: { latitude: 54.6872, longitude: 25.2797 },
  astronomy: 'available',
  astronomy_reason: null,
  dawn: '2026-08-29 04:56:00',
  sunrise: '2026-08-29 05:30:00',
  sunset: '2026-08-29 20:45:00',
  dusk: '2026-08-29 21:30:00',
  clock_ok: true,
  scheduler_ok: true
};
const schedules = [
  {
    name: 'morning_on',
    enabled: true,
    status: 'active',
    rule: { kind: 'astronomical', summary: 'sunrise - 1h30m' },
    next_occurrence: '2026-08-29 05:30:00',
    next_occurrence_utc_ms: 1_756_500_000_000,
    relative_ms: 3_600_000,
    utc_offset: 10800,
    unavailable_reason: null,
    last_result: { status: 'delivered', execution_id: 8, time_ms: 1_756_500_123_000 }
  },
  {
    name: 'heartbeat',
    enabled: true,
    status: 'active',
    rule: { kind: 'interval', summary: 'every 60s, aligned to UTC' },
    next_occurrence: '2026-08-29 05:31:00',
    next_occurrence_utc_ms: 1_756_500_060_000,
    relative_ms: 60_000,
    utc_offset: 10800,
    unavailable_reason: null,
    last_result: { status: 'failed', execution_id: 7, time_ms: 1_756_499_940_000 }
  },
  {
    name: 'evening_off',
    enabled: false,
    status: 'paused',
    rule: { kind: 'fixed', summary: '23:30:00 every day' },
    next_occurrence: null,
    next_occurrence_utc_ms: null,
    relative_ms: null,
    utc_offset: null,
    unavailable_reason: null,
    last_result: null
  }
];
const scheduleDefinitions = [
  { name: 'morning_on', enabled: true, kind: 'astronomical', anchor: 'sunrise', offset: '-1h30m' },
  { name: 'heartbeat', enabled: true, kind: 'interval', every: '60s' },
  { name: 'evening_off', enabled: false, kind: 'fixed', at: '23:30' }
];
function dateTime(year, month, day, hour, minute, second, weekday) {
  return { available: true, year, month, day, hour, minute, second, weekday };
}
const timeContext = {
  now: dateTime(2026, 8, 29, 5, 30, 0, 'Saturday'),
  sun: {
    dawn: dateTime(2026, 8, 29, 4, 56, 0, 'Saturday'),
    sunrise: dateTime(2026, 8, 29, 5, 30, 0, 'Saturday'),
    sunset: dateTime(2026, 8, 29, 20, 45, 0, 'Saturday'),
    dusk: dateTime(2026, 8, 29, 21, 30, 0, 'Saturday'),
    elevation_degrees: 42.5,
    azimuth_degrees: 180.0
  }
};
function scheduleTrigger() {
  return { type: 'schedule', name: 'morning_on', kind: 'astronomical', scheduled_for_utc_ms: 1_756_500_000_000, detected_at_utc_ms: 1_756_500_123_000, handled_at_utc_ms: 1_756_500_123_456, late_by_ms: 123_000, queue_delay_ms: 456, coalesced_count: 1, structural_revision: '17827391827' };
}
function timerTrigger() {
  return { type: 'timer', endpoint: '', dpt: { major: 0, subtype: 0 }, value: { kind: 'bool', value: false }, previous: null, changed: false, rising: false, falling: false, name: 'dim', scheduled_at_ms: 800, due_at_ms: 5300, fired_at_ms: 5312, late_by_ms: 12, scheduled_logic_revision: '12' };
}
function scheduleExecution() {
  return { execution_id: 8, time_ms: 1_756_500_123, duration_us: 42, logic_revision: '12', status: 'succeeded', trigger: scheduleTrigger(), inputs: [], state_before: state, state_after: state, transition: { state, effects: [], timers: [{ name: 'off', action: 'scheduled', after_ms: 5000, due_at_ms: 1_756_500_128_000 }] }, effects: [], timer_effects: [], time_context: timeContext, error: null };
}
function timerExecution() {
  return { execution_id: 7, time_ms: 5312, duration_us: 42, logic_revision: '12', status: 'succeeded', trigger: timerTrigger(), inputs: [{ endpoint: 'wall_switch', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true }, valid: true, age_ms: 5312 }], state_before: state, state_after: { ...state, phase: { kind: 'string', value: 'dimmed' } }, transition: { state: { phase: { kind: 'string', value: 'dimmed' } }, effects: [], timers: [{ name: 'off', action: 'replaced', previous_due_at_ms: 5800, after_ms: 5000, due_at_ms: 10312 }] }, effects: [], timer_effects: [], time_context: timeContext, error: null };
}
function httpExecution() {
  const temperature = { kind: 'temperature', value: 21.75 };
  return { execution_id: 9, time_ms: 1_756_500_124_000, duration_us: 38, logic_revision: '12', status: 'succeeded', trigger: { type: 'input', endpoint: 'today_temperature_max', dpt: { major: 9, subtype: 1 }, value: temperature, previous: null, changed: true, rising: false, falling: false }, origin: { kind: 'http', poll: 'berlin_today_forecast', value: 'today_temperature_max' }, inputs: [{ endpoint: 'today_temperature_max', dpt: { major: 9, subtype: 1 }, value: temperature, valid: true, age_ms: 200 }], state_before: state, state_after: state, transition: { state, effects: [], timers: [] }, effects: [], timer_effects: [], time_context: timeContext, error: null };
}
function blocks() {
  const values = [{
    id: 'scheduled_light_test',
    active_enabled: true,
    saved_enabled: true,
    active_revision: '12',
    saved_revision: '12',
    active_logic_revision: 12,
    saved_logic_revision: 12,
    source: 'function handle(event, input, meta, state, ctx)\n  if event.type == "schedule" and event.schedule == "morning_on" then\n    return { outputs = { scheduled_light = true }, timers = { off = { after = seconds(5) } } }\n  end\n  return nil\nend',
    inputs: [
      { name: 'wall_switch', dpt: { major: 1, subtype: 1 }, address: '1/2/3', observed: { kind: 'bool', value: true } },
      { name: 'today_temperature_max', dpt: { major: 9, subtype: 1 }, bindingKind: 'http', source: 'today_temperature_max', observed: { kind: 'temperature', value: 21.75 } },
      { name: 'external_override', dpt: { major: 1, subtype: 1 }, bindingKind: 'webhook', source: 'external_override', observed: { kind: 'bool', value: true } }
    ],
    outputs: [{ name: 'scheduled_light', dpt: { major: 1, subtype: 1 }, address: '1/2/4', observed: { kind: 'bool', value: false }, requested: null }],
    knx_bindings: [{ endpoint: 'wall_switch', group_address: '1/2/3' }, { endpoint: 'scheduled_light', group_address: '1/2/4' }],
    http_bindings: [{ endpoint: 'today_temperature_max', source: 'today_temperature_max', poll: 'berlin_today_forecast', value: 'today_temperature_max' }],
    webhook_bindings: [{ endpoint: 'external_override', source: 'external_override' }],
    state,
    pending_timers: pendingTimers,
    schedules,
    executions: [scheduleExecution(), timerExecution(), httpExecution()],
    last_result: { status: 'succeeded', execution_id: 8, time_ms: 1_756_500_123, error: null }
  }, {
    id: 'occupancy_source',
    activeEnabled: true,
    savedEnabled: true,
    activeRevision: '3',
    savedRevision: '3',
    activeLogicRevision: 3,
    savedLogicRevision: 3,
    source: 'return nil',
    inputs: [{ name: 'wall_switch', dpt: '1.001', address: '1/2/3', bindingKind: 'knx', observed: { kind: 'bool', value: true } }],
    outputs: [{ name: 'occupied', dpt: '1.001', bindingKind: 'signal', signal: 'house_occupied', observed: { kind: 'bool', value: true } }],
    knxBindings: [{ endpoint: 'wall_switch', groupAddress: '1/2/3' }],
    signalBindings: [{ endpoint: 'occupied', signal: 'house_occupied' }],
    state: {},
    pendingTimers: [],
    schedules: [],
    executions: [{ id: 101, timeMs: 1200, durationUs: 42, logicRevision: '3', status: 'succeeded', trigger: { type: 'input', endpoint: 'wall_switch', dpt: '1.001', value: { kind: 'bool', value: true }, previous: null, changed: true, rising: true, falling: false }, inputs: [], stateBefore: {}, stateAfter: {}, transition: { state: {}, effects: [], signalEffects: [{ endpoint: 'occupied', signal: 'house_occupied', dpt: '1.001', value: { kind: 'bool', value: true }, changed: true, producer: { blockId: 'occupancy_source', endpoint: 'occupied' }, producingExecutionId: 101, consumers: [{ blockId: 'lighting_policy', endpoint: 'occupied' }] }], timers: [] }, signalEffects: [{ endpoint: 'occupied', signal: 'house_occupied', dpt: '1.001', value: { kind: 'bool', value: true }, changed: true, producer: { blockId: 'occupancy_source', endpoint: 'occupied' }, producingExecutionId: 101, consumers: [{ blockId: 'lighting_policy', endpoint: 'occupied' }] }], causalProducerExecutionId: null, error: null }],
    lastResult: { status: 'succeeded', executionId: 101, timeMs: 1200, error: null }
  }, {
    id: 'lighting_policy',
    activeEnabled: true,
    savedEnabled: true,
    activeRevision: '4',
    savedRevision: '4',
    activeLogicRevision: 4,
    savedLogicRevision: 4,
    source: 'return nil',
    inputs: [{ name: 'occupied', dpt: '1.001', bindingKind: 'signal', signal: 'house_occupied', observed: { kind: 'bool', value: true } }],
    outputs: [{ name: 'allowed', dpt: '1.001', bindingKind: 'signal', signal: 'lighting_allowed', observed: { kind: 'bool', value: true } }],
    signalBindings: [{ endpoint: 'occupied', signal: 'house_occupied' }, { endpoint: 'allowed', signal: 'lighting_allowed' }],
    state: {},
    pendingTimers: [],
    schedules: [],
    executions: [{ id: 102, timeMs: 1210, durationUs: 43, logicRevision: '4', status: 'succeeded', trigger: { type: 'input', endpoint: 'occupied', dpt: '1.001', value: { kind: 'bool', value: true }, previous: null, changed: true, rising: true, falling: false }, inputs: [], stateBefore: {}, stateAfter: {}, transition: { state: {}, effects: [], signalEffects: [{ endpoint: 'allowed', signal: 'lighting_allowed', dpt: '1.001', value: { kind: 'bool', value: true }, changed: true, producer: { blockId: 'lighting_policy', endpoint: 'allowed' }, producingExecutionId: 102, consumers: [{ blockId: 'hall_light', endpoint: 'allowed' }] }], timers: [] }, signalEffects: [{ endpoint: 'allowed', signal: 'lighting_allowed', dpt: '1.001', value: { kind: 'bool', value: true }, changed: true, producer: { blockId: 'lighting_policy', endpoint: 'allowed' }, producingExecutionId: 102, consumers: [{ blockId: 'hall_light', endpoint: 'allowed' }] }], causalProducerExecutionId: 101, causalSignal: 'house_occupied', causalProducerBlockId: 'occupancy_source', error: null }],
    lastResult: { status: 'succeeded', executionId: 102, timeMs: 1210, error: null }
  }, {
    id: 'hall_light',
    activeEnabled: false,
    savedEnabled: false,
    activeRevision: '5',
    savedRevision: '5',
    activeLogicRevision: 5,
    savedLogicRevision: 5,
    source: 'return nil',
    inputs: [{ name: 'allowed', dpt: '1.001', bindingKind: 'signal', signal: 'lighting_allowed', observed: { kind: 'bool', value: true } }],
    outputs: [{ name: 'actuator', dpt: '1.001', address: '1/2/4', bindingKind: 'knx', observed: { kind: 'bool', value: false }, requested: null }],
    knxBindings: [{ endpoint: 'actuator', groupAddress: '1/2/4' }],
    signalBindings: [{ endpoint: 'allowed', signal: 'lighting_allowed' }],
    state: {},
    pendingTimers: [],
    schedules: [],
    executions: [],
    lastResult: { status: 'none', executionId: null, timeMs: null, error: null }
  }];
  return values.map((item) => {
    const current = m12State.blocks.get(item.id);
    if (!current) return item;
    return {
      ...item,
      active_enabled: current.enabled,
      saved_enabled: current.enabled,
      activeEnabled: current.enabled,
      savedEnabled: current.enabled,
      active_revision: current.revision,
      saved_revision: current.revision,
      active_logic_revision: current.revision,
      saved_logic_revision: current.revision,
      source: current.source ?? item.source,
      pending_timers: item.id === 'scheduled_light_test' && current.revision !== '12' ? [] : item.pending_timers
    };
  });
}

const decimalRevision = /^(0|[1-9]\d*)$/;
function m12Fingerprint(source) {
  let hash = 2166136261;
  for (const byte of new TextEncoder().encode(source)) {
    hash ^= byte;
    hash = Math.imul(hash, 16777619);
  }
  return `fnv1a-${(hash >>> 0).toString(16).padStart(8, '0')}`;
}
function m12Block(blockId) {
  return m12State.blocks.get(blockId) ?? null;
}
function m12Validation(source) {
  if (typeof source !== 'string') return [{ path: 'source', category: 'request', message: 'must be a string', line: null }];
  if (new TextEncoder().encode(source).byteLength > 65_536) return [{ path: 'source', category: 'source_limit', message: 'source exceeds the 65536 byte limit', line: null }];
  if (!source.trim()) return [{ path: 'source', category: 'syntax', message: 'source must not be empty', line: 1 }];
  if (/syntax[_ ]error|invalid[_ ]source/i.test(source)) return [{ path: 'source', category: 'syntax', message: 'unexpected symbol near \'end\'', line: 1 }];
  return [];
}
function m12BodyErrors(body, required, optional = []) {
  const errors = contractErrors(body, required, optional);
  if (body && 'source_fingerprint' in body && typeof body.source_fingerprint !== 'string') errors.push({ path: 'source_fingerprint', message: 'must be a string' });
  for (const key of ['expected_revision', 'expected_structural_revision']) {
    if (body && key in body && (typeof body[key] !== 'string' || !decimalRevision.test(body[key]))) {
      errors.push({ path: key, message: 'must be a decimal string' });
    }
  }
  return errors;
}
function m12Conflict(body, current) {
  if (body.expected_revision !== current.revision || body.expected_structural_revision !== m12State.structuralRevision) {
    return {
      error: 'active block or structural revision changed; refresh and retry',
      current_revision: current.revision,
      current_structural_revision: m12State.structuralRevision
    };
  }
  return null;
}
function m12NextRevision(current) {
  return (BigInt(current.revision) + 1n).toString();
}
function m12Trigger(body, blockId) {
  const trigger = body.trigger ?? { type: 'input', endpoint: 'wall_switch', value: { kind: 'bool', value: true }, previous: null };
  const typed = trigger.value ?? { kind: 'bool', value: true };
  const inputEndpoint = trigger.endpoint ?? (blockId === 'lighting_policy' ? 'occupied' : blockId === 'hall_light' ? 'allowed' : 'wall_switch');
  return trigger.type === 'timer'
    ? { type: 'timer', name: trigger.name ?? trigger.timer ?? 'off', scheduled_at_ms: 800, due_at_ms: trigger.fired_at_ms ?? trigger.firedAtMs ?? 5_800, fired_at_ms: trigger.fired_at_ms ?? trigger.firedAtMs ?? 5_800, late_by_ms: 0, scheduled_logic_revision: m12Block(blockId)?.revision ?? '0' }
    : { type: 'input', endpoint: inputEndpoint, dpt: { major: 1, subtype: 1 }, value: typed, previous: trigger.previous ?? null, changed: true, rising: typed.value === true, falling: false };
}
function m12Simulation(body, current) {
  const blockId = body.block_id;
  const trigger = m12Trigger(body, blockId);
  const stateBefore = body.state ?? {};
  const inputs = (body.inputs ?? []).map((input) => ({ ...input, dpt: input.dpt ?? { major: 1, subtype: 1 } }));
  const emptyTransition = { state: stateBefore, effects: [], signalEffects: [], timers: [] };
  if (/runtime[_ ]error/i.test(body.source)) {
    return { block_id: blockId, source_fingerprint: m12Fingerprint(body.source), block_revision: current.revision, structural_revision: m12State.structuralRevision, logic_revision: current.revision, duration_us: 17, status: 'failed', trigger, inputs, state_before: stateBefore, state_after: stateBefore, transition: emptyTransition, effects: [], signalEffects: [], eligibleConsumers: [], timer_effects: [], pending_timers: body.pending_timers ?? [], time_context: timeContext, error: { category: 'runtime', message: 'attempt to call a nil value', line: 3 } };
  }
  if (blockId === 'lighting_policy') {
    const effect = { endpoint: 'allowed', signal: 'lighting_allowed', dpt: { major: 1, subtype: 1 }, value: trigger.value ?? { kind: 'bool', value: true }, changed: true, producer: { blockId, endpoint: 'allowed' }, producingExecutionId: null, consumers: [{ blockId: 'hall_light', endpoint: 'allowed' }] };
    const stateAfter = /state[_ ]change/i.test(body.source) ? { ...stateBefore, policy_seen: { kind: 'bool', value: true } } : stateBefore;
    const transition = { state: stateAfter, effects: [], signalEffects: [effect], timers: [] };
    return { block_id: blockId, source_fingerprint: m12Fingerprint(body.source), block_revision: current.revision, structural_revision: m12State.structuralRevision, logic_revision: current.revision, duration_us: 19, status: 'succeeded', trigger, inputs, state_before: stateBefore, state_after: stateAfter, transition, effects: [], signalEffects: [effect], eligibleConsumers: effect.consumers, timer_effects: [], pending_timers: body.pending_timers ?? [], time_context: timeContext, error: null };
  }
  if (blockId === 'hall_light') {
    const output = { endpoint: 'actuator', destination: '1/2/4', dpt: { major: 1, subtype: 1 }, value: trigger.value ?? { kind: 'bool', value: true } };
    const transition = { state: stateBefore, effects: [output], signalEffects: [], timers: [] };
    return { block_id: blockId, source_fingerprint: m12Fingerprint(body.source), block_revision: current.revision, structural_revision: m12State.structuralRevision, logic_revision: current.revision, duration_us: 21, status: 'succeeded', trigger, inputs, state_before: stateBefore, state_after: stateBefore, transition, effects: [output], signalEffects: [], eligibleConsumers: [], timer_effects: [], pending_timers: body.pending_timers ?? [], time_context: timeContext, error: null };
  }
  const result = simulation({ ...body, block_id: blockId, state: stateBefore, pending_timers: body.pending_timers ?? [] });
  return { ...result, block_id: blockId, source_fingerprint: m12Fingerprint(body.source), block_revision: current.revision, structural_revision: m12State.structuralRevision, logic_revision: current.revision };
}
function m12ValidationResponse(blockId, source) {
  const errors = m12Validation(source);
  return { status: errors.length ? 'invalid' : 'valid', block_id: blockId, source_fingerprint: typeof source === 'string' ? m12Fingerprint(source) : null, active_revision: m12Block(blockId)?.revision ?? null, structural_revision: m12State.structuralRevision, errors };
}
function m12ActivationResponse(blockId, body, current) {
  const nextRevision = m12NextRevision(current);
  const cancelledTimers = blockId === 'scheduled_light_test' ? pendingTimers.map((timer) => timer.name) : [];
  current.revision = nextRevision;
  current.source = body.source;
  return { block_id: blockId, source_fingerprint: m12Fingerprint(body.source), active_revision: nextRevision, saved_revision: nextRevision, active_logic_revision: nextRevision, saved_logic_revision: nextRevision, active_structural_revision: m12State.structuralRevision, saved_structural_revision: m12State.structuralRevision, active_enabled: current.enabled, saved_enabled: current.enabled, restart_required: false, cancelled_timers: cancelledTimers };
}
function m12EnableResponse(blockId, body, current) {
  const nextRevision = m12NextRevision(current);
  const cancelledTimers = !body.enabled && blockId === 'scheduled_light_test' ? pendingTimers.map((timer) => timer.name) : [];
  current.revision = nextRevision;
  current.enabled = body.enabled;
  return { block_id: blockId, enabled: body.enabled, active_enabled: body.enabled, saved_enabled: body.enabled, active_revision: nextRevision, saved_revision: nextRevision, active_structural_revision: m12State.structuralRevision, saved_structural_revision: m12State.structuralRevision, cancelled_timers: cancelledTimers, restart_required: false };
}
function externalInputs() {
  return {
    http_polls: [{
      kind: 'http',
      name: 'berlin_today_forecast',
      url: 'https://api.open-meteo.com/v1/forecast?latitude=52.52&longitude=13.41&token=secret',
      interval_ms: 21_600_000,
      status: 'healthy',
      last_attempt_at_ms: 1_756_500_000_000,
      next_attempt_at_ms: 1_756_521_600_000,
      last_success_at_ms: 1_756_500_000_100,
      stale_at_ms: 1_756_543_200_000,
      consecutive_failures: 0,
      last_error: null,
      values: [{
        name: 'today_temperature_max',
        dpt: { major: 9, subtype: 1 },
        json_pointer: '/daily/temperature_2m_max/0',
        value: { kind: 'temperature', value: 21.75 },
        valid: true,
        age_ms: 200,
        consumers: [{ block_id: 'scheduled_light_test', endpoint: 'today_temperature_max' }]
      }]
    }],
    webhook_inputs: [{
      kind: 'webhook',
      name: 'external_override',
      route: '/api/webhooks/external_override',
      dpt: { major: 1, subtype: 1 },
      json_pointer: '/enabled',
      status: 'healthy',
      authentication_required: true,
      authentication_configured: true,
      last_accepted_at_ms: 1_756_500_000_200,
      accepted_count: 3,
      rejected_count: 1,
      value: { kind: 'bool', value: true },
      valid: true,
      age_ms: 300,
      consumers: [{ block_id: 'scheduled_light_test', endpoint: 'external_override' }]
    }]
  };
}
function snapshot() {
  return { revision: 4, captured_at_ms: 1_000, connection: { state: 'connected' }, site_time: siteTime, active_structural_revision: m12State.structuralRevision, saved_structural_revision: m12State.structuralRevision, active_logic_revision: m12Block('scheduled_light_test')?.revision ?? '0', saved_logic_revision: m12Block('scheduled_light_test')?.revision ?? '0', restart_required: false, signals: [{ name: 'house_occupied', dpt: '1.001', value: { kind: 'bool', value: true }, status: 'valid', observedAtMs: 1200, changedAtMs: 1200, producer: { blockId: 'occupancy_source', endpoint: 'occupied' }, producingExecutionId: 101, consumers: [{ blockId: 'lighting_policy', endpoint: 'occupied' }], recentChanges: [{ value: { kind: 'bool', value: true }, observedAtMs: 1200, changedAtMs: 1200, executionId: 101 }], structuralRevision: m12State.structuralRevision }, { name: 'lighting_allowed', dpt: '1.001', value: { kind: 'bool', value: true }, status: 'valid', observedAtMs: 1210, changedAtMs: 1210, producer: { blockId: 'lighting_policy', endpoint: 'allowed' }, producingExecutionId: 102, consumers: [{ blockId: 'hall_light', endpoint: 'allowed' }], recentChanges: [{ value: { kind: 'bool', value: true }, observedAtMs: 1210, changedAtMs: 1210, executionId: 102 }], structuralRevision: m12State.structuralRevision }, { name: 'night_mode', dpt: '1.001', value: null, status: 'unknown', observedAtMs: null, changedAtMs: null, producer: null, producingExecutionId: null, consumers: [], recentChanges: [], structuralRevision: m12State.structuralRevision }, { name: 'fallback_mode', dpt: '1.001', value: { kind: 'bool', value: false }, status: 'producer_disabled', observedAtMs: 1200, changedAtMs: null, producer: { blockId: 'hall_light', endpoint: 'allowed' }, producingExecutionId: null, consumers: [], recentChanges: [], structuralRevision: m12State.structuralRevision }], external_inputs: externalInputs(), blocks: blocks(), telegrams: [], logs: [] };
}
function simulation(body) {
  if (body.block_id === 'occupancy_source' && body.trigger?.type === 'input') {
    const effect = { endpoint: 'occupied', signal: 'house_occupied', dpt: '1.001', value: { kind: 'bool', value: true }, changed: true, producer: { blockId: 'occupancy_source', endpoint: 'occupied' }, producingExecutionId: null, consumers: [{ blockId: 'lighting_policy', endpoint: 'occupied' }] };
    const inputs = (body.inputs ?? []).map((input) => ({ ...input, dpt: { major: 1, subtype: 1 } }));
    return { block_id: 'occupancy_source', logic_revision: 3, duration_us: 18, status: 'succeeded', trigger: { type: 'input', endpoint: body.trigger.endpoint, dpt: { major: 1, subtype: 1 }, value: body.trigger.value, previous: body.trigger.previous ?? null, changed: true, rising: true, falling: false }, inputs, state_before: body.state ?? {}, state_after: body.state ?? {}, transition: { state: body.state ?? {}, effects: [], signalEffects: [effect], timers: [] }, pending_timers: [], effects: [], signalEffects: [effect], eligibleConsumers: effect.consumers, timer_effects: [], time_context: timeContext, error: null };
  }
  if (body.trigger?.type === 'schedule') {
    const schedule = body.trigger.schedule ?? 'morning_on';
    if (body.trigger.occurrence_at_ms === undefined || body.trigger.occurrence_at_ms === null) {
      return { block_id: body.block_id, schedule, rule: { kind: 'astronomical', summary: 'sunrise - 1h30m' }, occurrences: [
        { utc_ms: 1_756_500_000_000, local: '2026-08-29 05:30:00', utc_offset: 10800, weekday: 'Saturday' },
        { utc_ms: 1_756_586_400_000, local: '2026-08-30 05:30:00', utc_offset: 10800, weekday: 'Sunday' },
        { utc_ms: 1_756_672_800_000, local: '2026-08-31 05:30:00', utc_offset: 10800, weekday: 'Monday' }
      ] };
    }
    return { logic_revision: 12, duration_us: 24, status: 'succeeded', trigger: { type: 'schedule', name: schedule, kind: 'astronomical', scheduled_for_utc_ms: body.trigger.occurrence_at_ms, detected_at_utc_ms: body.trigger.occurrence_at_ms, handled_at_utc_ms: body.trigger.occurrence_at_ms, late_by_ms: 0, queue_delay_ms: 0, coalesced_count: 0, structural_revision: '17827391827' }, inputs: [], state_before: body.state ?? state, state_after: { ...(body.state ?? state), phase: { kind: 'string', value: 'dimmed' } }, transition: { state: { ...(body.state ?? state), phase: { kind: 'string', value: 'dimmed' } }, effects: [{ endpoint: 'scheduled_light', destination: '1/2/4', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true } }], signalEffects: [], timers: [{ name: 'off', action: 'scheduled', after_ms: 5000, due_at_ms: body.trigger.occurrence_at_ms + 5000 }] }, pending_timers: [], effects: [], signalEffects: [], eligibleConsumers: [], timer_effects: [], time_context: timeContext, error: null };
  }
  const timer = body.trigger?.type === 'timer';
  const selected = body.trigger?.name ?? 'off';
  const timerTrigger = { ...timerTrigger(), name: selected, fired_at_ms: body.trigger?.fired_at_ms ?? 5800, late_by_ms: 0 };
  const inputs = (body.inputs ?? []).map((input) => ({ ...input, dpt: { major: 1, subtype: 1 } }));
  const nextTimers = timer ? [] : [{ name: 'dim', scheduled_at_ms: 1000, due_at_ms: 5500, logic_revision: 12 }, { name: 'off', scheduled_at_ms: 1000, due_at_ms: 6000, logic_revision: 12 }];
  return { logic_revision: 12, duration_us: 24, status: 'succeeded', trigger: timer ? timerTrigger : { type: 'input', endpoint: 'wall_switch', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true }, previous: { kind: 'bool', value: false }, changed: true, rising: true, falling: false }, inputs, state_before: body.state ?? state, state_after: { ...(body.state ?? state), phase: { kind: 'string', value: timer ? 'idle' : 'waiting_to_turn_off' } }, transition: { state: { phase: { kind: 'string', value: timer ? 'idle' : 'waiting_to_turn_off' } }, effects: timer ? [{ endpoint: 'scheduled_light', destination: '1/2/4', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: false } }] : [{ endpoint: 'scheduled_light', destination: '1/2/4', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true } }], signalEffects: [], timers: timer ? [{ name: selected, action: 'cancelled', previous_due_at_ms: 5800 }] : [{ name: 'dim', action: 'scheduled', after_ms: 4500, due_at_ms: 5500 }, { name: 'off', action: 'scheduled', after_ms: 5000, due_at_ms: 6000 }] }, pending_timers: nextTimers, effects: [], signalEffects: [], eligibleConsumers: [], timer_effects: [], time_context: timeContext, error: null };
}
async function readJson(request) {
  let raw = '';
  for await (const chunk of request) raw += chunk;
  try { return JSON.parse(raw); } catch { return null; }
}
function contractErrors(body, required, optional = []) {
  const errors = [];
  if (!body || typeof body !== 'object' || Array.isArray(body)) return [{ path: 'request', message: 'expected a JSON object' }];
  for (const key of required) if (!(key in body) || body[key] === null || body[key] === undefined) errors.push({ path: key, message: 'required' });
  for (const key of Object.keys(body)) if (!required.includes(key) && !optional.includes(key)) errors.push({ path: key, message: 'unknown field' });
  return errors;
}
function schedulePreview(body) {
  const base = simulation({ block_id: body.block_id, trigger: { type: 'schedule', schedule: body.schedule } });
  const occurrences = base.occurrences ?? [];
  return { block_id: body.block_id, schedule: body.schedule, rule: { kind: 'astronomical', summary: 'sunrise - 1h30m' }, occurrences: occurrences.slice(0, body.count ?? 3) };
}
function schedulePreviewResponse(body) {
  // These names make the fixture useful for manually checking the final
  // desktop error contract without adding fake schedules to the snapshot.
  if (body.block_id !== 'scheduled_light_test') return { status: 404, value: { error: 'unknown logic block' } };
  if (body.schedule === 'missing_schedule') return { status: 404, value: { error: 'unknown schedule' } };
  return { status: 200, value: schedulePreview(body) };
}
function scheduleSimulation(body) {
  const result = simulation({ block_id: body.block_id, trigger: { type: 'schedule', schedule: body.schedule, occurrence_at_ms: body.occurrence_at_utc_ms } });
  return { ...result, logic_revision: body.expected_revision };
}
function scheduleSimulationResponse(body) {
  if (body.block_id !== 'scheduled_light_test') return { status: 404, value: { error: 'unknown logic block' } };
  if (body.schedule === 'missing_schedule') return { status: 404, value: { error: 'unknown schedule' } };
  if (body.schedule === 'stale_schedule') return {
    status: 409,
    value: {
      error: 'active schedule or block revision changed; refresh and run the schedule simulation again',
      current_revision: '18446744073709551615',
      current_structural_revision: '9223372036854775808'
    }
  };
  return { status: 200, value: scheduleSimulation(body) };
}
function documentDpt(value) {
  if (typeof value === 'string') return value;
  if (value && typeof value === 'object' && Number.isInteger(value.major) && Number.isInteger(value.subtype)) return `${value.major}.${String(value.subtype).padStart(3, '0')}`;
  return '1.001';
}
function sendJson(response, value, status = 200) { response.writeHead(status, { 'content-type': 'application/json' }); response.end(JSON.stringify(value)); }
const server = createServer(async (request, response) => {
  if (request.url?.startsWith('/api/snapshot')) return sendJson(response, snapshot());
  if (request.url?.startsWith('/api/automation')) {
    if (request.method === 'PUT') { for await (const _chunk of request) {} return sendJson(response, { revision: 13, logic_activated: true, active_logic_revision: 13, restart_required: false, cancelled_timers: ['dim', 'off'] }); }
    const document = { signals: snapshot().signals.map((item) => ({ name: item.name, dpt: item.dpt })), http_polls: [{ name: 'berlin_today_forecast', url: 'https://api.open-meteo.com/v1/forecast?latitude=52.52&longitude=13.41', every: '6h', timeout: '10s', stale_after: '12h', headers: [], values: [{ name: 'today_temperature_max', dpt: '9.001', json_pointer: '/daily/temperature_2m_max/0' }] }], webhook_inputs: [{ name: 'external_override', route: '/api/webhooks/external_override', dpt: '1.001', json_pointer: '/enabled', bearer_token: 'configured' }], blocks: blocks().map((item) => ({ id: item.id, enabled: item.active_enabled ?? item.activeEnabled ?? true, revision: 1, inputs: item.inputs.map((input) => ({ name: input.name, dpt: documentDpt(input.dpt) })), outputs: item.outputs.map((output) => ({ name: output.name, dpt: documentDpt(output.dpt) })), knx_bindings: item.knx_bindings ?? item.knxBindings ?? [], signal_bindings: item.signal_bindings ?? item.signalBindings ?? [], http_bindings: item.http_bindings ?? item.httpBindings ?? [], webhook_bindings: item.webhook_bindings ?? item.webhookBindings ?? [], source: item.source, schedules: item.schedules?.length ? scheduleDefinitions : [] })) };
    return sendJson(response, { document, revision: 12, active_logic_revision: 12, saved_logic_revision: 12, active_structural_revision: 1, saved_structural_revision: 1, restart_required: false, blocks: [{ id: 'scheduled_light_test', active_revision: '12', saved_revision: '12', active_logic_revision: 12, saved_logic_revision: 12, active_enabled: true, saved_enabled: true }] });
  }
  if (request.url?.startsWith('/api/test/reset')) {
    resetM12State();
    return sendJson(response, { status: 'reset' });
  }
  const m12Route = request.url?.match(/^\/api\/blocks\/([^/?]+)\/(validate|simulate|source|enabled)(?:\?.*)?$/);
  if (m12Route) {
    const [, blockId, operation] = m12Route;
    const current = m12Block(blockId);
    if (!current) return sendJson(response, { error: 'unknown logic block' }, 404);
    const body = await readJson(request);
    if (operation === 'validate') {
      const errors = m12BodyErrors(body, ['source', 'source_fingerprint', 'expected_revision'], ['expected_structural_revision']);
      if (errors.length) return sendJson(response, { errors }, 422);
      return sendJson(response, m12ValidationResponse(blockId, body.source));
    }
    if (operation === 'simulate') {
      const errors = m12BodyErrors(body, ['block_id', 'source', 'source_fingerprint', 'expected_revision', 'expected_structural_revision', 'trigger', 'inputs'], ['state', 'pending_timers']);
      if (errors.length) return sendJson(response, { errors }, 422);
      const conflict = m12Conflict(body, current);
      if (conflict) return sendJson(response, conflict, 409);
      const sourceErrors = m12Validation(body.source);
      if (sourceErrors.length) return sendJson(response, { errors: sourceErrors }, 422);
      return sendJson(response, m12Simulation({ ...body, block_id: blockId }, current));
    }
    if (operation === 'source') {
      const errors = m12BodyErrors(body, ['source', 'source_fingerprint', 'expected_revision', 'expected_structural_revision']);
      if (errors.length) return sendJson(response, { errors }, 422);
      const conflict = m12Conflict(body, current);
      if (conflict) return sendJson(response, conflict, 409);
      const sourceErrors = m12Validation(body.source);
      if (sourceErrors.length) return sendJson(response, { errors: sourceErrors }, 422);
      if (/persist[_ ]failure/i.test(body.source)) return sendJson(response, { error: { category: 'persistence', message: 'automation.toml could not be replaced atomically' } }, 500);
      if (/activation[_ ]failure/i.test(body.source)) return sendJson(response, { error: { category: 'activation', message: 'runtime rejected the validated source' } }, 503);
      return sendJson(response, m12ActivationResponse(blockId, body, current));
    }
    const errors = m12BodyErrors(body, ['enabled', 'expected_revision', 'expected_structural_revision']);
    if (body && 'enabled' in body && typeof body.enabled !== 'boolean') errors.push({ path: 'enabled', message: 'must be a boolean' });
    if (errors.length) return sendJson(response, { errors }, 422);
    const conflict = m12Conflict(body, current);
    if (conflict) return sendJson(response, conflict, 409);
    return sendJson(response, m12EnableResponse(blockId, body, current));
  }
  if (request.url?.startsWith('/api/schedules/preview')) {
    const body = await readJson(request); const errors = contractErrors(body, ['block_id', 'schedule'], ['after_utc_ms', 'count']);
    if (body && body.count !== undefined && (!Number.isInteger(body.count) || body.count < 1 || body.count > 10)) errors.push({ path: 'count', message: 'must be between 1 and 10' });
    if (errors.length) return sendJson(response, { errors }, 422);
    const result = schedulePreviewResponse(body);
    return sendJson(response, result.value, result.status);
  }
  if (request.url?.startsWith('/api/schedules/simulate')) {
    const body = await readJson(request); const errors = contractErrors(body, ['block_id', 'schedule', 'occurrence_at_utc_ms', 'expected_revision', 'expected_structural_revision']);
    for (const key of ['expected_revision', 'expected_structural_revision']) if (body && key in body && (typeof body[key] !== 'string' || !/^(0|[1-9]\d*)$/.test(body[key]))) errors.push({ path: key, message: 'must be a decimal string' });
    if (errors.length) return sendJson(response, { errors }, 422);
    const result = scheduleSimulationResponse(body);
    return sendJson(response, result.value, result.status);
  }
  if (request.url?.startsWith('/api/simulate')) { const body = await readJson(request); return sendJson(response, simulation(body ?? {})); }
  if (request.url?.startsWith('/api/events')) { response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', connection: 'keep-alive' }); response.write(': fixture connected\n\n'); request.on('close', () => response.end()); return; }
  const path = request.url === '/' ? '/index.html' : request.url;
  try { const data = await readFile(join(root, path)); const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css' }; response.writeHead(200, { 'content-type': types[extname(path)] ?? 'application/octet-stream' }); response.end(data); } catch { sendJson(response, { error: 'not found' }, 404); }
});
server.listen(port, '127.0.0.1', () => console.log(`fixture listening on http://127.0.0.1:${server.address().port}`));
