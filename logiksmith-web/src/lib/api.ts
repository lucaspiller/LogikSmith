import type {
  ConnectionState,
  DashboardEvent,
  DisplayAutomation,
  DisplayBinding,
  DisplayEndpoint,
  DisplayExecution,
  DisplayExecutionEffect,
  DisplayExecutionInput,
  DisplayExecutionTrigger,
  DisplayLogicError,
  DisplayLog,
  DisplaySnapshot,
  DisplayTelegram,
  DisplayTimer,
  DisplayWrite,
  DisplaySimulation,
  SimulationScenario,
  SimulationTypedValue,
  TimerState,
  WriteStatus
} from './state';

type JsonObject = Record<string, unknown>;
type JsonPrimitive = string | number | boolean | null;
type FetchLike = typeof fetch;
type EventSourceLike = {
  onopen: (() => void) | null;
  onerror: (() => void) | null;
  addEventListener(type: string, listener: (event: MessageEvent<string>) => void): void;
  close(): void;
};
type EventSourceConstructor = new (url: string) => EventSourceLike;

export class ApiDecodeError extends Error {
  constructor(path: string, message: string) {
    super(`Malformed dashboard data at ${path}: ${message}`);
    this.name = 'ApiDecodeError';
  }
}

export interface SimulationFieldError { path: string; message: string; }

export class SimulationApiError extends Error {
  readonly status: number;
  readonly fieldErrors: SimulationFieldError[];

  constructor(status: number, message: string, fieldErrors: SimulationFieldError[] = []) {
    super(message);
    this.name = 'SimulationApiError';
    this.status = status;
    this.fieldErrors = fieldErrors;
  }
}

const isObject = (value: unknown): value is JsonObject => typeof value === 'object' && value !== null && !Array.isArray(value);

function object(value: unknown, path: string): JsonObject {
  if (!isObject(value)) throw new ApiDecodeError(path, 'expected an object');
  return value;
}

function required(value: unknown, path: string): unknown {
  if (value === undefined || value === null) throw new ApiDecodeError(path, 'required field is missing');
  return value;
}

function nullableField(source: JsonObject, name: string, path: string): unknown {
  if (!(name in source)) throw new ApiDecodeError(path, 'required field is missing');
  return source[name];
}

function field(record: JsonObject, path: string, ...names: string[]): unknown {
  for (const name of names) if (name in record) return record[name];
  return undefined;
}

function stringValue(value: unknown, path: string): string {
  if (typeof value !== 'string' || value.length === 0) throw new ApiDecodeError(path, 'expected a non-empty string');
  return value;
}

function optionalString(value: unknown, path: string): string | null {
  if (value === undefined || value === null) return null;
  return stringValue(value, path);
}

function timeValue(value: unknown, path: string): string {
  if (typeof value === 'string') return stringValue(value, path);
  const number = nonNegativeNumber(value, path);
  return `${number} ms`;
}

function finiteNumber(value: unknown, path: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw new ApiDecodeError(path, 'expected a finite number');
  return value;
}

function nonNegativeNumber(value: unknown, path: string): number {
  const number = finiteNumber(value, path);
  if (number < 0) throw new ApiDecodeError(path, 'expected a non-negative number');
  return number;
}

function revision(value: unknown, path: string): number {
  const number = nonNegativeNumber(value, path);
  if (!Number.isInteger(number)) throw new ApiDecodeError(path, 'expected an integer');
  return number;
}

function integer(value: unknown, path: string): number {
  const number = nonNegativeNumber(value, path);
  if (!Number.isInteger(number)) throw new ApiDecodeError(path, 'expected a non-negative integer');
  return number;
}

function nullableBoolean(value: unknown, path: string): boolean | null {
  if (value === undefined || value === null) return null;
  if (typeof value !== 'boolean') throw new ApiDecodeError(path, 'expected a boolean or null');
  return value;
}

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
  return {
    address: field(record, path, 'address', 'group_address', 'groupAddress') === undefined ? '' : stringValue(field(record, path, 'address', 'group_address', 'groupAddress'), `${path}.address`),
    dpt: dpt(required(field(record, path, 'dpt'), `${path}.dpt`), `${path}.dpt`)
  };
}

function connection(value: unknown): { state: ConnectionState } {
  const record = typeof value === 'string' ? { state: value } : object(value, 'connection');
  const state = stringValue(required(field(record, 'connection', 'state', 'status'), 'connection.state'), 'connection.state');
  if (!['starting', 'connecting', 'connected', 'disconnected', 'failed'].includes(state)) {
    throw new ApiDecodeError('connection.state', `unsupported state ${state}`);
  }
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
    if (kind === 'percent') {
      const percentage = nonNegativeNumber(raw, `${path}.value`);
      if (percentage > 100) throw new ApiDecodeError(`${path}.value`, 'percentage must be between 0 and 100');
      return percentage;
    }
  }
  throw new ApiDecodeError(path, 'expected a boolean, percentage, or null');
}

function valueFrom(value: unknown, path: string, name: string): boolean | number | null {
  if (isObject(value)) return displayValue(field(value, path, name, 'value'), `${path}.${name}`);
  return displayValue(value, path);
}

function timer(value: unknown, receivedAtMs: number): DisplayTimer {
  const record = object(value, 'timer');
  const stateValue = stringValue(required(field(record, 'timer', 'state', 'status'), 'timer.state'), 'timer.state');
  if (stateValue !== 'idle' && stateValue !== 'pending') throw new ApiDecodeError('timer.state', `unsupported state ${stateValue}`);
  const deadlineRaw = field(record, 'timer', 'deadline_ms', 'deadlineMs', 'off_deadline_ms', 'off_deadline');
  const remainingRaw = field(record, 'timer', 'remaining_ms', 'remainingMs');
  const referenceRaw = field(record, 'timer', 'time_reference_ms', 'timeReferenceMs', 'server_time_ms', 'serverTimeMs');
  const deadlineMs = deadlineRaw === undefined || deadlineRaw === null ? null : nonNegativeNumber(deadlineRaw, 'timer.deadline_ms');
  const remainingMs = remainingRaw === undefined || remainingRaw === null ? null : nonNegativeNumber(remainingRaw, 'timer.remaining_ms');
  const reference = referenceRaw === undefined || referenceRaw === null ? receivedAtMs : nonNegativeNumber(referenceRaw, 'timer.time_reference_ms');
  return { state: stateValue as TimerState, deadlineMs, remainingMs, sampledAtMs: reference };
}

function telegram(value: unknown, index: number): DisplayTelegram {
  const path = `telegrams[${index}]`;
  const record = object(value, path);
  const rawValue = field(record, path, 'value', 'data');
  return {
    time: timeValue(required(field(record, path, 'time', 'time_ms', 'timeMs', 'timestamp', 'at'), `${path}.time`), `${path}.time`),
    source: optionalString(field(record, path, 'source', 'source_address'), `${path}.source`),
    destination: stringValue(required(field(record, path, 'destination', 'destination_address'), `${path}.destination`), `${path}.destination`),
    service: stringValue(required(field(record, path, 'service'), `${path}.service`), `${path}.service`),
    dpt: dpt(required(field(record, path, 'dpt'), `${path}.dpt`), `${path}.dpt`),
    value: valueFrom(rawValue, path, 'value')
  };
}

function log(value: unknown, index: number): DisplayLog {
  const path = `logs[${index}]`;
  const record = object(value, path);
  const fieldsRaw = field(record, path, 'fields', 'context');
  const fields: Record<string, JsonPrimitive> = {};
  if (fieldsRaw !== undefined && fieldsRaw !== null) {
    const source = object(fieldsRaw, `${path}.fields`);
    for (const [key, item] of Object.entries(source)) {
      if (item !== null && typeof item !== 'string' && typeof item !== 'number' && typeof item !== 'boolean') {
        throw new ApiDecodeError(`${path}.fields.${key}`, 'expected a JSON primitive');
      }
      fields[key] = item as JsonPrimitive;
    }
  }
  return {
    time: timeValue(required(field(record, path, 'time', 'time_ms', 'timeMs', 'timestamp', 'at'), `${path}.time`), `${path}.time`),
    level: stringValue(required(field(record, path, 'level'), `${path}.level`), `${path}.level`),
    target: stringValue(required(field(record, path, 'target'), `${path}.target`), `${path}.target`),
    message: stringValue(required(field(record, path, 'message'), `${path}.message`), `${path}.message`),
    fields
  };
}

function write(value: unknown): DisplayWrite {
  if (value === undefined || value === null) return { status: 'idle', requestId: null, value: null, error: null };
  const record = typeof value === 'boolean' ? { value } : object(value, 'write');
  const statusRaw = field(record, 'write', 'status', 'state');
  const status: WriteStatus = statusRaw === undefined || statusRaw === null ? 'idle' : stringValue(statusRaw, 'write.status') as WriteStatus;
  if (!['idle', 'pending', 'succeeded', 'failed'].includes(status)) throw new ApiDecodeError('write.status', `unsupported status ${status}`);
  const requestRaw = field(record, 'write', 'request_id', 'requestId');
  const requestId = requestRaw === undefined || requestRaw === null ? null : revision(requestRaw, 'write.request_id');
  return {
    status,
    requestId,
    value: displayValue(field(record, 'write', 'value'), 'write.value'),
    error: optionalString(field(record, 'write', 'error'), 'write.error')
  };
}

function array(value: unknown, path: string): unknown[] {
  if (!Array.isArray(value)) throw new ApiDecodeError(path, 'expected an array');
  return value;
}

function optionalRevision(value: unknown, path: string): number | null {
  return value === undefined || value === null ? null : revision(value, path);
}

function displayEndpoint(value: unknown, path: string, direction: 'input' | 'output', bindings: Map<string, string>, values: Map<string, { observed: boolean | number | null; requested: boolean | number | null }>): DisplayEndpoint {
  const source = object(value, path);
  const nameRaw = field(source, path, 'name', 'endpoint');
  const name = nameRaw === undefined || nameRaw === null ? undefined : stringValue(nameRaw, `${path}.name`);
  const bindingRaw = field(source, path, 'binding');
  const binding = isObject(bindingRaw) ? object(bindingRaw, `${path}.binding`) : null;
  const addressRaw = field(source, path, 'address', 'group_address', 'groupAddress') ?? (typeof bindingRaw === 'string' ? bindingRaw : binding ? field(binding, `${path}.binding`, 'address', 'group_address', 'groupAddress') : undefined);
  const mappedValue = name ? values.get(name) : undefined;
  return {
    ...(name ? { name } : {}),
    address: addressRaw === undefined || addressRaw === null ? (name ? bindings.get(name) ?? '' : '') : stringValue(addressRaw, `${path}.address`),
    dpt: dpt(required(field(source, path, 'dpt'), `${path}.dpt`), `${path}.dpt`),
    direction,
    observed: mappedValue?.observed ?? displayValue(field(source, path, 'observed'), `${path}.observed`),
    requested: direction === 'output' ? mappedValue?.requested ?? displayValue(field(source, path, 'requested'), `${path}.requested`) : undefined
  };
}

function endpointValues(values: JsonObject): Map<string, { observed: boolean | number | null; requested: boolean | number | null }> {
  const result = new Map<string, { observed: boolean | number | null; requested: boolean | number | null }>();
  const raw = field(values, 'values', 'endpoints', 'endpoint_values', 'endpointValues');
  const groups: Array<[string, unknown]> = [['inputs', field(values, 'values', 'inputs')], ['outputs', field(values, 'values', 'outputs')]].filter((entry): entry is [string, unknown] => entry[1] !== undefined);
  const add = (name: string, value: unknown, path: string): void => {
    const source = isObject(value) ? value : { observed: value };
    result.set(name, {
      observed: displayValue(field(source, path, 'observed', 'value'), `${path}.observed`),
      requested: displayValue(field(source, path, 'requested'), `${path}.requested`)
    });
  };
  if (isObject(raw)) {
    for (const [name, value] of Object.entries(raw)) {
      add(name, value, `values.endpoints.${name}`);
    }
  } else if (Array.isArray(raw)) {
    raw.forEach((value, index) => {
      const source = object(value, `values.endpoints[${index}]`);
      const name = stringValue(required(field(source, `values.endpoints[${index}]`, 'name'), `values.endpoints[${index}].name`), `values.endpoints[${index}].name`);
      add(name, source, `values.endpoints[${index}]`);
    });
  }
  groups.forEach(([groupName, group]) => {
    if (isObject(group)) {
      Object.entries(group).forEach(([name, value]) => add(name, value, `values.${groupName}.${name}`));
    } else if (Array.isArray(group)) {
      group.forEach((value, index) => {
        const source = object(value, `values.${groupName}[${index}]`);
        const name = stringValue(required(field(source, `values.${groupName}[${index}]`, 'name', 'endpoint'), `values.${groupName}[${index}].name`), `values.${groupName}[${index}].name`);
        add(name, source, `values.${groupName}[${index}]`);
      });
    }
  });
  return result;
}

function decodeDisplayAutomation(root: JsonObject, config: JsonObject, values: JsonObject): DisplayAutomation | undefined {
  const nested = isObject(field(root, 'snapshot', 'automation')) ? object(field(root, 'snapshot', 'automation'), 'automation') : null;
  const active = isObject(field(config, 'config', 'active')) ? object(field(config, 'config', 'active'), 'config.active') : null;
  const source = nested ?? active ?? config;
  const inputsRaw = field(source, 'automation', 'inputs') ?? field(root, 'snapshot', 'active_inputs', 'activeInputs');
  const outputsRaw = field(source, 'automation', 'outputs') ?? field(root, 'snapshot', 'active_outputs', 'activeOutputs');
  if (inputsRaw === undefined && outputsRaw === undefined) return undefined;
  const bindings = new Map<string, string>();
  const bindingsRaw = field(source, 'automation', 'knx_bindings', 'bindings') ?? field(config, 'config', 'knx_bindings', 'bindings');
  if (bindingsRaw !== undefined) {
    array(bindingsRaw, 'automation.knx_bindings').forEach((item, index) => {
      const binding = object(item, `automation.knx_bindings[${index}]`);
      const name = stringValue(required(field(binding, `automation.knx_bindings[${index}]`, 'endpoint'), `automation.knx_bindings[${index}].endpoint`), `automation.knx_bindings[${index}].endpoint`);
      const address = stringValue(required(field(binding, `automation.knx_bindings[${index}]`, 'group_address', 'groupAddress', 'address'), `automation.knx_bindings[${index}].group_address`), `automation.knx_bindings[${index}].group_address`);
      bindings.set(name, address);
    });
  }
  const valuesByName = endpointValues(values);
  const inputs = array(inputsRaw ?? [], 'automation.inputs').map((item, index) => displayEndpoint(item, `automation.inputs[${index}]`, 'input', bindings, valuesByName));
  const outputs = array(outputsRaw ?? [], 'automation.outputs').map((item, index) => displayEndpoint(item, `automation.outputs[${index}]`, 'output', bindings, valuesByName));
  const logicSource = field(source, 'automation', 'logic');
  const logic = isObject(logicSource) ? stringValue(required(field(logicSource, 'automation.logic', 'source'), 'automation.logic.source'), 'automation.logic.source') : (typeof logicSource === 'string' ? logicSource : '');
  return { inputs, outputs, bindings: [...bindings.entries()].map(([endpoint, groupAddress]) => ({ endpoint, groupAddress } as DisplayBinding)), source: logic };
}

function logicError(value: unknown, path: string): DisplayLogicError {
  const source = object(value, path);
  const category = stringValue(required(field(source, path, 'category', 'kind', 'type'), `${path}.category`), `${path}.category`);
  const message = stringValue(required(field(source, path, 'message', 'error'), `${path}.message`), `${path}.message`);
  const lineRaw = field(source, path, 'line', 'line_number', 'lineNumber');
  const line = lineRaw === undefined || lineRaw === null ? null : revision(lineRaw, `${path}.line`);
  return { category, message, line };
}

function executionDpt(value: unknown, path: string): string {
  return dpt(value, path);
}

function typedValue(value: unknown, path: string): SimulationTypedValue {
  const source = object(value, path);
  const kind = field(source, path, 'kind');
  const raw = required(field(source, path, 'value'), `${path}.value`);
  if (kind === 'bool') {
    if (typeof raw !== 'boolean') throw new ApiDecodeError(`${path}.value`, 'expected a boolean');
    return { kind: 'bool', value: raw };
  }
  if (kind === 'percent') {
    const percentage = nonNegativeNumber(raw, `${path}.value`);
    if (percentage > 100) throw new ApiDecodeError(`${path}.value`, 'percentage must be between 0 and 100');
    return { kind: 'percent', value: percentage };
  }
  throw new ApiDecodeError(`${path}.kind`, 'expected bool or percent');
}

function executionValue(value: unknown, path: string): boolean | number | null {
  if (value === null) return null;
  return typedValue(value, path).value;
}

function executionTrigger(value: unknown, path: string): DisplayExecutionTrigger {
  const source = object(value, path);
  const current = executionValue(required(field(source, path, 'value'), `${path}.value`), `${path}.value`);
  if (current === null) throw new ApiDecodeError(`${path}.value`, 'trigger value cannot be null');
  const previous = executionValue(nullableField(source, 'previous', `${path}.previous`), `${path}.previous`);
  return {
    endpoint: stringValue(required(field(source, path, 'endpoint'), `${path}.endpoint`), `${path}.endpoint`),
    dpt: executionDpt(required(field(source, path, 'dpt'), `${path}.dpt`), `${path}.dpt`),
    value: current,
    previous,
    changed: booleanField(source, path, 'changed'),
    rising: booleanField(source, path, 'rising'),
    falling: booleanField(source, path, 'falling')
  };
}

function booleanField(source: JsonObject, path: string, name: string): boolean {
  const value = required(field(source, path, name), `${path}.${name}`);
  if (typeof value !== 'boolean') throw new ApiDecodeError(`${path}.${name}`, 'expected a boolean');
  return value;
}

function executionInput(value: unknown, executionIndex: number, index: number): DisplayExecutionInput {
  const path = `logic.executions[${executionIndex}].inputs[${index}]`;
  const source = object(value, path);
  const snapshotValue = executionValue(nullableField(source, 'value', `${path}.value`), `${path}.value`);
  const valid = booleanField(source, path, 'valid');
  const ageRaw = nullableField(source, 'age_ms', `${path}.age_ms`);
  const ageMs = ageRaw === null ? null : integer(ageRaw, `${path}.age_ms`);
  if (valid !== (snapshotValue !== null)) throw new ApiDecodeError(path, 'valid must match value presence');
  if (!valid && ageMs !== null) throw new ApiDecodeError(`${path}.age_ms`, 'invalid inputs must have a null age');
  if (valid && ageMs === null) throw new ApiDecodeError(`${path}.age_ms`, 'valid inputs must have an age');
  return {
    endpoint: stringValue(required(field(source, path, 'endpoint'), `${path}.endpoint`), `${path}.endpoint`),
    dpt: executionDpt(required(field(source, path, 'dpt'), `${path}.dpt`), `${path}.dpt`),
    value: snapshotValue,
    valid,
    ageMs
  };
}

function executionEffect(value: unknown, executionIndex: number, index: number): DisplayExecutionEffect {
  const path = `logic.executions[${executionIndex}].effects[${index}]`;
  const source = object(value, path);
  const effectValue = executionValue(required(field(source, path, 'value'), `${path}.value`), `${path}.value`);
  if (effectValue === null) throw new ApiDecodeError(`${path}.value`, 'effect value cannot be null');
  return {
    endpoint: stringValue(required(field(source, path, 'endpoint'), `${path}.endpoint`), `${path}.endpoint`),
    destination: stringValue(required(field(source, path, 'destination'), `${path}.destination`), `${path}.destination`),
    dpt: executionDpt(required(field(source, path, 'dpt'), `${path}.dpt`), `${path}.dpt`),
    value: effectValue
  };
}

function execution(value: unknown, index: number): DisplayExecution {
  const path = `logic.executions[${index}]`;
  const source = object(value, path);
  const status = stringValue(required(field(source, path, 'status'), `${path}.status`), `${path}.status`);
  if (status !== 'succeeded' && status !== 'failed') throw new ApiDecodeError(`${path}.status`, `unsupported status ${status}`);
  const triggerSource = object(required(field(source, path, 'trigger'), `${path}.trigger`), `${path}.trigger`);
  const inputs = array(required(field(source, path, 'inputs'), `${path}.inputs`), `${path}.inputs`).map((item, inputIndex) => executionInput(item, index, inputIndex));
  const effects = array(required(field(source, path, 'effects'), `${path}.effects`), `${path}.effects`).map((item, effectIndex) => executionEffect(item, index, effectIndex));
  const errorRaw = field(source, path, 'error');
  if (status === 'failed' && effects.length > 0) throw new ApiDecodeError(`${path}.effects`, 'failed executions cannot contain effects');
  return {
    executionId: integer(required(field(source, path, 'execution_id'), `${path}.execution_id`), `${path}.execution_id`),
    timeMs: integer(required(field(source, path, 'time_ms'), `${path}.time_ms`), `${path}.time_ms`),
    durationUs: integer(required(field(source, path, 'duration_us'), `${path}.duration_us`), `${path}.duration_us`),
    logicRevision: optionalRevision(required(field(source, path, 'logic_revision'), `${path}.logic_revision`), `${path}.logic_revision`),
    status: status as DisplayExecution['status'],
    trigger: executionTrigger(triggerSource, `${path}.trigger`),
    inputs,
    effects,
    error: errorRaw === undefined || errorRaw === null ? null : logicError(errorRaw, `${path}.error`)
  };
}

function simulationInput(value: unknown, index: number): DisplayExecutionInput {
  const path = `inputs[${index}]`;
  const source = object(value, path);
  const snapshotValue = executionValue(nullableField(source, 'value', `${path}.value`), `${path}.value`);
  const valid = booleanField(source, path, 'valid');
  const ageRaw = nullableField(source, 'age_ms', `${path}.age_ms`);
  const ageMs = ageRaw === null ? null : integer(ageRaw, `${path}.age_ms`);
  if (valid !== (snapshotValue !== null)) throw new ApiDecodeError(path, 'valid must match value presence');
  if (!valid && ageMs !== null) throw new ApiDecodeError(`${path}.age_ms`, 'invalid inputs must have a null age');
  if (valid && ageMs === null) throw new ApiDecodeError(`${path}.age_ms`, 'valid inputs must have an age');
  return {
    endpoint: stringValue(required(field(source, path, 'endpoint'), `${path}.endpoint`), `${path}.endpoint`),
    dpt: executionDpt(required(field(source, path, 'dpt'), `${path}.dpt`), `${path}.dpt`),
    value: snapshotValue,
    valid,
    ageMs
  };
}

function simulationEffect(value: unknown, index: number): DisplayExecutionEffect {
  const path = `effects[${index}]`;
  const source = object(value, path);
  const effectValue = executionValue(required(field(source, path, 'value'), `${path}.value`), `${path}.value`);
  if (effectValue === null) throw new ApiDecodeError(`${path}.value`, 'effect value cannot be null');
  return {
    endpoint: stringValue(required(field(source, path, 'endpoint'), `${path}.endpoint`), `${path}.endpoint`),
    destination: stringValue(required(field(source, path, 'destination'), `${path}.destination`), `${path}.destination`),
    dpt: executionDpt(required(field(source, path, 'dpt'), `${path}.dpt`), `${path}.dpt`),
    value: effectValue
  };
}

/** Decodes the execution-shaped, effect-only result returned by /api/simulate. */
export function decodeSimulation(input: unknown): DisplaySimulation {
  const root = object(input, 'simulation');
  const status = stringValue(required(field(root, 'simulation', 'status'), 'status'), 'status');
  if (status !== 'succeeded' && status !== 'failed') throw new ApiDecodeError('status', `unsupported status ${status}`);
  const trigger = executionTrigger(required(field(root, 'simulation', 'trigger'), 'trigger'), 'trigger');
  const inputs = array(required(field(root, 'simulation', 'inputs'), 'inputs'), 'inputs').map(simulationInput);
  const effects = array(required(field(root, 'simulation', 'effects'), 'effects'), 'effects').map(simulationEffect);
  if (status === 'failed' && effects.length > 0) throw new ApiDecodeError('effects', 'failed simulations cannot contain effects');
  const errorRaw = field(root, 'simulation', 'error');
  return {
    logicRevision: revision(required(field(root, 'simulation', 'logic_revision', 'logicRevision'), 'logic_revision'), 'logic_revision'),
    durationUs: integer(required(field(root, 'simulation', 'duration_us', 'durationUs'), 'duration_us'), 'duration_us'),
    status: status as DisplaySimulation['status'],
    trigger,
    inputs,
    effects,
    error: errorRaw === undefined || errorRaw === null ? null : logicError(errorRaw, 'error')
  };
}

/** Maps the internal wire DTO to the UI's small display model. */
export function decodeSnapshot(input: unknown, receivedAtMs = Date.now()): DisplaySnapshot {
  const root = object(input, 'snapshot');
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
  const offDelayRaw = field(config, 'config', 'off_delay_ms', 'offDelayMs', 'off_delay');
  const inputEndpoint = inputRaw === undefined ? { address: '', dpt: '1.001' } : (field(config, 'config', 'input') === undefined && firstInput ? firstInput : endpoint(inputRaw, 'config.input'));
  const outputEndpoint = outputRaw === undefined ? { address: '', dpt: '1.001' } : (field(config, 'config', 'output') === undefined && firstOutput ? firstOutput : endpoint(outputRaw, 'config.output'));
  const offDelayMs = offDelayRaw === undefined || offDelayRaw === null ? 0 : nonNegativeNumber(offDelayRaw, 'config.off_delay_ms');
  const activeRevisionRaw = field(root, 'snapshot', 'active_automation_revision', 'activeAutomationRevision') ?? field(config, 'config', 'active_automation_revision', 'activeAutomationRevision');
  const savedRevisionRaw = field(root, 'snapshot', 'saved_automation_revision', 'savedAutomationRevision') ?? field(config, 'config', 'saved_automation_revision', 'savedAutomationRevision');
  const logicStatus = isObject(field(root, 'snapshot', 'logic')) ? object(field(root, 'snapshot', 'logic'), 'logic') : null;
  const activeStructuralRevisionRaw = field(root, 'snapshot', 'active_structural_revision', 'activeStructuralRevision') ?? (logicStatus ? field(logicStatus, 'logic', 'active_structural_revision', 'activeStructuralRevision') : undefined) ?? field(config, 'config', 'active_structural_revision', 'activeStructuralRevision');
  const savedStructuralRevisionRaw = field(root, 'snapshot', 'saved_structural_revision', 'savedStructuralRevision') ?? (logicStatus ? field(logicStatus, 'logic', 'saved_structural_revision', 'savedStructuralRevision') : undefined) ?? field(config, 'config', 'saved_structural_revision', 'savedStructuralRevision');
  const activeLogicRevisionRaw = field(root, 'snapshot', 'active_logic_revision', 'activeLogicRevision') ?? (logicStatus ? field(logicStatus, 'logic', 'active_logic_revision', 'activeLogicRevision') : undefined) ?? field(config, 'config', 'active_logic_revision', 'activeLogicRevision');
  const savedLogicRevisionRaw = field(root, 'snapshot', 'saved_logic_revision', 'savedLogicRevision') ?? (logicStatus ? field(logicStatus, 'logic', 'saved_logic_revision', 'savedLogicRevision') : undefined) ?? field(config, 'config', 'saved_logic_revision', 'savedLogicRevision');
  const inputObserved = inputValuesRaw === undefined && firstInput?.name ? endpointValuesByName.get(firstInput.name)?.observed ?? null : valueFrom(inputValues, 'values.input', 'observed');
  const outputObserved = outputValuesRaw === undefined && firstOutput?.name ? endpointValuesByName.get(firstOutput.name)?.observed ?? null : valueFrom(outputValues, 'values.output', 'observed');
  const outputRequested = outputValuesRaw === undefined && firstOutput?.name ? endpointValuesByName.get(firstOutput.name)?.requested ?? null : valueFrom(outputValues, 'values.output', 'requested');

  const executions = logicStatus
    ? array(required(field(logicStatus, 'logic', 'executions'), 'logic.executions'), 'logic.executions').map(execution)
    : [];
  const activeAutomationRevision = optionalRevision(activeRevisionRaw, 'active_automation_revision');
  const savedAutomationRevision = optionalRevision(savedRevisionRaw, 'saved_automation_revision');
  const activeStructuralRevision = optionalRevision(activeStructuralRevisionRaw, 'active_structural_revision');
  const savedStructuralRevision = optionalRevision(savedStructuralRevisionRaw, 'saved_structural_revision');
  const activeLogicRevision = optionalRevision(activeLogicRevisionRaw, 'active_logic_revision');
  const savedLogicRevision = optionalRevision(savedLogicRevisionRaw, 'saved_logic_revision');
  const explicitRestart = field(root, 'snapshot', 'restart_required', 'restartRequired') ?? (logicStatus ? field(logicStatus, 'logic', 'restart_required', 'restartRequired') : undefined) ?? field(config, 'config', 'restart_required', 'restartRequired');
  const hasStructuralRevisions = activeStructuralRevision !== null && savedStructuralRevision !== null;
  const restartRequired = explicitRestart === true || (hasStructuralRevisions
    ? activeStructuralRevision !== savedStructuralRevision
    : activeAutomationRevision !== null && savedAutomationRevision !== null && activeAutomationRevision !== savedAutomationRevision);
  return {
    revision: revision(required(field(root, 'snapshot', 'revision'), 'revision'), 'revision'),
    connection: connection(required(field(root, 'snapshot', 'connection'), 'connection')),
    config: {
      input: inputEndpoint,
      output: outputEndpoint,
      offDelayMs
    },
    values: {
      input: { observed: inputObserved },
      output: {
        observed: outputObserved,
        requested: outputRequested
      }
    },
    automation,
    activeAutomationRevision,
    savedAutomationRevision,
    activeStructuralRevision,
    savedStructuralRevision,
    activeLogicRevision,
    savedLogicRevision,
    restartRequired,
    executions,
    write: write(field(root, 'write', 'write_status', 'last_write')),
    timer: field(root, 'snapshot', 'timer') === undefined ? { state: 'idle', deadlineMs: null, remainingMs: null, sampledAtMs: receivedAtMs } : timer(field(root, 'snapshot', 'timer'), receivedAtMs),
    telegrams: telegrams as DisplayTelegram[],
    logs: logs as DisplayLog[]
  };
}

function parseJson(value: string, path: string): unknown {
  try {
    return JSON.parse(value) as unknown;
  } catch {
    throw new ApiDecodeError(path, 'expected valid JSON');
  }
}

export function decodeEvent(input: unknown, eventName = 'update', eventId?: string): DashboardEvent {
  const root = object(typeof input === 'string' ? parseJson(input, 'event.data') : input, 'event');
  const eventRevisionRaw = field(root, 'event', 'revision') ?? eventId;
  const eventRevision = revision(required(eventRevisionRaw, 'event.revision'), 'event.revision');
  if (eventName === 'resync') return { kind: 'resync', revision: eventRevision };
  if (eventName !== 'update') throw new ApiDecodeError('event', `unsupported event ${eventName}`);
  const rawSnapshot = field(root, 'event', 'snapshot');
  const snapshot = decodeSnapshot(required(rawSnapshot, 'event.snapshot'));
  if (snapshot.revision !== eventRevision) throw new ApiDecodeError('event.revision', 'does not match event.snapshot.revision');
  return { kind: 'update', revision: eventRevision, snapshot };
}

async function jsonOrNull(response: Response): Promise<unknown> {
  try { return await response.json(); } catch { return null; }
}

function simulationFieldErrors(value: unknown): SimulationFieldError[] {
  if (!isObject(value)) return [];
  const raw = value.errors ?? value.field_errors ?? value.fields;
  if (Array.isArray(raw)) return raw.flatMap((item) => {
    if (!isObject(item) || (typeof item.path !== 'string' && typeof item.field !== 'string') || typeof item.message !== 'string') return [];
    return [{ path: (item.path ?? item.field) as string, message: item.message }];
  });
  if (isObject(raw)) return Object.entries(raw).flatMap(([path, message]) => typeof message === 'string' ? [{ path, message }] : []);
  return [];
}

function simulationErrorMessage(status: number, body: unknown): string {
  if (status === 409) return 'The active logic source changed. Refresh the dashboard and re-run the simulation.';
  if (status === 422) return 'The simulation scenario is invalid. Fix the highlighted fields and re-run.';
  if (isObject(body) && typeof body.error === 'string') return body.error;
  return `Simulation request failed (${status})`;
}

export async function simulateScenario(scenario: SimulationScenario, fetchImpl: FetchLike = fetch): Promise<DisplaySimulation> {
  const response = await fetchImpl('/api/simulate', {
    method: 'POST',
    headers: { accept: 'application/json', 'content-type': 'application/json' },
    body: JSON.stringify({
      expected_logic_revision: scenario.expectedLogicRevision,
      trigger: scenario.trigger,
      inputs: scenario.inputs.map((input) => ({
        endpoint: input.endpoint,
        value: input.value,
        valid: input.valid,
        age_ms: input.ageMs
      }))
    })
  });
  if (response.ok) return decodeSimulation(await response.json());
  const body = await jsonOrNull(response);
  throw new SimulationApiError(response.status, simulationErrorMessage(response.status, body), simulationFieldErrors(body));
}

export async function loadSnapshot(fetchImpl: FetchLike = fetch): Promise<DisplaySnapshot> {
  const response = await fetchImpl('/api/snapshot', { headers: { accept: 'application/json' } });
  if (!response.ok) throw new Error(`Snapshot request failed (${response.status})`);
  return decodeSnapshot(await response.json());
}

export interface DashboardClientHandlers {
  onSnapshot: (snapshot: DisplaySnapshot) => void;
  onEvent: (event: DashboardEvent) => void;
  onStreamOpen: () => void;
  onStreamLost: (error?: string) => void;
  onError: (error: Error) => void;
}

export interface DashboardClientOptions {
  handlers: DashboardClientHandlers;
  fetchImpl?: FetchLike;
  eventSource?: EventSourceConstructor;
  reconnectDelayMs?: number;
}

/** Owns HTTP/SSE transport; UI code only receives decoded display events. */
export class DashboardClient {
  private readonly fetchImpl: FetchLike;
  private readonly EventSourceImpl: EventSourceConstructor;
  private readonly handlers: DashboardClientHandlers;
  private readonly reconnectDelayMs: number;
  private source: EventSourceLike | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private revision = 0;
  private running = false;
  private reconnecting = false;
  private snapshotLoaded = false;
  private needsSnapshot = true;

  constructor(options: DashboardClientOptions) {
    this.fetchImpl = options.fetchImpl ?? fetch;
    this.EventSourceImpl = options.eventSource ?? (globalThis.EventSource as unknown as EventSourceConstructor);
    this.handlers = options.handlers;
    this.reconnectDelayMs = options.reconnectDelayMs ?? 1_000;
  }

  async start(): Promise<void> {
    this.running = true;
    try {
      const snapshot = await loadSnapshot(this.fetchImpl);
      this.revision = snapshot.revision;
      this.snapshotLoaded = true;
      this.needsSnapshot = false;
      this.handlers.onSnapshot(snapshot);
      this.connect();
    } catch (error) {
      this.handleError(error);
      this.scheduleReconnect();
    }
  }

  stop(): void {
    this.running = false;
    if (this.reconnectTimer !== null) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
    this.source?.close();
    this.source = null;
  }

  private connect(): void {
    if (!this.running || !this.EventSourceImpl) return;
    this.source?.close();
    this.reconnecting = false;
    const source = new this.EventSourceImpl(`/api/events?since=${encodeURIComponent(this.revision)}`);
    this.source = source;
    source.onopen = () => this.handlers.onStreamOpen();
    source.onerror = () => {
      if (this.source !== source) return;
      source.close();
      this.source = null;
      this.handlers.onStreamLost('The browser event stream disconnected.');
      this.scheduleReconnect();
    };
    source.addEventListener('update', (event) => this.handleEvent('update', event));
    source.addEventListener('resync', (event) => this.handleEvent('resync', event));
  }

  private handleEvent(name: string, event: MessageEvent<string>): void {
    try {
      const decoded = decodeEvent(event.data, name, event.lastEventId);
      if (decoded.kind === 'resync') {
        this.source?.close();
        this.source = null;
        this.needsSnapshot = true;
        void this.refreshSnapshot();
        return;
      }
      if (decoded.revision <= this.revision) return;
      if (decoded.revision !== this.revision + 1) {
        this.needsSnapshot = true;
        this.handlers.onStreamLost('The browser event stream skipped a revision.');
        this.source?.close();
        this.source = null;
        void this.refreshSnapshot();
        return;
      }
      this.revision = decoded.revision;
      this.handlers.onEvent(decoded);
    } catch (error) {
      this.handleError(error);
      this.source?.close();
      this.source = null;
      this.handlers.onStreamLost('The browser event stream sent malformed data.');
      this.scheduleReconnect();
    }
  }

  private async refreshSnapshot(): Promise<void> {
    if (!this.running || this.reconnecting) return;
    this.reconnecting = true;
    try {
      const snapshot = await loadSnapshot(this.fetchImpl);
      this.revision = snapshot.revision;
      this.snapshotLoaded = true;
      this.needsSnapshot = false;
      this.handlers.onSnapshot(snapshot);
      this.source?.close();
      this.source = null;
      this.connect();
    } catch (error) {
      this.handleError(error);
      this.scheduleReconnect();
    } finally {
      this.reconnecting = false;
    }
  }

  private scheduleReconnect(): void {
    if (!this.running || this.reconnectTimer !== null) return;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      if (this.needsSnapshot || !this.snapshotLoaded) void this.refreshSnapshot();
      else this.connect();
    }, this.reconnectDelayMs);
  }

  private handleError(error: unknown): void {
    const normalized = error instanceof Error ? error : new Error(String(error));
    this.handlers.onError(normalized);
  }
}
