<script lang="ts">
  import { onMount } from 'svelte';
  import { DashboardClient, loadSnapshot } from './lib/api';
  import { initialDashboardState, reduceDashboardState, type DashboardState } from './lib/state';
  import type { DisplayExecution } from './lib/dashboard-types';
  import ActivityView from './lib/ActivityView.svelte';
  import BlockWorkbench from './lib/BlockWorkbench.svelte';
  import KnxView from './lib/KnxView.svelte';
  import OverviewView from './lib/OverviewView.svelte';
  import SystemView from './lib/SystemView.svelte';

  type Page = 'overview' | 'blocks' | 'activity' | 'knx' | 'system';
  const pages: Array<{ id: Page; label: string }> = [
    { id: 'overview', label: 'Overview' },
    { id: 'blocks', label: 'Blocks' },
    { id: 'activity', label: 'Activity' },
    { id: 'knx', label: 'KNX' },
    { id: 'system', label: 'System' }
  ];

  let state: DashboardState = initialDashboardState;
  let dashboardClient: DashboardClient | null = null;
  let activePage: Page = 'overview';
  let blockPageMounted = false;
  $: snapshot = state.snapshot;
  $: blocks = snapshot?.blocks ?? [];
  $: pendingRestart = Boolean(snapshot && (snapshot.restartRequired || (snapshot.activeStructuralRevision !== null && snapshot.savedStructuralRevision !== null && String(snapshot.activeStructuralRevision) !== String(snapshot.savedStructuralRevision))));
  $: hostMeta = snapshot?.meta;

  const dispatch = (action: Parameters<typeof reduceDashboardState>[1]) => { state = reduceDashboardState(state, action); };

  function navigate(page: Page): void {
    activePage = page;
    if (page === 'blocks') blockPageMounted = true;
  }

  function openBlock(blockId: string): void {
    dashboardClient?.selectBlock(blockId);
    dispatch({ type: 'select_block', blockId });
    navigate('blocks');
  }

  function openExecution(execution: DisplayExecution): void {
    if (execution.blockId && blocks.some((block) => block.id === execution.blockId)) {
      dashboardClient?.selectBlock(execution.blockId);
      dispatch({ type: 'select_block', blockId: execution.blockId });
    }
    dispatch({ type: 'select_execution', executionId: execution.executionId });
    navigate('blocks');
  }

  function openExecutionId(executionId: number): void {
    dispatch({ type: 'select_execution', executionId });
    navigate('blocks');
  }

  async function refreshSnapshot(): Promise<void> {
    try {
      if (dashboardClient) { await dashboardClient.pollNow(); return; }
      dispatch({ type: 'snapshot_loaded', snapshot: await loadSnapshot(), nowMs: Date.now() });
    } catch (error) {
      dispatch({ type: 'stream_error', error: error instanceof Error ? error.message : String(error) });
    }
  }

  onMount(() => {
    const client = new DashboardClient({ eventSource: EventSource, handlers: {
      onSnapshot: (next) => dispatch({ type: 'snapshot_loaded', snapshot: next, nowMs: Date.now() }),
      onEvent: (event) => dispatch({ type: 'event_received', event }),
      onStreamOpen: () => dispatch({ type: 'stream_open' }),
      onStreamLost: (error) => dispatch({ type: 'stream_lost', error }),
      onError: (error) => dispatch({ type: 'stream_error', error: error.message })
    } });
    dashboardClient = client;
    void client.start();
    const ticker = window.setInterval(() => dispatch({ type: 'tick', nowMs: Date.now() }), 250);
    return () => { window.clearInterval(ticker); client.stop(); dashboardClient = null; };
  });
</script>

<main>
  <header class="page-header">
    <div class="brand-block">
      <p class="eyebrow">LogikSmith / {hostMeta?.host === 'embedded' ? 'Embedded device' : hostMeta?.host === 'desktop' ? 'Desktop runtime' : 'Automation runtime'}</p>
      <h1>LogikSmith</h1>
    </div>
    <div class="status-cluster" aria-label="Current runtime status">
      <span class="status-pill {snapshot?.connection.state === 'connected' ? 'good' : snapshot?.connection.state === 'failed' ? 'bad' : 'muted'}">KNX {snapshot?.connection.state ?? 'starting'}</span>
      {#if state.stale}<span class="stale">Live data stale</span>{:else}<span class="status-pill good">Live {state.streamStatus}</span>{/if}
      {#if hostMeta?.host === 'embedded'}<span class="status-pill {hostMeta.mutationLocked ? 'bad' : 'good'}">{hostMeta.mutationLocked ? 'programming mode required' : 'programming mode'}</span>{/if}
      {#if pendingRestart}<span class="status-pill bad">Structure {snapshot?.savedStructuralRevision ?? '—'} saved · restart required</span>{/if}
      {#if hostMeta?.overload}<span class="status-pill bad">management overloaded</span>{/if}
    </div>
  </header>

  <nav class="primary-nav" aria-label="Primary navigation">
    {#each pages as page}
      <button type="button" class:active={activePage === page.id} aria-current={activePage === page.id ? 'page' : undefined} on:click={() => navigate(page.id)}>{page.label}</button>
    {/each}
  </nav>

  {#if state.error}<div class="alert" role="alert">{state.error}</div>{/if}
  {#if snapshot}
    {#if hostMeta && (hostMeta.runtimeStatus !== 'ready' || hostMeta.storageStatus !== 'ready')}
      <div class="alert" role="status">
        {#if hostMeta.runtimeStatus !== 'ready'}Runtime {hostMeta.runtimeStatus}{hostMeta.runtimeReason ? ` · ${hostMeta.runtimeReason}` : ''}. {/if}
        {#if hostMeta.storageStatus !== 'ready'}Storage {hostMeta.storageStatus}{hostMeta.storageReason ? ` · ${hostMeta.storageReason}` : ''}.{/if}
      </div>
    {/if}

    <div hidden={activePage !== 'overview'}>
      <OverviewView {snapshot} stale={state.stale} on:selectBlock={(event) => openBlock(event.detail)} on:selectExecution={(event) => openExecution(event.detail)} />
    </div>
    {#if blockPageMounted}
      <div hidden={activePage !== 'blocks'}>
        <section class="page-intro"><p class="eyebrow">Blocks</p><h2>Runtime workbench</h2><p class="subtle">Select a block to inspect live values, executions, source, or a simulation.</p></section>
        <BlockWorkbench {blocks} selectedBlockId={state.selectedBlockId} selectedExecutionId={state.selectedExecutionId} {snapshot} stale={state.stale} staleAtMs={state.staleAtMs} nowMs={state.nowMs} structuralRevision={snapshot.activeStructuralRevision} restartRequired={pendingRestart} on:selectBlock={(event) => openBlock(event.detail)} on:selectExecution={(event) => openExecutionId(event.detail)} />
      </div>
    {/if}
    <div hidden={activePage !== 'activity'}>
      <ActivityView {snapshot} stale={state.stale} selectedBlockId={state.selectedBlockId} on:selectExecution={(event) => openExecution(event.detail)} />
    </div>
    <div hidden={activePage !== 'knx'}>
      <KnxView {snapshot} on:selectBlock={(event) => openBlock(event.detail)} on:selectExecution={(event) => openExecution(event.detail)} />
    </div>
    <div hidden={activePage !== 'system'}>
      <SystemView {snapshot} stale={state.stale} streamStatus={state.streamStatus} on:changed={() => void refreshSnapshot()} />
    </div>
  {:else}
    <section class="panel loading" aria-live="polite">Loading runtime snapshot…</section>
  {/if}
</main>
