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
  import { DashboardClient } from './lib/api';
  import {
    formatValue,
    hasPendingRestart,
    initialDashboardState,
    reduceDashboardState,
    type DashboardState,
    type DisplayEndpoint
  } from './lib/state';

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
        ? `Source activated for the next event (active logic revision ${result.activeLogicRevision ?? 'updated'}).`
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
        onSnapshot: (next) => { observeSnapshot(next); dispatch({ type: 'snapshot_loaded', snapshot: next, nowMs: Date.now() }); },
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
      {#if automationError}<div class="alert" role="alert">{automationError}</div>{/if}
      {#if conflictLatest}<div class="conflict" role="alert"><span>The saved file is newer than this draft.</span><button type="button" on:click={reloadConflict}>Reload latest</button></div>{/if}
      {#if pendingRestart}<div class="restart-notice" role="status">Structural changes are waiting for a restart. Live Lua activation is paused, but the full document can still be saved.</div>{/if}
      {#if saveNotice}<div class="success-notice" role="status">{saveNotice}</div>{/if}
      <p class="subtle revision-line">Saved document revision {automation.revision}; active logic revision {snapshot?.activeLogicRevision ?? automation.activeLogicRevision ?? 'unknown'}</p>

      <div class="source-editor">
        <div class="section-heading compact"><div><h3>Lua logic source</h3><p class="subtle">One global <code>handle(event, input)</code> function runs for each input write.</p></div><span class="source-size">{new TextEncoder().encode(source).byteLength} / 65536 bytes</span></div>
        <textarea aria-label="Lua source" spellcheck="false" value={source} on:input={(event) => updateSource(inputValue(event))}></textarea>
        {#if errorFor('logic.source')}<small class="field-error">{errorFor('logic.source')}</small>{/if}
        <details class="source-reference">
          <summary>Source reference</summary>
          <div class="reference-grid">
            <code>event.input</code><span>logical input name</span>
            <code>event.value</code><span>trigger value: boolean or 0–100 percentage</span>
            <code>input.name</code><span>current value for every configured input</span>
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
    <section class="grid two-columns" aria-label="Logic status and current values">
      <article class="panel">
        <h2>Logic runtime</h2>
        <dl class="facts">
          <dt>Saved logic revision</dt><dd>{snapshot.savedLogicRevision ?? automation?.savedLogicRevision ?? automation?.activeLogicRevision ?? 'unknown'}</dd>
          <dt>Active logic revision</dt><dd>{snapshot.activeLogicRevision ?? automation?.activeLogicRevision ?? 'unknown'}</dd>
          <dt>Structural state</dt><dd>{pendingRestart ? 'restart pending' : 'active'}</dd>
          <dt>Last execution</dt><dd><span class="status-pill logic-{snapshot.logicExecution.status}">{snapshot.logicExecution.status}</span></dd>
          <dt>Trigger</dt><dd>{snapshot.logicExecution.triggerInput ?? '—'}{snapshot.logicExecution.triggerValue !== null ? ` = ${formatValue(snapshot.logicExecution.triggerValue)}` : ''}</dd>
          <dt>Execution revision</dt><dd>{snapshot.logicExecution.logicRevision ?? '—'}</dd>
          <dt>Returned effects</dt><dd>{snapshot.logicExecution.effectCount}</dd>
        </dl>
        {#if snapshot.logicExecution.error}<div class="logic-error" role="alert"><strong>{snapshot.logicExecution.error.category}</strong>{#if snapshot.logicExecution.error.line !== null} line {snapshot.logicExecution.error.line}:{/if} {snapshot.logicExecution.error.message}</div>{/if}
      </article>
      <article class="panel">
        <h2>Active endpoints</h2>
        {#if snapshot.automation}
          <div class="endpoint-list">{#each snapshot.automation.inputs as endpoint}<div><span class="endpoint-name">{endpoint.name ?? '—'}</span> <small>input / {endpoint.dpt}</small> <code>{endpoint.address || '—'}</code> <span class="value">{endpointValue(endpoint)}</span></div>{/each}{#each snapshot.automation.outputs as endpoint}<div><span class="endpoint-name">{endpoint.name ?? '—'}</span> <small>output / {endpoint.dpt}</small> <code>{endpoint.address || '—'}</code> <span class="value">{endpointValue(endpoint)}</span> <span class="value requested">requested {formatValue(endpoint.requested ?? null)}</span></div>{/each}</div>
        {:else}<p class="empty">No endpoint projection available.</p>{/if}
      </article>
    </section>

    <section class="panel" aria-label="Recent logical output effects">
      <div class="section-heading"><h2>Recent logical output effects</h2><span>{snapshot.logicEffects.length}</span></div>
      {#if snapshot.logicEffects.length}<div class="table-wrap"><table><thead><tr><th>Time</th><th>Endpoint</th><th>DPT</th><th>Value</th><th>Resolved KNX address</th></tr></thead><tbody>{#each snapshot.logicEffects as effect}<tr><td>{displayTime(effect.time)}</td><td>{effect.endpoint}</td><td>{effect.dpt}</td><td><span class="value">{formatValue(effect.value)}</span></td><td><code>{effect.address}</code></td></tr>{/each}</tbody></table></div>{:else}<p class="empty">No logical effects returned yet.</p>{/if}
    </section>

    <section class="panel" aria-label="Values and writes"><h2>Values and writes</h2><dl class="facts"><dt>Observed input</dt><dd><span class="value">{formatValue(snapshot.values.input.observed)}</span></dd><dt>Observed output</dt><dd><span class="value">{formatValue(snapshot.values.output.observed)}</span></dd><dt>Requested output</dt><dd><span class="value requested">{formatValue(snapshot.values.output.requested)}</span></dd><dt>Write status</dt><dd>{snapshot.write.status}{snapshot.write.error ? ` — ${snapshot.write.error}` : ''}</dd></dl></section>

    <section class="panel" aria-label="Recent KNX telegrams"><div class="section-heading"><h2>Recent telegrams</h2><span>{snapshot.telegrams.length}</span></div>{#if snapshot.telegrams.length}<div class="table-wrap"><table><thead><tr><th>Time</th><th>Source</th><th>Destination</th><th>Service</th><th>DPT</th><th>Value</th></tr></thead><tbody>{#each snapshot.telegrams as telegram}<tr><td>{displayTime(telegram.time)}</td><td>{telegram.source ?? '—'}</td><td><code>{telegram.destination}</code></td><td>{telegram.service}</td><td>{telegram.dpt}</td><td>{formatValue(telegram.value)}</td></tr>{/each}</tbody></table></div>{:else}<p class="empty">No telegrams observed yet.</p>{/if}</section>
    <section class="panel" aria-label="Recent runtime logs"><div class="section-heading"><h2>Recent logs</h2><span>{snapshot.logs.length}</span></div>{#if snapshot.logs.length}<div class="table-wrap"><table><thead><tr><th>Time</th><th>Level</th><th>Target</th><th>Message</th><th>Fields</th></tr></thead><tbody>{#each snapshot.logs as log}<tr><td>{displayTime(log.time)}</td><td><span class="level level-{log.level.toLowerCase()}">{log.level}</span></td><td>{log.target}</td><td>{log.message}</td><td class="fields">{displayFields(log.fields) || '—'}</td></tr>{/each}</tbody></table></div>{:else}<p class="empty">No runtime logs captured yet.</p>{/if}</section>
  {:else}<section class="panel loading" aria-live="polite">Loading runtime snapshot…</section>{/if}
  <footer>Snapshot revision {state.revision}</footer>
</main>
