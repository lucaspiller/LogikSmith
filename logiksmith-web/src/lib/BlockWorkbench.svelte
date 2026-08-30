<script lang="ts">
  import { createEventDispatcher, onDestroy } from 'svelte';
  import type { DisplayBlock, DisplayExecution, DisplaySimulation, DisplaySnapshot } from './dashboard-types';
  import type { RevisionToken } from './revision';
  import BlockNavigator from './BlockNavigator.svelte';
  import AuthorView from './AuthorView.svelte';
  import TestView from './TestView.svelte';
  import InspectView from './InspectView.svelte';
  import { fetchScheduleOccurrences } from './api';
  import type { DisplayScheduleOccurrence } from './dashboard-types';
  import { createSimulationDraft, type SimulationDraft } from './simulation';
  import { createBlockDraft, discardDraft, reconcileBlockDraft, sourceFingerprint, updateDraftSource, type BlockDraft } from './block-workbench';

  export let blocks: DisplayBlock[] = [];
  export let selectedBlockId: string | null = null;
  export let snapshot: DisplaySnapshot;
  export let stale = false;
  export let staleAtMs: number | null = null;
  export let nowMs = Date.now();
  export let structuralRevision: RevisionToken | null = null;
  export let restartRequired = false;

  const dispatch = createEventDispatcher<{ selectBlock: string; selectExecution: number }>();
  let mode: 'author' | 'test' | 'inspect' = 'author';
  let drafts = new Map<string, BlockDraft>();
  let simulations = new Map<string, SimulationDraft>();
  let results = new Map<string, DisplaySimulation | null>();
  let enabledOverrides = new Map<string, boolean>();
  let wasSelectedBlock = '';
  let quickScheduleName: string | null = null;
  let quickOccurrences: DisplayScheduleOccurrence[] = [];
  let quickScheduleError: string | null = null;
  let quickScheduleLoading = false;
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
  function useScenario(execution: DisplayExecution): void { if (!block) return; const scenario = createSimulationDraft({ ...snapshot, blocks: [block], pendingTimers: block.pendingTimers, executions: block.executions, state: block.state, automation: { inputs: block.inputs, outputs: block.outputs, bindings: block.bindings, signalBindings: block.signalBindings, source: block.source } }, execution); simulations = new Map(simulations).set(block.id, scenario); results = new Map(results).set(block.id, null); mode = 'test'; }
  async function selectQuickSchedule(name: string): Promise<void> {
    quickScheduleName = quickScheduleName === name ? null : name; quickOccurrences = []; quickScheduleError = null;
    if (!quickScheduleName || !block) return;
    quickScheduleLoading = true;
    try { const preview = await fetchScheduleOccurrences(block.id, quickScheduleName, { count: 3 }); if (quickScheduleName === preview.schedule) quickOccurrences = preview.occurrences; } catch (error) { quickScheduleError = error instanceof Error ? error.message : String(error); } finally { quickScheduleLoading = false; }
  }
  function dirtyDrafts(): boolean { return [...drafts.values()].some((draft) => draft.dirty); }
  function beforeUnload(event: BeforeUnloadEvent): void { if (!dirtyDrafts()) return; event.preventDefault(); event.returnValue = ''; }
  onDestroy(() => window.removeEventListener('beforeunload', beforeUnload));
  $: if (typeof window !== 'undefined') { window.removeEventListener('beforeunload', beforeUnload); if (dirtyDrafts()) window.addEventListener('beforeunload', beforeUnload); }
</script>

<BlockNavigator {blocks} {selectedBlockId} {drafts} {stale} on:select={(event) => dispatch('selectBlock', event.detail)} />
{#if block && block.schedules.length}<section class="panel quick-schedules" aria-label="Schedule quick inspection"><div class="section-heading"><div><h2>Schedules — {block.id}</h2><p class="subtle">Select a schedule to preview its next occurrences.</p></div><span>{block.schedules.length}</span></div><div class="quick-schedule-list">{#each block.schedules as schedule}<button type="button" class="small-button" aria-label={`Schedule ${schedule.name} detail`} class:selected={quickScheduleName === schedule.name} on:click={() => void selectQuickSchedule(schedule.name)}><code>{schedule.name}</code> · {schedule.ruleSummary}</button>{/each}</div>{#if quickScheduleName}<article class="schedule-detail"><h3>Schedule detail — {quickScheduleName}</h3>{#if quickScheduleLoading}<p class="subtle">Loading occurrence previews…</p>{/if}{#if quickScheduleError}<div class="alert" role="alert">{quickScheduleError}</div>{/if}{#if quickOccurrences.length}<h4>Next occurrences</h4><div class="table-wrap"><table><thead><tr><th>#</th><th>Local</th><th>UTC</th><th>Weekday</th><th>Offset</th></tr></thead><tbody>{#each quickOccurrences as occurrence, index}<tr><td>{index + 1}</td><td>{occurrence.local ?? '—'}</td><td>{new Date(occurrence.utcMs).toISOString()}</td><td>{occurrence.weekday ?? '—'}</td><td>{occurrence.utcOffsetSeconds ?? '—'}</td></tr>{/each}</tbody></table></div>{:else if !quickScheduleLoading && !quickScheduleError}<p class="empty">No future occurrences available.</p>{/if}</article>{/if}</section>{/if}
{#if effectiveBlock && currentDraft() && currentSimulation()}
  <section class="panel workbench-shell" aria-label="Selected block">
    <div class="workbench-tabs" role="tablist" aria-label="Block workflow views"><button type="button" role="tab" aria-selected={mode === 'author'} class:active={mode === 'author'} on:click={() => mode = 'author'}>Author</button><button type="button" role="tab" aria-selected={mode === 'test'} class:active={mode === 'test'} on:click={() => mode = 'test'}>Test</button><button type="button" role="tab" aria-selected={mode === 'inspect'} class:active={mode === 'inspect'} on:click={() => mode = 'inspect'}>Inspect</button></div>
    {#if mode === 'author'}<AuthorView block={effectiveBlock} draft={currentDraft()!} structuralRevision={structuralRevision} {stale} {restartRequired} on:sourcechange={(event) => updateSource(event.detail)} on:validation={(event) => applyValidation(event.detail)} on:activated={activated} on:enabled={enabled} on:discard={discard} on:selectExecution={(event) => dispatch('selectExecution', event.detail)} /><TestView block={effectiveBlock} source={currentDraft()!.source} sourceKey={sourceFingerprint(currentDraft()!.source)} simulationDraft={currentSimulation()!} result={results.get(effectiveBlock.id) ?? null} {stale} {structuralRevision} on:change={(event) => updateSimulation(event.detail)} on:result={(event) => updateResult(event.detail)} />{:else if mode === 'test'}<TestView block={effectiveBlock} source={currentDraft()!.source} sourceKey={sourceFingerprint(currentDraft()!.source)} simulationDraft={currentSimulation()!} result={results.get(effectiveBlock.id) ?? null} {stale} {structuralRevision} on:change={(event) => updateSimulation(event.detail)} on:result={(event) => updateResult(event.detail)} />{:else}<InspectView block={effectiveBlock} snapshot={snapshot} {stale} {staleAtMs} {nowMs} on:selectExecution={(event) => dispatch('selectExecution', event.detail)} on:useScenario={(event) => useScenario(event.detail)} />{/if}
  </section>
{/if}
