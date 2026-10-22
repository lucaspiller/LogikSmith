<script lang="ts">
  import type { DisplayExecutionTrace, DisplayTraceValue } from './dashboard-types';

  export let source = '';
  export let trace: DisplayExecutionTrace;
  export let title = 'Source trace';
  export let revisionNote: string | null = null;

  $: sourceLines = source ? source.split(/\r?\n/) : [];

  function valueSummary(value: DisplayTraceValue): string {
    if (value.kind === 'table' && value.value && typeof value.value === 'object') {
      const entries = Object.entries(value.value as Record<string, DisplayTraceValue>);
      const summary = entries.map(([key, child]) => `${key}: ${valueSummary(child)}`).join(', ');
      return `{${summary}${value.truncated ? `${summary ? ', ' : ''}…` : ''}}`;
    }
    if (value.value === undefined) return value.reason ? `<${value.kind}: ${value.reason}>` : `<${value.kind}>`;
    return String(value.value);
  }
</script>

<section class="execution-trace" aria-label="Source trace">
  <div class="section-heading">
    <div><h4>{title}</h4><p class="subtle">Lines and locals captured while this handler ran.</p></div>
    {#if trace.truncated}<span class="status-pill muted">trace truncated</span>{/if}
  </div>
  {#if revisionNote && trace.entries.length}<p class="selection-notice">{revisionNote}</p>{/if}
  {#if trace.entries.length}
    <div class="trace-entries">
      {#each trace.entries as entry, index}
        <article class="trace-entry">
          <header><strong>Line {entry.line}</strong>{#if source}<code>{sourceLines[entry.line - 1] ?? 'source line unavailable'}</code>{/if}<span>step {index + 1}</span></header>
          <div class="table-wrap"><table><thead><tr><th>Variable</th><th>Value</th><th>Type</th></tr></thead><tbody>
            {#each Object.entries(entry.locals) as [name, value]}
              <tr><td><code>{name}</code></td><td class="trace-value">{valueSummary(value)}</td><td><span class="trigger-kind">{value.kind}</span>{#if value.truncated}<span class="subtle"> · nested values truncated</span>{/if}</td></tr>
            {/each}
          </tbody></table></div>
        </article>
      {/each}
    </div>
  {:else}
    <p class="empty">No line values were captured for this execution.</p>
  {/if}
</section>
