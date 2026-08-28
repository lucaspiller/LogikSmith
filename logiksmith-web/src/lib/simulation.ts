import type {
  DisplayEndpoint,
  DisplayExecution,
  DisplayPendingTimer,
  DisplaySimulation,
  DisplaySnapshot,
  DisplayState,
  DisplayStateValue,
  SimulationInputRequest,
  SimulationScenario,
  SimulationTypedValue
} from './state';

export type SimulationValue = boolean | number | null;
export interface SimulationDraftInput { endpoint: string; dpt: string; value: SimulationValue; valid: boolean; ageMs: number | null; }
export interface SimulationDraft {
  triggerType: 'input' | 'timer';
  triggerEndpoint: string;
  triggerValue: SimulationValue;
  previousValue: SimulationValue;
  triggerTimerName: string;
  timerFiredAtMs: number | null;
  inputs: SimulationDraftInput[];
  state: DisplayState;
  pendingTimers: DisplayPendingTimer[];
}

function inputFromEndpoint(endpoint: DisplayEndpoint): SimulationDraftInput | null {
  if (!endpoint.name) return null;
  return { endpoint: endpoint.name, dpt: endpoint.dpt, value: null, valid: false, ageMs: null };
}

/** Starts an editable scenario from an execution, or with all active inputs unknown. */
export function createSimulationDraft(snapshot: DisplaySnapshot, execution: DisplayExecution | null = null): SimulationDraft {
  const inputs = execution
    ? execution.inputs.map((input) => ({ ...input }))
    : (snapshot.automation?.inputs ?? []).map(inputFromEndpoint).filter((input): input is SimulationDraftInput => input !== null);
  const timerTrigger = execution?.trigger.type === 'timer' ? execution.trigger : null;
  const pendingTimers = snapshot.pendingTimers.map((timer) => ({ ...timer }));
  if (timerTrigger && !pendingTimers.some((timer) => timer.name === timerTrigger.timer)) pendingTimers.push({ name: timerTrigger.timer, scheduledAtMs: timerTrigger.scheduledAtMs, dueAtMs: timerTrigger.dueAtMs, logicRevision: timerTrigger.scheduledLogicRevision });
  return {
    triggerType: timerTrigger ? 'timer' : 'input',
    triggerEndpoint: execution && execution.trigger.type === 'input' ? execution.trigger.endpoint : inputs[0]?.endpoint ?? '',
    triggerValue: execution && execution.trigger.type === 'input' ? execution.trigger.value : null,
    previousValue: execution && execution.trigger.type === 'input' ? execution.trigger.previous : null,
    triggerTimerName: timerTrigger?.timer ?? pendingTimers[0]?.name ?? '',
    timerFiredAtMs: timerTrigger?.firedAtMs ?? pendingTimers[0]?.dueAtMs ?? null,
    inputs,
    state: { ...(execution?.stateBefore ?? snapshot.state) },
    pendingTimers
  };
}

/** Keeps the input trigger's corresponding snapshot entry equivalent to the event. */
export function forceTriggerInput(draft: SimulationDraft): SimulationDraft {
  if (draft.triggerType === 'timer' || draft.triggerValue === null) return { ...draft, inputs: draft.inputs.map((input) => ({ ...input })) };
  return {
    ...draft,
    inputs: draft.inputs.map((input) => input.endpoint === draft.triggerEndpoint
      ? { ...input, value: draft.triggerValue, valid: true, ageMs: 0 }
      : { ...input })
  };
}

/** Advances the editable, isolated scenario after a successful simulation. */
export function applySimulationResult(draft: SimulationDraft, result: DisplaySimulation): SimulationDraft {
  if (result.status !== 'succeeded') return draft;
  const pendingTimers = result.pendingTimers.map((timer) => ({ ...timer }));
  const selectedTimer = pendingTimers.find((timer) => timer.name === draft.triggerTimerName) ?? pendingTimers[0];
  return {
    ...draft,
    inputs: result.inputs.map((input) => ({ ...input })),
    state: { ...result.stateAfter },
    pendingTimers,
    triggerTimerName: selectedTimer?.name ?? '',
    timerFiredAtMs: selectedTimer?.dueAtMs ?? null
  };
}

export function typedValueForDpt(dpt: string, value: SimulationValue): SimulationTypedValue | null {
  if (value === null) return null;
  if (dpt === '1.001' && typeof value === 'boolean') return { kind: 'bool', value };
  if (dpt === '5.001' && typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 100) return { kind: 'percent', value };
  return null;
}

function validStateValue(value: unknown): value is DisplayStateValue {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const candidate = value as { kind?: unknown; value?: unknown };
  if (candidate.kind === 'bool') return typeof candidate.value === 'boolean';
  if (candidate.kind === 'integer') return typeof candidate.value === 'number' && Number.isSafeInteger(candidate.value);
  if (candidate.kind === 'number') return typeof candidate.value === 'number' && Number.isFinite(candidate.value);
  return candidate.kind === 'string' && typeof candidate.value === 'string';
}

export function validateSimulationDraft(draft: SimulationDraft): string[] {
  const errors: string[] = [];
  for (const [key, value] of Object.entries(draft.state)) {
    if (!key) errors.push('State keys cannot be empty.');
    if (!validStateValue(value)) errors.push(`State ${key || 'entry'} needs a valid scalar value.`);
  }
  draft.pendingTimers.forEach((timer, index) => {
    if (!timer.name) errors.push(`Pending timer ${index + 1} needs a name.`);
    if (!Number.isInteger(timer.scheduledAtMs) || timer.scheduledAtMs < 0) errors.push(`Pending timer ${timer.name || index + 1} has an invalid scheduled time.`);
    if (!Number.isInteger(timer.dueAtMs) || timer.dueAtMs < 0) errors.push(`Pending timer ${timer.name || index + 1} has an invalid due time.`);
    if (!Number.isInteger(timer.logicRevision) || timer.logicRevision < 0) errors.push(`Pending timer ${timer.name || index + 1} has an invalid logic revision.`);
  });
  if (draft.triggerType === 'timer') {
    if (!draft.triggerTimerName) errors.push('Choose a pending timer.');
    if (!draft.pendingTimers.some((timer) => timer.name === draft.triggerTimerName)) errors.push('The timer trigger must be selected from pending simulated timers.');
    if (draft.timerFiredAtMs === null || !Number.isInteger(draft.timerFiredAtMs) || draft.timerFiredAtMs < 0) errors.push('Supply a non-negative integer fired time for the timer.');
    return errors;
  }
  if (!draft.triggerEndpoint) errors.push('Choose a triggering input.');
  const trigger = draft.inputs.find((input) => input.endpoint === draft.triggerEndpoint);
  if (!trigger) errors.push('The triggering input must be configured.');
  if (trigger && typedValueForDpt(trigger.dpt, draft.triggerValue) === null) errors.push('Choose a valid current trigger value.');
  if (draft.previousValue !== null && trigger && typedValueForDpt(trigger.dpt, draft.previousValue) === null) errors.push('Previous value does not match the trigger DPT.');
  draft.inputs.forEach((input, index) => {
    const prefix = `Input ${input.endpoint || index + 1}`;
    if (input.endpoint === draft.triggerEndpoint && draft.triggerValue !== null) return;
    if (!input.valid) {
      if (input.value !== null) errors.push(`${prefix} is invalid but has a value.`);
      if (input.ageMs !== null) errors.push(`${prefix} is invalid but has an age.`);
      return;
    }
    if (typedValueForDpt(input.dpt, input.value) === null) errors.push(`${prefix} needs a valid typed value.`);
    if (input.ageMs === null || !Number.isInteger(input.ageMs) || input.ageMs < 0) errors.push(`${prefix} needs a non-negative integer age.`);
  });
  return errors;
}

export function toSimulationScenario(draft: SimulationDraft, expectedLogicRevision: number): SimulationScenario | null {
  const prepared = forceTriggerInput(draft);
  const errors = validateSimulationDraft(prepared);
  if (errors.length) return null;
  const inputs: SimulationInputRequest[] = prepared.inputs.map((input) => ({ endpoint: input.endpoint, value: input.valid ? typedValueForDpt(input.dpt, input.value) : null, valid: input.valid, ageMs: input.valid ? input.ageMs : null }));
  if (prepared.triggerType === 'timer') {
    if (prepared.timerFiredAtMs === null) return null;
    return { expectedLogicRevision, trigger: { type: 'timer', name: prepared.triggerTimerName, firedAtMs: prepared.timerFiredAtMs }, inputs, state: { ...prepared.state }, pendingTimers: prepared.pendingTimers.map((timer) => ({ ...timer })) };
  }
  const trigger = prepared.inputs.find((input) => input.endpoint === prepared.triggerEndpoint);
  const triggerValue = trigger ? typedValueForDpt(trigger.dpt, prepared.triggerValue) : null;
  if (!trigger || !triggerValue) return null;
  return {
    expectedLogicRevision,
    trigger: { type: 'input', endpoint: prepared.triggerEndpoint, value: triggerValue, previous: typedValueForDpt(trigger.dpt, prepared.previousValue) },
    inputs,
    state: { ...prepared.state },
    pendingTimers: prepared.pendingTimers.map((timer) => ({ ...timer }))
  };
}
