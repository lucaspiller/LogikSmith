<script lang="ts">
  import { onMount } from 'svelte';
  import {
    AutomationApiError,
    compatibleEndpoints,
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
    countdownMs,
    formatCountdown,
    formatValue,
    hasPendingRestart,
    initialDashboardState,
    reduceDashboardState,
    type DashboardState
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
  $: snapshot = state.snapshot;
  $: countdown = countdownMs(snapshot, state.nowMs);
  $: connectionState = snapshot?.connection.state ?? 'starting';
  $: inputLabel = snapshot?.automation?.inputs[0]?.name ?? 'Input';
  $: outputLabel = snapshot?.automation?.outputs[0]?.name ?? 'Output';
  $: clientErrors = validateAutomation(draft);
  $: pendingRestart = hasPendingRestart(snapshot?.activeAutomationRevision ?? null, automation?.revision ?? null, restartRequired);

  const dispatch = (action: Parameters<typeof reduceDashboardState>[1]) => {
    state = reduceDashboardState(state, action);
  };

  async function refreshAutomation(): Promise<void> {
    automationLoading = true;
    automationError = null;
    try {
      const loaded = await loadAutomation();
      automation = loaded;
      draft = loaded.document;
      conflictLatest = null;
      serverErrors = [];
    } catch (error) {
      automationError = error instanceof Error ? error.message : String(error);
    } finally {
      automationLoading = false;
    }
  }

  function updateEndpoint(direction: 'inputs' | 'outputs', index: number, field: 'name' | 'dpt', value: string): void {
    const endpoint = draft[direction][index];
    if (!endpoint) return;
    if (field === 'name') {
      draft = renameEndpoint(draft, endpoint.name, value);
    } else {
      draft = { ...draft, [direction]: draft[direction].map((item, current) => current === index ? { ...item, dpt: value as Dpt } : item) };
    }
    serverErrors = [];
  }

  function addEndpoint(direction: 'inputs' | 'outputs'): void {
    const names = new Set([...draft.inputs, ...draft.outputs].map((endpoint) => endpoint.name));
    const prefix = direction === 'inputs' ? 'new_input' : 'new_output';
    let name = prefix;
    let suffix = 2;
    while (names.has(name)) name = `${prefix}_${suffix++}`;
    draft = { ...draft, [direction]: [...draft[direction], { name, dpt: '1.001' }] };
    serverErrors = [];
  }

  function updateBinding(index: number, field: 'endpoint' | 'group_address', value: string): void {
    draft = { ...draft, knx_bindings: draft.knx_bindings.map((binding, current) => current === index ? { ...binding, [field]: value } : binding) };
    serverErrors = [];
  }

  function addBinding(): void {
    draft = { ...draft, knx_bindings: [...draft.knx_bindings, { endpoint: draft.inputs[0]?.name ?? draft.outputs[0]?.name ?? '', group_address: '' }] };
    serverErrors = [];
  }

  function updateBehavior(rule: 'timed_bool' | 'percentage_forward', field: 'input' | 'output' | 'off_delay_ms', value: string): void {
    const current = draft.behaviors[rule];
    draft = {
      ...draft,
      behaviors: {
        ...draft.behaviors,
        [rule]: { ...current, [field]: field === 'off_delay_ms' ? Number(value) : value }
      }
    } as AutomationDocument;
    serverErrors = [];
  }

  function errorFor(path: string): string | null {
    return [...clientErrors, ...serverErrors].find((error) => error.path === path)?.message ?? null;
  }

  async function saveDraft(): Promise<void> {
    if (!automation || clientErrors.length || automationSaving) return;
    automationSaving = true;
    automationError = null;
    serverErrors = [];
    conflictLatest = null;
    try {
      const result = await saveAutomation(draft, automation.revision);
      automation = { document: draft, revision: result.revision };
      restartRequired = result.restartRequired;
    } catch (error) {
      if (error instanceof AutomationApiError) {
        serverErrors = error.fieldErrors;
        conflictLatest = error.latest;
        automationError = error.message;
      } else {
        automationError = error instanceof Error ? error.message : String(error);
      }
    } finally {
      automationSaving = false;
    }
  }

  function reloadConflict(): void {
    if (!conflictLatest) return;
    automation = conflictLatest;
    draft = conflictLatest.document;
    conflictLatest = null;
    automationError = null;
    serverErrors = [];
  }

  function observeSnapshot(next: NonNullable<DashboardState['snapshot']>): void {
    if (next?.activeAutomationRevision !== null && next?.activeAutomationRevision !== undefined && automation?.revision === next.activeAutomationRevision) restartRequired = false;
  }

  onMount(() => {
    void refreshAutomation();
    const client = new DashboardClient({
      handlers: {
        onSnapshot: (next) => {
          observeSnapshot(next);
          dispatch({ type: 'snapshot_loaded', snapshot: next, nowMs: Date.now() });
        },
        onEvent: (event) => {
          if (event.kind === 'update') observeSnapshot(event.snapshot);
          dispatch({ type: 'event_received', event });
        },
        onStreamOpen: () => dispatch({ type: 'stream_open' }),
        onStreamLost: (error) => dispatch({ type: 'stream_lost', error }),
        onError: (error) => dispatch({ type: 'stream_error', error: error.message })
      }
    });
    void client.start();
    const interval = window.setInterval(() => dispatch({ type: 'tick', nowMs: Date.now() }), 100);
    return () => {
      client.stop();
      window.clearInterval(interval);
    };
  });

  function displayTime(value: string): string {
    const date = new Date(value);
    return Number.isNaN(date.valueOf()) ? value : date.toLocaleTimeString();
  }

  function displayFields(fields: Record<string, string | number | boolean | null>): string {
    return Object.entries(fields).map(([key, value]) => `${key}=${String(value)}`).join(' ');
  }

  function inputValue(event: Event): string {
    return (event.currentTarget as HTMLInputElement).value;
  }

  function selectValue(event: Event): string {
    return (event.currentTarget as HTMLSelectElement).value;
  }
</script>

<svelte:head>
  <meta name="description" content="Live LogikSmith KNX runtime dashboard" />
</svelte:head>

<main>
  <header class="page-header">
    <div>
      <p class="eyebrow">LogikSmith / runtime</p>
      <h1>KNX dashboard</h1>
    </div>
    <div class="status-cluster" aria-live="polite">
      <span class:good={connectionState === 'connected'} class:bad={connectionState === 'failed'} class="status-pill">
        KNX {connectionState}
      </span>
      <span class:good={state.streamStatus === 'connected'} class="status-pill muted">
        Browser stream {state.streamStatus}
      </span>
      {#if state.stale}
        <span class="stale" title="The displayed snapshot is retained while the browser stream reconnects.">stale data</span>
      {/if}
    </div>
  </header>

  {#if state.error}
    <div class="alert" role="alert">{state.error}</div>
  {/if}

  <section class="panel editor" aria-label="Automation editor">
    <div class="section-heading">
      <div>
        <h2>Automation editor</h2>
        <p class="subtle">Edit saved configuration. Changes take effect after a desktop restart.</p>
      </div>
      {#if automation}
        <span class="revision-badge">Saved revision {automation.revision}</span>
      {/if}
    </div>

    {#if automationLoading}
      <p class="empty">Loading saved automation…</p>
    {:else if automation}
      {#if automationError}
        <div class="alert" role="alert">{automationError}</div>
      {/if}
      {#if conflictLatest}
        <div class="conflict" role="alert">
          <span>The saved file is newer than this draft.</span>
          <button type="button" on:click={reloadConflict}>Reload latest</button>
        </div>
      {/if}
      {#if pendingRestart}
        <div class="restart-notice" role="status">Saved configuration is waiting for a restart. The runtime still uses its active snapshot.</div>
      {/if}
      <p class="subtle revision-line">Saved revision {automation.revision}; active revision {snapshot?.activeAutomationRevision ?? 'unknown'}</p>

      <div class="editor-columns">
        <div>
          <div class="section-heading compact"><h3>Inputs</h3><button type="button" class="small-button" on:click={() => addEndpoint('inputs')}>Add input</button></div>
          <div class="table-wrap">
            <table class="editor-table">
              <thead><tr><th>Name</th><th>DPT</th><th></th></tr></thead>
              <tbody>
                {#each draft.inputs as endpoint, index}
                  <tr>
                    <td>
                      <input aria-label={`Input ${index + 1} name`} value={endpoint.name} on:input={(event) => updateEndpoint('inputs', index, 'name', inputValue(event))} />
                      {#if errorFor(`inputs[${index}].name`)}<small class="field-error">{errorFor(`inputs[${index}].name`)}</small>{/if}
                    </td>
                    <td>
                      <select aria-label={`Input ${index + 1} DPT`} value={endpoint.dpt} on:change={(event) => updateEndpoint('inputs', index, 'dpt', selectValue(event))}>
                        <option value="1.001">1.001 switch</option><option value="5.001">5.001 percentage</option>
                      </select>
                      {#if errorFor(`inputs[${index}].dpt`)}<small class="field-error">{errorFor(`inputs[${index}].dpt`)}</small>{/if}
                    </td>
                    <td><button type="button" class="danger-button" on:click={() => draft = removeEndpoint(draft, endpoint.name)}>Delete</button></td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
          {#if !draft.inputs.length}<p class="empty">No inputs.</p>{/if}
        </div>

        <div>
          <div class="section-heading compact"><h3>Outputs</h3><button type="button" class="small-button" on:click={() => addEndpoint('outputs')}>Add output</button></div>
          <div class="table-wrap">
            <table class="editor-table">
              <thead><tr><th>Name</th><th>DPT</th><th></th></tr></thead>
              <tbody>
                {#each draft.outputs as endpoint, index}
                  <tr>
                    <td>
                      <input aria-label={`Output ${index + 1} name`} value={endpoint.name} on:input={(event) => updateEndpoint('outputs', index, 'name', inputValue(event))} />
                      {#if errorFor(`outputs[${index}].name`)}<small class="field-error">{errorFor(`outputs[${index}].name`)}</small>{/if}
                    </td>
                    <td>
                      <select aria-label={`Output ${index + 1} DPT`} value={endpoint.dpt} on:change={(event) => updateEndpoint('outputs', index, 'dpt', selectValue(event))}>
                        <option value="1.001">1.001 switch</option><option value="5.001">5.001 percentage</option>
                      </select>
                      {#if errorFor(`outputs[${index}].dpt`)}<small class="field-error">{errorFor(`outputs[${index}].dpt`)}</small>{/if}
                    </td>
                    <td><button type="button" class="danger-button" on:click={() => draft = removeEndpoint(draft, endpoint.name)}>Delete</button></td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
          {#if !draft.outputs.length}<p class="empty">No outputs.</p>{/if}
        </div>
      </div>

      <div class="section-heading compact"><h3>KNX bindings</h3><button type="button" class="small-button" on:click={addBinding}>Add binding</button></div>
      <div class="table-wrap">
        <table class="editor-table bindings-table">
          <thead><tr><th>Endpoint</th><th>Group address</th><th></th></tr></thead>
          <tbody>
            {#each draft.knx_bindings as binding, index}
              <tr>
                <td>
                  <select aria-label={`Binding ${index + 1} endpoint`} value={binding.endpoint} on:change={(event) => updateBinding(index, 'endpoint', selectValue(event))}>
                    <option value="">Choose endpoint</option>
                    {#each [...draft.inputs, ...draft.outputs] as endpoint}<option value={endpoint.name}>{endpoint.name} ({endpoint.dpt})</option>{/each}
                  </select>
                  {#if errorFor(`knx_bindings[${index}].endpoint`)}<small class="field-error">{errorFor(`knx_bindings[${index}].endpoint`)}</small>{/if}
                </td>
                <td>
                  <input aria-label={`Binding ${index + 1} group address`} value={binding.group_address} placeholder="1/2/3" on:input={(event) => updateBinding(index, 'group_address', inputValue(event))} />
                  {#if errorFor(`knx_bindings[${index}].group_address`)}<small class="field-error">{errorFor(`knx_bindings[${index}].group_address`)}</small>{/if}
                </td>
                <td><button type="button" class="danger-button" on:click={() => draft = removeBinding(draft, index)}>Delete</button></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#if !draft.knx_bindings.length}<p class="empty">No bindings.</p>{/if}

      <div class="behavior-grid">
        <div>
          <h3>Timed boolean</h3>
          <label>Input
            <select value={draft.behaviors.timed_bool.input} on:change={(event) => updateBehavior('timed_bool', 'input', selectValue(event))}>
              {#if draft.behaviors.timed_bool.input && !compatibleEndpoints(draft, 'input', '1.001').some((endpoint) => endpoint.name === draft.behaviors.timed_bool.input)}<option value={draft.behaviors.timed_bool.input}>{draft.behaviors.timed_bool.input} (invalid)</option>{/if}
              {#each compatibleEndpoints(draft, 'input', '1.001') as endpoint}<option value={endpoint.name}>{endpoint.name}</option>{/each}
            </select>
          </label>
          <label>Output
            <select value={draft.behaviors.timed_bool.output} on:change={(event) => updateBehavior('timed_bool', 'output', selectValue(event))}>
              {#if draft.behaviors.timed_bool.output && !compatibleEndpoints(draft, 'output', '1.001').some((endpoint) => endpoint.name === draft.behaviors.timed_bool.output)}<option value={draft.behaviors.timed_bool.output}>{draft.behaviors.timed_bool.output} (invalid)</option>{/if}
              {#each compatibleEndpoints(draft, 'output', '1.001') as endpoint}<option value={endpoint.name}>{endpoint.name}</option>{/each}
            </select>
          </label>
          <label>Off delay (ms)
            <input type="number" min="0" step="1" value={draft.behaviors.timed_bool.off_delay_ms} on:input={(event) => updateBehavior('timed_bool', 'off_delay_ms', inputValue(event))} />
          </label>
          {#if errorFor('behaviors.timed_bool.input')}<small class="field-error">{errorFor('behaviors.timed_bool.input')}</small>{/if}
          {#if errorFor('behaviors.timed_bool.output')}<small class="field-error">{errorFor('behaviors.timed_bool.output')}</small>{/if}
        </div>
        <div>
          <h3>Percentage forward</h3>
          <label>Input
            <select value={draft.behaviors.percentage_forward.input} on:change={(event) => updateBehavior('percentage_forward', 'input', selectValue(event))}>
              {#if draft.behaviors.percentage_forward.input && !compatibleEndpoints(draft, 'input', '5.001').some((endpoint) => endpoint.name === draft.behaviors.percentage_forward.input)}<option value={draft.behaviors.percentage_forward.input}>{draft.behaviors.percentage_forward.input} (invalid)</option>{/if}
              {#each compatibleEndpoints(draft, 'input', '5.001') as endpoint}<option value={endpoint.name}>{endpoint.name}</option>{/each}
            </select>
          </label>
          <label>Output
            <select value={draft.behaviors.percentage_forward.output} on:change={(event) => updateBehavior('percentage_forward', 'output', selectValue(event))}>
              {#if draft.behaviors.percentage_forward.output && !compatibleEndpoints(draft, 'output', '5.001').some((endpoint) => endpoint.name === draft.behaviors.percentage_forward.output)}<option value={draft.behaviors.percentage_forward.output}>{draft.behaviors.percentage_forward.output} (invalid)</option>{/if}
              {#each compatibleEndpoints(draft, 'output', '5.001') as endpoint}<option value={endpoint.name}>{endpoint.name}</option>{/each}
            </select>
          </label>
          {#if errorFor('behaviors.percentage_forward.input')}<small class="field-error">{errorFor('behaviors.percentage_forward.input')}</small>{/if}
          {#if errorFor('behaviors.percentage_forward.output')}<small class="field-error">{errorFor('behaviors.percentage_forward.output')}</small>{/if}
        </div>
      </div>

      {#if clientErrors.length}<p class="validation-summary" role="alert">Fix {clientErrors.length} validation {clientErrors.length === 1 ? 'error' : 'errors'} before saving.</p>{/if}
      <div class="editor-actions">
        <button type="button" class="save-button" disabled={clientErrors.length > 0 || automationSaving} on:click={saveDraft}>{automationSaving ? 'Saving…' : 'Save configuration'}</button>
        <span class="subtle">{clientErrors.length ? 'Save is disabled while the draft is invalid.' : 'The active runtime is unchanged until restart.'}</span>
      </div>
    {:else}
      <p class="empty">The saved automation document could not be loaded.</p>
      <button type="button" on:click={refreshAutomation}>Retry</button>
    {/if}
  </section>

  {#if snapshot}
    <section class="grid two-columns" aria-label="Configuration and current values">
      <article class="panel">
        <h2>Active endpoints</h2>
        {#if snapshot.automation}
          <div class="endpoint-list">
            {#each snapshot.automation.inputs as endpoint}
              <div><span class="endpoint-name">{endpoint.name}</span> <small>input / {endpoint.dpt}</small> <code>{endpoint.address || '—'}</code> <span class="value">{formatValue(endpoint.observed ?? null)}</span></div>
            {/each}
            {#each snapshot.automation.outputs as endpoint}
              <div><span class="endpoint-name">{endpoint.name}</span> <small>output / {endpoint.dpt}</small> <code>{endpoint.address || '—'}</code> <span class="value">{formatValue(endpoint.observed ?? null)}</span> <span class="value requested">requested {formatValue(endpoint.requested ?? null)}</span></div>
            {/each}
          </div>
          <dl class="facts compact-facts">
            <dt>Timed rule</dt><dd>{snapshot.automation.behaviors.timedBool.input} → {snapshot.automation.behaviors.timedBool.output} ({snapshot.automation.behaviors.timedBool.offDelayMs} ms)</dd>
            <dt>Percentage rule</dt><dd>{snapshot.automation.behaviors.percentageForward.input} → {snapshot.automation.behaviors.percentageForward.output}</dd>
          </dl>
        {:else}
          <dl class="facts">
            <dt>Input</dt><dd><code>{snapshot.config.input.address}</code> <small>{snapshot.config.input.dpt}</small></dd>
            <dt>Output</dt><dd><code>{snapshot.config.output.address}</code> <small>{snapshot.config.output.dpt}</small></dd>
            <dt>Off delay</dt><dd>{snapshot.config.offDelayMs} ms</dd>
          </dl>
        {/if}
        {#if snapshot.activeAutomationRevision !== null}<p class="subtle revision-line">Active automation revision {snapshot.activeAutomationRevision}</p>{/if}
      </article>

      <article class="panel">
        <h2>Values and writes</h2>
        <dl class="facts">
          <dt>{inputLabel} observed</dt><dd><span class="value">{formatValue(snapshot.values.input.observed)}</span></dd>
          <dt>{outputLabel} observed</dt><dd><span class="value">{formatValue(snapshot.values.output.observed)}</span></dd>
          <dt>{outputLabel} requested</dt><dd><span class="value requested">{formatValue(snapshot.values.output.requested)}</span></dd>
          <dt>Write status</dt><dd>{snapshot.write.status}{snapshot.write.error ? ` — ${snapshot.write.error}` : ''}</dd>
        </dl>
      </article>
    </section>

    <section class="panel timer-panel" aria-label="Timer state">
      <div>
        <h2>Off timer</h2>
        <p class="timer-state">{snapshot.timer.state}</p>
      </div>
      <div class="countdown" aria-live="polite">{formatCountdown(countdown)}</div>
      <div class="timer-detail">
        {#if snapshot.timer.state === 'pending'}
          {#if snapshot.timer.deadlineMs !== null}<span>deadline {snapshot.timer.deadlineMs} ms</span>{/if}
          {#if snapshot.timer.remainingMs !== null}<span>server remaining {snapshot.timer.remainingMs} ms</span>{/if}
        {:else}
          <span>No pending off request</span>
        {/if}
      </div>
    </section>

    <section class="panel" aria-label="Recent KNX telegrams">
      <div class="section-heading"><h2>Recent telegrams</h2><span>{snapshot.telegrams.length}</span></div>
      {#if snapshot.telegrams.length}
        <div class="table-wrap">
          <table>
            <thead><tr><th>Time</th><th>Source</th><th>Destination</th><th>Service</th><th>DPT</th><th>Value</th></tr></thead>
            <tbody>
              {#each snapshot.telegrams as telegram}
                <tr>
                  <td>{displayTime(telegram.time)}</td><td>{telegram.source ?? '—'}</td><td><code>{telegram.destination}</code></td>
                  <td>{telegram.service}</td><td>{telegram.dpt}</td><td>{formatValue(telegram.value)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else}
        <p class="empty">No telegrams observed yet.</p>
      {/if}
    </section>

    <section class="panel" aria-label="Recent runtime logs">
      <div class="section-heading"><h2>Recent logs</h2><span>{snapshot.logs.length}</span></div>
      {#if snapshot.logs.length}
        <div class="table-wrap">
          <table>
            <thead><tr><th>Time</th><th>Level</th><th>Target</th><th>Message</th><th>Fields</th></tr></thead>
            <tbody>
              {#each snapshot.logs as log}
                <tr>
                  <td>{displayTime(log.time)}</td><td><span class="level level-{log.level.toLowerCase()}">{log.level}</span></td>
                  <td>{log.target}</td><td>{log.message}</td><td class="fields">{displayFields(log.fields) || '—'}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else}
        <p class="empty">No runtime logs captured yet.</p>
      {/if}
    </section>
  {:else}
    <section class="panel loading" aria-live="polite">Loading runtime snapshot…</section>
  {/if}

  <footer>Snapshot revision {state.revision}</footer>
</main>
