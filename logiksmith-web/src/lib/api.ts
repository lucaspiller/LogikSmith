import type {
  ConnectionState,
  DashboardEvent,
  DisplayLog,
  DisplaySnapshot,
  DisplayTelegram,
  DisplayTimer,
  DisplayWrite,
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

const isObject = (value: unknown): value is JsonObject => typeof value === 'object' && value !== null && !Array.isArray(value);

function object(value: unknown, path: string): JsonObject {
  if (!isObject(value)) throw new ApiDecodeError(path, 'expected an object');
  return value;
}

function required(value: unknown, path: string): unknown {
  if (value === undefined || value === null) throw new ApiDecodeError(path, 'required field is missing');
  return value;
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
    address: stringValue(required(field(record, path, 'address'), `${path}.address`), `${path}.address`),
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

function valueFrom(value: unknown, path: string, name: string): boolean | null {
  if (isObject(value)) return nullableBoolean(field(value, path, name, 'value'), `${path}.${name}`);
  return nullableBoolean(value, path);
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
    value: nullableBoolean(field(record, 'write', 'value'), 'write.value'),
    error: optionalString(field(record, 'write', 'error'), 'write.error')
  };
}

function array(value: unknown, path: string): unknown[] {
  if (!Array.isArray(value)) throw new ApiDecodeError(path, 'expected an array');
  return value;
}

/** Maps the internal wire DTO to the UI's small display model. */
export function decodeSnapshot(input: unknown, receivedAtMs = Date.now()): DisplaySnapshot {
  const root = object(input, 'snapshot');
  const config = object(required(field(root, 'snapshot', 'config'), 'config'), 'config');
  const values = object(required(field(root, 'snapshot', 'values'), 'values'), 'values');
  const inputValues = object(required(field(values, 'values', 'input'), 'values.input'), 'values.input');
  const outputValues = object(required(field(values, 'values', 'output'), 'values.output'), 'values.output');
  const telegrams = array(required(field(root, 'snapshot', 'telegrams'), 'telegrams'), 'telegrams').map(telegram);
  const logs = array(required(field(root, 'snapshot', 'logs'), 'logs'), 'logs').map(log);
  const offDelayRaw = required(field(config, 'config', 'off_delay_ms', 'offDelayMs', 'off_delay'), 'config.off_delay_ms');
  const offDelayMs = nonNegativeNumber(offDelayRaw, 'config.off_delay_ms');

  return {
    revision: revision(required(field(root, 'snapshot', 'revision'), 'revision'), 'revision'),
    connection: connection(required(field(root, 'snapshot', 'connection'), 'connection')),
    config: {
      input: endpoint(required(field(config, 'config', 'input'), 'config.input'), 'config.input'),
      output: endpoint(required(field(config, 'config', 'output'), 'config.output'), 'config.output'),
      offDelayMs
    },
    values: {
      input: { observed: valueFrom(inputValues, 'values.input', 'observed') },
      output: {
        observed: valueFrom(outputValues, 'values.output', 'observed'),
        requested: valueFrom(outputValues, 'values.output', 'requested')
      }
    },
    write: write(field(root, 'write', 'write_status', 'last_write')),
    timer: timer(required(field(root, 'snapshot', 'timer'), 'timer'), receivedAtMs),
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
