<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import { fetchScheduleOccurrences, simulateScenario, SimulationApiError } from './api';
  import { simulateBlockDraft, BlockApiError } from './block-api';
  import { applySimulationResult, forceTriggerInput, schedulePreviewDefault, toSimulationScenario, validateSimulationDraft, type SimulationDraft, type SimulationValue } from './simulation';
  import type { DisplayBlock, DisplaySimulation } from './dashboard-types';
  import type { RevisionToken } from './revision';
  import { formatValue, formatDuration, formatStateEntry, triggerSummary } from './format';

  export let block: DisplayBlock;
  export let source: string;
  export let sourceKey: string;
  export let simulationDraft: SimulationDraft;
  export let result: DisplaySimulation | null = null;
  export let stale = false;
  export let structuralRevision: RevisionToken | null = null;

  const dispatch = createEventDispatcher<{ change: SimulationDraft; result: DisplaySimulation; scenario: SimulationDraft }>();
  let running = false;
  let error: string | null = null;
  let fieldErrors: string[] = [];
  let previewLoading = false;
  let previewError: string | null = null;
  let previewKey = '';
  $: draftErrors = validateSimulationDraft(simulationDraft);
  $: revision = block.activeRevision ?? block.activeLogicRevision;
  $: canRun = revision !== null && !stale && !running && draftErrors.length === 0;
  $: triggerInput = simulationDraft.triggerType === 'input' ? simulationDraft.inputs.find((input) => input.endpoint === simulationDraft.triggerEndpoint) ?? null : null;

  function update(next: Partial<SimulationDraft>): void {
    const updated = forceTriggerInput({ ...simulationDraft, ...next });
    simulationDraft = updated;
    error = null; fieldErrors = []; result = null;
    dispatch('change', updated);
  }
  function inputValue(event: Event): string { return (event.currentTarget as HTMLInputElement).value; }
  function selectValue(event: Event): string { return (event.currentTarget as HTMLSelectElement).value; }
  function booleanValue(event: Event): SimulationValue { const value = selectValue(event); return value === '' ? null : value === 'true'; }
  function numericValue(event: Event): SimulationValue { const raw = inputValue(event).trim(); if (!raw) return null; const value = Number(raw); return Number.isFinite(value) ? value : null; }
  function ageValue(event: Event): number | null { const value = numericValue(event); return typeof value === 'number' && Number.isInteger(value) && value >= 0 ? value : null; }
  function setTriggerEndpoint(endpoint: string): void { const input = simulationDraft.inputs.find((item) => item.endpoint === endpoint); update({ triggerType: 'input', triggerEndpoint: endpoint, triggerValue: input?.value ?? null, previousValue: null }); }
  function setTriggerType(triggerType: SimulationDraft['triggerType']): void { update({ triggerType }); if (triggerType === 'schedule') void loadPreviews(); }
  function setTimer(name: string): void { const timer = simulationDraft.pendingTimers.find((item) => item.name === name); update({ triggerType: 'timer', triggerTimerName: name, timerFiredAtMs: timer?.dueAtMs ?? null }); }
  function setSchedule(name: string): void { const schedule = block.schedules.find((item) => item.name === name); previewKey = ''; update({ triggerType: 'schedule', triggerScheduleName: name, scheduleOccurrenceAtMs: schedule?.nextOccurrenceUtcMs ?? null, schedulePreviews: [] }); if (name) void loadPreviews(name); }
  async function loadPreviews(scheduleName = simulationDraft.triggerScheduleName): Promise<void> {
    if (!scheduleName) return;
    const key = `${block.id}:${scheduleName}`; previewKey = key; previewLoading = true; previewError = null;
    try { const preview = await fetchScheduleOccurrences(block.id, scheduleName, { count: 3 }); if (previewKey === key) update({ schedulePreviews: preview.occurrences, scheduleOccurrenceAtMs: schedulePreviewDefault(preview.occurrences, simulationDraft.scheduleOccurrenceAtMs) }); } catch (cause) { if (previewKey === key) previewError = cause instanceof Error ? cause.message : String(cause); } finally { if (previewKey === key) previewLoading = false; }
  }
  async function run(): Promise<void> {
    if (!canRun || revision === null) return;
    const prepared = forceTriggerInput(simulationDraft);
    const scenario = toSimulationScenario(prepared, revision, structuralRevision);
    if (!scenario) return;
    running = true; error = null; fieldErrors = [];
    try {
      let simulated: DisplaySimulation;
      try { simulated = await simulateBlockDraft(block.id, source, scenario); }
      catch (cause) {
        // M11 desktop fixtures expose only /api/simulate. Keep this fallback
        // until the block-scoped endpoint is available, while the source is
        // still sent on the M12 path above.
        if (!(cause instanceof BlockApiError) || cause.status !== 404) throw cause;
        simulated = await simulateScenario(scenario);
      }
      result = simulated; simulationDraft = applySimulationResult(prepared, simulated); dispatch('result', simulated); dispatch('change', simulationDraft);
    } catch (cause) {
      if (cause instanceof SimulationApiError || cause instanceof BlockApiError) { error = cause.message; fieldErrors = cause.fieldErrors.map((item) => `${item.path}: ${item.message}`); } else error = cause instanceof Error ? cause.message : String(cause);
    } finally { running = false; }
  }
</script>

<section class="panel workbench-view simulation-panel" aria-label="Test view">
  <div class="section-heading"><div><p class="eyebrow">Test</p><h2>Draft simulation — {block.id}</h2><p class="subtle">Runs this exact source in a throwaway restricted program. No live outputs, state, timers, signals, or history are changed.</p></div><span class="revision-badge">Base {revision ?? '—'}</span></div>
  {#if stale}<div class="restart-notice" role="status">The live stream is stale. Scenario editing remains available; run is paused until a fresh snapshot.</div>{/if}
  <div class="simulation-kind" role="group" aria-label="Simulation trigger type"><span>Trigger type</span>{#each ['input', 'timer', 'schedule'] as kind}<label><input type="radio" name={`simulation-trigger-${block.id}`} checked={simulationDraft.triggerType === kind} on:change={() => setTriggerType(kind as SimulationDraft['triggerType'])} /> {kind}</label>{/each}</div>
  {#if simulationDraft.triggerType === 'input'}
    <div class="simulation-trigger-fields"><label>Input<select aria-label="Simulation triggering input" value={simulationDraft.triggerEndpoint} on:change={(event) => setTriggerEndpoint(selectValue(event))}><option value="">Choose input</option>{#each simulationDraft.inputs as input}<option value={input.endpoint}>{input.endpoint} ({input.dpt})</option>{/each}</select></label><label>Current value{#if triggerInput?.dpt === '1.001'}<select aria-label="Simulation current trigger value" value={simulationDraft.triggerValue === null ? '' : String(simulationDraft.triggerValue)} on:change={(event) => update({ triggerValue: booleanValue(event) })}><option value="">Choose value</option><option value="false">false</option><option value="true">true</option></select>{:else}<input aria-label="Simulation current trigger value" type="number" min="0" max={triggerInput?.dpt === '9.001' ? '670760' : '100'} step={triggerInput?.dpt === '9.001' ? '0.01' : '1'} value={simulationDraft.triggerValue ?? ''} on:input={(event) => update({ triggerValue: numericValue(event) })} />{/if}</label><label>Previous value{#if triggerInput?.dpt === '1.001'}<select aria-label="Simulation previous trigger value" value={simulationDraft.previousValue === null ? '' : String(simulationDraft.previousValue)} on:change={(event) => update({ previousValue: booleanValue(event) })}><option value="">unknown</option><option value="false">false</option><option value="true">true</option></select>{:else}<input aria-label="Simulation previous trigger value" type="number" value={simulationDraft.previousValue ?? ''} on:input={(event) => update({ previousValue: numericValue(event) })} />{/if}</label></div>
  {:else if simulationDraft.triggerType === 'timer'}
    <div class="simulation-trigger-fields"><label>Timer<select aria-label="Simulation timer" value={simulationDraft.triggerTimerName} on:change={(event) => setTimer(selectValue(event))}><option value="">Choose timer</option>{#each simulationDraft.pendingTimers as timer}<option value={timer.name}>{timer.name} (due {timer.dueAtMs} ms)</option>{/each}</select></label><label>Fired time<input aria-label="Simulation fired time" type="number" min="0" step="1" value={simulationDraft.timerFiredAtMs ?? ''} on:input={(event) => { const value = Number(inputValue(event)); update({ timerFiredAtMs: Number.isInteger(value) && value >= 0 ? value : null }); }} /></label></div>
  {:else}
    <div class="simulation-trigger-fields"><label>Schedule<select aria-label="Simulation schedule" value={simulationDraft.triggerScheduleName} on:change={(event) => setSchedule(selectValue(event))}><option value="">Choose schedule</option>{#each block.schedules as schedule}<option value={schedule.name}>{schedule.name} — {schedule.ruleSummary}</option>{/each}</select></label><label>Occurrence<select aria-label="Simulation occurrence" value={simulationDraft.scheduleOccurrenceAtMs === null ? '' : String(simulationDraft.scheduleOccurrenceAtMs)} on:change={(event) => { const value = Number(selectValue(event)); update({ scheduleOccurrenceAtMs: Number.isInteger(value) && value >= 0 ? value : null }); }}>{#if !simulationDraft.schedulePreviews.length}<option value="">{simulationDraft.scheduleOccurrenceAtMs === null ? 'Choose occurrence' : 'Next occurrence'}</option>{:else}{#each simulationDraft.schedulePreviews as occurrence, index}<option value={occurrence.utcMs}>{occurrence.local ?? String(occurrence.utcMs)}{index === 0 ? ' — next' : ''}</option>{/each}{/if}</select></label>{#if previewLoading}<span class="subtle">Loading occurrence previews…</span>{/if}{#if previewError}<span class="alert" role="alert">{previewError}</span>{/if}</div>
  {/if}
  <h3>Input snapshot</h3><div class="table-wrap"><table class="simulation-table"><thead><tr><th>Input</th><th>DPT</th><th>Value</th><th>Valid</th><th>Age</th></tr></thead><tbody>{#each simulationDraft.inputs as input, index}{@const trigger = simulationDraft.triggerType === 'input' && simulationDraft.triggerEndpoint === input.endpoint}<tr class:simulation-trigger-row={trigger}><td><code>{input.endpoint}</code></td><td>{input.dpt}</td><td>{#if input.dpt === '1.001'}<select aria-label={`Simulation ${input.endpoint} value`} disabled={!input.valid || trigger} value={input.value === null ? '' : String(input.value)} on:change={(event) => update({ inputs: simulationDraft.inputs.map((item, current) => current === index ? { ...item, value: booleanValue(event) } : item) })}><option value="">Choose value</option><option value="false">false</option><option value="true">true</option></select>{:else}<input aria-label={`Simulation ${input.endpoint} value`} disabled={!input.valid || trigger} type="number" value={input.value ?? ''} on:input={(event) => update({ inputs: simulationDraft.inputs.map((item, current) => current === index ? { ...item, value: numericValue(event) } : item) })} />{/if}</td><td><input aria-label={`Simulation ${input.endpoint} validity`} type="checkbox" checked={input.valid} disabled={trigger} on:change={(event) => update({ inputs: simulationDraft.inputs.map((item, current) => current === index ? { ...item, valid: (event.currentTarget as HTMLInputElement).checked } : item) })} /></td><td><input aria-label={`Simulation ${input.endpoint} age`} disabled={!input.valid || trigger} type="number" min="0" value={input.ageMs ?? ''} on:input={(event) => update({ inputs: simulationDraft.inputs.map((item, current) => current === index ? { ...item, ageMs: ageValue(event) } : item) })} /></td></tr>{/each}</tbody></table></div>
  {#if draftErrors.length}<div class="validation-summary" role="alert"><ul>{#each draftErrors as item}<li>{item}</li>{/each}</ul></div>{/if}
  {#if error}<div class="logic-error simulation-error" role="alert">{error}{#if fieldErrors.length}<ul>{#each fieldErrors as item}<li>{item}</li>{/each}</ul>{/if}</div>{/if}
  <div class="editor-actions"><button type="button" class="save-button" disabled={!canRun} on:click={run}>{running ? 'Running…' : 'Run simulation'}</button><span class="subtle">A successful simulation is advisory; it never enters live history.</span></div>
  {#if result}<article class:simulation-failed={result.status === 'failed'} class="simulation-result" aria-label="Simulation result"><div class="section-heading"><h3>Simulation {result.status}</h3><span class="revision-badge">Source {sourceKey}</span></div>{#if result.error}<div class="logic-error" role="alert"><strong>{result.error.category}</strong>{#if result.error.line !== null} line {result.error.line}:{/if} {result.error.message}</div>{:else}<p>{triggerSummary(result.trigger)} · {formatDuration(result.durationUs)}</p><section aria-label="Simulation capture"><div class="result-grid"><div><h4>State after</h4>{#if Object.keys(result.stateAfter).length}<ul>{#each Object.entries(result.stateAfter) as [key, value]}<li><code>{key}</code> = {formatStateEntry(value)}</li>{/each}</ul>{:else}<p class="empty">No state keys.</p>{/if}</div><div><h4>Proposed outputs</h4>{#if result.effects.length}<ul>{#each result.effects as effect}<li><code>{effect.endpoint}</code> = {formatValue(effect.value)} → <code>{effect.destination}</code></li>{/each}</ul>{:else}<p class="empty">No KNX outputs.</p>{/if}</div><div><h4>Proposed signal effects</h4>{#if result.signalEffects.length}<ul>{#each result.signalEffects as effect}<li><code>{effect.signal}</code> = {formatValue(effect.value)}</li>{/each}</ul>{:else}<p class="empty">No signal outputs.</p>{/if}{#if result.eligibleConsumers.length}<p><strong>Eligible consumers (not executed)</strong>: {result.eligibleConsumers.map((item) => `${item.blockId}.${item.endpoint}`).join(', ')}</p>{/if}<p class="subtle">This draft does not propagate or execute them.</p></div><div><h4>Proposed timers</h4>{#if result.timerEffects.length}<ul>{#each result.timerEffects as effect}<li><code>{effect.name}</code>: {effect.action}</li>{/each}</ul>{:else}<p class="empty">No timer operations.</p>{/if}</div></div></section>{/if}<p class="subtle no-write-note">No KNX write sent. Simulation result is labelled with its source fingerprint and base revision.</p></article>{/if}
</section>
