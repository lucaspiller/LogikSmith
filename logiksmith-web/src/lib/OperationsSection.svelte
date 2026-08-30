<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import { resumeBlock } from './block-api';
  import type { DisplayOperations, DisplaySnapshot } from './dashboard-types';

  export let snapshot: DisplaySnapshot;
  export let stale = false;

  const dispatch = createEventDispatcher<{ changed: void }>();
  let busy: string | null = null;
  let error: string | null = null;

  $: operations = snapshot.operations as DisplayOperations | undefined;

  async function resume(id: string): Promise<void> {
    if (!operations || stale || busy) return;
    busy = id;
    error = null;
    try {
      await resumeBlock({
        blockId: id,
        expectedRevision: snapshot.blocks.find((block) => block.id === id)?.activeRevision ?? '1',
        expectedStructuralRevision: snapshot.activeStructuralRevision
      });
      dispatch('changed');
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = null;
    }
  }

  function healthLabel(status: string): string {
    return status.replaceAll('_', ' ');
  }

  function usage(value: { used: number; capacity: number }): string {
    return `${value.used} / ${value.capacity}`;
  }
</script>

{#if operations}
  <section class="panel operations-panel" aria-label="Operations">
    <div class="section-heading"><div><h2>Operations</h2><p class="subtle">Profile <code>{operations.profile}</code> · bounded runtime health</p></div><span class="status-pill {operations.status === 'healthy' ? 'good' : 'bad'}">{operations.status}</span></div>
    {#if operations.fatal}<div class="alert" role="alert">Fatal: {operations.fatal}</div>{/if}
    {#if error}<div class="alert" role="alert">{error}</div>{/if}
    <div class="grid two-columns">
      <article><h3>Queues</h3><div class="table-wrap"><table><thead><tr><th>Lane</th><th>Depth / cap.</th><th>High water</th><th>Accepted</th><th>Rejected</th></tr></thead><tbody>{#each Object.entries(operations.queues) as [name, queue]}<tr><td><code>{name}</code></td><td>{queue.depth} / {queue.capacity}</td><td>{queue.highWater}</td><td>{queue.accepted}</td><td>{queue.rejected}</td></tr>{/each}</tbody></table></div></article>
      <article><h3>Timing and writes</h3><dl class="facts"><dt>Last host turn</dt><dd>{operations.hostTurn.lastDurationUs} μs</dd><dt>Maximum host turn</dt><dd>{operations.hostTurn.maxDurationUs} μs</dd><dt>Over budget / warnings</dt><dd>{operations.hostTurn.overBudgetCount} / {operations.hostTurn.warningCount}</dd><dt>Pending KNX writes</dt><dd>{operations.pendingKnxWrites} / {operations.pendingKnxWriteCapacity}</dd><dt>Write timeouts</dt><dd>{operations.pendingWriteTimeouts}</dd></dl></article>
    </div>
    <article><h3>Core usage</h3><dl class="facts"><dt>Logic blocks</dt><dd>{usage(operations.core.logicBlocks)}</dd><dt>Signals</dt><dd>{usage(operations.core.signals)}</dd><dt>Signal bindings</dt><dd>{usage(operations.core.signalBindings)}</dd><dt>Logic source bytes</dt><dd>{usage(operations.core.logicSourceBytes)}</dd><dt>State entries / bytes</dt><dd>{usage(operations.core.stateEntries)} / {usage(operations.core.stateBytes)}</dd><dt>Pending timers</dt><dd>{usage(operations.core.pendingTimers)}</dd></dl></article>
    <article><div class="section-heading"><h3>Block health</h3><span>{Object.keys(operations.blockHealth).length}</span></div><div class="table-wrap"><table><thead><tr><th>Block</th><th>Health</th><th>Failures</th><th>Executions / s</th><th>Action</th></tr></thead><tbody>{#each Object.entries(operations.blockHealth) as [id, health]}<tr><td><code>{id}</code></td><td><span class="status-pill {health.status === 'active' || health.status === 'disabled' ? 'good' : 'bad'}">{healthLabel(health.status)}</span></td><td>{health.consecutiveFailures}</td><td>{health.liveExecutionsLastSecond}</td><td>{#if health.status.startsWith('suspended_')}<button class="small-button" type="button" disabled={stale || busy !== null} on:click={() => void resume(id)}>{busy === id ? 'Resuming…' : 'Resume'}</button>{:else}—{/if}</td></tr>{/each}</tbody></table></div></article>
  </section>
{/if}
