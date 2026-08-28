import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(fileURLToPath(new URL('.', import.meta.url)), '..', 'dist');
const port = Number(process.argv[2] ?? 8090);
const state = {
  phase: { kind: 'string', value: 'waiting_to_turn_off' },
  activation_count: { kind: 'integer', value: 2 }
};
const pendingTimers = [
  { name: 'dim', scheduled_at_ms: 800, due_at_ms: 5300, logic_revision: 12 },
  { name: 'off', scheduled_at_ms: 800, due_at_ms: 5800, logic_revision: 12 }
];
const base = {
  connection: { state: 'connected' },
  config: { active: { inputs: [{ name: 'wall_switch', dpt: { major: 1, subtype: 1 } }], outputs: [{ name: 'staircase_light', dpt: { major: 1, subtype: 1 } }], knx_bindings: [{ endpoint: 'wall_switch', group_address: '1/2/3' }, { endpoint: 'staircase_light', group_address: '1/2/4' }], logic: { source: 'function handle(event, input, meta, state) return nil end' } } },
  automation: { inputs: [{ name: 'wall_switch', dpt: { major: 1, subtype: 1 } }], outputs: [{ name: 'staircase_light', dpt: { major: 1, subtype: 1 } }], knx_bindings: [{ endpoint: 'wall_switch', group_address: '1/2/3' }, { endpoint: 'staircase_light', group_address: '1/2/4' }], logic: { source: 'function handle(event, input, meta, state) return nil end' } },
  active_automation_revision: 12, saved_automation_revision: 12,
  values: { endpoints: [{ name: 'wall_switch', direction: 'input', dpt: { major: 1, subtype: 1 }, observed: { kind: 'bool', value: true } }, { name: 'staircase_light', direction: 'output', dpt: { major: 1, subtype: 1 }, observed: { kind: 'bool', value: true }, requested: { kind: 'bool', value: true } }] },
  write: { status: 'idle', request_id: null, value: null, error: null }, telegrams: [], logs: []
};
function trigger() {
  return { type: 'timer', endpoint: '', dpt: { major: 0, subtype: 0 }, value: { kind: 'bool', value: false }, previous: null, changed: false, rising: false, falling: false, name: 'dim', scheduled_at_ms: 800, due_at_ms: 5300, fired_at_ms: 5312, late_by_ms: 12, scheduled_logic_revision: 12 };
}
function execution() {
  return { execution_id: 7, time_ms: 5312, duration_us: 42, logic_revision: 12, status: 'succeeded', trigger: trigger(), inputs: [{ endpoint: 'wall_switch', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true }, valid: true, age_ms: 5312 }], state_before: state, state_after: { ...state, phase: { kind: 'string', value: 'dimmed' } }, transition: { state: { phase: { kind: 'string', value: 'dimmed' } }, effects: [], timers: [{ name: 'off', action: 'replaced', previous_due_at_ms: 5800, after_ms: 5000, due_at_ms: 10312 }] }, effects: [], timer_effects: [], error: null };
}
function snapshot() {
  return { revision: 3, captured_at_ms: 1000, ...base, state, pending_timers: pendingTimers, logic: { active_logic_revision: 12, saved_logic_revision: 12, active_structural_revision: 1, saved_structural_revision: 1, restart_required: false, state, pending_timers: pendingTimers, executions: [execution()] } };
}
function simulation(body) {
  const timer = body.trigger?.type === 'timer';
  const selected = body.trigger?.name ?? 'off';
  const timerTrigger = { ...trigger(), name: selected, fired_at_ms: body.trigger?.fired_at_ms ?? 5800, late_by_ms: 0 };
  const inputs = (body.inputs ?? []).map((input) => ({ ...input, dpt: { major: 1, subtype: 1 } }));
  const nextTimers = timer ? [] : [{ name: 'dim', scheduled_at_ms: 1000, due_at_ms: 5500, logic_revision: 12 }, { name: 'off', scheduled_at_ms: 1000, due_at_ms: 6000, logic_revision: 12 }];
  return { logic_revision: 12, duration_us: 24, status: 'succeeded', trigger: timer ? timerTrigger : { type: 'input', endpoint: 'wall_switch', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true }, previous: { kind: 'bool', value: false }, changed: true, rising: true, falling: false }, inputs, state_before: body.state ?? state, state_after: { ...(body.state ?? state), phase: { kind: 'string', value: timer ? 'idle' : 'waiting_to_turn_off' } }, transition: { state: { phase: { kind: 'string', value: timer ? 'idle' : 'waiting_to_turn_off' } }, effects: timer ? [{ endpoint: 'staircase_light', destination: '1/2/4', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: false } }] : [{ endpoint: 'staircase_light', destination: '1/2/4', dpt: { major: 1, subtype: 1 }, value: { kind: 'bool', value: true } }], timers: timer ? [{ name: selected, action: 'cancelled', previous_due_at_ms: 5800 }] : [{ name: 'dim', action: 'scheduled', after_ms: 4500, due_at_ms: 5500 }, { name: 'off', action: 'scheduled', after_ms: 5000, due_at_ms: 6000 }] }, pending_timers: nextTimers, effects: [], timer_effects: [], error: null };
}
function sendJson(response, value, status = 200) { response.writeHead(status, { 'content-type': 'application/json' }); response.end(JSON.stringify(value)); }
const server = createServer(async (request, response) => {
  if (request.url?.startsWith('/api/snapshot')) return sendJson(response, snapshot());
  if (request.url?.startsWith('/api/automation')) {
    if (request.method === 'PUT') { for await (const _chunk of request) {} return sendJson(response, { revision: 13, logic_activated: true, active_logic_revision: 13, restart_required: false, cancelled_timers: ['dim', 'off'] }); }
    return sendJson(response, { document: base.automation, revision: 12, active_logic_revision: 12, saved_logic_revision: 12, restart_required: false });
  }
  if (request.url?.startsWith('/api/simulate')) { let raw = ''; for await (const chunk of request) raw += chunk; return sendJson(response, simulation(JSON.parse(raw))); }
  if (request.url?.startsWith('/api/events')) { response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', connection: 'keep-alive' }); response.write(': fixture connected\n\n'); request.on('close', () => response.end()); return; }
  const path = request.url === '/' ? '/index.html' : request.url;
  try { const data = await readFile(join(root, path)); const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css' }; response.writeHead(200, { 'content-type': types[extname(path)] ?? 'application/octet-stream' }); response.end(data); } catch { sendJson(response, { error: 'not found' }, 404); }
});
server.listen(port, '127.0.0.1', () => console.log(`fixture listening on http://127.0.0.1:${port}`));
