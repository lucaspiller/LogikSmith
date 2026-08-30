<script lang="ts">
  import { createEventDispatcher, onDestroy } from 'svelte';
  import { activateBlockSource, BlockApiError, setBlockEnabled, validateBlockSource } from './block-api';
  import { sourceFingerprint, type BlockDraft, type DraftValidation } from './block-workbench';
  import type { DisplayBlock } from './dashboard-types';
  import LuaEditor from './LuaEditor.svelte';
  import ExecutionSignalDetail from './ExecutionSignalDetail.svelte';
  import { formatValue, triggerLabel } from './format';

  export let block: DisplayBlock;
  export let draft: BlockDraft;
  export let structuralRevision: string | number | null = null;
  export let stale = false;
  export let restartRequired = false;

  const dispatch = createEventDispatcher<{
    sourcechange: string;
    validation: DraftValidation;
    activated: { revision: string | number; cancelledTimers: string[] };
    enabled: { enabled: boolean; revision: string | number; cancelledTimers: string[] };
    discard: void;
    selectExecution: number;
  }>();
  let validationBusy = false;
  let operationBusy = false;
  let operationError: string | null = null;
  let operationNotice: string | null = null;
  let requestSequence = 0;
  let validationTimer: ReturnType<typeof setTimeout> | null = null;

  $: fingerprint = sourceFingerprint(draft.source);
  $: byteCount = new TextEncoder().encode(draft.source).byteLength;
  $: validationCurrent = draft.validation?.sourceFingerprint === fingerprint;
  $: valid = validationCurrent && draft.validation?.status === 'valid' && draft.validation.errors.length === 0;
  $: canActivate = draft.dirty && valid && !draft.conflict && !stale && !restartRequired && !validationBusy && !operationBusy;

  function beginValidation(source = draft.source): void {
    if (validationTimer !== null) clearTimeout(validationTimer);
    const sequence = ++requestSequence;
    const expectedRevision = draft.baseActiveRevision ?? block.activeRevision ?? block.activeLogicRevision;
    if (expectedRevision === null) return;
    validationTimer = setTimeout(async () => {
      validationBusy = true;
      try {
        const result = await validateBlockSource({ blockId: block.id, source, expectedRevision, expectedStructuralRevision: draft.baseStructuralRevision ?? structuralRevision });
        if (sequence === requestSequence && result.sourceFingerprint === sourceFingerprint(source)) dispatch('validation', result);
      } catch (error) {
        if (sequence !== requestSequence) return;
        operationError = error instanceof Error ? error.message : String(error);
      } finally {
        if (sequence === requestSequence) validationBusy = false;
      }
    }, 450);
  }

  function handleSource(source: string): void {
    operationError = null;
    operationNotice = null;
    dispatch('sourcechange', source);
    beginValidation(source);
  }

  function discard(): void {
    if (!draft.dirty || window.confirm('Discard the unsaved draft and restore the latest active source?')) dispatch('discard');
  }

  async function activate(): Promise<void> {
    if (!canActivate) return;
    const cancelled = block.pendingTimers.map((timer) => timer.name);
    if (cancelled.length && !window.confirm(`Activate ${block.id}? This cancels ${cancelled.length} pending timer(s) and creates no synthetic execution.`)) return;
    operationBusy = true; operationError = null; operationNotice = null;
    try {
      const expectedRevision = draft.baseActiveRevision ?? block.activeRevision ?? block.activeLogicRevision;
      if (expectedRevision === null) throw new Error('The active block revision is unavailable. Refresh the dashboard.');
      const result = await activateBlockSource({ blockId: block.id, source: draft.source, expectedRevision, expectedStructuralRevision: draft.baseStructuralRevision ?? structuralRevision });
      operationNotice = result.cancelledTimers.length ? `Activated. Cancelled timers: ${result.cancelledTimers.join(', ')}.` : 'Activated. The next real event will use this source.';
      dispatch('activated', { revision: result.activeRevision, cancelledTimers: result.cancelledTimers });
    } catch (error) {
      operationError = error instanceof BlockApiError && error.status === 409 ? `${error.message} The draft is preserved for review.` : error instanceof Error ? error.message : String(error);
    } finally { operationBusy = false; }
  }

  async function toggleEnabled(): Promise<void> {
    if (stale || restartRequired || operationBusy || draft.conflict) return;
    const enabled = !block.activeEnabled;
    const cancelled = !enabled ? block.pendingTimers.map((timer) => timer.name) : [];
    const recentOutput = block.executions.some((execution) => execution.effects.length > 0);
    if (!enabled && (cancelled.length || recentOutput) && !window.confirm(`Disable ${block.id}? ${cancelled.length ? `${cancelled.length} pending timer(s) will be cancelled. ` : ''}${recentOutput ? 'Recent output activity is present. ' : ''}No cleanup output is sent.`)) return;
    operationBusy = true; operationError = null; operationNotice = null;
    try {
      const expectedRevision = draft.baseActiveRevision ?? block.activeRevision ?? block.activeLogicRevision;
      if (expectedRevision === null) throw new Error('The active block revision is unavailable. Refresh the dashboard.');
      const result = await setBlockEnabled({ blockId: block.id, enabled, expectedRevision, expectedStructuralRevision: draft.baseStructuralRevision ?? structuralRevision });
      operationNotice = enabled ? 'Block enabled. No synthetic execution was created.' : result.cancelledTimers.length ? `Block disabled. Cancelled timers: ${result.cancelledTimers.join(', ')}.` : 'Block disabled. No cleanup output was sent.';
      dispatch('enabled', { enabled, revision: result.activeRevision, cancelledTimers: result.cancelledTimers });
    } catch (error) {
      operationError = error instanceof BlockApiError && error.status === 409 ? `${error.message} Refresh before changing enablement.` : error instanceof Error ? error.message : String(error);
    } finally { operationBusy = false; }
  }

  function forceValidate(): void { if (!validationBusy) beginValidation(draft.source); }
  function handleSave(event: CustomEvent<void>): void { event.preventDefault(); forceValidate(); }
  onDestroy(() => { if (validationTimer !== null) clearTimeout(validationTimer); });
</script>

<section class="panel workbench-view author-view" aria-label="Author view">
  <div class="section-heading"><div><p class="eyebrow">Author</p><h2>Source — {block.id}</h2></div><span class="revision-badge">Active {block.activeRevision ?? block.activeLogicRevision ?? '—'}</span></div>
  <div class="author-toolbar" aria-live="polite">
    <span class="status-pill {draft.dirty ? 'bad' : 'good'}">{draft.dirty ? 'unsaved draft' : 'clean'}</span>
    {#if draft.conflict}<span class="status-pill bad">conflict · active revision {draft.conflictRevision ?? 'changed'}</span>{/if}
    {#if validationBusy}<span class="subtle">Validating…</span>{:else if validationCurrent}<span class="status-pill {valid ? 'good' : 'bad'}">{valid ? 'valid source' : 'invalid source'}</span>{:else}<span class="subtle">Edit to validate</span>{/if}
  </div>
  {#if stale}<div class="restart-notice" role="status">Live stream is stale. Inspection remains available, but activation and enablement are paused until a fresh snapshot arrives.</div>{/if}
  {#if restartRequired}<div class="restart-notice" role="status">Structural restart is required. Source activation and enablement are unavailable until the active topology catches up.</div>{/if}
  {#if operationError}<div class="alert" role="alert">{operationError}</div>{/if}
  {#if operationNotice}<div class="success-notice" role="status">{operationNotice}</div>{/if}
  <div class="source-editor">
    <div class="section-heading"><label class="editor-label" for="lua-source-host">Lua source</label><span class:over-limit={byteCount > 65536} class="source-size">{byteCount.toLocaleString()} / 65,536 bytes</span></div>
    <LuaEditor label={`Lua source for ${block.id}`} source={draft.source} diagnostics={validationCurrent ? (draft.validation?.errors ?? []) : []} disabled={operationBusy} on:change={(event) => handleSource(event.detail)} on:save={handleSave} />
  </div>
  {#if validationCurrent && draft.validation && draft.validation.errors.length}
    <div class="validation-summary" role="alert" aria-live="assertive"><strong>Source validation</strong><ol>{#each draft.validation.errors as error}<li>{#if error.line !== null}<code>line {error.line}</code> {/if}<strong>{error.category}</strong>: {error.message}</li>{/each}</ol></div>
  {/if}
  <div class="editor-actions">
    <button type="button" class="small-button" disabled={!draft.dirty || operationBusy} on:click={discard}>Revert draft</button>
    <button type="button" class="small-button" disabled={validationBusy || operationBusy} on:click={forceValidate}>Validate now</button>
    <button type="button" class="save-button" disabled={!canActivate} on:click={activate}>{operationBusy ? 'Working…' : 'Activate source'}</button>
    <button type="button" class="small-button" disabled={stale || restartRequired || operationBusy || draft.conflict} on:click={toggleEnabled}>{block.activeEnabled ? 'Disable block' : 'Enable block'}</button>
  </div>
  <p class="subtle keyboard-note">Cmd/Ctrl+S runs effect-free validation. Activation always requires the labelled button.</p>
  <div class="author-details">
    <article><h3>Supported handler</h3><pre class="source-example">function handle(event, input, meta, state, ctx)
    return &#123;
        state = &#123;&#125;,
        outputs = &#123;&#125;,
        timers = &#123;&#125;,
    &#125;
end</pre></article>
    <article><h3>Selected block contract</h3><dl class="reference-grid"><dt>Inputs</dt><dd>{block.inputs.map((item) => `${item.name} (${item.dpt})`).join(', ') || 'none'}</dd><dt>Outputs</dt><dd>{block.outputs.map((item) => `${item.name} (${item.dpt})`).join(', ') || 'none'}</dd><dt>State</dt><dd>bool, integer, number, or string scalar keys</dd><dt>Time</dt><dd><code>ctx.now</code> and <code>ctx.sun</code> values when available</dd><dt>Effects</dt><dd>Outputs and named timer operations are proposed atomically</dd></dl></article>
  </div>
  <section class="author-endpoints"><h3>Endpoints and bindings</h3><div class="table-wrap"><table><thead><tr><th>Endpoint</th><th>Direction</th><th>DPT</th><th>Binding</th><th>Current</th></tr></thead><tbody>{#each [...block.inputs.map((item) => ({ ...item, direction: 'input' as const })), ...block.outputs.map((item) => ({ ...item, direction: 'output' as const }))] as endpoint}<tr><td><code>{endpoint.name}</code></td><td>{endpoint.direction}</td><td>{endpoint.dpt}</td><td><span class="status-pill {endpoint.bindingKind === 'signal' || endpoint.bindingKind === 'knx' || endpoint.bindingKind === 'http' || endpoint.bindingKind === 'webhook' ? 'good' : 'muted'}">{endpoint.bindingKind ?? 'unbound'}</span>{#if endpoint.signal} · <code>{endpoint.signal}</code>{/if}{#if endpoint.source} · <code>{endpoint.source}</code>{/if}{#if endpoint.address} · <code>{endpoint.address}</code>{/if}</td><td>{formatValue(endpoint.observed ?? null)}</td></tr>{/each}</tbody></table></div></section>
  {#if block.executions.length}<section class="author-history"><h3>Recent executions</h3><div class="table-wrap"><table class="execution-history"><thead><tr><th>ID</th><th>Trigger</th><th>Time</th><th>Status</th></tr></thead><tbody>{#each block.executions as execution}<tr tabindex="0" role="button" on:click={() => dispatch('selectExecution', execution.executionId)} on:keydown={(event) => { if (event.key === 'Enter' || event.key === ' ') dispatch('selectExecution', execution.executionId); }}><td>{execution.executionId}</td><td>{triggerLabel(execution.trigger)}</td><td>{execution.timeMs} ms</td><td>{execution.status}</td></tr>{/each}</tbody></table></div></section><ExecutionSignalDetail execution={block.executions[0]} />{/if}
</section>
