<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import { fetchScheduleOccurrences } from './api';
  import ExecutionTraceView from './ExecutionTraceView.svelte';
  import type { DisplayBlock, DisplayExecution, DisplayScheduleOccurrence, DisplaySnapshot, DisplayStateValue } from './dashboard-types';
  import { formatAge, formatCountdown, formatDuration, formatExecutionTime, formatStateEntry, formatUtcDateTime, formatUtcOffset, formatValue, triggerLabel, triggerSummary } from './format';

  export let block: DisplayBlock;
  export let snapshot: DisplaySnapshot;
  export let selectedExecutionId: number | null = null;
  export let pinned = false;
  export let nowMs = Date.now();
  export let stale = false;
  export let staleAtMs: number | null = null;

  const dispatch = createEventDispatcher<{ selectExecution: number; useScenario: DisplayExecution }>();
  let triggerFilter = 'all';
  let statusFilter = 'all';
  let selectedScheduleName: string | null = null;
  let schedulePreviews: DisplayScheduleOccurrence[] = [];
  let scheduleLoading = false;
  let scheduleError: string | null = null;
  $: selectedExecution = block.executions.find((item) => item.executionId === selectedExecutionId)
    ?? snapshot.executions.find((item) => item.executionId === selectedExecutionId && item.blockId === block.id)
    ?? null;
  $: activeSourceRevision = block.activeRevision ?? block.activeLogicRevision;
  $: selectedSourceMatches = Boolean(selectedExecution && selectedExecution.logicRevision !== null && activeSourceRevision !== null && String(selectedExecution.logicRevision) === String(activeSourceRevision));
  $: selectedSourceNote = selectedExecution && !selectedSourceMatches ? `This execution used source revision ${selectedExecution.logicRevision ?? 'unknown'}; active source is revision ${activeSourceRevision ?? 'unknown'}. Line numbers remain available, while source text is shown only when revisions match.` : null;
  $: filteredExecutions = block.executions.filter((execution) => (triggerFilter === 'all' || execution.trigger.type === triggerFilter) && (statusFilter === 'all' || execution.status === statusFilter));
  $: selectedSchedule = block.schedules.find((item) => item.name === selectedScheduleName) ?? null;

  function selectExecution(execution: DisplayExecution): void {
    dispatch('selectExecution', execution.executionId);
  }
  function stateEntries(state: Record<string, DisplayStateValue>): Array<[string, DisplayStateValue]> { return Object.entries(state).sort(([left], [right]) => left.localeCompare(right)); }
  function stateDiff(before: Record<string, DisplayStateValue>, after: Record<string, DisplayStateValue>): Array<{ key: string; kind: 'added' | 'changed' | 'unchanged' | 'removed'; before?: DisplayStateValue; after?: DisplayStateValue }> {
    const keys = new Set([...Object.keys(before), ...Object.keys(after)]);
    return [...keys].sort().map((key) => {
      const previous = before[key]; const next = after[key];
      if (!previous) return { key, kind: 'added', after: next };
      if (!next) return { key, kind: 'removed', before: previous };
      return { key, kind: formatStateEntry(previous) === formatStateEntry(next) ? 'unchanged' : 'changed', before: previous, after: next };
    });
  }
  function countdown(timer: { dueAtMs: number }): string {
    const anchor = stale ? (staleAtMs ?? nowMs) : nowMs;
    return stale ? 'stale' : formatCountdown(Math.max(0, timer.dueAtMs - (anchor - snapshot.capturedAtMs - (snapshot.clockOffsetMs ?? 0))));
  }
  async function selectSchedule(name: string): Promise<void> {
    selectedScheduleName = selectedScheduleName === name ? null : name; schedulePreviews = []; scheduleError = null;
    if (!selectedScheduleName) return;
    scheduleLoading = true;
    try { const preview = await fetchScheduleOccurrences(block.id, selectedScheduleName, { count: 3 }); if (selectedScheduleName === preview.schedule) schedulePreviews = preview.occurrences; } catch (error) { scheduleError = error instanceof Error ? error.message : String(error); } finally { scheduleLoading = false; }
  }
</script>

<section class="panel workbench-view inspect-view" aria-label="Inspect view">
  <div class="section-heading"><div><p class="eyebrow">Live</p><h2>Runtime inspection — {block.id}</h2><p class="subtle">Endpoint values, transient state, timers, schedules, signal bindings, and recent executions.</p></div><span class="status-pill {stale ? 'bad' : 'good'}">{stale ? 'stale snapshot' : 'live snapshot'}</span></div>
  <h3>Endpoints and values</h3><div class="table-wrap"><table><thead><tr><th>Endpoint</th><th>Direction</th><th>DPT</th><th>Binding</th><th>Current</th><th>Requested</th><th>Age</th></tr></thead><tbody>{#each [...block.inputs.map((item) => ({ ...item, direction: 'input' as const })), ...block.outputs.map((item) => ({ ...item, direction: 'output' as const }))] as endpoint}<tr><td><code>{endpoint.name}</code></td><td>{endpoint.direction}</td><td>{endpoint.dpt}</td><td>{#if endpoint.bindingKind}<span>{endpoint.bindingKind.toUpperCase()} {endpoint.bindingKind}</span>{:else}unbound{/if}{#if endpoint.signal} · <code>{endpoint.signal}</code>{/if}{#if endpoint.source} · <code>{endpoint.source}</code>{/if}{#if endpoint.address} · <code>{endpoint.address}</code>{/if}</td><td><span class="value">{endpoint.observed === null || endpoint.observed === undefined ? 'unknown' : formatValue(endpoint.observed)}</span></td><td>{endpoint.direction === 'output' ? formatValue(endpoint.requested ?? null) : '—'}</td><td>{endpoint.observed === null || endpoint.observed === undefined ? '—' : 'sampled'}</td></tr>{/each}</tbody></table></div>
  <div class="runtime-projections"><article class="runtime-projection"><h3>Transient state</h3>{#if Object.keys(block.state).length}<div class="table-wrap"><table><thead><tr><th>Key</th><th>Value</th></tr></thead><tbody>{#each stateEntries(block.state) as [key, value]}<tr><td><code>{key}</code></td><td>{formatStateEntry(value)}</td></tr>{/each}</tbody></table></div>{:else}<p class="empty">No transient state keys.</p>{/if}</article><article class="runtime-projection"><h3>Pending timers</h3>{#if block.pendingTimers.length}<div class="table-wrap"><table><thead><tr><th>Name</th><th>Scheduled</th><th>Deadline</th><th>Remaining</th><th>Logic revision</th></tr></thead><tbody>{#each block.pendingTimers as timer}<tr><td><code>{timer.name}</code></td><td>{timer.scheduledAtMs} ms</td><td>{timer.dueAtMs} ms</td><td><span class:stale class="countdown">{countdown(timer)}</span></td><td>{timer.logicRevision}</td></tr>{/each}</tbody></table></div>{:else}<p class="empty">No pending timers.</p>{/if}<p class="subtle">Countdowns freeze when the stream is stale.</p></article></div>
  <h3>Schedules</h3>{#if block.schedules.length}<div class="table-wrap"><table><thead><tr><th>Schedule</th><th>Status</th><th>Rule</th><th>Next</th><th>Countdown</th><th>Offset</th><th>Last result</th></tr></thead><tbody>{#each block.schedules as schedule}<tr class:selected={selectedScheduleName === schedule.name} tabindex="0" role="button" aria-label={`Schedule ${schedule.name} detail`} on:click={() => selectSchedule(schedule.name)} on:keydown={(event) => { if (event.key === 'Enter' || event.key === ' ') void selectSchedule(schedule.name); }}><td><code>{schedule.name}</code>{#if !schedule.enabled}<span class="status-pill muted">disabled</span>{/if}</td><td><span class="status-pill {schedule.status === 'active' ? 'good' : schedule.status === 'paused' ? 'muted' : 'bad'}">{schedule.status}</span></td><td>{schedule.ruleSummary}</td><td>{schedule.nextOccurrenceLocal ?? '—'}</td><td><span class:stale class="countdown">{stale ? 'stale' : schedule.relativeMs === null ? '—' : formatCountdown(schedule.relativeMs)}</span></td><td>{formatUtcOffset(schedule.utcOffsetSeconds)}</td><td>{schedule.lastOutcome.status === 'none' ? '—' : schedule.lastOutcome.status}</td></tr>{/each}</tbody></table></div>{:else}<p class="empty">No schedules configured.</p>{/if}
  {#if selectedSchedule}<article class="schedule-detail"><h3>Schedule detail — {selectedSchedule.name}</h3><dl class="facts"><dt>Kind</dt><dd>{selectedSchedule.kind}</dd><dt>Rule</dt><dd>{selectedSchedule.ruleSummary}</dd><dt>Next (local)</dt><dd>{selectedSchedule.nextOccurrenceLocal ?? '—'}</dd><dt>Next (UTC)</dt><dd>{selectedSchedule.nextOccurrenceUtcMs === null ? '—' : formatUtcDateTime(selectedSchedule.nextOccurrenceUtcMs)}</dd></dl>{#if scheduleLoading}<p class="subtle">Loading occurrence previews…</p>{/if}{#if scheduleError}<div class="alert" role="alert">{scheduleError}</div>{/if}{#if schedulePreviews.length}<h4>Next occurrences</h4><div class="table-wrap"><table><thead><tr><th>#</th><th>Local</th><th>UTC</th><th>Weekday</th><th>Offset</th></tr></thead><tbody>{#each schedulePreviews as occurrence, index}<tr><td>{index + 1}</td><td>{occurrence.local ?? '—'}</td><td>{formatUtcDateTime(occurrence.utcMs)}</td><td>{occurrence.weekday ?? '—'}</td><td>{formatUtcOffset(occurrence.utcOffsetSeconds)}</td></tr>{/each}</tbody></table></div>{:else if !scheduleLoading && !scheduleError}<p class="empty">No future occurrences available.</p>{/if}</article>{/if}

  <div class="section-heading history-heading"><div><h3>Execution history</h3><p class="subtle">Bounded to the latest 50 records for this block. Selecting a row pins its detail.</p></div><span>{filteredExecutions.length} / {block.executions.length}</span></div><div class="filter-row"><label>Trigger<select aria-label="History trigger filter" bind:value={triggerFilter}><option value="all">all triggers</option><option value="input">input</option><option value="timer">timer</option><option value="schedule">schedule</option></select></label><label>Status<select aria-label="History status filter" bind:value={statusFilter}><option value="all">all results</option><option value="succeeded">succeeded</option><option value="failed">failed</option></select></label></div>{#if filteredExecutions.length}<div class="table-wrap"><table class="execution-history"><thead><tr><th>ID</th><th>Trigger</th><th>Time</th><th>Duration</th><th>Status</th><th>Outputs</th></tr></thead><tbody>{#each filteredExecutions as execution}<tr class:selected={execution.executionId === selectedExecutionId} class:pinned={execution.executionId === selectedExecutionId && pinned} tabindex="0" role="button" on:click={() => selectExecution(execution)} on:keydown={(event) => { if (event.key === 'Enter' || event.key === ' ') selectExecution(execution); }}><td>{execution.executionId}</td><td><span class="trigger-kind">{triggerLabel(execution.trigger)}</span></td><td>{formatExecutionTime(execution.timeMs, execution.timeContext)}</td><td>{formatDuration(execution.durationUs)}</td><td><span class="status-pill logic-{execution.status}">{execution.status}</span></td><td>{execution.effects.length}</td></tr>{/each}</tbody></table></div>{:else}<p class="empty">No executions match these filters.</p>{/if}
  {#if selectedExecution}
    <article class="execution-detail">
      <div class="section-heading">
        <div><h3>Execution {selectedExecution.executionId}</h3><span class="subtle">{pinned ? 'pinned selection' : 'following newest'}</span></div>
        <button type="button" class="small-button" on:click={() => dispatch('useScenario', selectedExecution)}>Simulate this execution</button>
      </div>
      <dl class="facts">
        <dt>Status</dt><dd>{selectedExecution.status}</dd>
        <dt>Trigger</dt><dd>{triggerSummary(selectedExecution.trigger)}</dd>
        <dt>Time</dt><dd>{formatExecutionTime(selectedExecution.timeMs, selectedExecution.timeContext)}</dd>
        <dt>Logic revision</dt><dd>{selectedExecution.logicRevision ?? '—'}</dd>
        <dt>Duration</dt><dd>{formatDuration(selectedExecution.durationUs)}</dd>
      </dl>
      {#if selectedExecution.error}<div class="logic-error" role="alert"><strong>{selectedExecution.error.category}</strong>{#if selectedExecution.error.line !== null} line {selectedExecution.error.line}:{/if} {selectedExecution.error.message}</div>{/if}

      <h4>State diff</h4>
      {#if stateDiff(selectedExecution.stateBefore, selectedExecution.stateAfter).length}
        <div class="table-wrap"><table><thead><tr><th>Key</th><th>Change</th><th>Before</th><th>After</th></tr></thead><tbody>
          {#each stateDiff(selectedExecution.stateBefore, selectedExecution.stateAfter) as change}
            <tr><td><code>{change.key}</code></td><td>{change.kind}</td><td>{change.before ? formatStateEntry(change.before) : '—'}</td><td>{change.after ? formatStateEntry(change.after) : '—'}</td></tr>
          {/each}
        </tbody></table></div>
      {:else}<p class="empty">No state changes.</p>{/if}

      <h4>Input snapshot</h4>
      <div class="table-wrap"><table><thead><tr><th>Input</th><th>DPT</th><th>Value</th><th>Age</th></tr></thead><tbody>
        {#each selectedExecution.inputs as input}
          <tr><td>{input.endpoint}</td><td>{input.dpt}</td><td>{formatValue(input.value)}</td><td>{formatAge(input.ageMs)}</td></tr>
        {/each}
      </tbody></table></div>

      <h4>Output effects</h4>
      {#if selectedExecution.effects.length}
        <ul>{#each selectedExecution.effects as effect}<li><code>{effect.endpoint}</code> = {formatValue(effect.value)} → <code>{effect.destination}</code></li>{/each}</ul>
      {:else}<p class="empty">No output effects.</p>{/if}
      {#if selectedExecution.timerEffects.length}<h4>Timer effects</h4><ul>{#each selectedExecution.timerEffects as effect}<li><code>{effect.name}</code>: {effect.action}</li>{/each}</ul>{/if}

      {#if selectedExecution.causalProducerExecutionId !== null || selectedExecution.causalLinks.length || selectedExecution.signalEffects.length}
        <section class="signal-path" aria-label="Signal path">
          <h4>Signal path</h4>
          {#if selectedExecution.causalProducerExecutionId !== null}
            <p>Triggered by <button type="button" class="small-button" on:click={() => dispatch('selectExecution', selectedExecution.causalProducerExecutionId!)}>execution {selectedExecution.causalProducerExecutionId}</button>{#if selectedExecution.causalSignal} via <code>{selectedExecution.causalSignal}</code>{/if}{#if selectedExecution.causalProducerBlockId} from block <code>{selectedExecution.causalProducerBlockId}</code>{/if}.</p>
          {/if}
          {#if selectedExecution.causalLinks.length}
            <ul>{#each selectedExecution.causalLinks as link}<li>Producer <button type="button" class="small-button" on:click={() => dispatch('selectExecution', link.producerExecutionId)}>execution {link.producerExecutionId}</button> → consumer execution {link.consumerExecutionId}{#if link.signal} via <code>{link.signal}</code>{/if}</li>{/each}</ul>
          {/if}
          {#if selectedExecution.signalEffects.length}
            <ul>{#each selectedExecution.signalEffects as effect}<li><code>{effect.endpoint}</code> publishes <code>{effect.signal}</code> = {formatValue(effect.value)}{#if effect.changed === false} (unchanged){/if}{#if effect.consumers?.length} · {effect.consumers.length} eligible consumer{effect.consumers.length === 1 ? '' : 's'}{/if}</li>{/each}</ul>
          {/if}
        </section>
      {/if}
    </article>
    <ExecutionTraceView source={selectedSourceMatches ? block.source : ''} trace={selectedExecution.trace} revisionNote={selectedSourceNote} />
  {/if}

</section>
