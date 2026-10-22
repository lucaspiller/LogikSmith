<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import type { DisplayExecution, DisplayLog, DisplaySnapshot } from './dashboard-types';
  import { formatDuration, formatExecutionTime, triggerLabel } from './format';

  export let snapshot: DisplaySnapshot;
  export let stale = false;
  export let selectedBlockId: string | null = null;

  const dispatch = createEventDispatcher<{ selectExecution: DisplayExecution }>();
  let blockFilter = 'all';
  let triggerFilter = 'all';
  let resultFilter = 'all';
  let timeRange = 'all';
  let logBlockFilter = '';
  let logExecutionFilter = '';
  let logTextFilter = '';

  $: blockOptions = [...new Set(snapshot.blocks.map((block) => block.id))].sort();
  $: cutoff = timeRange === 'hour' ? snapshot.capturedAtMs - 60 * 60 * 1000 : timeRange === 'day' ? snapshot.capturedAtMs - 24 * 60 * 60 * 1000 : null;
  $: executions = snapshot.executions.filter((execution) => (blockFilter === 'all' || execution.blockId === blockFilter) && (triggerFilter === 'all' || execution.trigger.type === triggerFilter) && (resultFilter === 'all' || execution.status === resultFilter) && (cutoff === null || execution.timeMs >= cutoff));
  $: logs = snapshot.logs.filter((log) => (!logBlockFilter || logBlock(log).toLowerCase().includes(logBlockFilter.toLowerCase())) && (!logExecutionFilter || String(logExecutionId(log) ?? '').includes(logExecutionFilter)) && (!logTextFilter || `${log.target} ${log.message} ${Object.values(log.fields).join(' ')}`.toLowerCase().includes(logTextFilter.toLowerCase())));

  function selectExecution(execution: DisplayExecution): void {
    dispatch('selectExecution', execution);
  }

  function logExecutionId(log: DisplayLog): number | null {
    const value = log.fields.execution_id ?? log.fields.executionId;
    if (typeof value === 'number' && Number.isSafeInteger(value)) return value;
    if (typeof value === 'string' && /^\d+$/.test(value)) return Number(value);
    return null;
  }

  function logBlock(log: DisplayLog): string {
    const value = log.fields.block_id ?? log.fields.blockId;
    return value === null || value === undefined ? '' : String(value);
  }

  function executionForLog(log: DisplayLog): DisplayExecution | undefined {
    const executionId = logExecutionId(log);
    return executionId === null ? undefined : snapshot.executions.find((item) => item.executionId === executionId);
  }
</script>

<section class="panel activity-view" aria-label="Activity">
  <div class="section-heading">
    <div>
      <p class="eyebrow">Activity</p>
      <h2>Execution timeline</h2>
      <p class="subtle">Cross-block history, newest first. Select a run to inspect its inputs, source trace, and effects.</p>
    </div>
    <span class="status-pill {stale ? 'bad' : 'good'}">{stale ? 'stale snapshot' : 'live snapshot'}</span>
  </div>

  <div class="filter-row" aria-label="Activity filters">
    <label>Block<select aria-label="Filter activity by block" bind:value={blockFilter}><option value="all">all blocks</option>{#each blockOptions as block}<option value={block}>{block}</option>{/each}</select></label>
    <label>Trigger<select aria-label="Filter activity by trigger" bind:value={triggerFilter}><option value="all">all triggers</option><option value="input">input</option><option value="timer">timer</option><option value="schedule">schedule</option></select></label>
    <label>Result<select aria-label="Filter activity by result" bind:value={resultFilter}><option value="all">all results</option><option value="succeeded">succeeded</option><option value="failed">failed</option></select></label>
    <label>Time<select aria-label="Filter activity by time" bind:value={timeRange}><option value="all">all captured</option><option value="hour">last hour</option><option value="day">last 24 hours</option></select></label>
    <span class="subtle">{executions.length} / {snapshot.executions.length} executions</span>
  </div>

  {#if executions.length}
    <div class="table-wrap">
      <table class="execution-history">
        <thead><tr><th>Block</th><th>Trigger</th><th>Time</th><th>Duration</th><th>Result</th><th>Effects</th><th>Source revision</th></tr></thead>
        <tbody>
          {#each executions as execution}
            <tr tabindex="0" role="button" on:click={() => selectExecution(execution)} on:keydown={(event) => { if (event.key === 'Enter' || event.key === ' ') selectExecution(execution); }}>
              <td><code>{execution.blockId ?? '—'}</code></td>
              <td><span class="trigger-kind">{triggerLabel(execution.trigger)}</span></td>
              <td>{formatExecutionTime(execution.timeMs, execution.timeContext)}</td>
              <td>{formatDuration(execution.durationUs)}</td>
              <td><span class="status-pill logic-{execution.status}">{execution.status}</span></td>
              <td>{execution.effects.length + execution.signalEffects.length + execution.timerEffects.length}</td>
              <td>{execution.logicRevision ?? '—'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else}
    <p class="empty">No executions match these filters.</p>
  {/if}

  <section class="activity-logs" aria-labelledby="activity-logs-heading">
    <div class="section-heading"><div><h3 id="activity-logs-heading">Runtime logs</h3><p class="subtle">Logs with an execution id link back to that run.</p></div><span>{logs.length} / {snapshot.logs.length}</span></div>
    <div class="filter-row log-filters">
      <label>Block<input aria-label="Filter logs by block" bind:value={logBlockFilter} placeholder="all blocks" /></label>
      <label>Execution<input aria-label="Filter logs by execution" bind:value={logExecutionFilter} placeholder="all executions" /></label>
      <label>Text<input aria-label="Search logs" bind:value={logTextFilter} placeholder="target, message, or field" /></label>
      {#if selectedBlockId}<button type="button" class="small-button" on:click={() => logBlockFilter = selectedBlockId}>Only {selectedBlockId}</button>{/if}
      {#if logBlockFilter || logExecutionFilter || logTextFilter}<button type="button" class="small-button" on:click={() => { logBlockFilter = ''; logExecutionFilter = ''; logTextFilter = ''; }}>Clear log filters</button>{/if}
    </div>
    {#if logs.length}
      <div class="table-wrap logs-follow"><table><thead><tr><th>Time</th><th>Level</th><th>Block</th><th>Target</th><th>Message</th><th>Fields</th><th>Execution</th></tr></thead><tbody>
        {#each logs as log}
          {@const execution = executionForLog(log)}
          <tr><td>{log.time}</td><td><span class="level level-{log.level.toLowerCase()}">{log.level}</span></td><td>{logBlock(log) || '—'}</td><td>{log.target}</td><td class="fields">{log.message}</td><td class="fields">{Object.entries(log.fields).map(([key, value]) => `${key}=${String(value)}`).join(' ') || '—'}</td><td>{#if execution}<button type="button" class="small-button" on:click={() => selectExecution(execution)}>exec {execution.executionId}</button>{:else}<span class="subtle">—</span>{/if}</td></tr>
        {/each}
      </tbody></table></div>
    {:else}
      <p class="empty">No logs match these filters.</p>
    {/if}
  </section>
</section>
