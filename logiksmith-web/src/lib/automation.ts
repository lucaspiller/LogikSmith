export const DPTS = ['1.001', '5.001'] as const;
export const MAX_OFF_DELAY_MS = 86_400_000;
export type Dpt = (typeof DPTS)[number];

export interface AutomationEndpoint {
  name: string;
  dpt: Dpt;
}

export interface KnxBinding {
  endpoint: string;
  group_address: string;
}

export interface TimedBoolBehavior {
  input: string;
  output: string;
  off_delay_ms: number;
}

export interface PercentageForwardBehavior {
  input: string;
  output: string;
}

export interface AutomationDocument {
  inputs: AutomationEndpoint[];
  outputs: AutomationEndpoint[];
  knx_bindings: KnxBinding[];
  behaviors: {
    timed_bool: TimedBoolBehavior;
    percentage_forward: PercentageForwardBehavior;
  };
}

export interface AutomationFieldError {
  path: string;
  message: string;
}

export interface AutomationEnvelope {
  document: AutomationDocument;
  revision: number;
}

export interface AutomationSaveResult {
  revision: number;
  restartRequired: boolean;
}

type FetchLike = typeof fetch;
type JsonRecord = Record<string, unknown>;

export class AutomationDecodeError extends Error {
  constructor(path: string, message: string) {
    super(`Malformed automation data at ${path}: ${message}`);
    this.name = 'AutomationDecodeError';
  }
}

export class AutomationApiError extends Error {
  readonly status: number;
  readonly fieldErrors: AutomationFieldError[];
  readonly latest: AutomationEnvelope | null;

  constructor(status: number, message: string, fieldErrors: AutomationFieldError[] = [], latest: AutomationEnvelope | null = null) {
    super(message);
    this.name = 'AutomationApiError';
    this.status = status;
    this.fieldErrors = fieldErrors;
    this.latest = latest;
  }
}

export const emptyAutomation = (): AutomationDocument => ({
  inputs: [],
  outputs: [],
  knx_bindings: [],
  behaviors: {
    timed_bool: { input: '', output: '', off_delay_ms: 5_000 },
    percentage_forward: { input: '', output: '' }
  }
});

const isRecord = (value: unknown): value is JsonRecord => typeof value === 'object' && value !== null && !Array.isArray(value);

function record(value: unknown, path: string): JsonRecord {
  if (!isRecord(value)) throw new AutomationDecodeError(path, 'expected an object');
  return value;
}

function required(source: JsonRecord, name: string, path: string): unknown {
  if (!(name in source) || source[name] === null || source[name] === undefined) {
    throw new AutomationDecodeError(path, 'required field is missing');
  }
  return source[name];
}

function nonEmptyString(value: unknown, path: string): string {
  if (typeof value !== 'string' || value.length === 0) throw new AutomationDecodeError(path, 'expected a non-empty string');
  return value;
}

function dpt(value: unknown, path: string): Dpt {
  let normalized = value;
  if (isRecord(value) && typeof value.major === 'number' && typeof value.subtype === 'number' && Number.isInteger(value.major) && Number.isInteger(value.subtype)) {
    normalized = `${value.major}.${value.subtype.toString().padStart(3, '0')}`;
  }
  if (normalized !== '1.001' && normalized !== '5.001') throw new AutomationDecodeError(path, 'expected DPT 1.001 or 5.001');
  return normalized;
}

function integer(value: unknown, path: string): number {
  if (typeof value !== 'number' || !Number.isInteger(value) || value < 0) {
    throw new AutomationDecodeError(path, 'expected a non-negative integer');
  }
  return value;
}

function endpoint(value: unknown, path: string): AutomationEndpoint {
  const source = record(value, path);
  return {
    name: nonEmptyString(required(source, 'name', `${path}.name`), `${path}.name`),
    dpt: dpt(required(source, 'dpt', `${path}.dpt`), `${path}.dpt`)
  };
}

function list(value: unknown, path: string): unknown[] {
  if (!Array.isArray(value)) throw new AutomationDecodeError(path, 'expected an array');
  return value;
}

function behavior(value: unknown, path: string): JsonRecord {
  return record(value, path);
}

export function decodeAutomationDocument(input: unknown, path = 'document'): AutomationDocument {
  const source = record(input, path);
  const timed = behavior(required(record(required(source, 'behaviors', `${path}.behaviors`), `${path}.behaviors`), 'timed_bool', `${path}.behaviors.timed_bool`), `${path}.behaviors.timed_bool`);
  const percentage = behavior(required(record(required(source, 'behaviors', `${path}.behaviors`), `${path}.behaviors`), 'percentage_forward', `${path}.behaviors.percentage_forward`), `${path}.behaviors.percentage_forward`);
  const bindings = list(required(source, 'knx_bindings', `${path}.knx_bindings`), `${path}.knx_bindings`).map((value, index) => {
    const binding = record(value, `${path}.knx_bindings[${index}]`);
    return {
      endpoint: nonEmptyString(required(binding, 'endpoint', `${path}.knx_bindings[${index}].endpoint`), `${path}.knx_bindings[${index}].endpoint`),
      group_address: nonEmptyString(required(binding, 'group_address', `${path}.knx_bindings[${index}].group_address`), `${path}.knx_bindings[${index}].group_address`)
    };
  });
  return {
    inputs: list(required(source, 'inputs', `${path}.inputs`), `${path}.inputs`).map((value, index) => endpoint(value, `${path}.inputs[${index}]`)),
    outputs: list(required(source, 'outputs', `${path}.outputs`), `${path}.outputs`).map((value, index) => endpoint(value, `${path}.outputs[${index}]`)),
    knx_bindings: bindings,
    behaviors: {
      timed_bool: {
        input: nonEmptyString(required(timed, 'input', `${path}.behaviors.timed_bool.input`), `${path}.behaviors.timed_bool.input`),
        output: nonEmptyString(required(timed, 'output', `${path}.behaviors.timed_bool.output`), `${path}.behaviors.timed_bool.output`),
        off_delay_ms: integer(required(timed, 'off_delay_ms', `${path}.behaviors.timed_bool.off_delay_ms`), `${path}.behaviors.timed_bool.off_delay_ms`)
      },
      percentage_forward: {
        input: nonEmptyString(required(percentage, 'input', `${path}.behaviors.percentage_forward.input`), `${path}.behaviors.percentage_forward.input`),
        output: nonEmptyString(required(percentage, 'output', `${path}.behaviors.percentage_forward.output`), `${path}.behaviors.percentage_forward.output`)
      }
    }
  };
}

function revision(value: unknown): number {
  if (typeof value !== 'number' || !Number.isInteger(value) || value < 0) throw new AutomationDecodeError('revision', 'expected a non-negative integer');
  return value;
}

/** Accepts the documented { document, revision } envelope and the equivalent { automation, revision } spelling. */
export function decodeAutomation(input: unknown): AutomationEnvelope {
  const source = record(input, 'automation');
  const rawDocument = source.document ?? source.automation ?? source;
  const document = decodeAutomationDocument(rawDocument);
  return { document, revision: revision(source.revision ?? source.content_revision) };
}

function fieldErrors(value: unknown): AutomationFieldError[] {
  if (!isRecord(value)) return [];
  const raw = value.errors ?? value.field_errors ?? value.fields;
  if (Array.isArray(raw)) {
    return raw.flatMap((item) => {
      if (!isRecord(item) || (typeof item.path !== 'string' && typeof item.field !== 'string') || typeof item.message !== 'string') return [];
      return [{ path: (item.path ?? item.field) as string, message: item.message }];
    });
  }
  if (isRecord(raw)) {
    return Object.entries(raw).flatMap(([path, message]) => {
      if (typeof message === 'string') return [{ path, message }];
      if (Array.isArray(message) && typeof message[0] === 'string') return [{ path, message: message[0] }];
      return [];
    });
  }
  return [];
}

async function jsonOrNull(response: Response): Promise<unknown> {
  try { return await response.json(); } catch { return null; }
}

export async function loadAutomation(fetchImpl: FetchLike = fetch): Promise<AutomationEnvelope> {
  const response = await fetchImpl('/api/automation', { headers: { accept: 'application/json' } });
  if (!response.ok) throw new AutomationApiError(response.status, `Automation request failed (${response.status})`);
  return decodeAutomation(await response.json());
}

export async function saveAutomation(
  document: AutomationDocument,
  replacedRevision: number,
  fetchImpl: FetchLike = fetch
): Promise<AutomationSaveResult> {
  const response = await fetchImpl('/api/automation', {
    method: 'PUT',
    headers: { accept: 'application/json', 'content-type': 'application/json' },
    body: JSON.stringify({ document, revision: replacedRevision })
  });
  if (response.ok) {
    const source = record(await response.json(), 'save');
    return {
      revision: revision(source.revision ?? source.content_revision),
      restartRequired: source.restart_required === true || source.restartRequired === true
    };
  }
  const body = await jsonOrNull(response);
  const errors = fieldErrors(body);
  let latest: AutomationEnvelope | null = null;
  if (response.status === 409 && body !== null) {
    try {
      latest = decodeAutomation(isRecord(body) && body.latest !== undefined ? body.latest : isRecord(body) && body.current !== undefined ? body.current : body);
    } catch { latest = null; }
  }
  throw new AutomationApiError(response.status, response.status === 409 ? 'The saved automation changed. Reload the latest document before saving.' : `Automation save failed (${response.status})`, errors, latest);
}

const namePattern = /^[a-z][a-z0-9_.-]*$/;
const groupAddressPattern = /^(0|[1-9]\d{0,1})\/(0|[1-7])\/(0|[1-9]\d{0,2})$/;

function addError(errors: AutomationFieldError[], path: string, message: string): void {
  errors.push({ path, message });
}

function directionEndpoints(document: AutomationDocument, direction: 'input' | 'output'): AutomationEndpoint[] {
  return direction === 'input' ? document.inputs : document.outputs;
}

function hasName(document: AutomationDocument, name: string, direction: 'input' | 'output', dptValue: Dpt): boolean {
  return directionEndpoints(document, direction).some((endpoint) => endpoint.name === name && endpoint.dpt === dptValue);
}

/** Client-side checks mirror the stable server paths and intentionally leave the server as authority. */
export function validateAutomation(document: AutomationDocument): AutomationFieldError[] {
  const errors: AutomationFieldError[] = [];
  const names = new Map<string, string>();
  const endpointGroups: Array<['inputs' | 'outputs', AutomationEndpoint[]]> = [['inputs', document.inputs], ['outputs', document.outputs]];
  for (const [direction, endpoints] of endpointGroups) {
    endpoints.forEach((endpoint, index) => {
      const path = `${direction}[${index}]`;
      if (!namePattern.test(endpoint.name)) addError(errors, `${path}.name`, 'must start with a lowercase ASCII letter and contain only lowercase letters, digits, _, -, or .');
      const prior = names.get(endpoint.name);
      if (prior) addError(errors, `${path}.name`, `duplicates ${prior}`);
      else names.set(endpoint.name, path);
      if (!DPTS.includes(endpoint.dpt)) addError(errors, `${path}.dpt`, 'must be 1.001 or 5.001');
    });
  }

  const bindingNames = new Map<string, string>();
  document.knx_bindings.forEach((binding, index) => {
    const path = `knx_bindings[${index}]`;
    const existing = bindingNames.get(binding.endpoint);
    if (existing) addError(errors, `${path}.endpoint`, `duplicate binding; already declared at ${existing}`);
    else bindingNames.set(binding.endpoint, path);
    if (!names.has(binding.endpoint)) addError(errors, `${path}.endpoint`, 'must reference an existing endpoint');
    const match = groupAddressPattern.exec(binding.group_address);
    if (!match || Number(match[1]) > 31 || Number(match[2]) > 7 || Number(match[3]) > 255 || binding.group_address === '0/0/0') {
      addError(errors, `${path}.group_address`, 'must be a canonical non-broadcast group address');
    }
  });
  for (const [name, path] of names) if (!bindingNames.has(name)) addError(errors, `${path}.name`, 'must have exactly one KNX binding');
  const addresses = new Map<string, string>();
  document.knx_bindings.forEach((binding, index) => {
    const prior = addresses.get(binding.group_address);
    if (prior) addError(errors, `knx_bindings[${index}].group_address`, `duplicates ${prior}`);
    else addresses.set(binding.group_address, `knx_bindings[${index}].group_address`);
  });

  const timed = document.behaviors.timed_bool;
  if (!hasName(document, timed.input, 'input', '1.001')) addError(errors, 'behaviors.timed_bool.input', 'must reference a DPT 1.001 input');
  if (!hasName(document, timed.output, 'output', '1.001')) addError(errors, 'behaviors.timed_bool.output', 'must reference a DPT 1.001 output');
  if (!Number.isInteger(timed.off_delay_ms) || timed.off_delay_ms < 1 || timed.off_delay_ms > MAX_OFF_DELAY_MS) addError(errors, 'behaviors.timed_bool.off_delay_ms', `must be an integer from 1 to ${MAX_OFF_DELAY_MS}`);
  const percentage = document.behaviors.percentage_forward;
  if (!hasName(document, percentage.input, 'input', '5.001')) addError(errors, 'behaviors.percentage_forward.input', 'must reference a DPT 5.001 input');
  if (!hasName(document, percentage.output, 'output', '5.001')) addError(errors, 'behaviors.percentage_forward.output', 'must reference a DPT 5.001 output');

  const addressFor = (name: string) => document.knx_bindings.find((binding) => binding.endpoint === name)?.group_address;
  if (addressFor(timed.input) && addressFor(timed.input) === addressFor(timed.output)) addError(errors, 'behaviors.timed_bool.output', 'input and output group addresses must differ');
  if (addressFor(percentage.input) && addressFor(percentage.input) === addressFor(percentage.output)) addError(errors, 'behaviors.percentage_forward.output', 'input and output group addresses must differ');
  return errors;
}

export function renameEndpoint(document: AutomationDocument, from: string, to: string): AutomationDocument {
  const rename = (name: string) => name === from ? to : name;
  return {
    ...document,
    inputs: document.inputs.map((endpoint) => endpoint.name === from ? { ...endpoint, name: to } : endpoint),
    outputs: document.outputs.map((endpoint) => endpoint.name === from ? { ...endpoint, name: to } : endpoint),
    knx_bindings: document.knx_bindings.map((binding) => binding.endpoint === from ? { ...binding, endpoint: to } : binding),
    behaviors: {
      timed_bool: { ...document.behaviors.timed_bool, input: rename(document.behaviors.timed_bool.input), output: rename(document.behaviors.timed_bool.output) },
      percentage_forward: { ...document.behaviors.percentage_forward, input: rename(document.behaviors.percentage_forward.input), output: rename(document.behaviors.percentage_forward.output) }
    }
  };
}

/** Remove only the declaration; references intentionally remain to make the draft visibly invalid. */
export function removeEndpoint(document: AutomationDocument, name: string): AutomationDocument {
  return {
    ...document,
    inputs: document.inputs.filter((endpoint) => endpoint.name !== name),
    outputs: document.outputs.filter((endpoint) => endpoint.name !== name)
  };
}

export function removeBinding(document: AutomationDocument, index: number): AutomationDocument {
  return { ...document, knx_bindings: document.knx_bindings.filter((_, current) => current !== index) };
}

export function compatibleEndpoints(document: AutomationDocument, direction: 'input' | 'output', dptValue: Dpt): AutomationEndpoint[] {
  return directionEndpoints(document, direction).filter((endpoint) => endpoint.dpt === dptValue);
}
