export const DPTS = ['1.001', '5.001'] as const;
export const MAX_SOURCE_BYTES = 64 * 1024;
export const MAX_BLOCKS = 64;
export type Dpt = (typeof DPTS)[number];

export interface AutomationEndpoint { name: string; dpt: Dpt; }
export interface KnxBinding { endpoint: string; group_address: string; }
export interface AutomationLogic { source: string; }
export interface AutomationBlock {
  id: string;
  revision: number;
  enabled: boolean;
  inputs: AutomationEndpoint[];
  outputs: AutomationEndpoint[];
  knx_bindings: KnxBinding[];
  source: string;
}
export interface AutomationDocument {
  blocks?: AutomationBlock[];
  /** Transitional fields are required only by the TypeScript compatibility shape; canonical saves serialize blocks alone. */
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
  activeLogicRevision?: RevisionToken | null;
  savedLogicRevision?: RevisionToken | null;
  restartRequired?: boolean;
  blocks?: Array<{ id: string; activeLogicRevision: RevisionToken | null; savedLogicRevision: RevisionToken | null; activeEnabled: boolean | null; savedEnabled: boolean | null }>;
}
export interface CancelledTimersByBlock { blockId: string; timers: string[]; }
export interface AutomationSaveResult {
  revision: number;
  logicActivated: boolean;
  activeLogicRevision: RevisionToken | null;
  restartRequired: boolean;
  changedBlockIds?: string[];
  cancelledTimers?: string[];
  cancelledTimersByBlock?: CancelledTimersByBlock[];
  blocks?: Array<{ id: string; savedRevision: number }>;
}

type FetchLike = typeof fetch;
type JsonRecord = Record<string, unknown>;
export class AutomationDecodeError extends Error {
  constructor(path: string, message: string) { super(`Malformed automation data at ${path}: ${message}`); this.name = 'AutomationDecodeError'; }
}
export class AutomationApiError extends Error {
  readonly status: number; readonly fieldErrors: AutomationFieldError[]; readonly latest: AutomationEnvelope | null;
  constructor(status: number, message: string, fieldErrors: AutomationFieldError[] = [], latest: AutomationEnvelope | null = null) { super(message); this.name = 'AutomationApiError'; this.status = status; this.fieldErrors = fieldErrors; this.latest = latest; }
}
export const emptyAutomation = (): AutomationDocument => ({ blocks: [], inputs: [], outputs: [], knx_bindings: [], logic: { source: '' } });
function canonicalBlocks(document: AutomationDocument): AutomationBlock[] {
  if (document.inputs && document.outputs && document.knx_bindings && document.logic) return [{ id: 'default', revision: 1, enabled: true, inputs: document.inputs, outputs: document.outputs, knx_bindings: document.knx_bindings, source: document.logic.source }];
  if (Array.isArray(document.blocks) && document.blocks.length) return document.blocks;
  return document.blocks ?? [];
}
const isRecord = (value: unknown): value is JsonRecord => typeof value === 'object' && value !== null && !Array.isArray(value);
function record(value: unknown, path: string): JsonRecord { if (!isRecord(value)) throw new AutomationDecodeError(path, 'expected an object'); return value; }
function required(source: JsonRecord, name: string, path: string): unknown { if (!(name in source) || source[name] === null || source[name] === undefined) throw new AutomationDecodeError(path, 'required field is missing'); return source[name]; }
function field(source: JsonRecord, ...names: string[]): unknown { for (const name of names) if (name in source) return source[name]; return undefined; }
function nonEmptyString(value: unknown, path: string): string { if (typeof value !== 'string' || value.length === 0) throw new AutomationDecodeError(path, 'expected a non-empty string'); return value; }
function nonNegativeInteger(value: unknown, path: string): number { if (typeof value !== 'number' || !Number.isInteger(value) || value < 0) throw new AutomationDecodeError(path, 'expected a non-negative integer'); return value; }
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
function binding(value: unknown, path: string): KnxBinding {
  const source = record(value, path);
  const endpointName = field(source, 'endpoint', 'name');
  const address = field(source, 'group_address', 'groupAddress', 'address');
  return { endpoint: nonEmptyString(endpointName, `${path}.endpoint`), group_address: nonEmptyString(address, `${path}.group_address`) };
}
function sourceText(value: unknown, path: string): string {
  if (typeof value !== 'string' || value.length === 0) throw new AutomationDecodeError(path, 'expected a non-empty string');
  if (new TextEncoder().encode(value).byteLength > MAX_SOURCE_BYTES) throw new AutomationDecodeError(path, `must be at most ${MAX_SOURCE_BYTES} bytes`);
  return value;
}
function blockId(value: unknown, path: string): string { const id = nonEmptyString(value, path); if (new TextEncoder().encode(id).byteLength > 64 || !/^[a-z][a-z0-9_]*$/.test(id)) throw new AutomationDecodeError(path, 'must be 1–64 bytes, start with a lowercase ASCII letter, and contain only lowercase letters, digits, or _'); return id; }
function block(value: unknown, path: string): AutomationBlock {
  const source = record(value, path);
  const inputs = list(required(source, 'inputs', `${path}.inputs`), `${path}.inputs`).map((item, index) => endpoint(item, `${path}.inputs[${index}]`));
  const outputs = list(required(source, 'outputs', `${path}.outputs`), `${path}.outputs`).map((item, index) => endpoint(item, `${path}.outputs[${index}]`));
  const bindings = list(required(source, 'knx_bindings', `${path}.knx_bindings`), `${path}.knx_bindings`).map((item, index) => binding(item, `${path}.knx_bindings[${index}]`));
  const logic = field(source, 'source', 'logic');
  const sourceTextValue = isRecord(logic) ? required(logic, 'source', `${path}.source`) : logic;
  if (typeof source.enabled !== 'boolean') throw new AutomationDecodeError(`${path}.enabled`, 'expected a boolean');
  return { id: blockId(required(source, 'id', `${path}.id`), `${path}.id`), revision: typeof source.revision === 'number' && Number.isInteger(source.revision) && source.revision > 0 ? source.revision : 1, enabled: source.enabled, inputs, outputs, knx_bindings: bindings, source: sourceText(sourceTextValue, `${path}.source`) };
}

/** Decode canonical blocks. The legacy branch exists only to keep Milestone 7 fixtures readable during migration. */
export function decodeAutomationDocument(input: unknown, path = 'document'): AutomationDocument {
  const source = record(input, path); const rawBlocks = field(source, 'blocks');
  if (rawBlocks !== undefined) {
    const values = list(rawBlocks, `${path}.blocks`);
    if (values.length === 0) throw new AutomationDecodeError(`${path}.blocks`, 'must contain at least one block');
    if (values.length > MAX_BLOCKS) throw new AutomationDecodeError(`${path}.blocks`, `must contain at most ${MAX_BLOCKS} blocks`);
    const blocks = values.map((value, index) => block(value, `${path}.blocks[${index}]`)); const seen = new Set<string>();
    blocks.forEach((item, index) => { if (seen.has(item.id)) throw new AutomationDecodeError(`${path}.blocks[${index}].id`, `duplicate block id ${item.id}`); seen.add(item.id); });
    return { blocks } as AutomationDocument;
  }
  const legacyLogic = record(required(source, 'logic', `${path}.logic`), `${path}.logic`); const inputs = list(required(source, 'inputs', `${path}.inputs`), `${path}.inputs`).map((value, index) => endpoint(value, `${path}.inputs[${index}]`)); const outputs = list(required(source, 'outputs', `${path}.outputs`), `${path}.outputs`).map((value, index) => endpoint(value, `${path}.outputs[${index}]`)); const bindings = list(required(source, 'knx_bindings', `${path}.knx_bindings`), `${path}.knx_bindings`).map((value, index) => binding(value, `${path}.knx_bindings[${index}]`)); const sourceValue = sourceText(required(legacyLogic, 'source', `${path}.logic.source`), `${path}.logic.source`);
  return { inputs, outputs, knx_bindings: bindings, logic: { source: sourceValue } };
}
function logicRevision(value: unknown, path: string): RevisionToken {
  const token = parseRevisionToken(value);
  if (token === null) throw new AutomationDecodeError(path, 'expected a non-negative decimal revision token');
  return token;
}
function optionalLogicRevision(value: unknown, path: string): RevisionToken | null { return value === undefined || value === null ? null : logicRevision(value, path); }
export function decodeAutomation(input: unknown): AutomationEnvelope {
  const source = record(input, 'automation'); if (!('revision' in source) && !('content_revision' in source)) source.revision = 0; const document = decodeAutomationDocument(source.document ?? source.automation ?? source);
  const rawBlocks = field(source, 'blocks');
  const blockRevisions = Array.isArray(rawBlocks) ? rawBlocks.map((value, index) => { const item = record(value, `automation.blocks[${index}]`); const id = nonEmptyString(required(item, 'id', `automation.blocks[${index}].id`), `automation.blocks[${index}].id`); const active = field(item, 'active_logic_revision', 'activeLogicRevision'); const saved = field(item, 'saved_logic_revision', 'savedLogicRevision'); const activeEnabled = field(item, 'active_enabled', 'activeEnabled'); const savedEnabled = field(item, 'saved_enabled', 'savedEnabled'); return { id, activeLogicRevision: optionalLogicRevision(active, `automation.blocks[${index}].active_logic_revision`), savedLogicRevision: optionalLogicRevision(saved, `automation.blocks[${index}].saved_logic_revision`), activeEnabled: typeof activeEnabled === 'boolean' ? activeEnabled : null, savedEnabled: typeof savedEnabled === 'boolean' ? savedEnabled : null }; }) : undefined;
  return { document, revision: nonNegativeInteger(source.revision ?? source.content_revision, 'revision'), ...(source.active_structural_revision !== undefined || source.activeStructuralRevision !== undefined ? { activeStructuralRevision: nonNegativeInteger(source.active_structural_revision ?? source.activeStructuralRevision, 'active_structural_revision') } : {}), ...(source.saved_structural_revision !== undefined || source.savedStructuralRevision !== undefined ? { savedStructuralRevision: nonNegativeInteger(source.saved_structural_revision ?? source.savedStructuralRevision, 'saved_structural_revision') } : {}), ...(source.active_logic_revision !== undefined || source.activeLogicRevision !== undefined ? { activeLogicRevision: optionalLogicRevision(source.active_logic_revision ?? source.activeLogicRevision, 'active_logic_revision') } : {}), ...(source.saved_logic_revision !== undefined || source.savedLogicRevision !== undefined ? { savedLogicRevision: optionalLogicRevision(source.saved_logic_revision ?? source.savedLogicRevision, 'saved_logic_revision') } : {}), ...(source.restart_required !== undefined || source.restartRequired !== undefined ? { restartRequired: source.restart_required === true || source.restartRequired === true } : {}), ...(blockRevisions ? { blocks: blockRevisions } : {}) };
}
function fieldErrors(value: unknown): AutomationFieldError[] {
  if (!isRecord(value)) return []; const raw = value.errors ?? value.field_errors ?? value.fields;
  if (Array.isArray(raw)) return raw.flatMap((item) => { if (!isRecord(item) || (typeof item.path !== 'string' && typeof item.field !== 'string') || typeof item.message !== 'string') return []; return [{ path: (item.path ?? item.field) as string, message: item.message }]; });
  if (isRecord(raw)) return Object.entries(raw).flatMap(([path, message]) => typeof message === 'string' ? [{ path, message }] : Array.isArray(message) && typeof message[0] === 'string' ? [{ path, message: message[0] }] : []); return [];
}
async function jsonOrNull(response: Response): Promise<unknown> { try { return await response.json(); } catch { return null; } }
export async function loadAutomation(fetchImpl: FetchLike = fetch): Promise<AutomationEnvelope> { const response = await fetchImpl('/api/automation', { headers: { accept: 'application/json' } }); if (!response.ok) throw new AutomationApiError(response.status, `Automation request failed (${response.status})`); return decodeAutomation(await response.json()); }
export async function saveAutomation(document: AutomationDocument, replacedRevision: number, fetchImpl: FetchLike = fetch): Promise<AutomationSaveResult> {
  const persisted = document.blocks?.length ? { blocks: document.blocks } : document;
  const response = await fetchImpl('/api/automation', { method: 'PUT', headers: { accept: 'application/json', 'content-type': 'application/json' }, body: JSON.stringify({ document: persisted }) });
  if (response.ok) {
    const source = record(await response.json(), 'save'); const cancellationRaw = field(source, 'cancelled_timers', 'cancelledTimers');
    const cancelledTimersByBlock = Array.isArray(cancellationRaw) && cancellationRaw.every((item) => isRecord(item)) ? cancellationRaw.map((item, index) => ({ blockId: nonEmptyString(field(item, 'block_id', 'blockId', 'id'), `save.cancelled_timers[${index}].block_id`), timers: list(field(item, 'timers', 'names') ?? [], `save.cancelled_timers[${index}].timers`).map((timer, timerIndex) => nonEmptyString(timer, `save.cancelled_timers[${index}].timers[${timerIndex}]`)) })) : undefined;
    const flatCancelled = Array.isArray(cancellationRaw) && !cancelledTimersByBlock ? cancellationRaw.map((value, index) => nonEmptyString(value, `save.cancelled_timers[${index}]`)) : undefined; const changedRaw = field(source, 'changed_block_ids', 'changedBlockIds');
    const blockStatusRaw = Array.isArray(source.blocks) ? source.blocks : [];
    const blockStatus = blockStatusRaw.flatMap((item, index) => { if (!isRecord(item) || typeof item.id !== 'string') return []; const saved = item.saved_revision ?? item.savedRevision ?? item.saved_logic_revision ?? item.savedLogicRevision; return typeof saved === 'string' && /^\d+$/.test(saved) ? [{ id: item.id, savedRevision: Number(saved) }] : typeof saved === 'number' && Number.isInteger(saved) ? [{ id: item.id, savedRevision: saved }] : []; });
    return { revision: nonNegativeInteger(source.revision ?? source.content_revision ?? 0, 'save.revision'), logicActivated: source.logic_activated === true || source.logicActivated === true || source.activated === true, activeLogicRevision: optionalLogicRevision(source.active_logic_revision ?? source.activeLogicRevision, 'save.active_logic_revision'), restartRequired: source.restart_required === true || source.restartRequired === true, ...(blockStatus.length ? { blocks: blockStatus } : {}), ...(Array.isArray(changedRaw) ? { changedBlockIds: changedRaw.map((value, index) => nonEmptyString(value, `save.changed_block_ids[${index}]`)) } : {}), ...(cancelledTimersByBlock ? { cancelledTimersByBlock } : {}), ...(flatCancelled ? { cancelledTimers: flatCancelled } : {}) };
  }
  const body = await jsonOrNull(response); const errors = fieldErrors(body); let latest: AutomationEnvelope | null = null; if (response.status === 409 && body !== null) { try { latest = decodeAutomation(isRecord(body) && body.latest !== undefined ? body.latest : isRecord(body) && body.current !== undefined ? body.current : body); } catch { latest = null; } }
  throw new AutomationApiError(response.status, response.status === 409 ? 'The saved automation changed. Reload the latest document before saving.' : `Automation save failed (${response.status})`, errors, latest);
}
const namePattern = /^[a-z][a-z0-9_]*$/; const groupAddressPattern = /^(0|[1-9]\d{0,1})\/(0|[1-7])\/(0|[1-9]\d{0,2})$/;
function addError(errors: AutomationFieldError[], path: string, message: string): void { errors.push({ path, message }); }
function validateBlock(blockValue: AutomationBlock, index: number, errors: AutomationFieldError[]): void {
  const base = `blocks[${index}]`; if (!namePattern.test(blockValue.id)) addError(errors, `${base}.id`, 'must start with a lowercase ASCII letter and contain only lowercase letters, digits, or _'); const names = new Map<string, string>();
  for (const [direction, endpoints] of [['inputs', blockValue.inputs], ['outputs', blockValue.outputs]] as const) endpoints.forEach((item, endpointIndex) => { const path = `${base}.${direction}[${endpointIndex}]`; if (!namePattern.test(item.name)) addError(errors, `${path}.name`, 'must start with a lowercase ASCII letter and contain only lowercase letters, digits, or _'); const prior = names.get(item.name); if (prior) addError(errors, `${path}.name`, `duplicates ${prior}`); else names.set(item.name, path); if (!DPTS.includes(item.dpt)) addError(errors, `${path}.dpt`, 'must be 1.001 or 5.001'); });
  const bindingNames = new Map<string, string>(); const addresses = new Map<string, string>(); blockValue.knx_bindings.forEach((item, bindingIndex) => { const path = `${base}.knx_bindings[${bindingIndex}]`; const prior = bindingNames.get(item.endpoint); if (prior) addError(errors, `${path}.endpoint`, `duplicate binding; already declared at ${prior}`); else bindingNames.set(item.endpoint, path); if (!names.has(item.endpoint)) addError(errors, `${path}.endpoint`, 'must reference an existing endpoint'); const match = groupAddressPattern.exec(item.group_address); if (!match || Number(match[1]) > 31 || Number(match[2]) > 7 || Number(match[3]) > 255 || item.group_address === '0/0/0') addError(errors, `${path}.group_address`, 'must be a canonical non-broadcast group address'); const priorAddress = addresses.get(item.group_address); if (priorAddress) addError(errors, `${path}.group_address`, `duplicates ${priorAddress} within this block`); else addresses.set(item.group_address, path); });
  for (const [name, path] of names) if (!bindingNames.has(name)) addError(errors, `${path}.name`, 'must have exactly one KNX binding'); if (typeof blockValue.source !== 'string' || blockValue.source.trim().length === 0) addError(errors, `${base}.source`, 'must contain a Lua source program'); else if (new TextEncoder().encode(blockValue.source).byteLength > MAX_SOURCE_BYTES) addError(errors, `${base}.source`, `must be at most ${MAX_SOURCE_BYTES} bytes`);
}
export function validateAutomation(document: AutomationDocument): AutomationFieldError[] { const errors: AutomationFieldError[] = []; const legacy = Boolean(document.logic && document.inputs && document.outputs && document.knx_bindings); const blocks = canonicalBlocks(document); if (!blocks.length) { addError(errors, 'blocks', 'must contain at least one block'); return errors; } if (blocks.length > MAX_BLOCKS) addError(errors, 'blocks', `must contain at most ${MAX_BLOCKS} blocks`); const ids = new Map<string, number>(); blocks.forEach((item, index) => { const prior = ids.get(item.id); if (prior !== undefined) addError(errors, `${legacy ? '' : `blocks[${index}].`}id`, `duplicates ${legacy ? '' : `blocks[${prior}].`}id`); else ids.set(item.id, index); const before = errors.length; validateBlock(item, index, errors); if (legacy) errors.splice(before, errors.length - before, ...errors.slice(before).map((error) => ({ ...error, path: error.path.replace(`blocks[${index}].`, '') === 'source' ? 'logic.source' : error.path.replace(`blocks[${index}].`, '') }))); }); return errors; }
export function blockWithSource(document: AutomationDocument, id: string, source: string): AutomationDocument { return { blocks: canonicalBlocks(document).map((item) => item.id === id ? { ...item, source } : item) } as AutomationDocument; }
export function blockById(document: AutomationDocument, id: string): AutomationBlock | null { return canonicalBlocks(document).find((item) => item.id === id) ?? null; }
export function renameEndpoint(document: AutomationDocument, blockIdOrFrom: string, fromOrTo: string, maybeTo?: string): AutomationDocument {
  const legacy = maybeTo === undefined; const blockId = legacy ? 'default' : blockIdOrFrom; const from = legacy ? blockIdOrFrom : fromOrTo; const to = legacy ? fromOrTo : maybeTo;
  const next = { blocks: canonicalBlocks(document).map((blockValue) => blockValue.id === blockId ? { ...blockValue, inputs: blockValue.inputs.map((item) => item.name === from ? { ...item, name: to } : item), outputs: blockValue.outputs.map((item) => item.name === from ? { ...item, name: to } : item), knx_bindings: blockValue.knx_bindings.map((item) => item.endpoint === from ? { ...item, endpoint: to } : item) } : blockValue) };
  return legacy ? { inputs: next.blocks[0]?.inputs ?? [], outputs: next.blocks[0]?.outputs ?? [], knx_bindings: next.blocks[0]?.knx_bindings ?? [], logic: { source: next.blocks[0]?.source ?? '' }, blocks: next.blocks } : next as AutomationDocument;
}
export function removeEndpoint(document: AutomationDocument, blockIdOrName: string, maybeName?: string): AutomationDocument { const legacy = maybeName === undefined; const blockId = legacy ? 'default' : blockIdOrName; const name = legacy ? blockIdOrName : maybeName; const blocks = canonicalBlocks(document).map((blockValue) => blockValue.id === blockId ? { ...blockValue, inputs: blockValue.inputs.filter((item) => item.name !== name), outputs: blockValue.outputs.filter((item) => item.name !== name) } : blockValue); return legacy ? { ...document, inputs: blocks[0]?.inputs ?? [], outputs: blocks[0]?.outputs ?? [], blocks } : { blocks } as AutomationDocument; }
export function removeBinding(document: AutomationDocument, blockIdOrIndex: string | number, indexMaybe?: number): AutomationDocument { const legacy = indexMaybe === undefined; const blockId = legacy ? 'default' : String(blockIdOrIndex); const index = legacy ? Number(blockIdOrIndex) : indexMaybe; const blocks = canonicalBlocks(document).map((blockValue) => blockValue.id === blockId ? { ...blockValue, knx_bindings: blockValue.knx_bindings.filter((_, current) => current !== index) } : blockValue); return legacy ? { ...document, blocks } : { blocks } as AutomationDocument; }
export function compatibleEndpoints(document: AutomationDocument, blockIdOrDirection: string, directionOrDpt: 'input' | 'output' | Dpt, dptMaybe?: Dpt): AutomationEndpoint[] { const legacy = dptMaybe === undefined; const blockId = legacy ? 'default' : blockIdOrDirection; const direction = (legacy ? blockIdOrDirection : directionOrDpt) as 'input' | 'output'; const dptValue = (legacy ? directionOrDpt : dptMaybe) as Dpt; const blockValue = blockById(document, blockId); return (direction === 'input' ? blockValue?.inputs : blockValue?.outputs)?.filter((item) => item.dpt === dptValue) ?? []; }
import { parseRevisionToken, type RevisionToken } from './revision';
