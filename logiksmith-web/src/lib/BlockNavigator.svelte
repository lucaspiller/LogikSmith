<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import type { DisplayBlock } from './dashboard-types';
  import type { BlockDraft } from './block-workbench';
  import { formatExecutionTime, triggerLabel } from './format';

  export let blocks: DisplayBlock[] = [];
  export let selectedBlockId: string | null = null;
  export let drafts: Map<string, BlockDraft> = new Map();
  export let stale = false;

  const dispatch = createEventDispatcher<{ select: string }>();
  let filter = '';
  let statusFilter: 'all' | 'failed' | 'disabled' | 'timers' = 'all';
  $: visibleBlocks = blocks.filter((block) => {
    const matchesText = !filter.trim() || block.id.toLowerCase().includes(filter.trim().toLowerCase());
    const matchesStatus = statusFilter === 'all'
      || (statusFilter === 'failed' && block.lastResult.status === 'failed')
      || (statusFilter === 'disabled' && !block.activeEnabled)
      || (statusFilter === 'timers' && block.pendingTimers.length > 0);
    return matchesText && matchesStatus;
  }).slice(0, 64);
</script>

<section class="panel block-workspace" aria-label="Logic blocks">
  <div class="section-heading"><div><h2>Logic blocks</h2><p class="subtle">Select a block to inspect, simulate, or view source · {blocks.length}/64 blocks</p></div><span class="revision-badge">{stale ? 'stale snapshot' : 'live'}</span></div>
  <div class="filter-row block-filter-row">
    {#if blocks.length > 8}<label>Search<input aria-label="Search blocks" type="search" bind:value={filter} placeholder="block id" /></label>{/if}
    {#if blocks.length > 1}<label>Show<select aria-label="Filter blocks by status" bind:value={statusFilter}><option value="all">all blocks</option><option value="failed">failed</option><option value="disabled">disabled</option><option value="timers">active timers</option></select></label>{/if}
    <span class="subtle">{visibleBlocks.length} / {blocks.length} blocks</span>
  </div>
  {#if !visibleBlocks.length}<p class="empty">No blocks match this filter.</p>{:else}<div class="block-list" role="list">{#each visibleBlocks as block}<button type="button" class:selected={block.id === selectedBlockId} class:failed={block.lastResult.status === 'failed'} class="block-row" on:click={() => dispatch('select', block.id)} aria-pressed={block.id === selectedBlockId}><span class="block-row-main"><strong>{block.id}</strong><span class="status-pill {block.activeEnabled ? 'good' : 'muted'}">{block.activeEnabled ? 'enabled' : 'disabled'}</span>{#if drafts.get(block.id)?.dirty}<span class="status-pill bad">draft</span>{/if}{#if drafts.get(block.id)?.conflict}<span class="status-pill bad">conflict</span>{/if}</span><span class="block-row-facts"><span>{block.lastResult.status === 'none' ? 'never run' : block.lastResult.status}</span>{#if block.executions[0]}<span>{triggerLabel(block.executions[0].trigger)} · {formatExecutionTime(block.executions[0].timeMs, block.executions[0].timeContext)}</span>{/if}<span>{block.pendingTimers.length} timer{block.pendingTimers.length === 1 ? '' : 's'}</span><span>rev {block.activeRevision ?? block.activeLogicRevision ?? '—'}</span></span></button>{/each}</div>{/if}
</section>
