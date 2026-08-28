export const DPTS = ['1.001', '5.001'] as const;
export const MAX_SOURCE_BYTES = 64 * 1024;
export type Dpt = (typeof DPTS)[number];

export interface AutomationEndpoint { name: string; dpt: Dpt; }
export interface KnxBinding { endpoint: string; group_address: string; }
export interface AutomationLogic { source: string; }

export interface AutomationDocument {
  inputs: AutomationEndpoint[];
  outputs: AutomationEndpoint[];
  knx_bindings: KnxBinding[];
  logic: AutomationLogic;
}

export interface AutomationFieldError { path: string; message: string; }
export interface AutomationEnvelope {
  document: AutomationDocument;
  revision: number;
  activeStructuralRevision?: number | null;
  savedStructuralRevision?: number | null;
  activeLogicRevision?: number | null;
  savedLogicRevision?: number | null;
  restartRequired?: boolean;
}
export interface AutomationSaveResult {
  revision: number;
  logicActivated: boolean;
  activeLogicRevision: number | null;
  restartRequired: boolean;
  cancelledTimers?: string[];
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

export const emptyAutomation = (): AutomationDocument => ({ inputs: [], outputs: [], knx_bindings: [], logic: { source: '' } });
const isRecord = (value: unknown): value is JsonRecord => typeof value === 'object' && value !== null && !Array.isArray(value);
function record(value: unknown, path: string): JsonRecord { if (!isRecord(value)) throw new AutomationDecodeError(path, 'expected an object'); return value; }
function required(source: JsonRecord, name: string, path: string): unknown {
  if (!(name in source) || source[name] === null || source[name] === undefined) throw new AutomationDecodeError(path, 'required field is missing');
  return source[name];
}
function nonEmptyString(value: unknown, path: string): string { if (typeof value !== 'string' || value.length === 0) throw new AutomationDecodeError(path, 'expected a non-empty string'); return value; }
function nonNegativeInteger(value: unknown, path: string): number {
  if (typeof value !== 'number' || !Number.isInteger(value) || value < 0) throw new AutomationDecodeError(path, 'expected a non-negative integer');
  return value;
}
function dpt(value: unknown, path: string): Dpt {
  let normalized = value;
  if (isRecord(value) && typeof value.major === 'number' && typeof value.subtype === 'number' && Number.isInteger(value.major) && Number.isInteger(value.subtype)) normalized = `${value.major}.${value.subtype.toString().padStart(3, '0')}`;
  if (normalized !== '1.001' && normalized !== '5.001') throw new AutomationDecodeError(path, 'expected DPT 1.001 or 5.001');
  return normalized;
}
function list(value: unknown, path: string): unknown[] { if (!Array.isArray(value)) throw new AutomationDecodeError(path, 'expected an array'); return value; }
function endpoint(value: unknown, path: string): AutomationEndpoint {
  const source = record(value, path);
  return { name: nonEmptyString(required(source, 'name', `${path}.name`), `${path}.name`), dpt: dpt(required(source, 'dpt', `${path}.dpt`), `${path}.dpt`) };
}
export function decodeAutomationDocument(input: unknown, path = 'document'): AutomationDocument {
  const source = record(input, path);
  const logic = record(required(source, 'logic', `${path}.logic`), `${path}.logic`);
  const rawSource = required(logic, 'source', `${path}.logic.source`);
  if (typeof rawSource !== 'string' || rawSource.length === 0) throw new AutomationDecodeError(`${path}.logic.source`, 'expected a non-empty string');
  if (new TextEncoder().encode(rawSource).byteLength > MAX_SOURCE_BYTES) throw new AutomationDecodeError(`${path}.logic.source`, `must be at most ${MAX_SOURCE_BYTES} bytes`);
  const bindings = list(required(source, 'knx_bindings', `${path}.knx_bindings`), `${path}.knx_bindings`).map((value, index) => {
    const binding = record(value, `${path}.knx_bindings[${index}]`);
    return { endpoint: nonEmptyString(required(binding, 'endpoint', `${path}.knx_bindings[${index}].endpoint`), `${path}.knx_bindings[${index}].endpoint`), group_address: nonEmptyString(required(binding, 'group_address', `${path}.knx_bindings[${index}].group_address`), `${path}.knx_bindings[${index}].group_address`) };
  });
  return {
    inputs: list(required(source, 'inputs', `${path}.inputs`), `${path}.inputs`).map((value, index) => endpoint(value, `${path}.inputs[${index}]`)),
    outputs: list(required(source, 'outputs', `${path}.outputs`), `${path}.outputs`).map((value, index) => endpoint(value, `${path}.outputs[${index}]`)),
    knx_bindings: bindings,
    logic: { source: rawSource }
  };
}
function revision(value: unknown, path: string): number { return nonNegativeInteger(value, path); }
function optionalRevision(value: unknown, path: string): number | null { return value === undefined || value === null ? null : revision(value, path); }

export function decodeAutomation(input: unknown): AutomationEnvelope {
  const source = record(input, 'automation');
  const document = decodeAutomationDocument(source.document ?? source.automation ?? source);
  return {
    document,
    revision: revision(source.revision ?? source.content_revision, 'revision'),
    ...(source.active_structural_revision !== undefined || source.activeStructuralRevision !== undefined ? { activeStructuralRevision: optionalRevision(source.active_structural_revision ?? source.activeStructuralRevision, 'active_structural_revision') } : {}),
    ...(source.saved_structural_revision !== undefined || source.savedStructuralRevision !== undefined ? { savedStructuralRevision: optionalRevision(source.saved_structural_revision ?? source.savedStructuralRevision, 'saved_structural_revision') } : {}),
    ...(source.active_logic_revision !== undefined || source.activeLogicRevision !== undefined ? { activeLogicRevision: optionalRevision(source.active_logic_revision ?? source.activeLogicRevision, 'active_logic_revision') } : {}),
    ...(source.saved_logic_revision !== undefined || source.savedLogicRevision !== undefined ? { savedLogicRevision: optionalRevision(source.saved_logic_revision ?? source.savedLogicRevision, 'saved_logic_revision') } : {}),
    ...(source.restart_required !== undefined || source.restartRequired !== undefined ? { restartRequired: source.restart_required === true || source.restartRequired === true } : {})
  };
}
function fieldErrors(value: unknown): AutomationFieldError[] {
  if (!isRecord(value)) return [];
  const raw = value.errors ?? value.field_errors ?? value.fields;
  if (Array.isArray(raw)) return raw.flatMap((item) => {
    if (!isRecord(item) || (typeof item.path !== 'string' && typeof item.field !== 'string') || typeof item.message !== 'string') return [];
    return [{ path: (item.path ?? item.field) as string, message: item.message }];
  });
  if (isRecord(raw)) return Object.entries(raw).flatMap(([path, message]) => {
    if (typeof message === 'string') return [{ path, message }];
    if (Array.isArray(message) && typeof message[0] === 'string') return [{ path, message: message[0] }];
    return [];
  });
  return [];
}
async function jsonOrNull(response: Response): Promise<unknown> { try { return await response.json(); } catch { return null; } }
export async function loadAutomation(fetchImpl: FetchLike = fetch): Promise<AutomationEnvelope> {
  const response = await fetchImpl('/api/automation', { headers: { accept: 'application/json' } });
  if (!response.ok) throw new AutomationApiError(response.status, `Automation request failed (${response.status})`);
  return decodeAutomation(await response.json());
}
export async function saveAutomation(document: AutomationDocument, replacedRevision: number, fetchImpl: FetchLike = fetch): Promise<AutomationSaveResult> {
  const response = await fetchImpl('/api/automation', { method: 'PUT', headers: { accept: 'application/json', 'content-type': 'application/json' }, body: JSON.stringify({ document, revision: replacedRevision }) });
  if (response.ok) {
    const source = record(await response.json(), 'save');
    const cancellationRaw = source.cancelled_timers ?? source.cancelledTimers;
    return {
      revision: revision(source.revision ?? source.content_revision, 'save.revision'),
      logicActivated: source.logic_activated === true || source.logicActivated === true,
      activeLogicRevision: optionalRevision(source.active_logic_revision ?? source.activeLogicRevision, 'save.active_logic_revision'),
      restartRequired: source.restart_required === true || source.restartRequired === true,
      ...(Array.isArray(cancellationRaw)
        ? { cancelledTimers: (cancellationRaw as unknown[]).map((value: unknown, index: number) => nonEmptyString(value, `save.cancelled_timers[${index}]`)) }
        : {})
    };
  }
  const body = await jsonOrNull(response);
  const errors = fieldErrors(body);
  let latest: AutomationEnvelope | null = null;
  if (response.status === 409 && body !== null) {
    try { latest = decodeAutomation(isRecord(body) && body.latest !== undefined ? body.latest : isRecord(body) && body.current !== undefined ? body.current : body); } catch { latest = null; }
  }
  throw new AutomationApiError(response.status, response.status === 409 ? 'The saved automation changed. Reload the latest document before saving.' : `Automation save failed (${response.status})`, errors, latest);
}

const namePattern = /^[a-z][a-z0-9_.-]*$/;
const groupAddressPattern = /^(0|[1-9]\d{0,1})\/(0|[1-7])\/(0|[1-9]\d{0,2})$/;
function addError(errors: AutomationFieldError[], path: string, message: string): void { errors.push({ path, message }); }
function directionEndpoints(document: AutomationDocument, direction: 'input' | 'output'): AutomationEndpoint[] { return direction === 'input' ? document.inputs : document.outputs; }

/** Client-side structural checks mirror stable server paths; the server remains authoritative for Lua syntax. */
export function validateAutomation(document: AutomationDocument): AutomationFieldError[] {
  const errors: AutomationFieldError[] = [];
  const names = new Map<string, string>();
  for (const [direction, endpoints] of [['inputs', document.inputs], ['outputs', document.outputs]] as const) endpoints.forEach((endpoint, index) => {
    const path = `${direction}[${index}]`;
    if (!namePattern.test(endpoint.name)) addError(errors, `${path}.name`, 'must start with a lowercase ASCII letter and contain only lowercase letters, digits, _, -, or .');
    const prior = names.get(endpoint.name);
    if (prior) addError(errors, `${path}.name`, `duplicates ${prior}`); else names.set(endpoint.name, path);
    if (!DPTS.includes(endpoint.dpt)) addError(errors, `${path}.dpt`, 'must be 1.001 or 5.001');
  });
  const bindingNames = new Map<string, string>();
  document.knx_bindings.forEach((binding, index) => {
    const path = `knx_bindings[${index}]`;
    const existing = bindingNames.get(binding.endpoint);
    if (existing) addError(errors, `${path}.endpoint`, `duplicate binding; already declared at ${existing}`); else bindingNames.set(binding.endpoint, path);
    if (!names.has(binding.endpoint)) addError(errors, `${path}.endpoint`, 'must reference an existing endpoint');
    const match = groupAddressPattern.exec(binding.group_address);
    if (!match || Number(match[1]) > 31 || Number(match[2]) > 7 || Number(match[3]) > 255 || binding.group_address === '0/0/0') addError(errors, `${path}.group_address`, 'must be a canonical non-broadcast group address');
  });
  for (const [name, path] of names) if (!bindingNames.has(name)) addError(errors, `${path}.name`, 'must have exactly one KNX binding');
  const addresses = new Map<string, string>();
  document.knx_bindings.forEach((binding, index) => { const prior = addresses.get(binding.group_address); if (prior) addError(errors, `knx_bindings[${index}].group_address`, `duplicates ${prior}`); else addresses.set(binding.group_address, `knx_bindings[${index}].group_address`); });
  const source = document.logic.source;
  if (typeof source !== 'string' || source.trim().length === 0) addError(errors, 'logic.source', 'must contain a Lua source program');
  else if (new TextEncoder().encode(source).byteLength > MAX_SOURCE_BYTES) addError(errors, 'logic.source', `must be at most ${MAX_SOURCE_BYTES} bytes`);
  return errors;
}
export function renameEndpoint(document: AutomationDocument, from: string, to: string): AutomationDocument {
  return { ...document, inputs: document.inputs.map((endpoint) => endpoint.name === from ? { ...endpoint, name: to } : endpoint), outputs: document.outputs.map((endpoint) => endpoint.name === from ? { ...endpoint, name: to } : endpoint), knx_bindings: document.knx_bindings.map((binding) => binding.endpoint === from ? { ...binding, endpoint: to } : binding) };
}
export function removeEndpoint(document: AutomationDocument, name: string): AutomationDocument { return { ...document, inputs: document.inputs.filter((endpoint) => endpoint.name !== name), outputs: document.outputs.filter((endpoint) => endpoint.name !== name) }; }
export function removeBinding(document: AutomationDocument, index: number): AutomationDocument { return { ...document, knx_bindings: document.knx_bindings.filter((_, current) => current !== index) }; }
export function compatibleEndpoints(document: AutomationDocument, direction: 'input' | 'output', dptValue: Dpt): AutomationEndpoint[] { return directionEndpoints(document, direction).filter((endpoint) => endpoint.dpt === dptValue); }
