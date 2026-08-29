import type { DashboardState, DisplayDateTimeValue, DisplayExecutionTrigger, DisplaySnapshot, DisplayStateValue } from './dashboard-types';
import type { RevisionToken } from './revision';

export function formatCountdown(milliseconds: number | null): string { if (milliseconds === null || !Number.isFinite(milliseconds)) return '—'; const value = Math.max(0, milliseconds); if (value >= 60_000) return `${Math.floor(value / 60_000)}:${Math.floor((value % 60_000) / 1_000).toString().padStart(2, '0')}`; return `${(value / 1_000).toFixed(1)} s`; }
export function formatValue(value: boolean | number | string | null): string { return value === null ? 'unknown' : String(value); }
export function formatStateValue(value: DisplayStateValue | null | undefined): string { if (!value) return 'unknown'; return value.kind === 'string' ? JSON.stringify(value.value) : String(value.value); }
export function formatStateEntry(value: DisplayStateValue | null | undefined): string { return value ? `${formatStateValue(value)} (${value.kind})` : 'unknown'; }
function trimNumber(value: number, digits: number): string { return value.toFixed(digits).replace(/(\.\d*?)0+$/, '$1').replace(/\.$/, ''); }
export function formatDuration(microseconds: number): string { if (!Number.isFinite(microseconds) || microseconds < 0) return '—'; if (microseconds < 1_000) return `${microseconds} μs`; if (microseconds < 1_000_000) return `${trimNumber(microseconds / 1_000, 2)} ms`; return `${trimNumber(microseconds / 1_000_000, 2)} s`; }
export function formatAge(milliseconds: number | null): string { if (milliseconds === null || !Number.isFinite(milliseconds) || milliseconds < 0) return '—'; if (milliseconds < 1_000) return `${milliseconds} ms`; return `${trimNumber(milliseconds / 1_000, 1)} s`; }
/** Formats a UTC offset in seconds as "UTC+03:00" (or "UTC" for zero). */
export function formatUtcOffset(offsetSeconds: number | null): string {
  if (offsetSeconds === null || !Number.isFinite(offsetSeconds)) return '—';
  if (offsetSeconds === 0) return 'UTC';
  const sign = offsetSeconds < 0 ? '-' : '+';
  const total = Math.abs(offsetSeconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  return `UTC${sign}${String(hours).padStart(2, '0')}:${String(minutes).padStart(2, '0')}`;
}
export function hasPendingRestart(activeRevision: RevisionToken | null, savedRevision: RevisionToken | null, restartRequired = false): boolean { return restartRequired || (activeRevision !== null && savedRevision !== null && String(activeRevision) !== String(savedRevision)); }

/** Labels an execution trigger: "input:wall_switch", "timer:off", or "schedule:morning_on". */
export function triggerLabel(trigger: DisplayExecutionTrigger): string {
  if (trigger.type === 'schedule') return `schedule:${trigger.name}`;
  if (trigger.type === 'timer') return `timer:${trigger.timer}`;
  return `input:${trigger.endpoint}`;
}
/** Human-readable trigger summary; schedule triggers include lateness and queue delay. */
export function triggerSummary(trigger: DisplayExecutionTrigger): string {
  if (trigger.type === 'schedule') return `${trigger.name} (intended ${formatUtcDateTime(trigger.scheduledForUtcMs)}, ${formatAge(trigger.lateByMs)} late, ${formatAge(trigger.queueDelayMs)} queue, ${trigger.coalescedCount} coalesced)`;
  if (trigger.type === 'timer') return `${trigger.timer} (fired ${trigger.firedAtMs} ms, ${formatAge(trigger.lateByMs)} late)`;
  return `${formatValue(trigger.previous)} → ${formatValue(trigger.value)}${trigger.changed ? ' (changed)' : ''}`;
}
function padTwo(value: number | null): string { return value === null ? '??' : String(value).padStart(2, '0'); }
/** Renders a captured calendar value as "2026-08-29 05:30:00 (Saturday)". */
export function formatDateTimeValue(value: DisplayDateTimeValue | null): string {
  if (!value || !value.available) return '—';
  const fields = `${String(value.year)}-${padTwo(value.month)}-${padTwo(value.day)} ${padTwo(value.hour)}:${padTwo(value.minute)}:${padTwo(value.second)}`;
  return value.weekday ? `${fields} (${value.weekday})` : fields;
}
/** Renders a UTC epoch instant as "2026-08-29 05:30:00 UTC". */
export function formatUtcDateTime(utcMs: number): string {
  const date = new Date(utcMs);
  if (Number.isNaN(date.valueOf())) return '—';
  return `${date.getUTCFullYear()}-${padTwo(date.getUTCMonth() + 1)}-${padTwo(date.getUTCDate())} ${padTwo(date.getUTCHours())}:${padTwo(date.getUTCMinutes())}:${padTwo(date.getUTCSeconds())} UTC`;
}
