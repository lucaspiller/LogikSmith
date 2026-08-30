<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import type { DisplayBlock } from './dashboard-types';
  import type { BlockDraft } from './block-workbench';

  export let blocks: DisplayBlock[] = [];
  export let selectedBlockId: string | null = null;
  export let drafts: Map<string, BlockDraft> = new Map();
  export let stale = false;

  const dispatch = createEventDispatcher<{ select: string }>();
  let filter = '';
  $: visibleBlocks = blocks.filter((block) => !filter.trim() || block.id.toLowerCase().includes(filter.trim().toLowerCase())).slice(0, 64);
</script>

<section class="panel block-workspace" aria-label="Logic blocks">
  <div class="section-heading"><div><h2>Logic blocks</h2><p class="subtle">Select a block to author, test, and inspect · {blocks.length}/64 blocks</p></div><span class="revision-badge">{stale ? 'stale snapshot' : 'live'}</span></div>
  {#if blocks.length > 8}<label class="block-filter">Filter blocks<input aria-label="Filter blocks" type="search" bind:value={filter} placeholder="block id" /></label>{/if}
  {#if !visibleBlocks.length}<p class="empty">No blocks match this filter.</p>{:else}<div class="block-list" role="list">{#each visibleBlocks as block}<button type="button" class:selected={block.id === selectedBlockId} class:failed={block.lastResult.status === 'failed'} class="block-row" on:click={() => dispatch('select', block.id)} aria-pressed={block.id === selectedBlockId}><span class="block-row-main"><strong>{block.id}</strong><span class="status-pill {block.activeEnabled ? 'good' : 'muted'}">{block.activeEnabled ? 'enabled' : 'disabled'}</span>{#if drafts.get(block.id)?.dirty}<span class="status-pill bad">draft</span>{/if}{#if drafts.get(block.id)?.conflict}<span class="status-pill bad">conflict</span>{/if}</span><span class="block-row-facts"><span>{block.lastResult.status === 'none' ? 'never run' : block.lastResult.status}</span><span>{block.pendingTimers.length} timer{block.pendingTimers.length === 1 ? '' : 's'}</span><span>rev {block.activeRevision ?? block.activeLogicRevision ?? '—'}</span></span></button>{/each}</div>{/if}
</section>
