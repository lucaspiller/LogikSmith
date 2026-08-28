import type {
  DisplayEndpoint,
  DisplayExecution,
  DisplaySnapshot,
  SimulationInputRequest,
  SimulationScenario,
  SimulationTypedValue
} from './state';

export type SimulationValue = boolean | number | null;

export interface SimulationDraftInput {
  endpoint: string;
  dpt: string;
  value: SimulationValue;
  valid: boolean;
  ageMs: number | null;
}

export interface SimulationDraft {
  triggerEndpoint: string;
  triggerValue: SimulationValue;
  previousValue: SimulationValue;
  inputs: SimulationDraftInput[];
}

function inputFromEndpoint(endpoint: DisplayEndpoint): SimulationDraftInput | null {
  if (!endpoint.name) return null;
  return { endpoint: endpoint.name, dpt: endpoint.dpt, value: null, valid: false, ageMs: null };
}

/** Starts an editable scenario from an execution, or with all active inputs unknown. */
export function createSimulationDraft(snapshot: DisplaySnapshot, execution: DisplayExecution | null = null): SimulationDraft {
  if (execution) {
    return {
      triggerEndpoint: execution.trigger.endpoint,
      triggerValue: execution.trigger.value,
      previousValue: execution.trigger.previous,
      inputs: execution.inputs.map((input) => ({ ...input }))
    };
  }
  const inputs = (snapshot.automation?.inputs ?? []).map(inputFromEndpoint).filter((input): input is SimulationDraftInput => input !== null);
  return {
    triggerEndpoint: inputs[0]?.endpoint ?? '',
    triggerValue: null,
    previousValue: null,
    inputs
  };
}

/** Keeps the trigger's corresponding snapshot entry equivalent to the event. */
export function forceTriggerInput(draft: SimulationDraft): SimulationDraft {
  if (draft.triggerValue === null) return { ...draft, inputs: draft.inputs.map((input) => ({ ...input })) };
  return {
    ...draft,
    inputs: draft.inputs.map((input) => input.endpoint === draft.triggerEndpoint
      ? { ...input, value: draft.triggerValue, valid: true, ageMs: 0 }
      : { ...input })
  };
}

export function typedValueForDpt(dpt: string, value: SimulationValue): SimulationTypedValue | null {
  if (value === null) return null;
  if (dpt === '1.001' && typeof value === 'boolean') return { kind: 'bool', value };
  if (dpt === '5.001' && typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 100) {
    return { kind: 'percent', value };
  }
  return null;
}

export function validateSimulationDraft(draft: SimulationDraft): string[] {
  const errors: string[] = [];
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
  const trigger = prepared.inputs.find((input) => input.endpoint === prepared.triggerEndpoint);
  const triggerValue = trigger ? typedValueForDpt(trigger.dpt, prepared.triggerValue) : null;
  if (errors.length || !trigger || !triggerValue) return null;
  const inputs: SimulationInputRequest[] = prepared.inputs.map((input) => ({
    endpoint: input.endpoint,
    value: input.valid ? typedValueForDpt(input.dpt, input.value) : null,
    valid: input.valid,
    ageMs: input.valid ? input.ageMs : null
  }));
  return {
    expectedLogicRevision,
    trigger: {
      endpoint: prepared.triggerEndpoint,
      value: triggerValue,
      previous: typedValueForDpt(trigger.dpt, prepared.previousValue)
    },
    inputs
  };
}
