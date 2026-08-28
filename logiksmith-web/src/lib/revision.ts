/**
 * Revisions are opaque decimal tokens at the JSON boundary. The number branch
 * is only for safe-integer Milestone 7 fixtures; current desktop responses use
 * strings, and outbound values are always encoded as strings.
 */
export type RevisionToken = string | number;

const revisionPattern = /^(0|[1-9]\d*)$/;

export function parseRevisionToken(value: unknown): RevisionToken | null {
  if (typeof value === 'string' && revisionPattern.test(value)) return value;
  if (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0) return value;
  return null;
}

export function isRevisionToken(value: unknown): value is RevisionToken {
  return parseRevisionToken(value) !== null;
}

export function encodeRevisionToken(value: RevisionToken): string {
  const token = parseRevisionToken(value);
  if (token === null) throw new TypeError('Revision must be a non-negative decimal token.');
  return String(token);
}
