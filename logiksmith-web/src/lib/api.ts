import type {
  ConnectionState, DashboardEvent, DisplayAutomation, DisplayBinding, DisplayEndpoint, DisplayExecution,
  DisplayExecutionEffect, DisplayExecutionInput, DisplayExecutionTrigger, DisplayInputExecutionTrigger,
  DisplayBlock, DisplayBlockSchedule, DisplayDateTimeValue, DisplayLastResult, DisplayLogicError, DisplayLog, DisplayPendingTimer, DisplayScheduleExecutionTrigger, DisplayScheduleKind, DisplayScheduleOccurrence, DisplaySchedulePreview, DisplaySimulation, DisplaySiteTime, DisplaySnapshot, DisplayState,
  DisplayStateValue, DisplaySunContext, DisplayTelegram, DisplayTimeContext, DisplayTimer, DisplayTimerEffect, DisplayTimerEffectAction,
  DisplayTransition, DisplayTimerExecutionTrigger, DisplayWrite, SimulationScenario, SimulationTypedValue,
  TimerState, WriteStatus
} from './state';
import { encodeRevisionToken, parseRevisionToken, type RevisionToken } from './revision';

type JsonObject = Record<string, unknown>;
type JsonPrimitive = string | number | boolean | null;
type FetchLike = typeof fetch;
type EventSourceLike = { onopen: (() => void) | null; onerror: (() => void) | null; addEventListener(type: string, listener: (event: MessageEvent<string>) => void): void; close(): void; };
type EventSourceConstructor = new (url: string) => EventSourceLike;

export class ApiDecodeError extends Error { constructor(path: string, message: string) { super(`Malformed dashboard data at ${path}: ${message}`); this.name = 'ApiDecodeError'; } }
export interface SimulationFieldError { path: string; message: string; }
export class SimulationApiError extends Error {
  readonly status: number; readonly fieldErrors: SimulationFieldError[];
  /** The current active block revision returned by a schedule conflict. */
  readonly currentRevision: RevisionToken | null;
  /** The current active automation structure revision returned by a schedule conflict. */
  readonly currentStructuralRevision: RevisionToken | null;
  /**
   * Compatibility alias for the generic simulation endpoint's old conflict
   * field. Dedicated schedule conflicts expose currentRevision instead.
   */
  readonly currentLogicRevision: RevisionToken | null;
  constructor(status: number, message: string, fieldErrors: SimulationFieldError[] = [], currentRevision: RevisionToken | null = null, currentStructuralRevision: RevisionToken | null = null) {
    super(message);
    this.name = 'SimulationApiError';
    this.status = status;
    this.fieldErrors = fieldErrors;
    this.currentRevision = currentRevision;
    this.currentStructuralRevision = currentStructuralRevision;
    this.currentLogicRevision = currentRevision;
  }
}
const isObject = (value: unknown): value is JsonObject => typeof value === 'object' && value !== null && !Array.isArray(value);
function object(value: unknown, path: string): JsonObject { if (!isObject(value)) throw new ApiDecodeError(path, 'expected an object'); return value; }
function required(value: unknown, path: string): unknown { if (value === undefined || value === null) throw new ApiDecodeError(path, 'required field is missing'); return value; }
function nullableField(source: JsonObject, name: string, path: string): unknown { if (!(name in source)) throw new ApiDecodeError(path, 'required field is missing'); return source[name]; }
function field(record: JsonObject, _path: string, ...names: string[]): unknown { for (const name of names) if (name in record) return record[name]; return undefined; }
function stringValue(value: unknown, path: string): string { if (typeof value !== 'string' || value.length === 0) throw new ApiDecodeError(path, 'expected a non-empty string'); return value; }
function optionalString(value: unknown, path: string): string | null { return value === undefined || value === null ? null : stringValue(value, path); }
function finiteNumber(value: unknown, path: string): number { if (typeof value !== 'number' || !Number.isFinite(value)) throw new ApiDecodeError(path, 'expected a finite number'); return value; }
function nonNegativeNumber(value: unknown, path: string): number { const number = finiteNumber(value, path); if (number < 0) throw new ApiDecodeError(path, 'expected a non-negative number'); return number; }
function revision(value: unknown, path: string): number { const number = nonNegativeNumber(value, path); if (!Number.isInteger(number)) throw new ApiDecodeError(path, 'expected an integer'); return number; }
function integer(value: unknown, path: string): number { return revision(value, path); }
function signedInteger(value: unknown, path: string): number { const number = finiteNumber(value, path); if (!Number.isInteger(number)) throw new ApiDecodeError(path, 'expected an integer'); return number; }
function optionalInteger(value: unknown, path: string): number | null { return value === undefined || value === null ? null : integer(value, path); }
function optionalSignedInteger(value: unknown, path: string): number | null { return value === undefined || value === null ? null : signedInteger(value, path); }
function nullableBoolean(value: unknown, path: string): boolean | null { if (value === undefined || value === null) return null; if (typeof value !== 'boolean') throw new ApiDecodeError(path, 'expected a boolean or null'); return value; }
function array(value: unknown, path: string): unknown[] { if (!Array.isArray(value)) throw new ApiDecodeError(path, 'expected an array'); return value; }
function logicRevision(value: unknown, path: string): RevisionToken { const token = parseRevisionToken(value); if (token === null) throw new ApiDecodeError(path, 'expected a non-negative decimal logic revision token'); return token; }
function optionalLogicRevision(value: unknown, path: string): RevisionToken | null { return value === undefined || value === null ? null : logicRevision(value, path); }
function optionalRevision(value: unknown, path: string): number | null { return value === undefined || value === null ? null : revision(value, path); }
function timeValue(value: unknown, path: string): string { return typeof value === 'string' ? stringValue(value, path) : `${nonNegativeNumber(value, path)} ms`; }

function dpt(value: unknown, path: string): string {
  if (typeof value === 'string') return stringValue(value, path);
  const record = object(value, path);
  const major = revision(required(field(record, path, 'major'), `${path}.major`), `${path}.major`);
  const subtype = revision(required(field(record, path, 'subtype'), `${path}.subtype`), `${path}.subtype`);
  if (major === 0 || subtype > 999) throw new ApiDecodeError(path, 'DPT is out of range');
  return `${major}.${subtype.toString().padStart(3, '0')}`;
}
function endpoint(value: unknown, path: string): { address: string; dpt: string } {
  const record = object(value, path);
  const address = field(record, path, 'address', 'group_address', 'groupAddress');
  return { address: address === undefined || address === null ? '' : stringValue(address, `${path}.address`), dpt: dpt(required(field(record, path, 'dpt'), `${path}.dpt`), `${path}.dpt`) };
}
function connection(value: unknown): { state: ConnectionState } {
  const record = typeof value === 'string' ? { state: value } : object(value, 'connection');
  const state = stringValue(required(field(record, 'connection', 'state', 'status'), 'connection.state'), 'connection.state');
  if (!['starting', 'connecting', 'connected', 'disconnected', 'failed'].includes(state)) throw new ApiDecodeError('connection.state', `unsupported state ${state}`);
  return { state: state as ConnectionState };
}

function displayValue(value: unknown, path: string): boolean | number | null {
  if (value === undefined || value === null) return null;
  if (typeof value === 'boolean') return value;
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (isObject(value)) {
    const kind = field(value, path, 'kind');
    const raw = field(value, path, 'value', 'data');
    if (kind === 'bool') return nullableBoolean(raw, `${path}.value`);
    if (kind === 'percent') { const percentage = nonNegativeNumber(raw, `${path}.value`); if (percentage > 100) throw new ApiDecodeError(`${path}.value`, 'percentage must be between 0 and 100'); return percentage; }
  }
  throw new ApiDecodeError(path, 'expected a boolean, percentage, or null');
}
function valueFrom(value: unknown, path: string, name: string): boolean | number | null { return isObject(value) ? displayValue(field(value, path, name, 'value'), `${path}.${name}`) : displayValue(value, path); }

function stateValue(value: unknown, path: string): DisplayStateValue {
  const source = object(value, path);
  const kind = field(source, path, 'kind');
  const raw = required(field(source, path, 'value', 'data'), `${path}.value`);
  if (kind === 'bool') { if (typeof raw !== 'boolean') throw new ApiDecodeError(`${path}.value`, 'expected a boolean'); return { kind: 'bool', value: raw }; }
  if (kind === 'integer') { if (typeof raw !== 'number' || !Number.isSafeInteger(raw)) throw new ApiDecodeError(`${path}.value`, 'expected a safe integer'); return { kind: 'integer', value: raw }; }
  if (kind === 'number') return { kind: 'number', value: finiteNumber(raw, `${path}.value`) };
  if (kind === 'string') { if (typeof raw !== 'string') throw new ApiDecodeError(`${path}.value`, 'expected a string'); return { kind: 'string', value: raw }; }
  throw new ApiDecodeError(`${path}.kind`, 'expected bool, integer, number, or string');
}
function stateMap(value: unknown, path: string): DisplayState {
  if (value === undefined || value === null) return {};
  const source = object(value, path);
  return Object.fromEntries(Object.keys(source).sort().map((key) => [key, stateValue(source[key], `${path}.${key}`)]));
}

function timer(value: unknown, receivedAtMs: number): DisplayTimer {
  const record = object(value, 'timer');
  const stateValue = stringValue(required(field(record, 'timer', 'state', 'status'), 'timer.state'), 'timer.state');
  if (stateValue !== 'idle' && stateValue !== 'pending') throw new ApiDecodeError('timer.state', `unsupported state ${stateValue}`);
  const deadlineRaw = field(record, 'timer', 'deadline_ms', 'deadlineMs', 'off_deadline_ms', 'off_deadline');
  const remainingRaw = field(record, 'timer', 'remaining_ms', 'remainingMs');
  const referenceRaw = field(record, 'timer', 'time_reference_ms', 'timeReferenceMs', 'server_time_ms', 'serverTimeMs');
  return { state: stateValue as TimerState, deadlineMs: deadlineRaw === undefined || deadlineRaw === null ? null : nonNegativeNumber(deadlineRaw, 'timer.deadline_ms'), remainingMs: remainingRaw === undefined || remainingRaw === null ? null : nonNegativeNumber(remainingRaw, 'timer.remaining_ms'), sampledAtMs: referenceRaw === undefined || referenceRaw === null ? receivedAtMs : nonNegativeNumber(referenceRaw, 'timer.time_reference_ms') };
}
function pendingTimer(value: unknown, index: number, prefix = 'logic.pending_timers'): DisplayPendingTimer {
  const path = `${prefix}[${index}]`; const source = object(value, path);
  return { name: stringValue(required(field(source, path, 'name', 'timer'), `${path}.name`), `${path}.name`), scheduledAtMs: integer(required(field(source, path, 'scheduled_at_ms', 'scheduledAtMs'), `${path}.scheduled_at_ms`), `${path}.scheduled_at_ms`), dueAtMs: integer(required(field(source, path, 'due_at_ms', 'dueAtMs'), `${path}.due_at_ms`), `${path}.due_at_ms`), logicRevision: logicRevision(required(field(source, path, 'logic_revision', 'logicRevision'), `${path}.logic_revision`), `${path}.logic_revision`) };
}
function telegram(value: unknown, index: number): DisplayTelegram {
  const path = `telegrams[${index}]`; const record = object(value, path); const rawValue = field(record, path, 'value', 'data');
  return { time: timeValue(required(field(record, path, 'time', 'time_ms', 'timeMs', 'timestamp', 'at'), `${path}.time`), `${path}.time`), source: optionalString(field(record, path, 'source', 'source_address'), `${path}.source`), destination: stringValue(required(field(record, path, 'destination', 'destination_address'), `${path}.destination`), `${path}.destination`), service: stringValue(required(field(record, path, 'service'), `${path}.service`), `${path}.service`), dpt: dpt(required(field(record, path, 'dpt'), `${path}.dpt`), `${path}.dpt`), value: valueFrom(rawValue, path, 'value') };
}
function log(value: unknown, index: number): DisplayLog {
  const path = `logs[${index}]`; const record = object(value, path); const fieldsRaw = field(record, path, 'fields', 'context'); const fields: Record<string, JsonPrimitive> = {};
  if (fieldsRaw !== undefined && fieldsRaw !== null) { const source = object(fieldsRaw, `${path}.fields`); for (const [key, item] of Object.entries(source)) { if (item !== null && typeof item !== 'string' && typeof item !== 'number' && typeof item !== 'boolean') throw new ApiDecodeError(`${path}.fields.${key}`, 'expected a JSON primitive'); fields[key] = item as JsonPrimitive; } }
  return { time: timeValue(required(field(record, path, 'time', 'time_ms', 'timeMs', 'timestamp', 'at'), `${path}.time`), `${path}.time`), level: stringValue(required(field(record, path, 'level'), `${path}.level`), `${path}.level`), target: stringValue(required(field(record, path, 'target'), `${path}.target`), `${path}.target`), message: stringValue(required(field(record, path, 'message'), `${path}.message`), `${path}.message`), fields };
}
function write(value: unknown): DisplayWrite {
  if (value === undefined || value === null) return { status: 'idle', requestId: null, value: null, error: null };
  const record = typeof value === 'boolean' ? { value } : object(value, 'write'); const statusRaw = field(record, 'write', 'status', 'state'); const status: WriteStatus = statusRaw === undefined || statusRaw === null ? 'idle' : stringValue(statusRaw, 'write.status') as WriteStatus;
  if (!['idle', 'pending', 'succeeded', 'failed'].includes(status)) throw new ApiDecodeError('write.status', `unsupported status ${status}`);
  const requestRaw = field(record, 'write', 'request_id', 'requestId'); const blockRaw = field(record, 'write', 'block_id', 'blockId'); const executionRaw = field(record, 'write', 'execution_id', 'executionId'); return { status, requestId: requestRaw === undefined || requestRaw === null ? null : revision(requestRaw, 'write.request_id'), blockId: blockRaw === undefined || blockRaw === null ? null : stringValue(blockRaw, 'write.block_id'), executionId: executionRaw === undefined || executionRaw === null ? null : revision(executionRaw, 'write.execution_id'), value: displayValue(field(record, 'write', 'value'), 'write.value'), error: optionalString(field(record, 'write', 'error'), 'write.error') };
}

function endpointValues(values: JsonObject): Map<string, { observed: boolean | number | null; requested: boolean | number | null }> {
  const result = new Map<string, { observed: boolean | number | null; requested: boolean | number | null }>();
  const raw = field(values, 'values', 'endpoints', 'endpoint_values', 'endpointValues');
  const groups: Array<[string, unknown]> = [['inputs', field(values, 'values', 'inputs')], ['outputs', field(values, 'values', 'outputs')]].filter((entry): entry is [string, unknown] => entry[1] !== undefined);
  const add = (name: string, value: unknown, path: string): void => { const source = isObject(value) ? value : { observed: value }; result.set(name, { observed: displayValue(field(source, path, 'observed', 'value'), `${path}.observed`), requested: displayValue(field(source, path, 'requested'), `${path}.requested`) }); };
  if (isObject(raw)) for (const [name, value] of Object.entries(raw)) add(name, value, `values.endpoints.${name}`);
  else if (Array.isArray(raw)) raw.forEach((value, index) => { const source = object(value, `values.endpoints[${index}]`); const name = stringValue(required(field(source, `values.endpoints[${index}]`, 'name'), `values.endpoints[${index}].name`), `values.endpoints[${index}].name`); add(name, source, `values.endpoints[${index}]`); });
  groups.forEach(([groupName, group]) => { if (isObject(group)) Object.entries(group).forEach(([name, value]) => add(name, value, `values.${groupName}.${name}`)); else if (Array.isArray(group)) group.forEach((value, index) => { const source = object(value, `values.${groupName}[${index}]`); const name = stringValue(required(field(source, `values.${groupName}[${index}]`, 'name', 'endpoint'), `values.${groupName}[${index}].name`), `values.${groupName}[${index}].name`); add(name, source, `values.${groupName}[${index}]`); }); });
  return result;
}
function decodeDisplayAutomation(root: JsonObject, config: JsonObject, values: JsonObject): DisplayAutomation | undefined {
  const nested = isObject(field(root, 'snapshot', 'automation')) ? object(field(root, 'snapshot', 'automation'), 'automation') : null; const active = isObject(field(config, 'config', 'active')) ? object(field(config, 'config', 'active'), 'config.active') : null; const source = nested ?? active ?? config;
  const inputsRaw = field(source, 'automation', 'inputs') ?? field(root, 'snapshot', 'active_inputs', 'activeInputs'); const outputsRaw = field(source, 'automation', 'outputs') ?? field(root, 'snapshot', 'active_outputs', 'activeOutputs'); if (inputsRaw === undefined && outputsRaw === undefined) return undefined;
  const bindings = new Map<string, string>(); const bindingsRaw = field(source, 'automation', 'knx_bindings', 'bindings') ?? field(config, 'config', 'knx_bindings', 'bindings');
  if (bindingsRaw !== undefined) array(bindingsRaw, 'automation.knx_bindings').forEach((item, index) => { const binding = object(item, `automation.knx_bindings[${index}]`); const name = stringValue(required(field(binding, `automation.knx_bindings[${index}]`, 'endpoint'), `automation.knx_bindings[${index}].endpoint`), `automation.knx_bindings[${index}].endpoint`); const address = stringValue(required(field(binding, `automation.knx_bindings[${index}]`, 'group_address', 'groupAddress', 'address'), `automation.knx_bindings[${index}].group_address`), `automation.knx_bindings[${index}].group_address`); bindings.set(name, address); });
  const valuesByName = endpointValues(values); const display = (value: unknown, path: string, direction: 'input' | 'output'): DisplayEndpoint => { const item = object(value, path); const nameRaw = field(item, path, 'name', 'endpoint'); const name = nameRaw === undefined || nameRaw === null ? undefined : stringValue(nameRaw, `${path}.name`); const bindingRaw = field(item, path, 'binding'); const binding = isObject(bindingRaw) ? bindingRaw : null; const addressRaw = field(item, path, 'address', 'group_address', 'groupAddress') ?? (typeof bindingRaw === 'string' ? bindingRaw : binding ? field(binding, `${path}.binding`, 'address', 'group_address', 'groupAddress') : undefined); const mapped = name ? valuesByName.get(name) : undefined; return { ...(name ? { name } : {}), address: addressRaw === undefined || addressRaw === null ? (name ? bindings.get(name) ?? '' : '') : stringValue(addressRaw, `${path}.address`), dpt: dpt(required(field(item, path, 'dpt'), `${path}.dpt`), `${path}.dpt`), direction, observed: mapped?.observed ?? displayValue(field(item, path, 'observed'), `${path}.observed`), requested: direction === 'output' ? mapped?.requested ?? displayValue(field(item, path, 'requested'), `${path}.requested`) : undefined }; };
  const inputs = array(inputsRaw ?? [], 'automation.inputs').map((item, index) => display(item, `automation.inputs[${index}]`, 'input')); const outputs = array(outputsRaw ?? [], 'automation.outputs').map((item, index) => display(item, `automation.outputs[${index}]`, 'output')); const logicSource = field(source, 'automation', 'logic'); const logic = isObject(logicSource) ? stringValue(required(field(logicSource, 'automation.logic', 'source'), 'automation.logic.source'), 'automation.logic.source') : (typeof logicSource === 'string' ? logicSource : '');
  return { inputs, outputs, bindings: [...bindings.entries()].map(([endpoint, groupAddress]) => ({ endpoint, groupAddress } as DisplayBinding)), source: logic };
}
function logicError(value: unknown, path: string): DisplayLogicError { const source = object(value, path); const category = stringValue(required(field(source, path, 'category', 'kind', 'type'), `${path}.category`), `${path}.category`); const message = stringValue(required(field(source, path, 'message', 'error'), `${path}.message`), `${path}.message`); const lineRaw = field(source, path, 'line', 'line_number', 'lineNumber'); return { category, message, line: lineRaw === undefined || lineRaw === null ? null : revision(lineRaw, `${path}.line`) }; }
function typedValue(value: unknown, path: string): SimulationTypedValue { const source = object(value, path); const kind = field(source, path, 'kind'); const raw = required(field(source, path, 'value'), `${path}.value`); if (kind === 'bool') { if (typeof raw !== 'boolean') throw new ApiDecodeError(`${path}.value`, 'expected a boolean'); return { kind: 'bool', value: raw }; } if (kind === 'percent') { const percentage = nonNegativeNumber(raw, `${path}.value`); if (percentage > 100) throw new ApiDecodeError(`${path}.value`, 'percentage must be between 0 and 100'); return { kind: 'percent', value: percentage }; } throw new ApiDecodeError(`${path}.kind`, 'expected bool or percent'); }
function executionValue(value: unknown, path: string): boolean | number | null { if (value === null) return null; if (typeof value === 'boolean' || typeof value === 'number') return displayValue(value, path); return typedValue(value, path).value; }
function booleanField(source: JsonObject, path: string, name: string): boolean { const value = required(field(source, path, name), `${path}.${name}`); if (typeof value !== 'boolean') throw new ApiDecodeError(`${path}.${name}`, 'expected a boolean'); return value; }
// ---- Milestone 9: schedules and site time ----

function scheduleKind(value: unknown, path: string): DisplayScheduleKind {
  const kind = stringValue(value, path);
  if (kind !== 'fixed' && kind !== 'interval' && kind !== 'astronomical') throw new ApiDecodeError(path, `unsupported kind ${kind}`);
  return kind;
}
function dateTimeValue(value: unknown, path: string): DisplayDateTimeValue {
  const source = object(value, path);
  const available = booleanField(source, path, 'available');
  const fieldValue = (name: string): number | null => { const raw = nullableField(source, name, `${path}.${name}`); return raw === null ? null : integer(raw, `${path}.${name}`); };
  const year = fieldValue('year'); const month = fieldValue('month'); const day = fieldValue('day'); const hour = fieldValue('hour'); const minute = fieldValue('minute'); const second = fieldValue('second');
  const weekdayRaw = nullableField(source, 'weekday', `${path}.weekday`); const weekday = weekdayRaw === null ? null : stringValue(weekdayRaw, `${path}.weekday`);
  if (!available && (year !== null || month !== null || day !== null || hour !== null || minute !== null || second !== null || weekday !== null)) throw new ApiDecodeError(path, 'unavailable date-time values must expose null fields');
  if (available && (year === null || month === null || day === null || hour === null || minute === null || second === null || weekday === null)) throw new ApiDecodeError(path, 'available date-time values must expose every field');
  return { available, year, month, day, hour, minute, second, weekday };
}
function sunContext(value: unknown, path: string): DisplaySunContext {
  const source = object(value, path);
  const event = (name: string): DisplayDateTimeValue => dateTimeValue(required(field(source, path, name), `${path}.${name}`), `${path}.${name}`);
  const elevationRaw = field(source, path, 'elevation_degrees', 'elevationDegrees'); const azimuthRaw = field(source, path, 'azimuth_degrees', 'azimuthDegrees');
  return { dawn: event('dawn'), sunrise: event('sunrise'), sunset: event('sunset'), dusk: event('dusk'), elevationDegrees: elevationRaw === undefined || elevationRaw === null ? null : finiteNumber(elevationRaw, `${path}.elevation_degrees`), azimuthDegrees: azimuthRaw === undefined || azimuthRaw === null ? null : finiteNumber(azimuthRaw, `${path}.azimuth_degrees`) };
}
function timeContext(value: unknown, path: string): DisplayTimeContext {
  const source = object(value, path);
  return { now: dateTimeValue(required(field(source, path, 'now'), `${path}.now`), `${path}.now`), sun: sunContext(required(field(source, path, 'sun'), `${path}.sun`), `${path}.sun`) };
}
function scheduleRule(value: unknown, path: string): { kind: DisplayScheduleKind; summary: string } {
  const source = object(value, path);
  return { kind: scheduleKind(required(field(source, path, 'kind'), `${path}.kind`), `${path}.kind`), summary: stringValue(required(field(source, path, 'summary'), `${path}.summary`), `${path}.summary`) };
}
function scheduleOccurrence(value: unknown, path: string): DisplayScheduleOccurrence {
  const source = object(value, path);
  return { utcMs: integer(required(field(source, path, 'utc_ms', 'utcMs', 'utc'), `${path}.utc_ms`), `${path}.utc_ms`), local: optionalString(field(source, path, 'local'), `${path}.local`), utcOffsetSeconds: optionalSignedInteger(field(source, path, 'utc_offset', 'utcOffset'), `${path}.utc_offset`), weekday: optionalString(field(source, path, 'weekday'), `${path}.weekday`) };
}
function blockSchedule(value: unknown, index: number, prefix = 'schedules'): DisplayBlockSchedule {
  const path = `${prefix}[${index}]`; const source = object(value, path);
  const status = stringValue(required(field(source, path, 'status'), `${path}.status`), `${path}.status`);
  if (status !== 'active' && status !== 'paused' && status !== 'unavailable' && status !== 'clock_error') throw new ApiDecodeError(`${path}.status`, `unsupported status ${status}`);
  const rule = scheduleRule(required(field(source, path, 'rule'), `${path}.rule`), `${path}.rule`);
  const lastResultRaw = field(source, path, 'last_result', 'lastResult');
  const lastOutcome = lastResultRaw === undefined || lastResultRaw === null ? { status: 'none' as const, executionId: null, timeMs: null } : (() => {
    const item = object(lastResultRaw, `${path}.last_result`);
    const lastStatus = stringValue(required(field(item, `${path}.last_result`, 'status'), `${path}.last_result.status`), `${path}.last_result.status`);
    if (lastStatus !== 'delivered' && lastStatus !== 'failed') throw new ApiDecodeError(`${path}.last_result.status`, `unsupported status ${lastStatus}`);
    return { status: lastStatus as 'delivered' | 'failed', executionId: field(item, `${path}.last_result`, 'execution_id', 'executionId') === undefined || field(item, `${path}.last_result`, 'execution_id', 'executionId') === null ? null : revision(field(item, `${path}.last_result`, 'execution_id', 'executionId'), `${path}.last_result.execution_id`), timeMs: field(item, `${path}.last_result`, 'time_ms', 'timeMs') === undefined || field(item, `${path}.last_result`, 'time_ms', 'timeMs') === null ? null : integer(field(item, `${path}.last_result`, 'time_ms', 'timeMs'), `${path}.last_result.time_ms`) };
  })();
  return {
    name: stringValue(required(field(source, path, 'name'), `${path}.name`), `${path}.name`),
    enabled: booleanField(source, path, 'enabled'),
    status: status as DisplayBlockSchedule['status'],
    kind: rule.kind,
    ruleSummary: rule.summary,
    nextOccurrenceLocal: optionalString(field(source, path, 'next_occurrence', 'nextOccurrenceLocal', 'nextOccurrence'), `${path}.next_occurrence`),
    nextOccurrenceUtcMs: optionalInteger(field(source, path, 'next_occurrence_utc_ms', 'nextOccurrenceUtcMs'), `${path}.next_occurrence_utc_ms`),
    relativeMs: optionalInteger(field(source, path, 'relative_ms', 'relativeMs'), `${path}.relative_ms`),
    utcOffsetSeconds: optionalSignedInteger(field(source, path, 'utc_offset', 'utcOffset'), `${path}.utc_offset`),
    reason: optionalString(field(source, path, 'unavailable_reason', 'unavailableReason', 'reason'), `${path}.unavailable_reason`),
    lastOutcome,
    occurrences: []
  };
}
function siteTime(value: unknown, path: string): DisplaySiteTime {
  const source = object(value, path);
  const astronomy = stringValue(required(field(source, path, 'astronomy'), `${path}.astronomy`), `${path}.astronomy`);
  if (astronomy !== 'available' && astronomy !== 'unavailable') throw new ApiDecodeError(`${path}.astronomy`, `unsupported value ${astronomy}`);
  const coordinatesRaw = field(source, path, 'coordinates');
  let coordinates: { latitude: number; longitude: number } | null = null;
  if (coordinatesRaw !== undefined && coordinatesRaw !== null) {
    const item = object(coordinatesRaw, `${path}.coordinates`);
    const latitude = finiteNumber(required(field(item, `${path}.coordinates`, 'latitude'), `${path}.coordinates.latitude`), `${path}.coordinates.latitude`);
    const longitude = finiteNumber(required(field(item, `${path}.coordinates`, 'longitude'), `${path}.coordinates.longitude`), `${path}.coordinates.longitude`);
    if (latitude < -90 || latitude > 90) throw new ApiDecodeError(`${path}.coordinates.latitude`, 'must be between -90 and 90');
    if (longitude < -180 || longitude >= 180) throw new ApiDecodeError(`${path}.coordinates.longitude`, 'must be between -180 and 180');
    coordinates = { latitude, longitude };
  }
  return {
    timezone: stringValue(required(field(source, path, 'timezone'), `${path}.timezone`), `${path}.timezone`),
    localTime: optionalString(field(source, path, 'local_time', 'localTime'), `${path}.local_time`),
    utcOffsetSeconds: optionalSignedInteger(field(source, path, 'utc_offset', 'utcOffset'), `${path}.utc_offset`),
    coordinates,
    astronomy: astronomy as DisplaySiteTime['astronomy'],
    astronomyReason: optionalString(field(source, path, 'astronomy_reason', 'astronomyReason'), `${path}.astronomy_reason`),
    sun: { dawn: optionalString(field(source, path, 'dawn'), `${path}.dawn`), sunrise: optionalString(field(source, path, 'sunrise'), `${path}.sunrise`), sunset: optionalString(field(source, path, 'sunset'), `${path}.sunset`), dusk: optionalString(field(source, path, 'dusk'), `${path}.dusk`) },
    clockOk: booleanField(source, path, 'clock_ok'),
    schedulerOk: booleanField(source, path, 'scheduler_ok')
  };
}

function inputTrigger(source: JsonObject, path: string): DisplayInputExecutionTrigger {
  const current = executionValue(required(field(source, path, 'value'), `${path}.value`), `${path}.value`); if (current === null) throw new ApiDecodeError(`${path}.value`, 'trigger value cannot be null'); const previousRaw = source.previous === undefined && source.previous_value !== undefined ? source.previous_value : nullableField(source, 'previous', `${path}.previous`); const previous = executionValue(previousRaw, `${path}.previous`);
  return { type: 'input', endpoint: stringValue(required(field(source, path, 'endpoint'), `${path}.endpoint`), `${path}.endpoint`), dpt: dpt(required(field(source, path, 'dpt'), `${path}.dpt`), `${path}.dpt`), value: current, previous, changed: booleanField(source, path, 'changed'), rising: booleanField(source, path, 'rising'), falling: booleanField(source, path, 'falling') };
}
function timerTrigger(source: JsonObject, path: string): DisplayTimerExecutionTrigger {
  const scheduledAtMs = integer(required(field(source, path, 'scheduled_at_ms', 'scheduledAtMs', 'scheduled_at'), `${path}.scheduled_at_ms`), `${path}.scheduled_at_ms`); const dueAtMs = integer(required(field(source, path, 'due_at_ms', 'dueAtMs', 'due_at'), `${path}.due_at_ms`), `${path}.due_at_ms`); const firedAtMs = integer(required(field(source, path, 'fired_at_ms', 'firedAtMs', 'fired_at'), `${path}.fired_at_ms`), `${path}.fired_at_ms`); const lateRaw = field(source, path, 'late_by_ms', 'lateByMs', 'lateness_ms', 'lateness');
  const name = stringValue(required(field(source, path, 'name', 'timer'), `${path}.name`), `${path}.name`);
  return { type: 'timer', name, timer: name, scheduledAtMs, dueAtMs, firedAtMs, lateByMs: lateRaw === undefined || lateRaw === null ? Math.max(0, firedAtMs - dueAtMs) : nonNegativeNumber(lateRaw, `${path}.late_by_ms`), scheduledLogicRevision: logicRevision(required(field(source, path, 'scheduled_logic_revision', 'scheduledLogicRevision', 'logic_revision'), `${path}.scheduled_logic_revision`), `${path}.scheduled_logic_revision`) };
}
function scheduleTrigger(source: JsonObject, path: string): DisplayScheduleExecutionTrigger {
  const blockIdRaw = field(source, path, 'block_id', 'blockId');
  return {
    type: 'schedule',
    name: stringValue(required(field(source, path, 'name', 'schedule'), `${path}.name`), `${path}.name`),
    kind: scheduleKind(required(field(source, path, 'kind'), `${path}.kind`), `${path}.kind`),
    blockId: blockIdRaw === undefined || blockIdRaw === null ? null : stringValue(blockIdRaw, `${path}.block_id`),
    scheduledForUtcMs: integer(required(field(source, path, 'scheduled_for_utc_ms', 'scheduledForUtcMs'), `${path}.scheduled_for_utc_ms`), `${path}.scheduled_for_utc_ms`),
    detectedAtUtcMs: integer(required(field(source, path, 'detected_at_utc_ms', 'detectedAtUtcMs'), `${path}.detected_at_utc_ms`), `${path}.detected_at_utc_ms`),
    handledAtUtcMs: integer(required(field(source, path, 'handled_at_utc_ms', 'handledAtUtcMs'), `${path}.handled_at_utc_ms`), `${path}.handled_at_utc_ms`),
    lateByMs: nonNegativeNumber(required(field(source, path, 'late_by_ms', 'lateByMs'), `${path}.late_by_ms`), `${path}.late_by_ms`),
    queueDelayMs: nonNegativeNumber(required(field(source, path, 'queue_delay_ms', 'queueDelayMs'), `${path}.queue_delay_ms`), `${path}.queue_delay_ms`),
    coalescedCount: integer(required(field(source, path, 'coalesced_count', 'coalescedCount'), `${path}.coalesced_count`), `${path}.coalesced_count`),
    structuralRevision: logicRevision(required(field(source, path, 'structural_revision', 'structuralRevision'), `${path}.structural_revision`), `${path}.structural_revision`)
  };
}
function executionTrigger(value: unknown, path: string): DisplayExecutionTrigger {
  const source = object(value, path); const type = field(source, path, 'type'); if (type === 'schedule') return scheduleTrigger(source, path); if (type === 'timer' || field(source, path, 'name', 'timer') !== undefined && field(source, path, 'fired_at_ms', 'firedAtMs', 'fired_at') !== undefined) return timerTrigger(source, path); return inputTrigger(source, path);
}
function executionInput(value: unknown, path: string): DisplayExecutionInput { const source = object(value, path); const snapshotValue = executionValue(nullableField(source, 'value', `${path}.value`), `${path}.value`); const valid = booleanField(source, path, 'valid'); const ageRaw = nullableField(source, 'age_ms', `${path}.age_ms`); const ageMs = ageRaw === null ? null : integer(ageRaw, `${path}.age_ms`); if (valid !== (snapshotValue !== null)) throw new ApiDecodeError(path, 'valid must match value presence'); if (!valid && ageMs !== null) throw new ApiDecodeError(`${path}.age_ms`, 'invalid inputs must have a null age'); if (valid && ageMs === null) throw new ApiDecodeError(`${path}.age_ms`, 'valid inputs must have an age'); return { endpoint: stringValue(required(field(source, path, 'endpoint'), `${path}.endpoint`), `${path}.endpoint`), dpt: dpt(required(field(source, path, 'dpt'), `${path}.dpt`), `${path}.dpt`), value: snapshotValue, valid, ageMs }; }
function executionEffect(value: unknown, path: string): DisplayExecutionEffect { const source = object(value, path); const effectValue = executionValue(required(field(source, path, 'value'), `${path}.value`), `${path}.value`); if (effectValue === null) throw new ApiDecodeError(`${path}.value`, 'effect value cannot be null'); return { endpoint: stringValue(required(field(source, path, 'endpoint'), `${path}.endpoint`), `${path}.endpoint`), destination: stringValue(required(field(source, path, 'destination'), `${path}.destination`), `${path}.destination`), dpt: dpt(required(field(source, path, 'dpt'), `${path}.dpt`), `${path}.dpt`), value: effectValue }; }
function timerEffect(value: unknown, path: string): DisplayTimerEffect {
  const source = object(value, path); let actionRaw = field(source, path, 'action', 'operation', 'kind', 'type'); let details = source;
  if (isObject(actionRaw)) { details = { ...source, ...actionRaw }; actionRaw = field(actionRaw, path, 'action', 'operation', 'kind', 'type'); }
  if (actionRaw === undefined) for (const action of ['scheduled', 'replaced', 'cancelled', 'cancel_noop'] as const) if (action in source) { actionRaw = action; details = isObject(source[action]) ? { ...source, ...(source[action] as JsonObject) } : source; break; }
  if (!['scheduled', 'replaced', 'cancelled', 'cancel_noop'].includes(String(actionRaw))) throw new ApiDecodeError(`${path}.action`, 'unsupported timer action');
  const optionalMs = (names: string[], label: string): number | null => { const raw = field(details, path, ...names); return raw === undefined || raw === null ? null : nonNegativeNumber(raw, `${path}.${label}`); };
  return { name: stringValue(required(field(source, path, 'name', 'timer'), `${path}.name`), `${path}.name`), action: actionRaw as DisplayTimerEffectAction, afterMs: optionalMs(['after_ms', 'afterMs'], 'after_ms'), dueAtMs: optionalMs(['due_at_ms', 'dueAtMs'], 'due_at_ms'), previousDueAtMs: optionalMs(['previous_due_at_ms', 'previousDueAtMs'], 'previous_due_at_ms') };
}
function transition(value: unknown, path: string): DisplayTransition {
  const source = object(value, path); const state = stateMap(field(source, path, 'state', 'state_patch'), `${path}.state`); const outputsRaw = field(source, path, 'outputs', 'effects'); const timersRaw = field(source, path, 'timers', 'timer_effects', 'timerEffects'); const effects = outputsRaw === undefined || outputsRaw === null ? [] : array(outputsRaw, `${path}.outputs`).map((item, index) => executionEffect(item, `${path}.outputs[${index}]`)); const timers = timersRaw === undefined || timersRaw === null ? [] : array(timersRaw, `${path}.timers`).map((item, index) => timerEffect(item, `${path}.timers[${index}]`)); return { state, effects, timers };
}
function execution(value: unknown, index: number, blockId: string | null = null, prefix = 'logic.executions'): DisplayExecution {
  const path = `${prefix}[${index}]`; const source = object(value, path); const status = stringValue(required(field(source, path, 'status'), `${path}.status`), `${path}.status`); if (status !== 'succeeded' && status !== 'failed') throw new ApiDecodeError(`${path}.status`, `unsupported status ${status}`);
  const inputs = array(required(field(source, path, 'inputs'), `${path}.inputs`), `${path}.inputs`).map((item, inputIndex) => executionInput(item, `${path}.inputs[${inputIndex}]`)); const transitionRaw = field(source, path, 'transition'); const effectsRaw = field(source, path, 'effects'); const timerEffectsRaw = field(source, path, 'timer_effects', 'timerEffects'); const parsedTransition = transitionRaw === undefined || transitionRaw === null ? null : transition(transitionRaw, `${path}.transition`); const effects = parsedTransition?.effects ?? (effectsRaw === undefined || effectsRaw === null ? [] : array(effectsRaw, `${path}.effects`).map((item, effectIndex) => executionEffect(item, `${path}.effects[${effectIndex}]`))); const timerEffects = parsedTransition?.timers ?? (timerEffectsRaw === undefined || timerEffectsRaw === null ? [] : array(timerEffectsRaw, `${path}.timer_effects`).map((item, effectIndex) => timerEffect(item, `${path}.timer_effects[${effectIndex}]`))); if (status === 'failed' && (effects.length > 0 || timerEffects.length > 0)) throw new ApiDecodeError(path, 'failed executions cannot contain effects');
  const before = stateMap(field(source, path, 'state_before', 'stateBefore'), `${path}.state_before`); const after = stateMap(field(source, path, 'state_after', 'stateAfter'), `${path}.state_after`); return { blockId, executionId: integer(required(field(source, path, 'execution_id', 'executionId'), `${path}.execution_id`), `${path}.execution_id`), timeMs: integer(required(field(source, path, 'time_ms', 'timeMs'), `${path}.time_ms`), `${path}.time_ms`), durationUs: integer(required(field(source, path, 'duration_us', 'durationUs'), `${path}.duration_us`), `${path}.duration_us`), logicRevision: optionalLogicRevision(required(field(source, path, 'logic_revision', 'logicRevision'), `${path}.logic_revision`), `${path}.logic_revision`), status: status as DisplayExecution['status'], trigger: executionTrigger(required(field(source, path, 'trigger'), `${path}.trigger`), `${path}.trigger`), inputs, transition: parsedTransition, stateBefore: before, stateAfter: after, effects, timerEffects, timeContext: field(source, path, 'time_context', 'timeContext') === undefined || field(source, path, 'time_context', 'timeContext') === null ? null : timeContext(field(source, path, 'time_context', 'timeContext'), `${path}.time_context`), error: field(source, path, 'error') === undefined || field(source, path, 'error') === null ? null : logicError(field(source, path, 'error'), `${path}.error`) };
}

function decodeSimulationInput(value: unknown, index: number): DisplayExecutionInput { return executionInput(value, `inputs[${index}]`); }
function decodeSimulation(input: unknown): DisplaySimulation {
  const root = object(input, 'simulation'); const status = stringValue(required(field(root, 'simulation', 'status'), 'status'), 'status'); if (status !== 'succeeded' && status !== 'failed') throw new ApiDecodeError('status', `unsupported status ${status}`); const trigger = executionTrigger(required(field(root, 'simulation', 'trigger'), 'trigger'), 'trigger'); const inputs = array(required(field(root, 'simulation', 'inputs'), 'inputs'), 'inputs').map(decodeSimulationInput); const transitionRaw = field(root, 'simulation', 'transition'); const parsedTransition = transitionRaw === undefined || transitionRaw === null ? null : transition(transitionRaw, 'transition'); const effects = parsedTransition?.effects ?? (field(root, 'simulation', 'effects') === undefined ? [] : array(field(root, 'simulation', 'effects'), 'effects').map((item, index) => executionEffect(item, `effects[${index}]`))); const timerEffects = parsedTransition?.timers ?? (field(root, 'simulation', 'timer_effects', 'timerEffects') === undefined ? [] : array(field(root, 'simulation', 'timer_effects', 'timerEffects'), 'timer_effects').map((item, index) => timerEffect(item, `timer_effects[${index}]`))); if (status === 'failed' && (effects.length || timerEffects.length)) throw new ApiDecodeError('effects', 'failed simulations cannot contain effects'); const stateBefore = stateMap(field(root, 'simulation', 'state_before', 'stateBefore'), 'state_before'); const stateAfter = stateMap(field(root, 'simulation', 'state_after', 'stateAfter'), 'state_after'); const pendingRaw = field(root, 'simulation', 'pending_timers', 'pendingTimers'); const blockIdRaw = field(root, 'simulation', 'block_id', 'blockId'); return { blockId: blockIdRaw === undefined || blockIdRaw === null ? null : stringValue(blockIdRaw, 'block_id'), logicRevision: logicRevision(required(field(root, 'simulation', 'logic_revision', 'logicRevision'), 'logic_revision'), 'logic_revision'), durationUs: integer(required(field(root, 'simulation', 'duration_us', 'durationUs'), 'duration_us'), 'duration_us'), status: status as DisplaySimulation['status'], trigger, inputs, transition: parsedTransition, stateBefore, stateAfter, effects, timerEffects, timeContext: field(root, 'simulation', 'time_context', 'timeContext') === undefined || field(root, 'simulation', 'time_context', 'timeContext') === null ? null : timeContext(field(root, 'simulation', 'time_context', 'timeContext'), 'time_context'), pendingTimers: pendingRaw === undefined || pendingRaw === null ? [] : array(pendingRaw, 'pending_timers').map((item, index) => pendingTimer(item, index)), error: field(root, 'simulation', 'error') === undefined || field(root, 'simulation', 'error') === null ? null : logicError(field(root, 'simulation', 'error'), 'error') };
}
export { decodeSimulation };

function blockEndpoint(value: unknown, path: string, direction: 'input' | 'output', values: Map<string, { observed: boolean | number | null; requested: boolean | number | null }>, bindings: Map<string, string>): DisplayEndpoint {
  const source = object(value, path); const name = stringValue(required(field(source, path, 'name', 'endpoint'), `${path}.name`), `${path}.name`); const bound = field(source, path, 'binding'); const boundObject = isObject(bound) ? bound : null;
  const addressRaw = field(source, path, 'address', 'group_address', 'groupAddress') ?? (typeof bound === 'string' ? bound : boundObject ? field(boundObject, `${path}.binding`, 'address', 'group_address', 'groupAddress') : bindings.get(name)); const mapped = values.get(name);
  return { name, address: addressRaw === undefined || addressRaw === null ? '' : stringValue(addressRaw, `${path}.address`), dpt: dpt(required(field(source, path, 'dpt'), `${path}.dpt`), `${path}.dpt`), direction, observed: mapped?.observed ?? displayValue(field(source, path, 'observed', 'value'), `${path}.observed`), requested: direction === 'output' ? mapped?.requested ?? displayValue(field(source, path, 'requested'), `${path}.requested`) : undefined };
}
function lastResult(value: unknown, executions: DisplayExecution[], path: string): DisplayLastResult {
  if (value === undefined || value === null) { const newest = executions[0]; return newest ? { status: newest.status, executionId: newest.executionId, timeMs: newest.timeMs, error: newest.error } : { status: 'none', executionId: null, timeMs: null, error: null }; }
  const source = object(value, path); const statusRaw = field(source, path, 'status', 'state'); const status = statusRaw === 'never_run' || statusRaw === 'none' || statusRaw === undefined ? 'none' : statusRaw;
  if (status !== 'none' && status !== 'succeeded' && status !== 'failed') throw new ApiDecodeError(`${path}.status`, 'expected none, succeeded, or failed');
  const errorRaw = field(source, path, 'error'); const error = errorRaw === undefined || errorRaw === null ? null : logicError(errorRaw, `${path}.error`); const executionIdRaw = field(source, path, 'execution_id', 'executionId'); const timeRaw = field(source, path, 'time_ms', 'timeMs');
  return { status, executionId: executionIdRaw === undefined || executionIdRaw === null ? null : revision(executionIdRaw, `${path}.execution_id`), timeMs: timeRaw === undefined || timeRaw === null ? null : integer(timeRaw, `${path}.time_ms`), error };
}
function decodeDisplayBlock(value: unknown, index: number): DisplayBlock {
  const path = `blocks[${index}]`; const source = object(value, path); const id = stringValue(required(field(source, path, 'id'), `${path}.id`), `${path}.id`);
  const valuesRaw = field(source, path, 'values', 'endpoint_values', 'endpointValues'); const values = isObject(valuesRaw) ? endpointValues(valuesRaw) : new Map<string, { observed: boolean | number | null; requested: boolean | number | null }>(); const bindingsRaw = required(field(source, path, 'knx_bindings', 'bindings'), `${path}.knx_bindings`); const bindings = new Map<string, string>(); array(bindingsRaw, `${path}.knx_bindings`).forEach((item, bindingIndex) => { const itemSource = object(item, `${path}.knx_bindings[${bindingIndex}]`); const name = stringValue(required(field(itemSource, `${path}.knx_bindings[${bindingIndex}]`, 'endpoint', 'name'), `${path}.knx_bindings[${bindingIndex}].endpoint`), `${path}.knx_bindings[${bindingIndex}].endpoint`); const address = stringValue(required(field(itemSource, `${path}.knx_bindings[${bindingIndex}]`, 'group_address', 'groupAddress', 'address'), `${path}.knx_bindings[${bindingIndex}].group_address`), `${path}.knx_bindings[${bindingIndex}].group_address`); bindings.set(name, address); });
  const inputs = array(required(field(source, path, 'inputs'), `${path}.inputs`), `${path}.inputs`).map((item, endpointIndex) => blockEndpoint(item, `${path}.inputs[${endpointIndex}]`, 'input', values, bindings)); const outputs = array(required(field(source, path, 'outputs'), `${path}.outputs`), `${path}.outputs`).map((item, endpointIndex) => blockEndpoint(item, `${path}.outputs[${endpointIndex}]`, 'output', values, bindings)); const executions = array(required(field(source, path, 'executions'), `${path}.executions`), `${path}.executions`).map((item, executionIndex) => execution(item, executionIndex, id, `${path}.executions`)); const pendingRaw = field(source, path, 'pending_timers', 'pendingTimers'); const pendingTimers = pendingRaw === undefined || pendingRaw === null ? [] : array(pendingRaw, `${path}.pending_timers`).map((item, timerIndex) => pendingTimer(item, timerIndex, `${path}.pending_timers`)); const state = stateMap(field(source, path, 'state', 'transient_state', 'transientState'), `${path}.state`);
  const activeLogicRevision = optionalLogicRevision(field(source, path, 'active_logic_revision', 'activeLogicRevision'), `${path}.active_logic_revision`); const savedLogicRevision = optionalLogicRevision(field(source, path, 'saved_logic_revision', 'savedLogicRevision'), `${path}.saved_logic_revision`); const enabledRaw = field(source, path, 'active_enabled', 'activeEnabled', 'enabled'); const savedEnabledRaw = field(source, path, 'saved_enabled', 'savedEnabled', 'enabled'); if (enabledRaw !== undefined && typeof enabledRaw !== 'boolean') throw new ApiDecodeError(`${path}.active_enabled`, 'expected a boolean'); if (savedEnabledRaw !== undefined && typeof savedEnabledRaw !== 'boolean') throw new ApiDecodeError(`${path}.saved_enabled`, 'expected a boolean'); const activeEnabled = enabledRaw === undefined ? true : enabledRaw as boolean; const savedEnabled = savedEnabledRaw === undefined ? activeEnabled : savedEnabledRaw as boolean;
  const schedulesRaw = field(source, path, 'schedules');
  const schedules = schedulesRaw === undefined || schedulesRaw === null ? [] : array(schedulesRaw, `${path}.schedules`).map((item, scheduleIndex) => blockSchedule(item, scheduleIndex, `${path}.schedules`));
  if (schedules.length > 32) throw new ApiDecodeError(`${path}.schedules`, 'must contain at most 32 schedules');
  const seenSchedules = new Set<string>(); schedules.forEach((item, scheduleIndex) => { if (seenSchedules.has(item.name)) throw new ApiDecodeError(`${path}.schedules[${scheduleIndex}].name`, `duplicate schedule name ${item.name}`); seenSchedules.add(item.name); });
  const activeRevision = optionalLogicRevision(field(source, path, 'active_revision', 'activeRevision', 'active_logic_revision', 'activeLogicRevision'), `${path}.active_revision`);
  const savedRevision = optionalLogicRevision(field(source, path, 'saved_revision', 'savedRevision', 'saved_logic_revision', 'savedLogicRevision'), `${path}.saved_revision`);
  const errorRaw = field(source, path, 'last_error', 'lastError'); const lastError = errorRaw === undefined || errorRaw === null ? null : logicError(errorRaw, `${path}.last_error`); const summary = lastResult(field(source, path, 'last_result', 'lastResult'), executions, `${path}.last_result`); return { id, activeEnabled, savedEnabled, source: stringValue(required(field(source, path, 'source', 'logic'), `${path}.source`), `${path}.source`), inputs, outputs, bindings: [...bindings.entries()].map(([endpoint, groupAddress]) => ({ endpoint, groupAddress })), state, pendingTimers, schedules, executions, activeRevision: activeRevision ?? activeLogicRevision, savedRevision: savedRevision ?? savedLogicRevision, activeLogicRevision, savedLogicRevision, lastResult: summary, lastError: lastError ?? summary.error };
}
function blockSnapshot(root: JsonObject): DisplayBlock[] {
  const logicStatus = isObject(field(root, 'snapshot', 'logic')) ? object(field(root, 'snapshot', 'logic'), 'logic') : null; const raw = field(root, 'snapshot', 'blocks', 'logic_blocks', 'logicBlocks') ?? (logicStatus ? field(logicStatus, 'logic', 'blocks') : undefined); if (raw === undefined) return []; const rawBlocks = array(raw, 'blocks'); if (rawBlocks.length === 0) throw new ApiDecodeError('blocks', 'must contain at least one block'); if (rawBlocks.length > 64) throw new ApiDecodeError('blocks', 'must contain at most 64 blocks'); const decoded = rawBlocks.map((item, index) => decodeDisplayBlock(item, index)); const seen = new Set<string>(); decoded.forEach((item, index) => { if (!/^[a-z][a-z0-9_]*$/.test(item.id) || new TextEncoder().encode(item.id).byteLength > 64) throw new ApiDecodeError(`blocks[${index}].id`, 'invalid block ID'); if (seen.has(item.id)) throw new ApiDecodeError(`blocks[${index}].id`, `duplicate block id ${item.id}`); seen.add(item.id); }); return decoded;
}
function decodeMultiSnapshot(root: JsonObject, blocks: DisplayBlock[], receivedAtMs: number): DisplaySnapshot {
  const configRaw = field(root, 'snapshot', 'config'); const config = isObject(configRaw) ? configRaw : {}; const valuesRaw = field(root, 'snapshot', 'values'); const values = isObject(valuesRaw) ? valuesRaw : {}; const first = blocks[0]; const firstInput = first?.inputs[0]; const firstOutput = first?.outputs[0]; const connectionValue = required(field(root, 'snapshot', 'connection'), 'connection'); const telegrams = array(required(field(root, 'snapshot', 'telegrams'), 'telegrams'), 'telegrams').map(telegram); const logs = array(required(field(root, 'snapshot', 'logs'), 'logs'), 'logs').map(log);
  const logicStatus = isObject(field(root, 'snapshot', 'logic')) ? object(field(root, 'snapshot', 'logic'), 'logic') : null; const readLogic = (names: string[]) => logicStatus ? field(logicStatus, 'logic', ...names) : undefined; const activeStructuralRevision = optionalLogicRevision(field(root, 'snapshot', 'active_structural_revision', 'activeStructuralRevision') ?? readLogic(['active_structural_revision', 'activeStructuralRevision']), 'active_structural_revision'); const savedStructuralRevision = optionalLogicRevision(field(root, 'snapshot', 'saved_structural_revision', 'savedStructuralRevision') ?? readLogic(['saved_structural_revision', 'savedStructuralRevision']), 'saved_structural_revision'); const activeLogicRevision = optionalLogicRevision(field(root, 'snapshot', 'active_logic_revision', 'activeLogicRevision') ?? readLogic(['active_logic_revision', 'activeLogicRevision']), 'active_logic_revision'); const savedLogicRevision = optionalLogicRevision(field(root, 'snapshot', 'saved_logic_revision', 'savedLogicRevision') ?? readLogic(['saved_logic_revision', 'savedLogicRevision']), 'saved_logic_revision'); const explicitRestart = field(root, 'snapshot', 'restart_required', 'restartRequired') ?? readLogic(['restart_required', 'restartRequired']); const restartRequired = explicitRestart === true || (activeStructuralRevision !== null && savedStructuralRevision !== null && String(activeStructuralRevision) !== String(savedStructuralRevision));
  const capturedRaw = field(root, 'snapshot', 'captured_at_ms', 'capturedAtMs') ?? readLogic(['captured_at_ms', 'capturedAtMs']); const capturedAtMs = capturedRaw === undefined || capturedRaw === null ? receivedAtMs : nonNegativeNumber(capturedRaw, 'captured_at_ms'); const clockOffsetMs = capturedRaw === undefined || capturedRaw === null || Math.abs(receivedAtMs - capturedAtMs) <= 86_400_000 ? 0 : receivedAtMs - capturedAtMs; const pendingTimers = blocks.flatMap((item) => item.pendingTimers); const executions = blocks.flatMap((item) => item.executions).sort((a, b) => b.executionId - a.executionId); const inputValue = firstInput?.observed ?? null; const outputValue = firstOutput?.observed ?? null; const requested = firstOutput?.requested ?? null; const firstAutomation = first ? { inputs: first.inputs, outputs: first.outputs, bindings: first.bindings, source: first.source } : undefined;
  const siteTimeRaw = field(root, 'snapshot', 'site_time', 'siteTime'); const siteTimeValue = siteTimeRaw === undefined || siteTimeRaw === null ? null : siteTime(siteTimeRaw, 'site_time');
  const configInput = field(config, 'input'); const configOutput = field(config, 'output'); const inputEndpoint = configInput === undefined ? { address: firstInput?.address ?? '', dpt: firstInput?.dpt ?? '1.001' } : endpoint(configInput, 'config.input'); const outputEndpoint = configOutput === undefined ? { address: firstOutput?.address ?? '', dpt: firstOutput?.dpt ?? '1.001' } : endpoint(configOutput, 'config.output'); const offDelayRaw = field(config, 'off_delay_ms', 'offDelayMs', 'off_delay');
  const revisionRaw = required(field(root, 'snapshot', 'revision'), 'revision'); return { revision: revision(revisionRaw, 'revision'), connection: connection(connectionValue), config: { input: inputEndpoint, output: outputEndpoint, offDelayMs: offDelayRaw === undefined || offDelayRaw === null ? 0 : nonNegativeNumber(offDelayRaw, 'config.off_delay_ms') }, values: { input: { observed: inputValue }, output: { observed: outputValue, requested } }, automation: firstAutomation, activeAutomationRevision: optionalRevision(field(root, 'snapshot', 'active_automation_revision', 'activeAutomationRevision'), 'active_automation_revision'), savedAutomationRevision: optionalRevision(field(root, 'snapshot', 'saved_automation_revision', 'savedAutomationRevision'), 'saved_automation_revision'), activeStructuralRevision, savedStructuralRevision, activeLogicRevision, savedLogicRevision, restartRequired, capturedAtMs, clockOffsetMs, state: first?.state ?? {}, pendingTimers, executions, siteTime: siteTimeValue, receivedAtMs, write: write(field(root, 'snapshot', 'write') ?? field(root, 'snapshot', 'last_write')), timer: { state: pendingTimers.length ? 'pending' : 'idle', deadlineMs: pendingTimers[0]?.dueAtMs ?? null, remainingMs: null, sampledAtMs: capturedAtMs }, telegrams, logs, blocks };
}

export function decodeSnapshot(input: unknown, receivedAtMs = Date.now()): DisplaySnapshot {
  const root = object(input, 'snapshot');
  const blocks = blockSnapshot(root);
  if (blocks.length > 0 || field(root, 'snapshot', 'blocks', 'logic_blocks', 'logicBlocks') !== undefined) return decodeMultiSnapshot(root, blocks, receivedAtMs);
  const config = object(required(field(root, 'snapshot', 'config'), 'config'), 'config');
  const values = object(required(field(root, 'snapshot', 'values'), 'values'), 'values');
  const endpointValuesByName = endpointValues(values);
  const inputValuesRaw = field(values, 'values', 'input');
  const outputValuesRaw = field(values, 'values', 'output');
  const inputValues = inputValuesRaw === undefined ? {} : object(inputValuesRaw, 'values.input');
  const outputValues = outputValuesRaw === undefined ? {} : object(outputValuesRaw, 'values.output');
  const telegrams = array(required(field(root, 'snapshot', 'telegrams'), 'telegrams'), 'telegrams').map(telegram);
  const logs = array(required(field(root, 'snapshot', 'logs'), 'logs'), 'logs').map(log);
  const automation = decodeDisplayAutomation(root, config, values);
  const firstInput = automation?.inputs[0];
  const firstOutput = automation?.outputs[0];
  const inputRaw = field(config, 'config', 'input') ?? firstInput;
  const outputRaw = field(config, 'config', 'output') ?? firstOutput;
  const inputEndpoint = inputRaw === undefined ? { address: '', dpt: '1.001' } : (field(config, 'config', 'input') === undefined && firstInput ? firstInput : endpoint(inputRaw, 'config.input'));
  const outputEndpoint = outputRaw === undefined ? { address: '', dpt: '1.001' } : (field(config, 'config', 'output') === undefined && firstOutput ? firstOutput : endpoint(outputRaw, 'config.output'));
  const offDelayRaw = field(config, 'config', 'off_delay_ms', 'offDelayMs', 'off_delay');
  const logicStatus = isObject(field(root, 'snapshot', 'logic')) ? object(field(root, 'snapshot', 'logic'), 'logic') : null;
  const readLogic = (names: string[]) => logicStatus ? field(logicStatus, 'logic', ...names) : undefined;
  const activeRevisionRaw = field(root, 'snapshot', 'active_automation_revision', 'activeAutomationRevision') ?? field(config, 'config', 'active_automation_revision', 'activeAutomationRevision');
  const savedRevisionRaw = field(root, 'snapshot', 'saved_automation_revision', 'savedAutomationRevision') ?? field(config, 'config', 'saved_automation_revision', 'savedAutomationRevision');
  const activeStructuralRevision = optionalLogicRevision(field(root, 'snapshot', 'active_structural_revision', 'activeStructuralRevision') ?? readLogic(['active_structural_revision', 'activeStructuralRevision']) ?? field(config, 'config', 'active_structural_revision', 'activeStructuralRevision'), 'active_structural_revision');
  const savedStructuralRevision = optionalLogicRevision(field(root, 'snapshot', 'saved_structural_revision', 'savedStructuralRevision') ?? readLogic(['saved_structural_revision', 'savedStructuralRevision']) ?? field(config, 'config', 'saved_structural_revision', 'savedStructuralRevision'), 'saved_structural_revision');
  const activeLogicRevision = optionalLogicRevision(field(root, 'snapshot', 'active_logic_revision', 'activeLogicRevision') ?? readLogic(['active_logic_revision', 'activeLogicRevision']), 'active_logic_revision');
  const savedLogicRevision = optionalLogicRevision(field(root, 'snapshot', 'saved_logic_revision', 'savedLogicRevision') ?? readLogic(['saved_logic_revision', 'savedLogicRevision']), 'saved_logic_revision');
  const executions = logicStatus ? array(required(field(logicStatus, 'logic', 'executions'), 'logic.executions'), 'logic.executions').map((item, index) => execution(item, index)) : [];
  const rootPendingRaw = field(root, 'snapshot', 'pending_timers', 'pendingTimers');
  const pendingRaw = logicStatus ? field(logicStatus, 'logic', 'pending_timers', 'pendingTimers') ?? rootPendingRaw : rootPendingRaw;
  const pendingTimers = pendingRaw === undefined || pendingRaw === null ? [] : array(pendingRaw, 'pending_timers').map((item, index) => pendingTimer(item, index));
  const legacyTimerRaw = field(root, 'snapshot', 'timer');
  const legacyTimer = legacyTimerRaw === undefined ? { state: 'idle' as const, deadlineMs: null, remainingMs: null, sampledAtMs: receivedAtMs } : timer(legacyTimerRaw, receivedAtMs);
  const capturedRaw = field(root, 'snapshot', 'captured_at_ms', 'capturedAtMs') ?? readLogic(['captured_at_ms', 'capturedAtMs']);
  const capturedAtMs = capturedRaw === undefined || capturedRaw === null ? receivedAtMs : nonNegativeNumber(capturedRaw, 'captured_at_ms');
  const clockOffsetMs = capturedRaw === undefined || capturedRaw === null || Math.abs(receivedAtMs - capturedAtMs) <= 86_400_000 ? 0 : receivedAtMs - capturedAtMs;
  const inputObserved = inputValuesRaw === undefined && firstInput?.name ? endpointValuesByName.get(firstInput.name)?.observed ?? null : valueFrom(inputValues, 'values.input', 'observed');
  const outputObserved = outputValuesRaw === undefined && firstOutput?.name ? endpointValuesByName.get(firstOutput.name)?.observed ?? null : valueFrom(outputValues, 'values.output', 'observed');
  const outputRequested = outputValuesRaw === undefined && firstOutput?.name ? endpointValuesByName.get(firstOutput.name)?.requested ?? null : valueFrom(outputValues, 'values.output', 'requested');
  const activeAutomationRevision = optionalRevision(activeRevisionRaw, 'active_automation_revision');
  const savedAutomationRevision = optionalRevision(savedRevisionRaw, 'saved_automation_revision');
  const explicitRestart = field(root, 'snapshot', 'restart_required', 'restartRequired') ?? readLogic(['restart_required', 'restartRequired']) ?? field(config, 'config', 'restart_required', 'restartRequired');
  const hasStructuralRevisions = activeStructuralRevision !== null && savedStructuralRevision !== null;
  const restartRequired = explicitRestart === true || (hasStructuralRevisions ? String(activeStructuralRevision) !== String(savedStructuralRevision) : activeAutomationRevision !== null && savedAutomationRevision !== null && activeAutomationRevision !== savedAutomationRevision);
  const stateRaw = field(root, 'snapshot', 'state') ?? readLogic(['state', 'transient_state', 'transientState']);
  const siteTimeRaw = field(root, 'snapshot', 'site_time', 'siteTime'); const siteTimeValue = siteTimeRaw === undefined || siteTimeRaw === null ? null : siteTime(siteTimeRaw, 'site_time');
  return {
    revision: revision(required(field(root, 'snapshot', 'revision'), 'revision'), 'revision'),
    connection: connection(required(field(root, 'snapshot', 'connection'), 'connection')),
    config: { input: inputEndpoint, output: outputEndpoint, offDelayMs: offDelayRaw === undefined || offDelayRaw === null ? 0 : nonNegativeNumber(offDelayRaw, 'config.off_delay_ms') },
    values: { input: { observed: inputObserved }, output: { observed: outputObserved, requested: outputRequested } },
    automation, activeAutomationRevision, savedAutomationRevision, activeStructuralRevision, savedStructuralRevision, activeLogicRevision, savedLogicRevision,
    restartRequired, capturedAtMs, clockOffsetMs, state: stateMap(stateRaw, 'state'), pendingTimers, executions, siteTime: siteTimeValue, receivedAtMs,
    write: write(field(root, 'snapshot', 'write') ?? field(root, 'write', 'write_status', 'last_write')),
    timer: legacyTimer, telegrams: telegrams as DisplayTelegram[], logs: logs as DisplayLog[], blocks: [{ id: 'default', activeEnabled: true, savedEnabled: true, source: automation?.source ?? '', inputs: automation?.inputs ?? [], outputs: automation?.outputs ?? [], bindings: automation?.bindings ?? [], state: stateMap(stateRaw, 'state'), pendingTimers, schedules: [], executions, activeRevision: activeLogicRevision, savedRevision: savedLogicRevision, activeLogicRevision, savedLogicRevision, lastResult: executions[0] ? { status: executions[0].status, executionId: executions[0].executionId, timeMs: executions[0].timeMs, error: executions[0].error } : { status: 'none', executionId: null, timeMs: null, error: null }, lastError: executions[0]?.error ?? null }]
  };
}
function parseJson(value: string, path: string): unknown { try { return JSON.parse(value) as unknown; } catch { throw new ApiDecodeError(path, 'expected valid JSON'); } }
export function decodeEvent(input: unknown, eventName = 'update', eventId?: string): DashboardEvent { const root = object(typeof input === 'string' ? parseJson(input, 'event.data') : input, 'event'); const eventRevision = revision(required(field(root, 'event', 'revision') ?? eventId, 'event.revision'), 'event.revision'); if (eventName === 'resync') return { kind: 'resync', revision: eventRevision }; if (eventName !== 'update') throw new ApiDecodeError('event', `unsupported event ${eventName}`); const snapshot = decodeSnapshot(required(field(root, 'event', 'snapshot'), 'event.snapshot')); if (snapshot.revision !== eventRevision) throw new ApiDecodeError('event.revision', 'does not match event.snapshot.revision'); return { kind: 'update', revision: eventRevision, snapshot }; }
function jsonOrNull(response: Response): Promise<unknown> { return response.json().catch(() => null); }
function simulationFieldErrors(value: unknown): SimulationFieldError[] { if (!isObject(value)) return []; const raw = value.errors ?? value.field_errors ?? value.fields; if (Array.isArray(raw)) return raw.flatMap((item) => isObject(item) && (typeof item.path === 'string' || typeof item.field === 'string') && typeof item.message === 'string' ? [{ path: (item.path ?? item.field) as string, message: item.message }] : []); if (isObject(raw)) return Object.entries(raw).flatMap(([path, message]) => typeof message === 'string' ? [{ path, message }] : []); return []; }
function simulationErrorMessage(status: number, body: unknown, blockScoped = false): string { if (status === 404) return 'The selected logic block no longer exists. Refresh the dashboard.'; if (status === 409) return blockScoped ? 'The selected block source changed. Refresh the dashboard and re-run the simulation.' : 'The active logic source changed. Refresh the dashboard and re-run the simulation.'; if (status === 422) return 'The simulation scenario is invalid. Fix the highlighted fields and re-run.'; if (isObject(body) && typeof body.error === 'string') return body.error; return `Simulation request failed (${status})`; }
function schedulePreviewErrorMessage(status: number, body: unknown): string { if (status === 404) return 'The selected block or schedule no longer exists. Refresh the dashboard.'; if (status === 422) return 'The schedule cannot be previewed with the current site clock. Fix the rule or refresh the dashboard.'; if (isObject(body) && typeof body.error === 'string') return body.error; return `Schedule preview request failed (${status})`; }
export async function simulateScenario(scenario: SimulationScenario, fetchImpl: FetchLike = fetch): Promise<DisplaySimulation> {
  if (scenario.trigger.type === 'schedule') {
    if (!scenario.blockId) throw new SimulationApiError(422, 'A block is required for schedule simulation.', [{ path: 'block_id', message: 'required' }]);
    if (scenario.trigger.occurrenceAtMs === null) throw new SimulationApiError(422, 'Pick a previewed occurrence for the schedule.', [{ path: 'occurrence_at_utc_ms', message: 'required' }]);
    if (scenario.expectedStructuralRevision === undefined || scenario.expectedStructuralRevision === null) throw new SimulationApiError(422, 'The active structural revision is unavailable. Refresh the dashboard before simulating.', [{ path: 'expected_structural_revision', message: 'required' }]);
    const response = await fetchImpl('/api/schedules/simulate', { method: 'POST', headers: { accept: 'application/json', 'content-type': 'application/json' }, body: JSON.stringify({ block_id: scenario.blockId, schedule: scenario.trigger.schedule, occurrence_at_utc_ms: scenario.trigger.occurrenceAtMs, expected_revision: encodeRevisionToken(scenario.expectedLogicRevision), expected_structural_revision: encodeRevisionToken(scenario.expectedStructuralRevision) }) });
    if (response.ok) return decodeSimulation(await response.json());
    const errorBody = await jsonOrNull(response);
    const currentRevisionRaw = isObject(errorBody) ? field(errorBody, 'schedule simulation conflict', 'current_revision', 'currentRevision') : undefined;
    const currentStructuralRevisionRaw = isObject(errorBody) ? field(errorBody, 'schedule simulation conflict', 'current_structural_revision', 'currentStructuralRevision') : undefined;
    const currentRevision = currentRevisionRaw === undefined || currentRevisionRaw === null ? null : parseRevisionToken(currentRevisionRaw);
    const currentStructuralRevision = currentStructuralRevisionRaw === undefined || currentStructuralRevisionRaw === null ? null : parseRevisionToken(currentStructuralRevisionRaw);
    throw new SimulationApiError(response.status, scheduleSimulationErrorMessage(response.status, errorBody), simulationFieldErrors(errorBody), currentRevision, currentStructuralRevision);
  }
  const trigger = scenario.trigger.type === 'timer' ? { type: 'timer', name: scenario.trigger.name ?? scenario.trigger.timer, fired_at_ms: scenario.trigger.firedAtMs } : { ...(scenario.trigger.type ? { type: 'input' } : {}), endpoint: scenario.trigger.endpoint, value: scenario.trigger.value, previous: scenario.trigger.previous };
  const body: Record<string, unknown> = { ...(scenario.blockId ? { block_id: scenario.blockId } : {}), expected_logic_revision: encodeRevisionToken(scenario.expectedLogicRevision), trigger, inputs: scenario.inputs.map((input) => ({ endpoint: input.endpoint, value: input.value, valid: input.valid, age_ms: input.ageMs })) };
  if (scenario.expectedStructuralRevision !== undefined && scenario.expectedStructuralRevision !== null) body.expected_structural_revision = encodeRevisionToken(scenario.expectedStructuralRevision);
  if (scenario.state !== undefined) body.state = scenario.state;
  if (scenario.pendingTimers !== undefined) body.pending_timers = scenario.pendingTimers.map((timer) => ({ name: timer.name, scheduled_at_ms: timer.scheduledAtMs, due_at_ms: timer.dueAtMs, logic_revision: encodeRevisionToken(timer.logicRevision) }));
  const response = await fetchImpl('/api/simulate', { method: 'POST', headers: { accept: 'application/json', 'content-type': 'application/json' }, body: JSON.stringify(body) });
  if (response.ok) return decodeSimulation(await response.json()); const errorBody = await jsonOrNull(response); const currentRaw = isObject(errorBody) ? field(errorBody, 'simulation conflict', 'current_logic_revision', 'currentLogicRevision') : undefined; const currentLogicRevision = currentRaw === undefined || currentRaw === null ? null : parseRevisionToken(currentRaw); throw new SimulationApiError(response.status, simulationErrorMessage(response.status, errorBody, Boolean(scenario.blockId)), simulationFieldErrors(errorBody), currentLogicRevision);
}
function scheduleSimulationErrorMessage(status: number, body: unknown): string { if (status === 404) return 'The selected block or schedule no longer exists. Refresh the dashboard.'; if (status === 409) return 'The schedule definition changed. Refresh the dashboard and re-run the simulation.'; if (status === 422) return 'The selected occurrence is no longer a valid previewed occurrence. Pick a fresh occurrence and re-run.'; if (isObject(body) && typeof body.error === 'string') return body.error; return `Simulation request failed (${status})`; }
export interface SchedulePreviewOptions {
  /** Wall-clock sample used as the lower bound for the preview. */
  afterUtcMs?: number;
  /** Number of occurrences requested; the desktop accepts 1 through 10. */
  count?: number;
}

/**
 * Fetch a stateless preview through the dedicated schedule endpoint. The
 * function keeps accepting a fetch implementation as its third argument for
 * the existing test seam; production callers may provide the preview window
 * and count as SchedulePreviewOptions.
 */
export async function fetchScheduleOccurrences(blockId: string, schedule: string, optionsOrFetch: SchedulePreviewOptions | FetchLike = {}, maybeFetch: FetchLike = fetch): Promise<DisplaySchedulePreview> {
  const legacyFetch = typeof optionsOrFetch === 'function';
  const options = legacyFetch ? {} : optionsOrFetch;
  const fetchImpl = legacyFetch ? optionsOrFetch : maybeFetch;
  const body: Record<string, unknown> = {
    block_id: blockId,
    schedule,
  };
  if (options.afterUtcMs !== undefined) body.after_utc_ms = options.afterUtcMs;
  if (options.count !== undefined) body.count = options.count;
  const response = await fetchImpl('/api/schedules/preview', { method: 'POST', headers: { accept: 'application/json', 'content-type': 'application/json' }, body: JSON.stringify(body) });
  if (response.ok) {
    const preview = decodeSchedulePreview(await response.json());
    if (preview.blockId !== blockId) throw new ApiDecodeError('block_id', 'does not match the requested block');
    if (preview.schedule !== schedule) throw new ApiDecodeError('schedule', 'does not match the requested schedule');
    return preview;
  }
  const errorBody = await jsonOrNull(response);
  throw new SimulationApiError(response.status, schedulePreviewErrorMessage(response.status, errorBody), simulationFieldErrors(errorBody));
}
function decodeSchedulePreview(input: unknown): DisplaySchedulePreview {
  const root = object(input, 'simulation');
  const rule = scheduleRule(required(field(root, 'simulation', 'rule'), 'rule'), 'rule');
  const occurrences = array(required(field(root, 'simulation', 'occurrences'), 'occurrences'), 'occurrences').map((item, index) => scheduleOccurrence(item, `occurrences[${index}]`));
  return { blockId: stringValue(required(field(root, 'simulation', 'block_id', 'blockId'), 'block_id'), 'block_id'), schedule: stringValue(required(field(root, 'simulation', 'schedule', 'name'), 'schedule'), 'schedule'), kind: rule.kind, ruleSummary: rule.summary, occurrences };
}
export async function loadSnapshot(fetchImpl: FetchLike = fetch): Promise<DisplaySnapshot> { const response = await fetchImpl('/api/snapshot', { headers: { accept: 'application/json' } }); if (!response.ok) throw new Error(`Snapshot request failed (${response.status})`); return decodeSnapshot(await response.json()); }
export interface DashboardClientHandlers { onSnapshot: (snapshot: DisplaySnapshot) => void; onEvent: (event: DashboardEvent) => void; onStreamOpen: () => void; onStreamLost: (error?: string) => void; onError: (error: Error) => void; }
export interface DashboardClientOptions { handlers: DashboardClientHandlers; fetchImpl?: FetchLike; eventSource?: EventSourceConstructor; reconnectDelayMs?: number; }
export class DashboardClient {
  private readonly fetchImpl: FetchLike; private readonly EventSourceImpl: EventSourceConstructor; private readonly handlers: DashboardClientHandlers; private readonly reconnectDelayMs: number; private source: EventSourceLike | null = null; private reconnectTimer: ReturnType<typeof setTimeout> | null = null; private revision = 0; private running = false; private reconnecting = false; private snapshotLoaded = false; private needsSnapshot = true;
  constructor(options: DashboardClientOptions) { this.fetchImpl = options.fetchImpl ?? fetch; this.EventSourceImpl = options.eventSource ?? (globalThis.EventSource as unknown as EventSourceConstructor); this.handlers = options.handlers; this.reconnectDelayMs = options.reconnectDelayMs ?? 1_000; }
  async start(): Promise<void> { this.running = true; try { const snapshot = await loadSnapshot(this.fetchImpl); this.revision = snapshot.revision; this.snapshotLoaded = true; this.needsSnapshot = false; this.handlers.onSnapshot(snapshot); this.connect(); } catch (error) { this.handleError(error); this.scheduleReconnect(); } }
  stop(): void { this.running = false; if (this.reconnectTimer !== null) clearTimeout(this.reconnectTimer); this.reconnectTimer = null; this.source?.close(); this.source = null; }
  private connect(): void { if (!this.running || !this.EventSourceImpl) return; this.source?.close(); this.reconnecting = false; const source = new this.EventSourceImpl(`/api/events?since=${encodeURIComponent(this.revision)}`); this.source = source; source.onopen = () => this.handlers.onStreamOpen(); source.onerror = () => { if (this.source !== source) return; source.close(); this.source = null; this.handlers.onStreamLost('The browser event stream disconnected.'); this.scheduleReconnect(); }; source.addEventListener('update', (event) => this.handleEvent('update', event)); source.addEventListener('resync', (event) => this.handleEvent('resync', event)); }
  private handleEvent(name: string, event: MessageEvent<string>): void { try { const decoded = decodeEvent(event.data, name, event.lastEventId); if (decoded.kind === 'resync') { this.source?.close(); this.source = null; this.needsSnapshot = true; void this.refreshSnapshot(); return; } if (decoded.revision <= this.revision) return; if (decoded.revision !== this.revision + 1) { this.needsSnapshot = true; this.handlers.onStreamLost('The browser event stream skipped a revision.'); this.source?.close(); this.source = null; void this.refreshSnapshot(); return; } this.revision = decoded.revision; this.handlers.onEvent(decoded); } catch (error) { this.handleError(error); this.source?.close(); this.source = null; this.handlers.onStreamLost('The browser event stream sent malformed data.'); this.scheduleReconnect(); } }
  private async refreshSnapshot(): Promise<void> { if (!this.running || this.reconnecting) return; this.reconnecting = true; try { const snapshot = await loadSnapshot(this.fetchImpl); this.revision = snapshot.revision; this.snapshotLoaded = true; this.needsSnapshot = false; this.handlers.onSnapshot(snapshot); this.source?.close(); this.source = null; this.connect(); } catch (error) { this.handleError(error); this.scheduleReconnect(); } finally { this.reconnecting = false; } }
  private scheduleReconnect(): void { if (!this.running || this.reconnectTimer !== null) return; this.reconnectTimer = setTimeout(() => { this.reconnectTimer = null; if (this.needsSnapshot || !this.snapshotLoaded) void this.refreshSnapshot(); else this.connect(); }, this.reconnectDelayMs); }
  private handleError(error: unknown): void { this.handlers.onError(error instanceof Error ? error : new Error(String(error))); }
}
