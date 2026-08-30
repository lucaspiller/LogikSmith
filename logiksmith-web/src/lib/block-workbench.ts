import type { DisplayBlock, DisplaySimulation } from './dashboard-types';
import { encodeRevisionToken, parseRevisionToken, type RevisionToken } from './revision';

/** Browser-only identity for correlating validation/simulation with editor text. */
export function sourceFingerprint(source: string): string {
  // FNV-1a is deliberately tiny and is not a security boundary. The server
  // still validates the complete source and treats this as a correlation key.
  let hash = 2166136261;
  for (const byte of new TextEncoder().encode(source)) {
    hash ^= byte;
    hash = Math.imul(hash, 16777619);
  }
  return `fnv1a-${(hash >>> 0).toString(16).padStart(8, '0')}`;
}

export interface DraftValidationError {
  category: string;
  message: string;
  line: number | null;
}

export interface DraftValidation {
  status: 'valid' | 'invalid';
  blockId: string;
  blockRevision: RevisionToken;
  structuralRevision: RevisionToken | null;
  sourceFingerprint: string;
  errors: DraftValidationError[];
}

export interface BlockDraft {
  blockId: string;
  source: string;
  baseActiveRevision: RevisionToken | null;
  baseStructuralRevision: RevisionToken | null;
  dirty: boolean;
  conflict: boolean;
  conflictRevision: RevisionToken | null;
  validation: DraftValidation | null;
  simulation: DisplaySimulation | null;
  simulationFingerprint: string | null;
}

const sameRevision = (left: RevisionToken | null, right: RevisionToken | null): boolean =>
  left === null || right === null ? left === right : String(left) === String(right);

export function createBlockDraft(block: DisplayBlock, structuralRevision: RevisionToken | null): BlockDraft {
  return {
    blockId: block.id,
    source: block.source,
    baseActiveRevision: block.activeRevision ?? block.activeLogicRevision,
    baseStructuralRevision: structuralRevision,
    dirty: false,
    conflict: false,
    conflictRevision: null,
    validation: null,
    simulation: null,
    simulationFingerprint: null
  };
}

/** Reconciles a live block without ever overwriting a dirty browser draft. */
export function reconcileBlockDraft(draft: BlockDraft | undefined, block: DisplayBlock, structuralRevision: RevisionToken | null): BlockDraft {
  if (!draft) return createBlockDraft(block, structuralRevision);
  const activeRevision = block.activeRevision ?? block.activeLogicRevision;
  const changed = !sameRevision(activeRevision, draft.baseActiveRevision) || !sameRevision(structuralRevision, draft.baseStructuralRevision);
  if (draft.dirty && changed) {
    return { ...draft, conflict: true, conflictRevision: activeRevision };
  }
  if (draft.dirty) return draft;
  return {
    ...draft,
    source: block.source,
    baseActiveRevision: activeRevision,
    baseStructuralRevision: structuralRevision,
    conflict: false,
    conflictRevision: null,
    validation: null,
    simulation: null,
    simulationFingerprint: null
  };
}

export function updateDraftSource(draft: BlockDraft, source: string): BlockDraft {
  const fingerprint = sourceFingerprint(source);
  return {
    ...draft,
    source,
    dirty: source !== draft.source || draft.dirty,
    validation: draft.validation?.sourceFingerprint === fingerprint ? draft.validation : null,
    simulation: null,
    simulationFingerprint: null
  };
}

export function markDraftClean(draft: BlockDraft, block: DisplayBlock, structuralRevision: RevisionToken | null, activeRevision?: RevisionToken | null): BlockDraft {
  return {
    ...draft,
    source: block.source,
    baseActiveRevision: activeRevision ?? block.activeRevision ?? block.activeLogicRevision,
    baseStructuralRevision: structuralRevision,
    dirty: false,
    conflict: false,
    conflictRevision: null,
    validation: null,
    simulation: null,
    simulationFingerprint: null
  };
}

export function discardDraft(block: DisplayBlock, structuralRevision: RevisionToken | null): BlockDraft {
  return createBlockDraft(block, structuralRevision);
}

export function draftFingerprint(draft: BlockDraft): string { return sourceFingerprint(draft.source); }

export function expectedRevision(draft: BlockDraft, block: DisplayBlock): RevisionToken | null {
  return draft.baseActiveRevision ?? block.activeRevision ?? block.activeLogicRevision;
}

export function expectedStructuralRevision(draft: BlockDraft, current: RevisionToken | null): RevisionToken | null {
  return draft.baseStructuralRevision ?? current;
}

export { encodeRevisionToken, parseRevisionToken };
