<script lang="ts">
  import { onMount } from 'svelte';
  import {
    AutomationApiError,
    emptyAutomation,
    loadAutomation,
    removeBinding,
    removeEndpoint,
    renameEndpoint,
    saveAutomation,
    validateAutomation,
    type AutomationDocument,
    type AutomationEnvelope,
    type AutomationFieldError,
    type Dpt
  } from './lib/automation';
  import { DashboardClient, simulateScenario, SimulationApiError } from './lib/api';
  import {
    formatAge,
    formatValue,
    formatDuration,
    hasPendingRestart,
    initialDashboardState,
    reduceDashboardState,
    type DashboardState,
    type DisplayEndpoint,
    type DisplayExecutionTrigger,
    type DisplaySimulation
  } from './lib/state';
  import {
    createSimulationDraft,
    forceTriggerInput,
    toSimulationScenario,
    validateSimulationDraft,
    type SimulationDraft,
    type SimulationValue
  } from './lib/simulation';

  let state: DashboardState = initialDashboardState;
  let automation: AutomationEnvelope | null = null;
  let draft: AutomationDocument = emptyAutomation();
  let automationLoading = true;
  let automationSaving = false;
  let automationError: string | null = null;
  let serverErrors: AutomationFieldError[] = [];
  let conflictLatest: AutomationEnvelope | null = null;
  let restartRequired = false;
  let saveNotice: string | null = null;
  let simulationDraft: SimulationDraft | null = null;
  let simulationResult: DisplaySimulation | null = null;
  let simulationRunning = false;
  let simulationError: string | null = null;
  let simulationFieldErrors: string[] = [];

  $: snapshot = state.snapshot;
  $: connectionState = snapshot?.connection.state ?? 'starting';
  $: clientErrors = validateAutomation(draft);
  $: activeStructural = snapshot?.activeStructuralRevision ?? null;
  $: savedStructural = snapshot?.savedStructuralRevision ?? null;
  $: legacyRevisionPending = snapshot?.activeStructuralRevision === null && snapshot?.savedStructuralRevision === null
    ? hasPendingRestart(snapshot?.activeAutomationRevision ?? null, automation?.revision ?? null, restartRequired)
    : false;
  $: envelopeStructuralPending = automation?.activeStructuralRevision !== undefined && automation?.savedStructuralRevision !== undefined && automation.activeStructuralRevision !== null && automation.savedStructuralRevision !== null && automation.activeStructuralRevision !== automation.savedStructuralRevision;
  $: pendingRestart = Boolean(snapshot?.restartRequired || automation?.restartRequired || restartRequired || legacyRevisionPending || envelopeStructuralPending || (activeStructural !== null && savedStructural !== null && activeStructural !== savedStructural));
  $: source = draft.logic?.source ?? '';
  $: selectedExecution = snapshot?.executions.find((execution) => execution.executionId === state.selectedExecutionId) ?? null;
  $: simulationErrors = simulationDraft ? validateSimulationDraft(simulationDraft) : [];
  $: simulationRevision = snapshot?.activeLogicRevision ?? automation?.activeLogicRevision ?? null;
  $: canSimulate = Boolean(simulationDraft && simulationRevision !== null && simulationErrors.length === 0 && !simulationRunning);
  $: simulationTriggerInput = simulationDraft ? simulationDraft.inputs.find((input) => input.endpoint === simulationDraft!.triggerEndpoint) ?? null : null;

  const dispatch = (action: Parameters<typeof reduceDashboardState>[1]) => { state = reduceDashboardState(state, action); };

  function documentWithLogic(document: AutomationDocument): AutomationDocument {
    return { ...document, logic: { source: document.logic?.source ?? '' } };
  }

  async function refreshAutomation(): Promise<void> {
    automationLoading = true;
    automationError = null;
    try {
      const loaded = await loadAutomation();
      automation = loaded;
      draft = documentWithLogic(loaded.document);
      conflictLatest = null;
      serverErrors = [];
      restartRequired = loaded.restartRequired === true;
      saveNotice = null;
    } catch (error) {
      automationError = error instanceof Error ? error.message : String(error);
    } finally {
      automationLoading = false;
    }
  }

  function updateSource(value: string): void {
    draft = { ...draft, logic: { source: value } };
    serverErrors = [];
    saveNotice = null;
  }

  function updateEndpoint(direction: 'inputs' | 'outputs', index: number, field: 'name' | 'dpt', value: string): void {
    const endpoint = draft[direction][index];
    if (!endpoint) return;
    if (field === 'name') draft = renameEndpoint(draft, endpoint.name, value);
    else draft = { ...draft, [direction]: draft[direction].map((item, current) => current === index ? { ...item, dpt: value as Dpt } : item) };
    serverErrors = [];
    saveNotice = null;
  }

  function addEndpoint(direction: 'inputs' | 'outputs'): void {
    const names = new Set([...draft.inputs, ...draft.outputs].map((endpoint) => endpoint.name));
    const prefix = direction === 'inputs' ? 'new_input' : 'new_output';
    let name = prefix;
    let suffix = 2;
    while (names.has(name)) name = `${prefix}_${suffix++}`;
    draft = { ...draft, [direction]: [...draft[direction], { name, dpt: '1.001' }] };
    serverErrors = [];
    saveNotice = null;
  }

  function updateBinding(index: number, field: 'endpoint' | 'group_address', value: string): void {
    draft = { ...draft, knx_bindings: draft.knx_bindings.map((binding, current) => current === index ? { ...binding, [field]: value } : binding) };
    serverErrors = [];
    saveNotice = null;
  }

  function addBinding(): void {
    draft = { ...draft, knx_bindings: [...draft.knx_bindings, { endpoint: draft.inputs[0]?.name ?? draft.outputs[0]?.name ?? '', group_address: '' }] };
    serverErrors = [];
    saveNotice = null;
  }

  function errorFor(path: string): string | null { return [...clientErrors, ...serverErrors].find((error) => error.path === path)?.message ?? null; }

  async function saveDraft(): Promise<void> {
    if (!automation || clientErrors.length || automationSaving) return;
    automationSaving = true;
    automationError = null;
    serverErrors = [];
    conflictLatest = null;
    saveNotice = null;
    const candidate = documentWithLogic(draft);
    try {
      const result = await saveAutomation(candidate, automation.revision);
      draft = candidate;
      automation = { ...automation, document: candidate, revision: result.revision, activeLogicRevision: result.activeLogicRevision, restartRequired: result.restartRequired };
      restartRequired = result.restartRequired;
      saveNotice = result.logicActivated && !result.restartRequired
      ? `Source activated for the next event (active document revision ${result.activeLogicRevision ?? 'updated'}).`
        : result.restartRequired
          ? 'Saved. Structural changes are waiting for a desktop restart; live source activation is paused.'
          : 'Saved. The active runtime will use this source after its next activation.';
    } catch (error) {
      if (error instanceof AutomationApiError) {
        serverErrors = error.fieldErrors;
        conflictLatest = error.latest;
        automationError = error.message;
      } else automationError = error instanceof Error ? error.message : String(error);
    } finally {
      automationSaving = false;
    }
  }

  function reloadConflict(): void {
    if (!conflictLatest) return;
    automation = conflictLatest;
    draft = documentWithLogic(conflictLatest.document);
    conflictLatest = null;
    automationError = null;
    serverErrors = [];
    saveNotice = null;
  }

  function observeSnapshot(next: NonNullable<DashboardState['snapshot']>): void {
    if (next.activeLogicRevision !== null && next.activeLogicRevision !== undefined && automation?.activeLogicRevision === next.activeLogicRevision) restartRequired = false;
    if (!next.restartRequired && next.activeStructuralRevision !== null && next.savedStructuralRevision !== null && next.activeStructuralRevision === next.savedStructuralRevision) {
      restartRequired = false;
      if (automation) automation = { ...automation, restartRequired: false, activeStructuralRevision: next.activeStructuralRevision, savedStructuralRevision: next.savedStructuralRevision, activeLogicRevision: next.activeLogicRevision };
    }
  }

  onMount(() => {
    void refreshAutomation();
    const client = new DashboardClient({
      handlers: {
        onSnapshot: (next) => {
          observeSnapshot(next);
          if (!simulationDraft) simulationDraft = createSimulationDraft(next);
          dispatch({ type: 'snapshot_loaded', snapshot: next, nowMs: Date.now() });
        },
        onEvent: (event) => { if (event.kind === 'update') observeSnapshot(event.snapshot); dispatch({ type: 'event_received', event }); },
        onStreamOpen: () => dispatch({ type: 'stream_open' }),
        onStreamLost: (error) => dispatch({ type: 'stream_lost', error }),
        onError: (error) => dispatch({ type: 'stream_error', error: error.message })
      }
    });
    void client.start();
    return () => client.stop();
  });

  function displayTime(value: string): string {
    const date = new Date(value);
    return Number.isNaN(date.valueOf()) ? value : date.toLocaleTimeString();
  }
  function displayFields(fields: Record<string, string | number | boolean | null>): string {
    return Object.entries(fields).map(([key, value]) => `${key}=${String(value)}`).join(' ');
  }
  function inputValue(event: Event): string { return (event.currentTarget as HTMLInputElement).value; }
  function selectValue(event: Event): string { return (event.currentTarget as HTMLSelectElement).value; }
  function endpointValue(endpoint: DisplayEndpoint): string { return formatValue(endpoint.observed ?? null); }
  function executionTime(milliseconds: number): string { return `${milliseconds} ms`; }
  function transitionForTrigger(trigger: DisplayExecutionTrigger): string {
    const previous = formatValue(trigger.previous);
    const current = formatValue(trigger.value);
    const flags = [trigger.changed ? 'changed' : null, trigger.rising ? 'rising' : null, trigger.falling ? 'falling' : null].filter(Boolean).join(', ');
    return `${previous} → ${current}${flags ? ` (${flags})` : ''}`;
  }
  function transition(execution: NonNullable<typeof selectedExecution>): string { return transitionForTrigger(execution.trigger); }
  function selectExecution(executionId: number): void { dispatch({ type: 'select_execution', executionId }); }

  function clearSimulationFeedback(): void {
    simulationResult = null;
    simulationError = null;
    simulationFieldErrors = [];
  }

  function useForSimulation(execution: NonNullable<typeof selectedExecution> | null = selectedExecution): void {
    if (!snapshot) return;
    simulationDraft = createSimulationDraft(snapshot, execution);
    clearSimulationFeedback();
  }

  function updateSimulationDraft(next: SimulationDraft): void {
    simulationDraft = forceTriggerInput(next);
    clearSimulationFeedback();
  }

  function simulationBoolean(event: Event): SimulationValue {
    const value = selectValue(event);
    return value === '' ? null : value === 'true';
  }

  function simulationNumber(event: Event): SimulationValue {
    const value = inputValue(event).trim();
    if (!value) return null;
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }

  function simulationAge(event: Event): number | null {
    const value = simulationNumber(event);
    return typeof value === 'number' ? value : null;
  }

  function updateSimulationTriggerEndpoint(endpoint: string): void {
    if (!simulationDraft) return;
    const selected = simulationDraft.inputs.find((input) => input.endpoint === endpoint);
    updateSimulationDraft({
      ...simulationDraft,
      triggerEndpoint: endpoint,
      triggerValue: selected?.value ?? null,
      previousValue: null
    });
  }

  function updateSimulationTriggerValue(value: SimulationValue): void {
    if (!simulationDraft) return;
    updateSimulationDraft({ ...simulationDraft, triggerValue: value });
  }

  function updateSimulationPreviousValue(value: SimulationValue): void {
    if (!simulationDraft) return;
    updateSimulationDraft({ ...simulationDraft, previousValue: value });
  }

  function updateSimulationInput(index: number, changes: Partial<SimulationDraft['inputs'][number]>): void {
    if (!simulationDraft) return;
    const inputs = simulationDraft.inputs.map((input, current) => current === index ? { ...input, ...changes } : input);
    updateSimulationDraft({ ...simulationDraft, inputs });
  }

  function updateSimulationValidity(index: number, valid: boolean): void {
    updateSimulationInput(index, valid ? { valid } : { valid, value: null, ageMs: null });
  }

  async function runSimulation(): Promise<void> {
    if (!simulationDraft || simulationRevision === null || simulationErrors.length > 0 || simulationRunning) return;
    const prepared = forceTriggerInput(simulationDraft);
    const scenario = toSimulationScenario(prepared, simulationRevision);
    if (!scenario) return;
    simulationDraft = prepared;
    simulationRunning = true;
    clearSimulationFeedback();
    try {
      simulationResult = await simulateScenario(scenario);
    } catch (error) {
      if (error instanceof SimulationApiError) {
        simulationError = error.message;
        simulationFieldErrors = error.fieldErrors.map((field) => `${field.path}: ${field.message}`);
      } else simulationError = error instanceof Error ? error.message : String(error);
    } finally {
      simulationRunning = false;
    }
  }

  function refreshDashboard(): void { window.location.reload(); }
</script>

<svelte:head><meta name="description" content="Live LogikSmith KNX runtime dashboard" /></svelte:head>

<main>
  <header class="page-header">
    <div><p class="eyebrow">LogikSmith / runtime</p><h1>KNX dashboard</h1></div>
    <div class="status-cluster" aria-live="polite">
      <span class:good={connectionState === 'connected'} class:bad={connectionState === 'failed'} class="status-pill">KNX {connectionState}</span>
      <span class:good={state.streamStatus === 'connected'} class="status-pill muted">Browser stream {state.streamStatus}</span>
      {#if state.stale}<span class="stale" title="The displayed snapshot is retained while the browser stream reconnects.">stale data</span>{/if}
    </div>
  </header>

  {#if state.error}<div class="alert" role="alert">{state.error}</div>{/if}

  <section class="panel editor" aria-label="Automation editor">
    <div class="section-heading">
      <div><h2>Automation editor</h2><p class="subtle">Edit the complete automation document. Lua source-only saves activate without a desktop restart.</p></div>
      {#if automation}<span class="revision-badge">Saved revision {automation.revision}</span>{/if}
    </div>
    {#if automationLoading}
      <p class="empty">Loading saved automation…</p>
    {:else if automation}
      {#if conflictLatest}
        <div class="conflict" role="alert"><span>The saved document changed. Reload the latest document before saving.</span><button type="button" on:click={reloadConflict}>Reload latest</button></div>
      {:else if automationError}
        <div class="alert" role="alert">{automationError}</div>
      {/if}
      {#if pendingRestart}<div class="restart-notice" role="status">Structural changes are waiting for a restart. Live Lua activation is paused, but the full document can still be saved.</div>{/if}
      {#if saveNotice}<div class="success-notice" role="status">{saveNotice}</div>{/if}
      <p class="subtle revision-line">Saved document revision {automation.revision}; active document revision {snapshot?.activeLogicRevision ?? automation.activeLogicRevision ?? 'unknown'}</p>

      <div class="source-editor">
        <div class="section-heading compact"><div><h3>Lua logic source</h3><p class="subtle">One global <code>handle(event, input, meta)</code> function runs for each input write.</p></div><span class="source-size">{new TextEncoder().encode(source).byteLength} / 65536 bytes</span></div>
        <textarea aria-label="Lua source" spellcheck="false" value={source} on:input={(event) => updateSource(inputValue(event))}></textarea>
        {#if errorFor('logic.source')}<small class="field-error">{errorFor('logic.source')}</small>{/if}
        <details class="source-reference">
          <summary>Source reference</summary>
          <div class="reference-grid">
            <code>event.input</code><span>logical input name</span>
            <code>event.value</code><span>trigger value: boolean or 0–100 percentage</span>
            <code>event.previous</code><span>previous value, or nil when unknown</span>
            <code>event.changed</code><span>true when a known previous value differs</span>
            <code>event.rising / event.falling</code><span>boolean false→true / true→false edge flags</span>
            <code>input.name</code><span>current value for every configured input</span>
            <code>meta.&lt;input&gt;.valid</code><span>whether that input has an observed value</span>
            <code>meta.&lt;input&gt;.age_ms</code><span>frozen age in milliseconds, or nil when invalid</span>
            <code>return &#123; outputs = &#123;...&#125; &#125;</code><span>named output values; omit outputs to do nothing</span>
          </div>
        </details>
      </div>

      <div class="editor-columns">
        <div>
          <div class="section-heading compact"><h3>Inputs</h3><button type="button" class="small-button" on:click={() => addEndpoint('inputs')}>Add input</button></div>
          <div class="table-wrap"><table class="editor-table"><thead><tr><th>Name</th><th>DPT</th><th></th></tr></thead><tbody>
            {#each draft.inputs as endpoint, index}
              <tr><td><input aria-label={`Input ${index + 1} name`} value={endpoint.name} on:input={(event) => updateEndpoint('inputs', index, 'name', inputValue(event))} />{#if errorFor(`inputs[${index}].name`)}<small class="field-error">{errorFor(`inputs[${index}].name`)}</small>{/if}</td>
                <td><select aria-label={`Input ${index + 1} DPT`} value={endpoint.dpt} on:change={(event) => updateEndpoint('inputs', index, 'dpt', selectValue(event))}><option value="1.001">1.001 switch</option><option value="5.001">5.001 percentage</option></select>{#if errorFor(`inputs[${index}].dpt`)}<small class="field-error">{errorFor(`inputs[${index}].dpt`)}</small>{/if}</td>
                <td><button type="button" class="danger-button" on:click={() => draft = removeEndpoint(draft, endpoint.name)}>Delete</button></td></tr>
            {/each}
          </tbody></table></div>
          {#if !draft.inputs.length}<p class="empty">No inputs.</p>{/if}
        </div>
        <div>
          <div class="section-heading compact"><h3>Outputs</h3><button type="button" class="small-button" on:click={() => addEndpoint('outputs')}>Add output</button></div>
          <div class="table-wrap"><table class="editor-table"><thead><tr><th>Name</th><th>DPT</th><th></th></tr></thead><tbody>
            {#each draft.outputs as endpoint, index}
              <tr><td><input aria-label={`Output ${index + 1} name`} value={endpoint.name} on:input={(event) => updateEndpoint('outputs', index, 'name', inputValue(event))} />{#if errorFor(`outputs[${index}].name`)}<small class="field-error">{errorFor(`outputs[${index}].name`)}</small>{/if}</td>
                <td><select aria-label={`Output ${index + 1} DPT`} value={endpoint.dpt} on:change={(event) => updateEndpoint('outputs', index, 'dpt', selectValue(event))}><option value="1.001">1.001 switch</option><option value="5.001">5.001 percentage</option></select>{#if errorFor(`outputs[${index}].dpt`)}<small class="field-error">{errorFor(`outputs[${index}].dpt`)}</small>{/if}</td>
                <td><button type="button" class="danger-button" on:click={() => draft = removeEndpoint(draft, endpoint.name)}>Delete</button></td></tr>
            {/each}
          </tbody></table></div>
          {#if !draft.outputs.length}<p class="empty">No outputs.</p>{/if}
        </div>
      </div>

      <div class="section-heading compact"><h3>KNX bindings</h3><button type="button" class="small-button" on:click={addBinding}>Add binding</button></div>
      <div class="table-wrap"><table class="editor-table bindings-table"><thead><tr><th>Endpoint</th><th>Group address</th><th></th></tr></thead><tbody>
        {#each draft.knx_bindings as binding, index}
          <tr><td><select aria-label={`Binding ${index + 1} endpoint`} value={binding.endpoint} on:change={(event) => updateBinding(index, 'endpoint', selectValue(event))}><option value="">Choose endpoint</option>{#each [...draft.inputs, ...draft.outputs] as endpoint}<option value={endpoint.name}>{endpoint.name} ({endpoint.dpt})</option>{/each}</select>{#if errorFor(`knx_bindings[${index}].endpoint`)}<small class="field-error">{errorFor(`knx_bindings[${index}].endpoint`)}</small>{/if}</td>
            <td><input aria-label={`Binding ${index + 1} group address`} value={binding.group_address} placeholder="1/2/3" on:input={(event) => updateBinding(index, 'group_address', inputValue(event))} />{#if errorFor(`knx_bindings[${index}].group_address`)}<small class="field-error">{errorFor(`knx_bindings[${index}].group_address`)}</small>{/if}</td>
            <td><button type="button" class="danger-button" on:click={() => draft = removeBinding(draft, index)}>Delete</button></td></tr>
        {/each}
      </tbody></table></div>
      {#if !draft.knx_bindings.length}<p class="empty">No bindings.</p>{/if}

      {#if clientErrors.length}<p class="validation-summary" role="alert">Fix {clientErrors.length} validation {clientErrors.length === 1 ? 'error' : 'errors'} before saving.</p>{/if}
      <div class="editor-actions"><button type="button" class="save-button" disabled={clientErrors.length > 0 || automationSaving} on:click={saveDraft}>{automationSaving ? 'Saving…' : 'Save document'}</button><span class="subtle">{clientErrors.length ? 'Save is disabled while the draft is invalid.' : pendingRestart ? 'Full document save remains available; restart is required for structural activation.' : 'Source-only edits activate between events.'}</span></div>
    {:else}
      <p class="empty">The saved automation document could not be loaded.</p><button type="button" on:click={refreshAutomation}>Retry</button>
    {/if}
  </section>

  {#if snapshot}
    <section class="panel inspector" aria-label="Execution inspector">
      <div class="section-heading"><div><h2>Execution inspector</h2><p class="subtle">The latest 50 immutable logic decisions, newest first.</p></div><span>{snapshot.executions.length}</span></div>
      <dl class="facts runtime-facts">
        <dt>Saved document revision</dt><dd>{snapshot.savedLogicRevision ?? automation?.savedLogicRevision ?? automation?.activeLogicRevision ?? 'unknown'}</dd>
        <dt>Active document revision</dt><dd>{snapshot.activeLogicRevision ?? automation?.activeLogicRevision ?? 'unknown'}</dd>
        <dt>Structural state</dt><dd>{pendingRestart ? 'restart pending' : 'active'}</dd>
      </dl>
      {#if state.selectionNotice}<p class="selection-notice" role="status">{state.selectionNotice}</p>{/if}
      {#if snapshot.executions.length}
        <div class="table-wrap execution-history"><table><thead><tr><th>Time</th><th>Trigger</th><th>Transition</th><th>Status</th><th>Effects</th><th>Document revision</th><th>Duration</th></tr></thead><tbody>
          {#each snapshot.executions as execution}
            <tr class:selected={execution.executionId === state.selectedExecutionId} class:pinned={execution.executionId === state.selectedExecutionId && state.selectionPinned} tabindex="0" role="button" on:click={() => selectExecution(execution.executionId)} on:keydown={(event) => { if (event.key === 'Enter' || event.key === ' ') selectExecution(execution.executionId); }}>
              <td>{executionTime(execution.timeMs)}</td><td>{execution.trigger.endpoint} = {formatValue(execution.trigger.value)}</td><td>{transition(execution)}</td><td><span class="status-pill logic-{execution.status}">{execution.status}</span></td><td>{execution.effects.length}</td><td>{execution.logicRevision ?? '—'}</td><td>{formatDuration(execution.durationUs)}</td>
            </tr>
          {/each}
        </tbody></table></div>
        {#if selectedExecution}
          <article class="execution-detail" aria-label="Selected execution details">
            <div class="section-heading"><div><h3>Execution {selectedExecution.executionId}</h3><span>{state.selectionPinned ? 'pinned selection' : 'following newest'}</span></div><button type="button" class="small-button" on:click={() => useForSimulation(selectedExecution)}>Use for simulation</button></div>
            <dl class="facts">
              <dt>Status</dt><dd><span class="status-pill logic-{selectedExecution.status}">{selectedExecution.status}</span></dd>
              <dt>Time</dt><dd>{executionTime(selectedExecution.timeMs)}</dd>
              <dt>Duration</dt><dd>{formatDuration(selectedExecution.durationUs)}</dd>
              <dt>Document revision</dt><dd>{selectedExecution.logicRevision ?? '—'}</dd>
              <dt>Trigger</dt><dd><code>{selectedExecution.trigger.endpoint}</code> / {selectedExecution.trigger.dpt} / {formatValue(selectedExecution.trigger.value)}</dd>
              <dt>Transition flags</dt><dd>previous {formatValue(selectedExecution.trigger.previous)}; changed {String(selectedExecution.trigger.changed)}; rising {String(selectedExecution.trigger.rising)}; falling {String(selectedExecution.trigger.falling)}</dd>
            </dl>
            <h3>Input snapshot</h3>
            <div class="table-wrap"><table><thead><tr><th>Endpoint</th><th>DPT</th><th>Value</th><th>Validity</th><th>Age</th></tr></thead><tbody>{#each selectedExecution.inputs as input}<tr><td>{input.endpoint}</td><td>{input.dpt}</td><td><span class="value">{formatValue(input.value)}</span></td><td>{input.valid ? 'valid' : 'invalid'}</td><td>{formatAge(input.ageMs)}</td></tr>{/each}</tbody></table></div>
            <h3>Returned effects</h3>
            {#if selectedExecution.effects.length}<div class="table-wrap"><table><thead><tr><th>Endpoint</th><th>DPT</th><th>Value</th><th>Resolved KNX address</th></tr></thead><tbody>{#each selectedExecution.effects as effect}<tr><td>{effect.endpoint}</td><td>{effect.dpt}</td><td><span class="value">{formatValue(effect.value)}</span></td><td><code>{effect.destination}</code></td></tr>{/each}</tbody></table></div>{:else}<p class="empty">No effects returned.</p>{/if}
            {#if selectedExecution.error}<div class="logic-error" role="alert"><strong>{selectedExecution.error.category}</strong>{#if selectedExecution.error.line !== null} line {selectedExecution.error.line}:{/if} {selectedExecution.error.message}</div>{/if}
          </article>
        {/if}
      {:else}<p class="empty">No executions yet. Trigger a configured input to inspect its decision.</p>{/if}
    </section>

    <section class="panel simulation-panel" aria-label="Simulation">
      <div class="section-heading">
        <div><h2>Simulation</h2><p class="subtle">Run the active logic against an editable input snapshot.</p></div>
        {#if simulationRevision !== null}<span class="revision-badge">Active revision {simulationRevision}</span>{/if}
      </div>
      <p class="simulation-safety" role="note"><strong>Safe test:</strong> simulation sends no KNX write and does not change live values, history, or runtime state.</p>
      {#if !simulationDraft}
        <p class="empty">Loading simulation inputs…</p>
      {:else if !simulationDraft.inputs.length}
        <p class="empty">No active configured inputs are available for simulation.</p>
      {:else}
        <div class="simulation-form">
          <div class="simulation-trigger-fields">
            <label>Triggering input
              <select aria-label="Simulation triggering input" value={simulationDraft.triggerEndpoint} on:change={(event) => updateSimulationTriggerEndpoint(selectValue(event))}>
                <option value="">Choose input</option>
                {#each simulationDraft.inputs as input}<option value={input.endpoint}>{input.endpoint} ({input.dpt})</option>{/each}
              </select>
            </label>
            <label>Current value
              {#if simulationTriggerInput?.dpt === '1.001'}
                <select aria-label="Simulation current trigger value" value={simulationDraft.triggerValue === null ? '' : String(simulationDraft.triggerValue)} on:change={(event) => updateSimulationTriggerValue(simulationBoolean(event))}>
                  <option value="">Choose value</option><option value="false">false</option><option value="true">true</option>
                </select>
              {:else if simulationTriggerInput?.dpt === '5.001'}
                <input aria-label="Simulation current trigger value" type="number" min="0" max="100" step="1" placeholder="0–100" value={simulationDraft.triggerValue ?? ''} on:input={(event) => updateSimulationTriggerValue(simulationNumber(event))} />
              {:else}<span class="subtle">Choose an input first.</span>{/if}
            </label>
            <label>Previous value <span class="subtle">(optional)</span>
              {#if simulationTriggerInput?.dpt === '1.001'}
                <select aria-label="Simulation previous trigger value" value={simulationDraft.previousValue === null ? '' : String(simulationDraft.previousValue)} on:change={(event) => updateSimulationPreviousValue(simulationBoolean(event))}>
                  <option value="">unknown</option><option value="false">false</option><option value="true">true</option>
                </select>
              {:else if simulationTriggerInput?.dpt === '5.001'}
                <input aria-label="Simulation previous trigger value" type="number" min="0" max="100" step="1" placeholder="unknown" value={simulationDraft.previousValue ?? ''} on:input={(event) => updateSimulationPreviousValue(simulationNumber(event))} />
              {:else}<span class="subtle">Choose an input first.</span>{/if}
            </label>
          </div>

          <h3>Input snapshot</h3>
          <div class="table-wrap"><table class="simulation-table"><thead><tr><th>Input</th><th>DPT</th><th>Value</th><th>Valid</th><th>Age (ms)</th></tr></thead><tbody>
            {#each simulationDraft.inputs as input, index}
              {@const isTrigger = input.endpoint === simulationDraft.triggerEndpoint}
              <tr class:simulation-trigger-row={isTrigger}>
                <td><code>{input.endpoint}</code>{#if isTrigger}<small> trigger</small>{/if}</td>
                <td>{input.dpt}</td>
                <td>
                  {#if input.dpt === '1.001'}
                    <select aria-label={'Simulation ' + input.endpoint + ' value'} disabled={!input.valid || isTrigger} value={input.value === null ? '' : String(input.value)} on:change={(event) => updateSimulationInput(index, { value: simulationBoolean(event) })}>
                      <option value="">Choose value</option><option value="false">false</option><option value="true">true</option>
                    </select>
                  {:else}
                    <input aria-label={'Simulation ' + input.endpoint + ' value'} disabled={!input.valid || isTrigger} type="number" min="0" max="100" step="1" placeholder="0–100" value={input.value ?? ''} on:input={(event) => updateSimulationInput(index, { value: simulationNumber(event) })} />
                  {/if}
                </td>
                <td><label class="checkbox-label"><input aria-label={'Simulation ' + input.endpoint + ' validity'} type="checkbox" checked={input.valid} disabled={isTrigger} on:change={(event) => updateSimulationValidity(index, (event.currentTarget as HTMLInputElement).checked)} /> valid</label></td>
                <td><input aria-label={'Simulation ' + input.endpoint + ' age'} disabled={!input.valid || isTrigger} type="number" min="0" step="1" placeholder="unknown" value={input.ageMs ?? ''} on:input={(event) => updateSimulationInput(index, { ageMs: simulationAge(event) })} /></td>
              </tr>
            {/each}
          </tbody></table></div>
          <p class="subtle simulation-trigger-note">The selected trigger is always submitted as valid with its current value and age 0.</p>

          {#if simulationErrors.length}
            <div class="validation-summary" role="alert"><strong>Complete the scenario before running:</strong><ul>{#each simulationErrors as error}<li>{error}</li>{/each}</ul></div>
          {/if}
          {#if simulationError}
            <div class="logic-error simulation-error" role="alert">{simulationError}
              {#if simulationFieldErrors.length}<ul>{#each simulationFieldErrors as error}<li>{error}</li>{/each}</ul>{/if}
              {#if simulationError.includes('active logic source changed')}<button type="button" class="small-button" on:click={refreshDashboard}>Refresh dashboard, then re-run</button>{/if}
            </div>
          {/if}
          <div class="editor-actions simulation-actions"><button type="button" class="save-button" disabled={!canSimulate} on:click={runSimulation}>{simulationRunning ? 'Running…' : 'Run simulation'}</button><span class="subtle">Uses active revision {simulationRevision ?? 'unknown'}.</span></div>

          {#if simulationResult}
            <article class:simulation-failed={simulationResult.status === 'failed'} class:simulation-succeeded={simulationResult.status === 'succeeded'} class="simulation-result" aria-label="Simulation result" aria-live="polite">
              <div class="section-heading"><h3>Simulation {simulationResult.status}</h3><span>{formatDuration(simulationResult.durationUs)} · revision {simulationResult.logicRevision}</span></div>
              <p>Trigger <code>{simulationResult.trigger.endpoint}</code> = {formatValue(simulationResult.trigger.value)}; previous {formatValue(simulationResult.trigger.previous)}; {transitionForTrigger(simulationResult.trigger)}</p>
              {#if simulationResult.status === 'failed' && simulationResult.error}
                <div class="logic-error" role="alert"><strong>{simulationResult.error.category}</strong>{#if simulationResult.error.line !== null} line {simulationResult.error.line}:{/if} {simulationResult.error.message}</div>
              {:else if simulationResult.effects.length}
                <div class="simulation-effects" aria-label="Proposed effects"><h3>Proposed effects</h3>{#each simulationResult.effects as effect}<div><strong>Would emit</strong> <code>{effect.endpoint}</code> = <span class="value">{formatValue(effect.value)}</span> ({effect.dpt}) to <code>{effect.destination}</code></div>{/each}</div>
              {:else}<p class="empty">No effects — the active logic would not emit anything.</p>{/if}
              <p class="subtle no-write-note">Simulation sends no KNX write.</p>
            </article>
          {/if}
        </div>
      {/if}
    </section>

    <section class="grid two-columns" aria-label="Current values">
      <article class="panel">
        <h2>Active endpoints</h2>
        {#if snapshot.automation}
          <div class="endpoint-list">{#each snapshot.automation.inputs as endpoint}<div><span class="endpoint-name">{endpoint.name ?? '—'}</span> <small>input / {endpoint.dpt}</small> <code>{endpoint.address || '—'}</code> <span class="value">{endpointValue(endpoint)}</span></div>{/each}{#each snapshot.automation.outputs as endpoint}<div><span class="endpoint-name">{endpoint.name ?? '—'}</span> <small>output / {endpoint.dpt}</small> <code>{endpoint.address || '—'}</code> <span class="value">{endpointValue(endpoint)}</span> <span class="value requested">requested {formatValue(endpoint.requested ?? null)}</span></div>{/each}</div>
        {:else}<p class="empty">No endpoint projection available.</p>{/if}
      </article>
    </section>

    <section class="panel" aria-label="Values and writes"><h2>Values and writes</h2><dl class="facts"><dt>Observed input</dt><dd><span class="value">{formatValue(snapshot.values.input.observed)}</span></dd><dt>Observed output</dt><dd><span class="value">{formatValue(snapshot.values.output.observed)}</span></dd><dt>Requested output</dt><dd><span class="value requested">{formatValue(snapshot.values.output.requested)}</span></dd><dt>Write status</dt><dd>{snapshot.write.status}{snapshot.write.error ? ` — ${snapshot.write.error}` : ''}</dd></dl></section>

    <section class="panel" aria-label="Recent KNX telegrams"><div class="section-heading"><h2>Recent telegrams</h2><span>{snapshot.telegrams.length}</span></div>{#if snapshot.telegrams.length}<div class="table-wrap"><table><thead><tr><th>Time</th><th>Source</th><th>Destination</th><th>Service</th><th>DPT</th><th>Value</th></tr></thead><tbody>{#each snapshot.telegrams as telegram}<tr><td>{displayTime(telegram.time)}</td><td>{telegram.source ?? '—'}</td><td><code>{telegram.destination}</code></td><td>{telegram.service}</td><td>{telegram.dpt}</td><td>{formatValue(telegram.value)}</td></tr>{/each}</tbody></table></div>{:else}<p class="empty">No telegrams observed yet.</p>{/if}</section>
    <section class="panel" aria-label="Recent runtime logs"><div class="section-heading"><h2>Recent logs</h2><span>{snapshot.logs.length}</span></div>{#if snapshot.logs.length}<div class="table-wrap"><table><thead><tr><th>Time</th><th>Level</th><th>Target</th><th>Message</th><th>Fields</th></tr></thead><tbody>{#each snapshot.logs as log}<tr><td>{displayTime(log.time)}</td><td><span class="level level-{log.level.toLowerCase()}">{log.level}</span></td><td>{log.target}</td><td>{log.message}</td><td class="fields">{displayFields(log.fields) || '—'}</td></tr>{/each}</tbody></table></div>{:else}<p class="empty">No runtime logs captured yet.</p>{/if}</section>
  {:else}<section class="panel loading" aria-live="polite">Loading runtime snapshot…</section>{/if}
  <footer>Snapshot revision {state.revision}</footer>
</main>
