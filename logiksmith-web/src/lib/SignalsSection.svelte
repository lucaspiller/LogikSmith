<script lang="ts">
  import { formatValue } from './format';
  import type { DisplaySignal, DisplaySignalConsumer } from './state';

  export let signals: DisplaySignal[] = [];
  let selectedSignalName: string | null = null;
  $: selectedSignal = signals.find((signal) => signal.name === selectedSignalName) ?? signals[0] ?? null;

  function statusClass(status: string): string {
    if (status === 'valid' || status === 'known') return 'good';
    if (status === 'producer_disabled' || status === 'invalid' || status === 'error') return 'bad';
    return 'muted';
  }
  function timestamp(value: number | null): string { return value === null ? '—' : `${value} ms`; }
  function producerLabel(signal: DisplaySignal): string {
    if (!signal.producer) return '—';
    return signal.producer.endpoint ? `${signal.producer.blockId}.${signal.producer.endpoint}` : signal.producer.blockId;
  }
  function consumerLabel(consumer: DisplaySignalConsumer): string {
    return consumer.endpoint ? `${consumer.blockId}.${consumer.endpoint}` : consumer.blockId;
  }
</script>

<section class="panel signals-panel" aria-label="Signals">
  <div class="section-heading"><div><h2>Signals</h2><p class="subtle">Typed internal values · live state is in-memory and resets to unknown on restart.</p></div><span>{signals.length}</span></div>
  {#if signals.length}
    <div class="table-wrap"><table class="signals-table"><thead><tr><th>Name</th><th>DPT</th><th>Current</th><th>Status</th><th>Producer</th><th>Consumers</th><th>Observed</th><th>Changed</th></tr></thead><tbody>
      {#each signals as signal}
        <tr class:selected={selectedSignal?.name === signal.name} tabindex="0" role="button" aria-label={`Signal ${signal.name} detail`} on:click={() => selectedSignalName = signal.name} on:keydown={(event) => { if (event.key === 'Enter' || event.key === ' ') selectedSignalName = signal.name; }}>
          <td><code>{signal.name}</code></td><td>{signal.dpt}</td><td><span class="value">{formatValue(signal.value)}</span></td><td><span class="status-pill {statusClass(signal.status)}">{signal.status}</span></td><td>{producerLabel(signal)}</td><td>{signal.consumers.length}</td><td>{timestamp(signal.observedAtMs)}</td><td>{timestamp(signal.changedAtMs)}</td>
        </tr>
      {/each}
    </tbody></table></div>
    {#if selectedSignal}
      <article class="signal-detail" aria-label={`Signal ${selectedSignal.name} detail`}>
        <div class="section-heading"><div><h3>{selectedSignal.name}</h3><p class="subtle">Signal detail</p></div><span class="revision-badge">Structure {selectedSignal.structuralRevision ?? '—'}</span></div>
        <div class="detail-columns"><div><dl class="facts"><dt>DPT</dt><dd>{selectedSignal.dpt}</dd><dt>Current observation</dt><dd>{formatValue(selectedSignal.value)}</dd><dt>Status</dt><dd><span class="status-pill {statusClass(selectedSignal.status)}">{selectedSignal.status}</span></dd><dt>Observed</dt><dd>{timestamp(selectedSignal.observedAtMs)}</dd><dt>Changed</dt><dd>{timestamp(selectedSignal.changedAtMs)}</dd><dt>Producing execution</dt><dd>{selectedSignal.producingExecutionId ?? selectedSignal.producer?.executionId ?? '—'}</dd></dl></div><div><h4>Producer</h4>{#if selectedSignal.producer}<p><code>{producerLabel(selectedSignal)}</code>{#if selectedSignal.producer.executionId !== null} · execution {selectedSignal.producer.executionId}{/if}</p>{:else}<p class="empty">No producer has published a value.</p>{/if}<h4>Consumers ({selectedSignal.consumers.length})</h4>{#if selectedSignal.consumers.length}<ul>{#each selectedSignal.consumers as consumer}<li><code>{consumerLabel(consumer)}</code></li>{/each}</ul>{:else}<p class="empty">No consumers configured.</p>{/if}</div></div>
        <h4>Recent changes</h4>{#if selectedSignal.recentChanges.length}<div class="table-wrap"><table><thead><tr><th>Value</th><th>Observed</th><th>Changed</th><th>Producing execution</th></tr></thead><tbody>{#each selectedSignal.recentChanges as change}<tr><td>{formatValue(change.value)}</td><td>{timestamp(change.observedAtMs)}</td><td>{timestamp(change.changedAtMs)}</td><td>{change.executionId ?? '—'}</td></tr>{/each}</tbody></table></div>{:else}<p class="empty">No recent signal changes.</p>{/if}
      </article>
    {/if}
  {:else}<p class="empty">No internal signals declared.</p>{/if}
</section>

<style>
  .signals-table tbody tr { cursor: pointer; }
  .signals-table tbody tr:hover, .signals-table tbody tr:focus { background: #f3f6f8; outline: none; }
  .signals-table tbody tr.selected { background: #e8f0f4; box-shadow: inset 3px 0 #244b63; }
  .signals-panel { border-top: 4px solid #244b63; }
  .signal-detail { border-top: 1px solid #e1e5e8; margin-top: 16px; padding-top: 14px; }
  .signal-detail h3, .signal-detail h4 { margin: 14px 0 8px; font-size: 0.82rem; letter-spacing: 0.04em; text-transform: uppercase; }
</style>
