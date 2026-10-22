<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import type { DisplayBlock, DisplayExecution, DisplaySnapshot } from './dashboard-types';
  import { formatDuration, formatExecutionTime, triggerLabel } from './format';

  export let snapshot: DisplaySnapshot;
  export let stale = false;

  const dispatch = createEventDispatcher<{ selectBlock: string; selectExecution: DisplayExecution }>();
  type Problem = { label: string; detail: string; blockId?: string };
  let blockFilter = '';

  $: blocks = snapshot.blocks;
  $: recentExecutions = snapshot.executions.slice(0, 12);
  $: visibleBlocks = blocks.filter((block) => !blockFilter.trim() || block.id.toLowerCase().includes(blockFilter.trim().toLowerCase())).slice(0, 32);
  $: problems = buildProblems(snapshot, stale);
  $: inputCount = blocks.reduce((count, block) => count + block.inputs.length, 0);
  $: outputCount = blocks.reduce((count, block) => count + block.outputs.length, 0);
  $: failedCount = blocks.filter((block) => block.lastResult.status === 'failed').length;

  function buildProblems(current: DisplaySnapshot, isStale: boolean): Problem[] {
    const result: Problem[] = [];
    if (isStale) result.push({ label: 'Live data is stale', detail: 'Reconnect before relying on current values.' });
    if (current.connection.state !== 'connected') result.push({ label: 'KNX is not connected', detail: current.connection.state });
    if (current.meta && current.meta.runtimeStatus !== 'ready') result.push({ label: `Runtime ${current.meta.runtimeStatus}`, detail: current.meta.runtimeReason ?? 'Runtime needs attention.' });
    if (current.meta && current.meta.storageStatus !== 'ready') result.push({ label: `Storage ${current.meta.storageStatus}`, detail: current.meta.storageReason ?? 'Storage needs attention.' });
    if (current.operations?.fatal) result.push({ label: 'Runtime reported a fatal condition', detail: current.operations.fatal });
    if (current.restartRequired) result.push({ label: 'Restart required', detail: 'Structural changes are waiting to take effect.' });
    current.blocks.forEach((block) => {
      if (!block.activeEnabled) result.push({ label: `${block.id} is disabled`, detail: 'Enable the block when it is ready to run.', blockId: block.id });
      if (block.lastResult.status === 'failed') result.push({ label: `${block.id} failed`, detail: block.lastError?.message ?? 'Open the latest execution for details.', blockId: block.id });
    });
    return result.slice(0, 12);
  }

  function blockFor(execution: DisplayExecution): DisplayBlock | undefined {
    return execution.blockId ? blocks.find((block) => block.id === execution.blockId) : undefined;
  }

  function selectExecution(execution: DisplayExecution): void {
    dispatch('selectExecution', execution);
  }
</script>

<section class="panel overview-view" aria-label="Overview">
  <div class="section-heading">
    <div>
      <p class="eyebrow">Overview</p>
      <h2>Runtime at a glance</h2>
      <p class="subtle">Health, recent work, and the blocks that need attention.</p>
    </div>
    <span class="status-pill {stale ? 'bad' : 'good'}">{stale ? 'stale snapshot' : 'live snapshot'}</span>
  </div>

  <div class="status-cluster" aria-label="Runtime status">
    <span class="status-pill {snapshot.connection.state === 'connected' ? 'good' : 'bad'}">KNX {snapshot.connection.state}</span>
    <span class="status-pill {snapshot.meta?.runtimeStatus === 'ready' || !snapshot.meta ? 'good' : 'bad'}">Runtime {snapshot.meta?.runtimeStatus ?? 'available'}</span>
    <span class="status-pill {snapshot.restartRequired ? 'bad' : 'good'}">{snapshot.restartRequired ? 'restart required' : 'configuration applied'}</span>
    <span class="revision-badge">{blocks.length} blocks · {snapshot.executions.length} executions · {snapshot.pendingTimers.length} timers</span>
  </div>
  <p class="overview-metrics">{blocks.length} blocks <span>·</span> {inputCount} inputs <span>·</span> {outputCount} outputs <span>·</span> {snapshot.pendingTimers.length} timers <span>·</span> <strong class:problem-count={failedCount > 0}>{failedCount} failed blocks</strong></p>

  <section aria-labelledby="overview-problems">
    <div class="section-heading"><h3 id="overview-problems">Problems</h3><span>{problems.length}</span></div>
    {#if problems.length}
      <div class="block-list">
        {#each problems as problem}
          {#if problem.blockId}
            <button type="button" class="block-row failed" on:click={() => dispatch('selectBlock', problem.blockId!)}>
              <span class="block-row-main"><strong>{problem.label}</strong><span class="subtle">{problem.detail}</span></span>
              <span class="status-pill bad">open block</span>
            </button>
          {:else}
            <div class="alert" role="status"><strong>{problem.label}</strong> · {problem.detail}</div>
          {/if}
        {/each}
      </div>
    {:else}
      <p class="empty">No actionable problems.</p>
    {/if}
  </section>

  <section aria-labelledby="overview-activity">
    <div class="section-heading"><h3 id="overview-activity">Recent executions</h3><span>{recentExecutions.length} latest</span></div>
    {#if recentExecutions.length}
      <div class="table-wrap">
        <table class="execution-history">
          <thead><tr><th>Block</th><th>Trigger</th><th>Time</th><th>Duration</th><th>Result</th><th>Effects</th></tr></thead>
          <tbody>
            {#each recentExecutions as execution}
              <tr tabindex="0" role="button" on:click={() => selectExecution(execution)} on:keydown={(event) => { if (event.key === 'Enter' || event.key === ' ') selectExecution(execution); }}>
                <td><code>{blockFor(execution)?.id ?? execution.blockId ?? '—'}</code></td>
                <td><span class="trigger-kind">{triggerLabel(execution.trigger)}</span></td>
                <td>{formatExecutionTime(execution.timeMs, execution.timeContext)}</td>
                <td>{formatDuration(execution.durationUs)}</td>
                <td><span class="status-pill logic-{execution.status}">{execution.status}</span></td>
                <td>{execution.effects.length + execution.signalEffects.length + execution.timerEffects.length}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <p class="empty">No executions captured yet.</p>
    {/if}
  </section>

  <section aria-labelledby="overview-blocks">
    <div class="section-heading"><div><h3 id="overview-blocks">Blocks</h3><p class="subtle">Select a block to inspect its runtime and source.</p></div><span>{visibleBlocks.length} / {blocks.length}</span></div>
    <label class="block-filter">Filter blocks<input aria-label="Filter overview blocks" type="search" bind:value={blockFilter} placeholder="block id" /></label>
    {#if visibleBlocks.length}
      <div class="block-list" role="list">
        {#each visibleBlocks as block}
          <button type="button" class="block-row" class:failed={block.lastResult.status === 'failed'} on:click={() => dispatch('selectBlock', block.id)}>
            <span class="block-row-main"><strong>{block.id}</strong><span class="status-pill {block.activeEnabled ? 'good' : 'muted'}">{block.activeEnabled ? 'enabled' : 'disabled'}</span></span>
            <span class="block-row-facts"><span>{block.lastResult.status === 'none' ? 'never run' : block.lastResult.status}</span>{#if block.executions[0]}<span>{triggerLabel(block.executions[0].trigger)} · {formatExecutionTime(block.executions[0].timeMs, block.executions[0].timeContext)}</span>{/if}<span>{block.pendingTimers.length} timer{block.pendingTimers.length === 1 ? '' : 's'}</span><span>rev {block.activeRevision ?? block.activeLogicRevision ?? '—'}</span></span>
          </button>
        {/each}
      </div>
    {:else}
      <p class="empty">No blocks match this filter.</p>
    {/if}
  </section>
</section>
