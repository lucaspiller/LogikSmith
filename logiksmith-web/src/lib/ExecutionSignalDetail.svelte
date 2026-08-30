<script lang="ts">
  import { formatValue } from './format';
  import type { DisplayExecution } from './state';

  export let execution: DisplayExecution | null = null;
</script>

{#if execution && (execution.signalEffects.length || execution.causalProducerExecutionId !== null || execution.causalLinks.length)}
  <article class="execution-signal-detail" aria-label={`Signal execution details for ${execution.executionId}`}>
    <h3>Signal path</h3>
    {#if execution.causalProducerExecutionId !== null}<p>Triggered by producer execution <code>{execution.causalProducerExecutionId}</code>{#if execution.causalSignal} through signal <code>{execution.causalSignal}</code>{/if}{#if execution.causalProducerBlockId} from block <code>{execution.causalProducerBlockId}</code>{/if}.</p>{/if}
    {#if execution.causalLinks.length}<ul>{#each execution.causalLinks as link}<li>Producer execution <code>{link.producerExecutionId}</code> → consumer execution <code>{link.consumerExecutionId}</code>{#if link.signal} via <code>{link.signal}</code>{/if}</li>{/each}</ul>{/if}
    {#if execution.signalEffects.length}<h4>Signal effects</h4><ul>{#each execution.signalEffects as effect}<li><code>{effect.endpoint}</code> publishes <code>{effect.signal}</code> = {formatValue(effect.value)}{#if effect.changed === false} (unchanged){/if}{#if effect.consumers?.length} · {effect.consumers.length} eligible consumer{effect.consumers.length === 1 ? '' : 's'}{/if}</li>{/each}</ul>{/if}
  </article>
{/if}

<style>
  .execution-signal-detail { border: 1px solid #c4ccd2; border-left: 3px solid #244b63; background: #fff; margin-bottom: 18px; padding: 14px 16px; }
  .execution-signal-detail h3, .execution-signal-detail h4 { margin: 0 0 8px; font-size: 0.82rem; letter-spacing: 0.04em; text-transform: uppercase; }
  .execution-signal-detail h4 { margin-top: 14px; }
</style>
