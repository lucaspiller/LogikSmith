import { describe, expect, it } from 'vitest';
import { decodeSnapshot, fetchScheduleOccurrences, simulateScenario, SimulationApiError } from '../src/lib/api';
import { createSimulationDraft, reconcileScheduleSelection, schedulePreviewDefault, toSimulationScenario } from '../src/lib/simulation';
import { displayedScheduleCountdownMs, formatDateTimeValue, formatUtcDateTime, formatUtcOffset, initialDashboardState, reduceDashboardState, triggerLabel, triggerSummary } from '../src/lib/state';

const UTC_MS = 1_756_500_000_000;

function schedule(name: string, overrides: Record<string, unknown> = {}) {
  return {
    name,
    enabled: true,
    status: 'active',
    rule: { kind: 'astronomical', summary: 'sunrise - 1h30m, earliest 05:30' },
    next_occurrence: '2026-08-29 05:30:00',
    next_occurrence_utc_ms: UTC_MS,
    relative_ms: 3_600_000,
    utc_offset: 10_800,
    unavailable_reason: null,
    last_result: null,
    ...overrides
  };
}

function dateTime(year: number, month: number, day: number, hour: number, minute: number, second: number, weekday: string) {
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

const scheduleTrigger = {
  type: 'schedule',
  name: 'morning_on',
  kind: 'astronomical',
  scheduled_for_utc_ms: UTC_MS,
  detected_at_utc_ms: UTC_MS + 123_000,
  handled_at_utc_ms: UTC_MS + 123_456,
  late_by_ms: 123_000,
  queue_delay_ms: 456,
  coalesced_count: 2,
  structural_revision: '17827391827'
};

function block(id: string, schedules: unknown[] = [], execution?: Record<string, unknown>) {
  return {
    id,
    active_enabled: true,
    saved_enabled: true,
    active_revision: '18446744073709551615',
    saved_revision: '18446744073709551615',
    active_logic_revision: 12,
    saved_logic_revision: 12,
    source: 'function handle(event, input, meta, state) return nil end',
    inputs: [],
    outputs: [],
    knx_bindings: [],
    state: {},
    pending_timers: [],
    schedules,
    executions: execution ? [execution] : [],
    last_result: null
  };
}

function snapshot(blocks: Array<Record<string, unknown>>, siteTime?: Record<string, unknown>) {
  return { revision: 4, captured_at_ms: 1_000, connection: { state: 'connected' }, blocks, telegrams: [], logs: [], active_structural_revision: 8, saved_structural_revision: 8, restart_required: false, ...(siteTime ? { site_time: siteTime } : {}) };
}

function scheduleExecution(trigger: Record<string, unknown>, overrides: Record<string, unknown> = {}) {
  return {
    execution_id: 9,
    time_ms: UTC_MS + 123_456,
    duration_us: 417,
    logic_revision: 12,
    status: 'succeeded',
    trigger,
    inputs: [],
    state_before: {},
    state_after: {},
    effects: [],
    timer_effects: [],
    time_context: timeContext,
    error: null,
    ...overrides
  };
}

describe('Milestone 9 site time', () => {
  it('decodes the site-time card with zone, local time, offset, coordinates, solar events, and health', () => {
    const decoded = decodeSnapshot(snapshot([block('staircase')], {
      timezone: 'Europe/Vilnius',
      local_time: '2026-08-29 05:30:00',
      utc_offset: 10_800,
      coordinates: { latitude: 54.6872, longitude: 25.2797 },
      astronomy: 'available',
      astronomy_reason: null,
      dawn: '2026-08-29 04:56:00',
      sunrise: '2026-08-29 05:30:00',
      sunset: '2026-08-29 20:45:00',
      dusk: '2026-08-29 21:30:00',
      clock_ok: true,
      scheduler_ok: true
    }), 1_000);
    expect(decoded.siteTime).toMatchObject({ timezone: 'Europe/Vilnius', localTime: '2026-08-29 05:30:00', utcOffsetSeconds: 10_800, astronomy: 'available', clockOk: true, schedulerOk: true, sun: { dawn: '2026-08-29 04:56:00', dusk: '2026-08-29 21:30:00' } });
    expect(decoded.siteTime?.coordinates).toEqual({ latitude: 54.6872, longitude: 25.2797 });
  });

  it('decodes an astronomy-unavailable card without coordinates', () => {
    const decoded = decodeSnapshot(snapshot([block('staircase')], {
      timezone: 'Europe/Vilnius',
      local_time: null,
      utc_offset: null,
      coordinates: null,
      astronomy: 'unavailable',
      astronomy_reason: 'no coordinates configured',
      dawn: null,
      sunrise: null,
      sunset: null,
      dusk: null,
      clock_ok: true,
      scheduler_ok: true
    }), 1_000);
    expect(decoded.siteTime?.astronomy).toBe('unavailable');
    expect(decoded.siteTime?.astronomyReason).toBe('no coordinates configured');
    expect(decoded.siteTime?.sun.dawn).toBeNull();
    expect(decoded.siteTime?.localTime).toBeNull();
  });

  it('rejects malformed site-time data with clear paths', () => {
    const base = { timezone: 'Europe/Vilnius', astronomy: 'available', clock_ok: true, scheduler_ok: true };
    expect(() => decodeSnapshot(snapshot([block('staircase')], { ...base, astronomy: 'maybe' }))).toThrow(/site_time\.astronomy/);
    expect(() => decodeSnapshot(snapshot([block('staircase')], { ...base, timezone: '' }))).toThrow(/site_time\.timezone/);
    expect(() => decodeSnapshot(snapshot([block('staircase')], { ...base, coordinates: { latitude: 91, longitude: 0 } }))).toThrow(/site_time\.coordinates\.latitude/);
    expect(() => decodeSnapshot(snapshot([block('staircase')], { ...base, coordinates: { latitude: 0, longitude: 180 } }))).toThrow(/site_time\.coordinates\.longitude/);
    expect(() => decodeSnapshot(snapshot([block('staircase')], { timezone: 'Europe/Vilnius', astronomy: 'available', clock_ok: true }))).toThrow(/site_time\.scheduler_ok/);
  });
});

describe('Milestone 9 per-block schedules', () => {
  it('decodes the schedule list with status, rule summary, occurrences, and outcome', () => {
    const decoded = decodeSnapshot(snapshot([block('staircase', [schedule('morning_on'), schedule('heartbeat', {
      name: 'heartbeat',
      status: 'paused',
      enabled: false,
      rule: { kind: 'interval', summary: 'every 60s aligned to UTC' },
      next_occurrence: null,
      next_occurrence_utc_ms: null,
      relative_ms: null,
      utc_offset: null,
      last_result: { status: 'failed', execution_id: 5, time_ms: 1_756_500_100_000 }
    }), schedule('evening_off', {
      name: 'evening_off',
      status: 'unavailable',
      rule: { kind: 'fixed', summary: '23:30:00' },
      next_occurrence: null,
      next_occurrence_utc_ms: null,
      relative_ms: null,
      utc_offset: null,
      unavailable_reason: 'no sunset on this date'
    })])]), 1_000);
    expect(decoded.blocks[0].schedules.map((item) => item.name)).toEqual(['morning_on', 'heartbeat', 'evening_off']);
    expect(decoded.blocks[0].activeRevision).toBe('18446744073709551615');
    expect(decoded.blocks[0].savedRevision).toBe('18446744073709551615');
    const morning = decoded.blocks[0].schedules[0];
    expect(morning).toMatchObject({ enabled: true, status: 'active', kind: 'astronomical', ruleSummary: 'sunrise - 1h30m, earliest 05:30', nextOccurrenceLocal: '2026-08-29 05:30:00', nextOccurrenceUtcMs: UTC_MS, relativeMs: 3_600_000, utcOffsetSeconds: 10_800, reason: null, lastOutcome: { status: 'none', executionId: null, timeMs: null } });
    expect(decoded.blocks[0].schedules[1]).toMatchObject({ status: 'paused', enabled: false, lastOutcome: { status: 'failed', executionId: 5, timeMs: 1_756_500_100_000 } });
    expect(decoded.blocks[0].schedules[2]).toMatchObject({ status: 'unavailable', reason: 'no sunset on this date' });
  });

  it('keeps the structural revision opaque when it is larger than a safe JS integer', () => {
    const raw = snapshot([block('staircase')]) as Record<string, unknown>;
    raw.active_structural_revision = '18446744073709551615';
    raw.saved_structural_revision = '18446744073709551615';
    const decoded = decodeSnapshot(raw);
    expect(decoded.activeStructuralRevision).toBe('18446744073709551615');
    expect(decoded.savedStructuralRevision).toBe('18446744073709551615');
  });

  it('rejects duplicate schedule names, an excessive schedule count, and malformed entries', () => {
    const duplicated = snapshot([block('staircase', [schedule('morning_on'), schedule('morning_on')])]);
    expect(() => decodeSnapshot(duplicated)).toThrow(/duplicate schedule name morning_on/);
    const tooMany = snapshot([block('staircase', Array.from({ length: 33 }, (_, index) => schedule(`s_${index}`)))]);
    expect(() => decodeSnapshot(tooMany)).toThrow(/at most 32 schedules/);
    expect(() => decodeSnapshot(snapshot([block('staircase', [schedule('morning_on', { status: 'bogus' })])]))).toThrow(/schedules\[0\]\.status/);
    expect(() => decodeSnapshot(snapshot([block('staircase', [schedule('morning_on', { rule: { kind: 'cron', summary: 'x' } })])]))).toThrow(/schedules\[0\]\.rule\.kind/);
    expect(() => decodeSnapshot(snapshot([block('staircase', [schedule('morning_on', { rule: { kind: 'fixed' } })])]))).toThrow(/rule\.summary/);
    expect(() => decodeSnapshot(snapshot([block('staircase', [schedule('morning_on', { last_result: { status: 'skipped', execution_id: 1, time_ms: 2 } })])]))).toThrow(/last_result\.status/);
  });

  it('accepts negative UTC offsets and a clock_error schedule', () => {
    const decoded = decodeSnapshot(snapshot([block('staircase', [schedule('evening_off', { name: 'evening_off', status: 'clock_error', rule: { kind: 'fixed', summary: '23:30:00' }, utc_offset: -18_000, unavailable_reason: 'invalid wall clock' })])]), 1_000);
    expect(decoded.blocks[0].schedules[0]).toMatchObject({ status: 'clock_error', utcOffsetSeconds: -18_000, reason: 'invalid wall clock' });
    expect(formatUtcOffset(-18_000)).toBe('UTC-05:00');
    expect(formatUtcOffset(0)).toBe('UTC');
    expect(formatUtcOffset(10_800)).toBe('UTC+03:00');
    expect(formatUtcOffset(null)).toBe('—');
  });
});

describe('Milestone 9 schedule countdown', () => {
  it('counts down from the captured server sample and freezes when the stream goes stale', () => {
    const mapped = decodeSnapshot(snapshot([block('staircase', [schedule('morning_on', { relative_ms: 4_800 })])], { timezone: 'Europe/Vilnius', astronomy: 'unavailable', clock_ok: true, scheduler_ok: true }), 1_000);
    let dashboard = reduceDashboardState(initialDashboardState, { type: 'snapshot_loaded', snapshot: mapped, nowMs: 1_000 });
    dashboard = reduceDashboardState(dashboard, { type: 'stream_open' });
    dashboard = reduceDashboardState(dashboard, { type: 'tick', nowMs: 2_000 });
    expect(displayedScheduleCountdownMs(dashboard, 'staircase', 'morning_on')).toBe(3_800);
    dashboard = reduceDashboardState(dashboard, { type: 'stream_lost' });
    expect(dashboard.staleAtMs).toBe(2_000);
    dashboard = reduceDashboardState(dashboard, { type: 'tick', nowMs: 9_000 });
    expect(displayedScheduleCountdownMs(dashboard, 'staircase', 'morning_on')).toBe(3_800);
  });

  it('returns null for unknown schedules or missing relative time', () => {
    const mapped = decodeSnapshot(snapshot([block('staircase', [schedule('morning_on', { relative_ms: null })])]), 1_000);
    expect(displayedScheduleCountdownMs({ ...initialDashboardState, snapshot: mapped, nowMs: 1_000 }, 'staircase', 'morning_on')).toBeNull();
    expect(displayedScheduleCountdownMs({ ...initialDashboardState, snapshot: mapped, nowMs: 1_000 }, 'staircase', 'missing')).toBeNull();
  });
});

describe('Milestone 9 schedule execution detail', () => {
  it('decodes a schedule trigger with lateness, queue delay, coalescing, and the full captured ctx', () => {
    const decoded = decodeSnapshot(snapshot([block('staircase', [], scheduleExecution(scheduleTrigger))]), 1_000);
    const execution = decoded.blocks[0].executions[0];
    expect(execution.trigger.type).toBe('schedule');
    if (execution.trigger.type !== 'schedule') throw new Error('expected a schedule trigger');
    expect(execution.trigger).toMatchObject({ name: 'morning_on', kind: 'astronomical', scheduledForUtcMs: UTC_MS, detectedAtUtcMs: UTC_MS + 123_000, handledAtUtcMs: UTC_MS + 123_456, lateByMs: 123_000, queueDelayMs: 456, coalescedCount: 2 });
    expect(execution.trigger.structuralRevision).toBe('17827391827');
    expect(execution.timeContext).toMatchObject({ now: { year: 2026, weekday: 'Saturday' }, sun: { elevationDegrees: 42.5, azimuthDegrees: 180.0 } });
    expect(triggerLabel(execution.trigger)).toBe('schedule:morning_on');
    expect(triggerSummary(execution.trigger)).toContain(`intended ${formatUtcDateTime(UTC_MS)}`);
    expect(triggerSummary(execution.trigger)).toContain('123 s late');
    expect(triggerSummary(execution.trigger)).toContain('456 ms queue');
    expect(triggerSummary(execution.trigger)).toContain('2 coalesced');
    expect(formatDateTimeValue(execution.timeContext!.now)).toBe('2026-08-29 05:30:00 (Saturday)');
    expect(formatDateTimeValue(execution.timeContext!.sun.dawn)).toBe('2026-08-29 04:56:00 (Saturday)');
    expect(formatUtcDateTime(UTC_MS)).toBe(`${new Date(UTC_MS).toISOString().replace('T', ' ').replace(/\.\d+Z$/, '')} UTC`);
  });

  it('rejects a schedule trigger missing required fields and accepts legacy executions without time context', () => {
    expect(() => decodeSnapshot(snapshot([block('staircase', [], scheduleExecution({ ...scheduleTrigger, scheduled_for_utc_ms: undefined }))]))).toThrow(/scheduled_for_utc_ms/);
    const legacy = decodeSnapshot(snapshot([block('staircase', [], scheduleExecution({ type: 'timer', name: 'off', scheduled_at_ms: 0, due_at_ms: 1, fired_at_ms: 1, late_by_ms: 0, scheduled_logic_revision: 12 }, { time_context: null }))]), 1_000);
    expect(legacy.blocks[0].executions[0].timeContext).toBeNull();
    expect(triggerLabel(legacy.blocks[0].executions[0].trigger)).toBe('timer:off');
  });
});

describe('Milestone 9 schedule simulation', () => {
  function simulationResponse(overrides: Record<string, unknown> = {}) {
    return { logic_revision: 12, duration_us: 24, status: 'succeeded', trigger: scheduleTrigger, inputs: [], state_before: {}, state_after: {}, effects: [], timer_effects: [], pending_timers: [], time_context: timeContext, error: null, ...overrides };
  }

  it('posts the dedicated schedule simulation contract with opaque revisions', async () => {
    let request: RequestInit | undefined;
    let requestUrl = '';
    const result = await simulateScenario({ blockId: 'staircase', expectedLogicRevision: 12, expectedStructuralRevision: 8, trigger: { type: 'schedule', schedule: 'morning_on', occurrenceAtMs: UTC_MS }, inputs: [] }, async (url, init) => { requestUrl = String(url); request = init; return new Response(JSON.stringify(simulationResponse()), { status: 200 }); });
    const body = JSON.parse(String(request?.body));
    expect(requestUrl).toBe('/api/schedules/simulate');
    expect(body).toEqual({ block_id: 'staircase', schedule: 'morning_on', occurrence_at_utc_ms: UTC_MS, expected_revision: '12', expected_structural_revision: '8' });
    expect(body.expected_structural_revision).toBe('8');
    expect(result.trigger.type).toBe('schedule');
    expect(result.timeContext).toMatchObject({ now: { year: 2026 } });
  });

  it('consumes paired opaque revisions from schedule conflicts and handles unknown schedules', async () => {
    const currentRevision = '18446744073709551615';
    const currentStructuralRevision = '9223372036854775808';
    await expect(simulateScenario({ blockId: 'staircase', expectedLogicRevision: 12, expectedStructuralRevision: 8, trigger: { type: 'schedule', schedule: 'morning_on', occurrenceAtMs: UTC_MS }, inputs: [] }, async () => new Response(JSON.stringify({ error: 'active schedule or block revision changed; refresh and run the schedule simulation again', current_revision: currentRevision, current_structural_revision: currentStructuralRevision }), { status: 409 }))).rejects.toMatchObject({ status: 409, message: 'The schedule definition changed. Refresh the dashboard and re-run the simulation.', currentRevision, currentStructuralRevision, currentLogicRevision: currentRevision });
    await expect(simulateScenario({ blockId: 'staircase', expectedLogicRevision: 12, expectedStructuralRevision: 8, trigger: { type: 'schedule', schedule: 'missing_schedule', occurrenceAtMs: UTC_MS }, inputs: [] }, async () => new Response(JSON.stringify({ error: 'unknown schedule' }), { status: 404 }))).rejects.toMatchObject({ status: 404, message: 'The selected block or schedule no longer exists. Refresh the dashboard.', currentRevision: null, currentStructuralRevision: null });
    await expect(simulateScenario({ blockId: 'staircase', expectedLogicRevision: 12, expectedStructuralRevision: 8, trigger: { type: 'schedule', schedule: 'morning_on', occurrenceAtMs: UTC_MS + 1 }, inputs: [] }, async () => new Response(JSON.stringify({ errors: [{ path: 'trigger.occurrence', message: 'not an occurrence' }] }), { status: 422 }))).rejects.toMatchObject({ status: 422, message: 'The selected occurrence is no longer a valid previewed occurrence. Pick a fresh occurrence and re-run.', fieldErrors: [{ path: 'trigger.occurrence', message: 'not an occurrence' }] });
    await expect(simulateScenario({ blockId: 'staircase', expectedLogicRevision: 12, trigger: { type: 'schedule', schedule: 'morning_on', occurrenceAtMs: UTC_MS }, inputs: [] }, async () => { throw new Error('the client must reject before posting'); })).rejects.toMatchObject({ status: 422, fieldErrors: [{ path: 'expected_structural_revision', message: 'required' }] });
  });

  it('fetches and decodes occurrence previews from the dedicated preview route', async () => {
    const previewBody = { block_id: 'staircase', schedule: 'morning_on', rule: { kind: 'astronomical', summary: 'sunrise - 1h30m' }, occurrences: [{ utc_ms: UTC_MS, local: '2026-08-29 05:30:00', utc_offset: 10_800, weekday: 'Saturday' }, { utc_ms: UTC_MS + 86_400_000, local: '2026-08-30 05:30:00', utc_offset: 10_800, weekday: 'Sunday' }] };
    let request: RequestInit | undefined;
    let requestUrl = '';
    const preview = await fetchScheduleOccurrences('staircase', 'morning_on', { count: 3 }, async (url, init) => { requestUrl = String(url); request = init; return new Response(JSON.stringify(previewBody), { status: 200 }); });
    expect(requestUrl).toBe('/api/schedules/preview');
    expect(JSON.parse(String(request?.body))).toEqual({ block_id: 'staircase', schedule: 'morning_on', count: 3 });
    expect(preview).toMatchObject({ blockId: 'staircase', schedule: 'morning_on', kind: 'astronomical', ruleSummary: 'sunrise - 1h30m' });
    expect(preview.occurrences[0]).toMatchObject({ utcMs: UTC_MS, local: '2026-08-29 05:30:00', utcOffsetSeconds: 10_800, weekday: 'Saturday' });
    await expect(fetchScheduleOccurrences('staircase', 'morning_on', async () => new Response(JSON.stringify({ block_id: 'staircase', schedule: 'morning_on', rule: { kind: 'fixed', summary: 'z' }, occurrences: 'nope' }), { status: 200 }))).rejects.toThrow(/occurrences/);
    await expect(fetchScheduleOccurrences('staircase', 'missing_schedule', async () => new Response(JSON.stringify({ error: 'unknown schedule' }), { status: 404 }))).rejects.toMatchObject({ status: 404, message: 'The selected block or schedule no longer exists. Refresh the dashboard.', currentRevision: null, currentStructuralRevision: null });
  });

  it('sends only the dedicated preview fields and preserves the selected wall-clock bound', async () => {
    let request: RequestInit | undefined;
    await fetchScheduleOccurrences('staircase', 'morning_on', { afterUtcMs: UTC_MS, count: 1 }, async (_url, init) => { request = init; return new Response(JSON.stringify({ block_id: 'staircase', schedule: 'morning_on', rule: { kind: 'fixed', summary: '06:00:00' }, occurrences: [] }), { status: 200 }); });
    expect(JSON.parse(String(request?.body))).toEqual({ block_id: 'staircase', schedule: 'morning_on', after_utc_ms: UTC_MS, count: 1 });
  });

  it('builds a schedule scenario from a draft defaulting to the next occurrence', () => {
    const decoded = decodeSnapshot(snapshot([block('staircase', [schedule('morning_on')])]), 1_000);
    const draft = createSimulationDraft(decoded, null);
    const scheduleDraft = { ...draft, triggerType: 'schedule' as const };
    const scenario = toSimulationScenario(scheduleDraft, 12, 8);
    expect(scenario?.trigger).toEqual({ type: 'schedule', schedule: 'morning_on', occurrenceAtMs: UTC_MS });
    expect(scenario?.expectedStructuralRevision).toBe(8);
  });

  it('does not leak another block\'s schedule state when the block changes', () => {
    const blockA = block('staircase', [schedule('morning_on'), schedule('evening_off')]);
    const heartbeatUtcMs = UTC_MS + 60_000;
    const blockB = block('utility_light', [schedule('heartbeat', { name: 'heartbeat', rule: { kind: 'interval', summary: 'every 60s' }, next_occurrence_utc_ms: heartbeatUtcMs, relative_ms: 60_000 })]);
    const decodedA = decodeSnapshot(snapshot([blockA, blockB]), 1_000);
    const draftA = createSimulationDraft(decodedA, null);
    const scheduleDraftA = { ...draftA, triggerType: 'schedule' as const, triggerScheduleName: 'morning_on', scheduleOccurrenceAtMs: UTC_MS, schedulePreviews: [{ utcMs: UTC_MS, local: '2026-08-29 05:30:00', utcOffsetSeconds: 10_800, weekday: 'Saturday' }] };
    expect(scheduleDraftA.triggerScheduleName).toBe('morning_on');
    expect(scheduleDraftA.scheduleOccurrenceAtMs).toBe(UTC_MS);
    const decodedB = decodeSnapshot(snapshot([blockB]), 1_000);
    const draftB = createSimulationDraft(decodedB, null);
    const scheduleDraftB = { ...draftB, triggerType: 'schedule' as const };
    expect(scheduleDraftB.blockId).toBe('utility_light');
    expect(scheduleDraftB.triggerScheduleName).not.toBe('morning_on');
    expect(scheduleDraftB.triggerScheduleName).toBe('heartbeat');
    expect(scheduleDraftB.schedulePreviews).toEqual([]);
    expect(scheduleDraftB.scheduleOccurrenceAtMs).toBe(heartbeatUtcMs);
    expect(createSimulationDraft(decodedB, null).triggerType).toBe('input');
  });

  it('reconciles a stale schedule selection after the block\'s schedules change', () => {
    const decoded = decodeSnapshot(snapshot([block('staircase', [schedule('heartbeat', { name: 'heartbeat', rule: { kind: 'interval', summary: 'every 60s' } })])]), 1_000);
    const heartbeatSchedules = decoded.blocks[0].schedules;
    const reconciled = reconcileScheduleSelection({ blockId: 'staircase', triggerType: 'schedule', triggerScheduleName: 'morning_on', scheduleOccurrenceAtMs: UTC_MS, schedulePreviews: [{ utcMs: UTC_MS, local: null, utcOffsetSeconds: null, weekday: null }], triggerEndpoint: '', triggerValue: null, previousValue: null, triggerTimerName: '', timerFiredAtMs: null, inputs: [], state: {}, pendingTimers: [] }, heartbeatSchedules);
    expect(reconciled.triggerScheduleName).toBe('heartbeat');
    expect(reconciled.schedulePreviews).toEqual([]);
    const kept = reconcileScheduleSelection(reconciled, heartbeatSchedules);
    expect(kept.triggerScheduleName).toBe('heartbeat');
  });

  it('defaults the occurrence selection to the next preview and keeps valid selections', () => {
    const previews = [{ utcMs: UTC_MS, local: null, utcOffsetSeconds: null, weekday: null }, { utcMs: UTC_MS + 86_400_000, local: null, utcOffsetSeconds: null, weekday: null }];
    expect(schedulePreviewDefault(previews, null)).toBe(UTC_MS);
    expect(schedulePreviewDefault(previews, UTC_MS + 86_400_000)).toBe(UTC_MS + 86_400_000);
    expect(schedulePreviewDefault(previews, UTC_MS + 1)).toBe(UTC_MS);
  });
});
