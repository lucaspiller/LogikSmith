<script lang="ts">
  import { onMount } from 'svelte';
  import { DashboardClient } from './lib/api';
  import { formatUtcOffset, formatValue, initialDashboardState, reduceDashboardState, type DashboardState, type DisplayExecutionOrigin } from './lib/state';
  import ExternalInputsSection from './lib/ExternalInputsSection.svelte';
  import SignalsSection from './lib/SignalsSection.svelte';
  import BlockWorkbench from './lib/BlockWorkbench.svelte';

  let state: DashboardState = initialDashboardState;
  $: snapshot = state.snapshot;
  $: blocks = snapshot?.blocks ?? [];
  $: selectedBlock = blocks.find((block) => block.id === state.selectedBlockId) ?? blocks[0] ?? null;
  $: selectedExecution = selectedBlock?.executions.find((execution) => execution.executionId === state.selectedExecutionId) ?? null;
  $: pendingRestart = Boolean(snapshot && (snapshot.restartRequired || (snapshot.activeStructuralRevision !== null && snapshot.savedStructuralRevision !== null && String(snapshot.activeStructuralRevision) !== String(snapshot.savedStructuralRevision))));

  const dispatch = (action: Parameters<typeof reduceDashboardState>[1]) => { state = reduceDashboardState(state, action); };
  function originSummary(origin: DisplayExecutionOrigin | null): string {
    if (!origin) return '—';
    if (origin.kind === 'http') return `HTTP poll ${origin.poll} / ${origin.value}`;
    if (origin.kind === 'webhook') return `Webhook ${origin.source}`;
    if (origin.kind === 'signal') return `Signal ${origin.signal}`;
    return origin.groupAddress ? `KNX ${origin.groupAddress}` : 'KNX';
  }
  function displayTime(value: string): string { const date = new Date(value); return Number.isNaN(date.valueOf()) ? value : date.toLocaleTimeString(); }
  function displayFields(fields: Record<string, string | number | boolean | null>): string { return Object.entries(fields).map(([key, value]) => `${key}=${String(value)}`).join(' '); }

  onMount(() => {
    const client = new DashboardClient({ handlers: {
      onSnapshot: (next) => dispatch({ type: 'snapshot_loaded', snapshot: next, nowMs: Date.now() }),
      onEvent: (event) => dispatch({ type: 'event_received', event }),
      onStreamOpen: () => dispatch({ type: 'stream_open' }),
      onStreamLost: (error) => dispatch({ type: 'stream_lost', error }),
      onError: (error) => dispatch({ type: 'stream_error', error: error.message })
    } });
    void client.start();
    const ticker = window.setInterval(() => dispatch({ type: 'tick', nowMs: Date.now() }), 250);
    return () => { window.clearInterval(ticker); client.stop(); };
  });
</script>

<main>
  <header class="page-header"><div><p class="eyebrow">LogikSmith / Milestone 12</p><h1>Automation dashboard</h1></div><div class="status-cluster"><span class="status-pill {snapshot?.connection.state === 'connected' ? 'good' : 'muted'}">KNX {snapshot?.connection.state ?? 'starting'}</span>{#if state.stale}<span class="stale">Stream stale — live mutations paused</span>{:else}<span class="status-pill good">Stream {state.streamStatus}</span>{/if}<span class="revision-badge">Snapshot {state.revision}</span><span class="revision-badge">Structural {snapshot?.activeStructuralRevision ?? '—'} / {snapshot?.savedStructuralRevision ?? '—'}</span>{#if pendingRestart}<span class="status-pill bad">restart required</span>{/if}</div></header>
  {#if state.error}<div class="alert" role="alert">{state.error}</div>{/if}
  {#if snapshot}
    <SignalsSection signals={snapshot.signals} />
    <ExternalInputsSection externalInputs={snapshot.externalInputs} />
    {#if selectedExecution}<section class="panel execution-origin-panel" aria-label="Selected execution origin"><div class="section-heading"><h2>Execution origin</h2><span class="trigger-kind">{originSummary(selectedExecution.origin)}</span></div><p class="subtle">Transport provenance is supplied by the host and does not alter the Lua input event.</p></section>{/if}
    <BlockWorkbench blocks={blocks} selectedBlockId={state.selectedBlockId} snapshot={snapshot} stale={state.stale} staleAtMs={state.staleAtMs} nowMs={state.nowMs} structuralRevision={snapshot.activeStructuralRevision} restartRequired={pendingRestart} on:selectBlock={(event) => dispatch({ type: 'select_block', blockId: event.detail })} on:selectExecution={(event) => dispatch({ type: 'select_execution', executionId: event.detail })} />
    <section class="grid two-columns" aria-label="Global diagnostics"><article class="panel"><h2>Site time</h2>{#if snapshot.siteTime}<dl class="facts"><dt>Zone</dt><dd><code>{snapshot.siteTime.timezone}</code></dd><dt>Local time at sample</dt><dd>{snapshot.siteTime.localTime ?? '—'} {#if !snapshot.siteTime.clockOk}<span class="status-pill bad">clock error</span>{/if}</dd><dt>UTC offset</dt><dd>{formatUtcOffset(snapshot.siteTime.utcOffsetSeconds)}</dd><dt>Coordinates</dt><dd>{snapshot.siteTime.coordinates ? `${snapshot.siteTime.coordinates.latitude.toFixed(4)}, ${snapshot.siteTime.coordinates.longitude.toFixed(4)}` : 'Astronomy unavailable'}</dd><dt>Astronomy</dt><dd>{snapshot.siteTime.astronomy}{snapshot.siteTime.astronomyReason ? ` — ${snapshot.siteTime.astronomyReason}` : ''}</dd><dt>Dawn</dt><dd>{snapshot.siteTime.sun.dawn ?? '—'}</dd><dt>Sunrise</dt><dd>{snapshot.siteTime.sun.sunrise ?? '—'}</dd><dt>Sunset</dt><dd>{snapshot.siteTime.sun.sunset ?? '—'}</dd><dt>Dusk</dt><dd>{snapshot.siteTime.sun.dusk ?? '—'}</dd></dl>{:else}<p class="empty">No site time information in this snapshot.</p>{/if}</article><article class="panel"><h2>Global connection and writes</h2><dl class="facts"><dt>Connection</dt><dd>{snapshot.connection.state}</dd><dt>Observed input</dt><dd>{formatValue(snapshot.values.input.observed)}</dd><dt>Observed output</dt><dd>{formatValue(snapshot.values.output.observed)}</dd><dt>Requested output</dt><dd>{formatValue(snapshot.values.output.requested)}</dd><dt>Write status</dt><dd>{snapshot.write.status}{snapshot.write.error ? ` — ${snapshot.write.error}` : ''}</dd></dl></article><article class="panel"><h2>Dashboard status</h2><dl class="facts"><dt>Snapshot event</dt><dd>{snapshot.revision}</dd><dt>Selected block</dt><dd>{selectedBlock?.id ?? '—'}</dd><dt>Active block revision</dt><dd>{selectedBlock?.activeRevision ?? selectedBlock?.activeLogicRevision ?? '—'}</dd><dt>Saved block revision</dt><dd>{selectedBlock?.savedRevision ?? selectedBlock?.savedLogicRevision ?? '—'}</dd></dl></article></section>
    <section class="panel" aria-label="Recent KNX telegrams"><div class="section-heading"><h2>Recent telegrams</h2><span>{snapshot.telegrams.length}</span></div>{#if snapshot.telegrams.length}<div class="table-wrap"><table><thead><tr><th>Time</th><th>Source</th><th>Destination</th><th>Service</th><th>DPT</th><th>Value</th></tr></thead><tbody>{#each snapshot.telegrams as telegram}<tr><td>{displayTime(telegram.time)}</td><td>{telegram.source ?? '—'}</td><td><code>{telegram.destination}</code></td><td>{telegram.service}</td><td>{telegram.dpt}</td><td>{formatValue(telegram.value)}</td></tr>{/each}</tbody></table></div>{:else}<p class="empty">No telegrams observed yet.</p>{/if}</section>
    <section class="panel" aria-label="Recent runtime logs"><div class="section-heading"><h2>Recent logs</h2><span>{snapshot.logs.length}</span></div>{#if snapshot.logs.length}<div class="table-wrap"><table><thead><tr><th>Time</th><th>Level</th><th>Target</th><th>Message</th><th>Fields</th></tr></thead><tbody>{#each snapshot.logs as log}<tr><td>{displayTime(log.time)}</td><td><span class="level level-{log.level.toLowerCase()}">{log.level}</span></td><td>{log.target}</td><td>{log.message}</td><td class="fields">{displayFields(log.fields) || '—'}</td></tr>{/each}</tbody></table></div>{:else}<p class="empty">No runtime logs captured yet.</p>{/if}</section>
  {:else}<section class="panel loading" aria-live="polite">Loading runtime snapshot…</section>{/if}
  <footer>Snapshot revision {state.revision}</footer>
</main>
