import type { DisplaySimulation, SimulationScenario } from './dashboard-types';
import { decodeSimulation } from './api';
import { encodeRevisionToken, parseRevisionToken, type RevisionToken } from './revision';
import { sourceFingerprint, type DraftValidation, type DraftValidationError } from './block-workbench';

type FetchLike = typeof fetch;
type JsonObject = Record<string, unknown>;

export interface BlockApiFieldError { path: string; message: string; }

export class BlockApiError extends Error {
  readonly status: number;
  readonly fieldErrors: BlockApiFieldError[];
  readonly currentRevision: RevisionToken | null;
  readonly currentStructuralRevision: RevisionToken | null;

  constructor(status: number, message: string, fieldErrors: BlockApiFieldError[] = [], currentRevision: RevisionToken | null = null, currentStructuralRevision: RevisionToken | null = null) {
    super(message);
    this.name = 'BlockApiError';
    this.status = status;
    this.fieldErrors = fieldErrors;
    this.currentRevision = currentRevision;
    this.currentStructuralRevision = currentStructuralRevision;
  }
}

export interface BlockMutationResult {
  blockId: string;
  activeRevision: RevisionToken;
  savedRevision: RevisionToken;
  activeEnabled: boolean;
  savedEnabled: boolean;
  structuralRevision: RevisionToken | null;
  cancelledTimers: string[];
  sourceFingerprint: string | null;
  message: string | null;
}

const isObject = (value: unknown): value is JsonObject => typeof value === 'object' && value !== null && !Array.isArray(value);
const field = (source: JsonObject, ...names: string[]): unknown => names.find((name) => name in source) === undefined ? undefined : source[names.find((name) => name in source)!];
const text = (value: unknown, path: string): string => { if (typeof value !== 'string') throw new Error(`${path} must be a string`); return value; };
const optionalText = (value: unknown): string | null => value === undefined || value === null ? null : typeof value === 'string' ? value : null;
const token = (value: unknown, path: string, required = true): RevisionToken | null => {
  if (value === undefined || value === null) {
    if (required) throw new Error(`${path} must be a decimal string`);
    return null;
  }
  // New M12 endpoint contracts intentionally reject numeric revisions. The
  // legacy dashboard decoder remains permissive for M7-M11 fixtures.
  if (typeof value !== 'string' || !/^(0|[1-9]\d*)$/.test(value)) throw new Error(`${path} must be a decimal string`);
  return parseRevisionToken(value);
};
const numberOrNull = (value: unknown): number | null => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null;

async function jsonOrNull(response: Response): Promise<unknown> { try { return await response.json(); } catch { return null; } }
function errors(value: unknown): BlockApiFieldError[] {
  if (!isObject(value)) return [];
  const raw = field(value, 'errors', 'field_errors', 'fields');
  if (Array.isArray(raw)) return raw.flatMap((item) => isObject(item) && (typeof item.path === 'string' || typeof item.field === 'string') && typeof item.message === 'string' ? [{ path: String(item.path ?? item.field), message: item.message }] : []);
  if (isObject(raw)) return Object.entries(raw).flatMap(([path, message]) => typeof message === 'string' ? [{ path, message }] : []);
  return [];
}
function responseError(status: number, body: unknown, operation: string): BlockApiError {
  const current = isObject(body) ? field(body, 'current_revision', 'currentRevision', 'active_revision', 'activeRevision') : undefined;
  const structural = isObject(body) ? field(body, 'current_structural_revision', 'currentStructuralRevision', 'active_structural_revision', 'activeStructuralRevision') : undefined;
  let currentRevision: RevisionToken | null = null;
  let currentStructuralRevision: RevisionToken | null = null;
  try { currentRevision = current === undefined || current === null ? null : token(current, 'current_revision', false); } catch { /* malformed conflict details still surface as a normal API failure */ }
  try { currentStructuralRevision = structural === undefined || structural === null ? null : token(structural, 'current_structural_revision', false); } catch { /* see above */ }
  const message = isObject(body) && typeof body.error === 'string' ? body.error : status === 409 ? 'The block changed. Refresh the live snapshot before retrying.' : `${operation} request failed (${status})`;
  return new BlockApiError(status, message, errors(body), currentRevision, currentStructuralRevision);
}

function validationError(value: unknown, path: string): DraftValidationError {
  if (!isObject(value)) return { category: 'validation', message: String(value), line: null };
  const category = typeof value.category === 'string' ? value.category : typeof value.kind === 'string' ? value.kind : 'validation';
  const message = typeof value.message === 'string' ? value.message : 'Source is invalid.';
  const lineRaw = value.line ?? value.line_number ?? value.lineNumber;
  return { category, message, line: typeof lineRaw === 'number' && Number.isSafeInteger(lineRaw) && lineRaw > 0 ? lineRaw : null };
}

function decodeValidation(value: unknown, blockId: string, source: string, fallbackRevision: RevisionToken, fallbackStructural: RevisionToken | null): DraftValidation {
  const root = isObject(value) ? value : {};
  const statusRaw = field(root, 'status', 'result');
  const status = statusRaw === 'valid' ? 'valid' : statusRaw === 'invalid' || statusRaw === 'error' ? 'invalid' : null;
  if (!status) throw new Error('validation.status must be valid or invalid');
  const revisionRaw = field(root, 'block_revision', 'blockRevision', 'active_revision', 'activeRevision');
  const blockRevision = token(revisionRaw ?? fallbackRevision, 'validation.block_revision');
  const structuralRaw = field(root, 'structural_revision', 'structuralRevision', 'active_structural_revision', 'activeStructuralRevision');
  const structuralRevision = structuralRaw === undefined || structuralRaw === null ? fallbackStructural : token(structuralRaw, 'validation.structural_revision', false);
  const fingerprint = text(field(root, 'source_fingerprint', 'sourceFingerprint') ?? sourceFingerprint(source), 'validation.source_fingerprint');
  const rawErrors = field(root, 'errors', 'diagnostics') ?? [];
  if (!Array.isArray(rawErrors)) throw new Error('validation.errors must be an array');
  return { status, blockId, blockRevision: blockRevision!, structuralRevision, sourceFingerprint: fingerprint, errors: rawErrors.map((item) => validationError(item, 'validation.errors')) };
}

export interface ValidateBlockOptions {
  blockId: string;
  source: string;
  expectedRevision: RevisionToken;
  expectedStructuralRevision: RevisionToken | null;
  fetchImpl?: FetchLike;
}

export async function validateBlockSource(options: ValidateBlockOptions): Promise<DraftValidation> {
  const fetchImpl = options.fetchImpl ?? fetch;
  const fingerprint = sourceFingerprint(options.source);
  const body: JsonObject = {
    source: options.source,
    source_fingerprint: fingerprint,
    expected_revision: encodeRevisionToken(options.expectedRevision),
    ...(options.expectedStructuralRevision === null ? {} : { expected_structural_revision: encodeRevisionToken(options.expectedStructuralRevision) })
  };
  const response = await fetchImpl(`/api/blocks/${encodeURIComponent(options.blockId)}/validate`, { method: 'POST', headers: { accept: 'application/json', 'content-type': 'application/json' }, body: JSON.stringify(body) });
  const payload = await jsonOrNull(response);
  if (!response.ok) throw responseError(response.status, payload, 'Validation');
  try { return decodeValidation(payload, options.blockId, options.source, options.expectedRevision, options.expectedStructuralRevision); } catch (error) { throw new BlockApiError(200, error instanceof Error ? error.message : String(error)); }
}

function decodeMutation(value: unknown, blockId: string, fallbackRevision: RevisionToken): BlockMutationResult {
  const root = isObject(value) ? value : {};
  const active = token(field(root, 'active_revision', 'activeRevision', 'block_revision', 'blockRevision') ?? fallbackRevision, 'mutation.active_revision');
  const saved = token(field(root, 'saved_revision', 'savedRevision') ?? active, 'mutation.saved_revision');
  const structuralRaw = field(root, 'structural_revision', 'structuralRevision', 'active_structural_revision', 'activeStructuralRevision');
  const cancelledRaw = field(root, 'cancelled_timers', 'cancelledTimers') ?? [];
  if (!Array.isArray(cancelledRaw) || cancelledRaw.some((item) => typeof item !== 'string')) throw new Error('mutation.cancelled_timers must be an array of strings');
  return {
    blockId: typeof field(root, 'block_id', 'blockId', 'id') === 'string' ? String(field(root, 'block_id', 'blockId', 'id')) : blockId,
    activeRevision: active!, savedRevision: saved!,
    activeEnabled: typeof field(root, 'active_enabled', 'activeEnabled', 'enabled') === 'boolean' ? Boolean(field(root, 'active_enabled', 'activeEnabled', 'enabled')) : true,
    savedEnabled: typeof field(root, 'saved_enabled', 'savedEnabled') === 'boolean' ? Boolean(field(root, 'saved_enabled', 'savedEnabled')) : true,
    structuralRevision: structuralRaw === undefined || structuralRaw === null ? null : token(structuralRaw, 'mutation.structural_revision', false),
    cancelledTimers: cancelledRaw as string[],
    sourceFingerprint: optionalText(field(root, 'source_fingerprint', 'sourceFingerprint')),
    message: optionalText(field(root, 'message', 'notice'))
  };
}

export interface ActivateBlockOptions {
  blockId: string;
  source: string;
  expectedRevision: RevisionToken;
  expectedStructuralRevision: RevisionToken | null;
  fetchImpl?: FetchLike;
}

export async function activateBlockSource(options: ActivateBlockOptions): Promise<BlockMutationResult> {
  const fetchImpl = options.fetchImpl ?? fetch;
  const body: JsonObject = { source: options.source, source_fingerprint: sourceFingerprint(options.source), expected_revision: encodeRevisionToken(options.expectedRevision), ...(options.expectedStructuralRevision === null ? {} : { expected_structural_revision: encodeRevisionToken(options.expectedStructuralRevision) }) };
  const response = await fetchImpl(`/api/blocks/${encodeURIComponent(options.blockId)}/source`, { method: 'PUT', headers: { accept: 'application/json', 'content-type': 'application/json' }, body: JSON.stringify(body) });
  const payload = await jsonOrNull(response);
  if (!response.ok) throw responseError(response.status, payload, 'Activation');
  try { return decodeMutation(payload, options.blockId, options.expectedRevision); } catch (error) { throw new BlockApiError(200, error instanceof Error ? error.message : String(error)); }
}

export interface SetBlockEnabledOptions {
  blockId: string;
  enabled: boolean;
  expectedRevision: RevisionToken;
  expectedStructuralRevision: RevisionToken | null;
  fetchImpl?: FetchLike;
}

export async function setBlockEnabled(options: SetBlockEnabledOptions): Promise<BlockMutationResult> {
  const fetchImpl = options.fetchImpl ?? fetch;
  const body: JsonObject = { enabled: options.enabled, expected_revision: encodeRevisionToken(options.expectedRevision), ...(options.expectedStructuralRevision === null ? {} : { expected_structural_revision: encodeRevisionToken(options.expectedStructuralRevision) }) };
  const response = await fetchImpl(`/api/blocks/${encodeURIComponent(options.blockId)}/enabled`, { method: 'PUT', headers: { accept: 'application/json', 'content-type': 'application/json' }, body: JSON.stringify(body) });
  const payload = await jsonOrNull(response);
  if (!response.ok) throw responseError(response.status, payload, options.enabled ? 'Enable' : 'Disable');
  try { return decodeMutation(payload, options.blockId, options.expectedRevision); } catch (error) { throw new BlockApiError(200, error instanceof Error ? error.message : String(error)); }
}

/**
 * Simulate a draft against the block-scoped endpoint. `SimulationScenario` is
 * accepted so the existing scenario editor and generic decoder remain shared.
 */
export async function simulateBlockDraft(blockId: string, source: string, scenario: SimulationScenario, fetchImpl: FetchLike = fetch): Promise<DisplaySimulation> {
  const trigger = scenario.trigger.type === 'timer'
    ? { type: 'timer', name: scenario.trigger.name ?? scenario.trigger.timer, fired_at_ms: scenario.trigger.firedAtMs }
    : scenario.trigger.type === 'schedule'
      ? { type: 'schedule', schedule: scenario.trigger.schedule, occurrence_at_ms: scenario.trigger.occurrenceAtMs }
      : { type: 'input', endpoint: scenario.trigger.endpoint, value: scenario.trigger.value, previous: scenario.trigger.previous };
  const body: JsonObject = {
    block_id: blockId,
    source,
    source_fingerprint: sourceFingerprint(source),
    expected_revision: encodeRevisionToken(scenario.expectedLogicRevision),
    ...(scenario.expectedStructuralRevision === undefined || scenario.expectedStructuralRevision === null ? {} : { expected_structural_revision: encodeRevisionToken(scenario.expectedStructuralRevision) }),
    trigger,
    inputs: scenario.inputs.map((input) => ({ endpoint: input.endpoint, value: input.value, valid: input.valid, age_ms: input.ageMs })),
    ...(scenario.state === undefined ? {} : { state: scenario.state }),
    ...(scenario.pendingTimers === undefined ? {} : { pending_timers: scenario.pendingTimers.map((timer) => ({ name: timer.name, scheduled_at_ms: timer.scheduledAtMs, due_at_ms: timer.dueAtMs, logic_revision: encodeRevisionToken(timer.logicRevision) })) })
  };
  const response = await fetchImpl(`/api/blocks/${encodeURIComponent(blockId)}/simulate`, { method: 'POST', headers: { accept: 'application/json', 'content-type': 'application/json' }, body: JSON.stringify(body) });
  const payload = await jsonOrNull(response);
  if (!response.ok) throw responseError(response.status, payload, 'Draft simulation');
  try {
    const result = decodeSimulation(payload);
    // The endpoint's returned logic revision is required to be a string by
    // the M12 wire contract; old generic simulation fixtures stay permissive.
    if (isObject(payload) && (field(payload, 'logic_revision', 'logicRevision') !== undefined) && typeof field(payload, 'logic_revision', 'logicRevision') !== 'string') throw new Error('simulation.logic_revision must be a decimal string');
    return result;
  } catch (error) { throw new BlockApiError(200, error instanceof Error ? error.message : String(error)); }
}

