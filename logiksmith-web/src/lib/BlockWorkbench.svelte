<script lang="ts">
  import { createEventDispatcher, onDestroy } from 'svelte';
  import type { DisplayBlock, DisplayExecution, DisplaySimulation, DisplaySnapshot } from './dashboard-types';
  import type { RevisionToken } from './revision';
  import BlockNavigator from './BlockNavigator.svelte';
  import AuthorView from './AuthorView.svelte';
  import TestView from './TestView.svelte';
  import InspectView from './InspectView.svelte';
  import { createSimulationDraft, type SimulationDraft } from './simulation';
  import { createBlockDraft, discardDraft, reconcileBlockDraft, sourceFingerprint, updateDraftSource, type BlockDraft } from './block-workbench';

  export let blocks: DisplayBlock[] = [];
  export let selectedBlockId: string | null = null;
  export let selectedExecutionId: number | null = null;
  export let snapshot: DisplaySnapshot;
  export let stale = false;
  export let staleAtMs: number | null = null;
  export let nowMs = Date.now();
  export let structuralRevision: RevisionToken | null = null;
  export let restartRequired = false;

  const dispatch = createEventDispatcher<{ selectBlock: string; selectExecution: number }>();
  let mode: 'source' | 'simulate' | 'live' = 'live';
  let drafts = new Map<string, BlockDraft>();
  let simulations = new Map<string, SimulationDraft>();
  let results = new Map<string, DisplaySimulation | null>();
  let enabledOverrides = new Map<string, boolean>();
  let wasSelectedBlock = '';
  $: block = blocks.find((item) => item.id === selectedBlockId) ?? blocks[0] ?? null;
  $: effectiveBlock = block ? { ...block, activeEnabled: enabledOverrides.get(block.id) ?? block.activeEnabled } : null;

  function syncDraft(currentBlock: DisplayBlock | null): void {
    if (!currentBlock) return;
    const previous = drafts.get(currentBlock.id);
    const next = reconcileBlockDraft(previous, currentBlock, structuralRevision);
    if (!previous || previous.source !== next.source || previous.baseActiveRevision !== next.baseActiveRevision || previous.baseStructuralRevision !== next.baseStructuralRevision || previous.conflict !== next.conflict || previous.validation !== next.validation) drafts = new Map(drafts).set(currentBlock.id, next);
    if (!simulations.has(currentBlock.id) || wasSelectedBlock !== currentBlock.id) {
      simulations = new Map(simulations).set(currentBlock.id, createSimulationDraft({ ...snapshot, blocks: [currentBlock], pendingTimers: currentBlock.pendingTimers, executions: currentBlock.executions, state: currentBlock.state, automation: { inputs: currentBlock.inputs, outputs: currentBlock.outputs, bindings: currentBlock.bindings, signalBindings: currentBlock.signalBindings, source: currentBlock.source } }, null));
      results = new Map(results).set(currentBlock.id, null);
    }
    wasSelectedBlock = currentBlock.id;
  }
  $: syncDraft(block);

  function currentDraft(): BlockDraft | null { return block ? drafts.get(block.id) ?? null : null; }
  function currentSimulation(): SimulationDraft | null { return block ? simulations.get(block.id) ?? null : null; }
  function updateSource(source: string): void { if (!block) return; const prior = currentDraft() ?? createBlockDraft(block, structuralRevision); drafts = new Map(drafts).set(block.id, updateDraftSource(prior, source)); }
  function applyValidation(validation: BlockDraft['validation']): void { if (!block || !validation) return; const prior = currentDraft(); if (!prior || validation.sourceFingerprint !== sourceFingerprint(prior.source)) return; drafts = new Map(drafts).set(block.id, { ...prior, validation }); }
  function discard(): void { if (!block) return; drafts = new Map(drafts).set(block.id, discardDraft(block, structuralRevision)); }
  function activated(event: CustomEvent<{ revision: RevisionToken; cancelledTimers: string[] }>): void {
    if (!block) return;
    const prior = currentDraft(); if (!prior) return;
    drafts = new Map(drafts).set(block.id, { ...prior, dirty: false, conflict: false, conflictRevision: null, baseActiveRevision: event.detail.revision, baseStructuralRevision: structuralRevision });
  }
  function enabled(event: CustomEvent<{ enabled: boolean; revision: RevisionToken; cancelledTimers: string[] }>): void { if (!block) return; enabledOverrides = new Map(enabledOverrides).set(block.id, event.detail.enabled); const prior = currentDraft(); if (prior) drafts = new Map(drafts).set(block.id, { ...prior, baseActiveRevision: event.detail.revision, conflict: false, conflictRevision: null }); }
  function updateSimulation(next: SimulationDraft): void { if (!block) return; simulations = new Map(simulations).set(block.id, next); results = new Map(results).set(block.id, null); }
  function updateResult(result: DisplaySimulation): void { if (!block) return; results = new Map(results).set(block.id, result); }
  function useScenario(execution: DisplayExecution): void { if (!block) return; const scenario = createSimulationDraft({ ...snapshot, blocks: [block], pendingTimers: block.pendingTimers, executions: block.executions, state: block.state, automation: { inputs: block.inputs, outputs: block.outputs, bindings: block.bindings, signalBindings: block.signalBindings, source: block.source } }, execution); simulations = new Map(simulations).set(block.id, scenario); results = new Map(results).set(block.id, null); mode = 'simulate'; }
  function dirtyDrafts(): boolean { return [...drafts.values()].some((draft) => draft.dirty); }
  function beforeUnload(event: BeforeUnloadEvent): void { if (!dirtyDrafts()) return; event.preventDefault(); event.returnValue = ''; }
  onDestroy(() => window.removeEventListener('beforeunload', beforeUnload));
  $: if (typeof window !== 'undefined') { window.removeEventListener('beforeunload', beforeUnload); if (dirtyDrafts()) window.addEventListener('beforeunload', beforeUnload); }
</script>

<BlockNavigator {blocks} {selectedBlockId} {drafts} {stale} on:select={(event) => dispatch('selectBlock', event.detail)} />
{#if effectiveBlock && currentDraft() && currentSimulation()}
  <section class="panel workbench-shell" aria-label="Selected block">
    <div class="workbench-tabs" role="tablist" aria-label="Block workflow views"><button type="button" role="tab" aria-selected={mode === 'live'} class:active={mode === 'live'} on:click={() => mode = 'live'}>Live</button><button type="button" role="tab" aria-selected={mode === 'source'} class:active={mode === 'source'} on:click={() => mode = 'source'}>Source</button><button type="button" role="tab" aria-selected={mode === 'simulate'} class:active={mode === 'simulate'} on:click={() => mode = 'simulate'}>Simulate</button></div>
    {#if mode === 'source'}<AuthorView block={effectiveBlock} draft={currentDraft()!} structuralRevision={structuralRevision} {stale} {restartRequired} on:sourcechange={(event) => updateSource(event.detail)} on:validation={(event) => applyValidation(event.detail)} on:activated={activated} on:enabled={enabled} on:discard={discard} />{:else if mode === 'simulate'}<TestView block={effectiveBlock} source={currentDraft()!.source} sourceKey={sourceFingerprint(currentDraft()!.source)} simulationDraft={currentSimulation()!} result={results.get(effectiveBlock.id) ?? null} {stale} {structuralRevision} on:change={(event) => updateSimulation(event.detail)} on:result={(event) => updateResult(event.detail)} />{:else}<InspectView block={effectiveBlock} snapshot={snapshot} selectedExecutionId={selectedExecutionId} pinned={selectedExecutionId !== effectiveBlock.executions[0]?.executionId} {stale} {staleAtMs} {nowMs} on:selectExecution={(event) => dispatch('selectExecution', event.detail)} on:useScenario={(event) => useScenario(event.detail)} />{/if}
  </section>
{/if}
