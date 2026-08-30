<script lang="ts">
  import { formatDateTimeValue, formatStateEntry, formatValue } from './state';
  import type { DisplaySimulation } from './state';

  export let result: DisplaySimulation | null = null;
</script>

{#if result}
  <section class="panel simulation-capture" aria-label="Simulation capture"><div class="section-heading"><div><h2>Simulation capture</h2><p class="subtle">Frozen inputs, context, and proposed state from the last effect-free run.</p></div><span class="status-pill {result.status === 'succeeded' ? 'good' : 'bad'}">{result.status}</span></div>
    <div class="detail-columns"><div><h3>Captured time context (ctx)</h3>{#if result.timeContext}<dl class="facts"><dt>ctx.now</dt><dd>{formatDateTimeValue(result.timeContext.now)}</dd><dt>ctx.sun.dawn</dt><dd>{formatDateTimeValue(result.timeContext.sun.dawn)}</dd><dt>ctx.sun.sunrise</dt><dd>{formatDateTimeValue(result.timeContext.sun.sunrise)}</dd><dt>ctx.sun.sunset</dt><dd>{formatDateTimeValue(result.timeContext.sun.sunset)}</dd><dt>ctx.sun.dusk</dt><dd>{formatDateTimeValue(result.timeContext.sun.dusk)}</dd><dt>ctx.sun.elevation</dt><dd>{result.timeContext.sun.elevationDegrees === null ? '—' : `${result.timeContext.sun.elevationDegrees.toFixed(1)}°`}</dd><dt>ctx.sun.azimuth</dt><dd>{result.timeContext.sun.azimuthDegrees === null ? '—' : `${result.timeContext.sun.azimuthDegrees.toFixed(1)}°`}</dd></dl>{:else}<p class="empty">No time context was returned.</p>{/if}</div><div><h3>State transition</h3><dl class="facts"><dt>Before</dt><dd>{Object.keys(result.stateBefore).length ? Object.entries(result.stateBefore).map(([key, value]) => `${key}=${formatStateEntry(value)}`).join(', ') : '—'}</dd><dt>After</dt><dd>{Object.keys(result.stateAfter).length ? Object.entries(result.stateAfter).map(([key, value]) => `${key}=${formatStateEntry(value)}`).join(', ') : '—'}</dd><dt>Pending timers</dt><dd>{result.pendingTimers.length ? result.pendingTimers.map((timer) => `${timer.name} @ ${timer.dueAtMs} ms`).join(', ') : '—'}</dd></dl>{#if result.inputs.length}<h3>Input snapshot</h3><ul>{#each result.inputs as input}<li><code>{input.endpoint}</code> = {formatValue(input.value)} ({input.valid ? 'valid' : 'invalid'})</li>{/each}</ul>{/if}</div></div>
    {#if result.signalEffects.length || result.eligibleConsumers.length}<div class="simulation-effects"><h3>Proposed signal effects</h3>{#if result.signalEffects.length}<ul>{#each result.signalEffects as effect}<li><code>{effect.endpoint}</code> proposes <code>{effect.signal}</code> = {formatValue(effect.value)}{#if effect.changed === false} (unchanged){/if}</li>{/each}</ul>{:else}<p class="empty">No signal value proposed.</p>{/if}{#if result.eligibleConsumers.length}<h4>Eligible consumers (not executed)</h4><ul>{#each result.eligibleConsumers as consumer}<li><code>{consumer.blockId}.{consumer.endpoint}</code></li>{/each}</ul>{/if}<p class="subtle">Signal consumers are shown as eligible only; simulation does not propagate or execute them.</p></div>{/if}
  </section>
{/if}

<style>
  .simulation-capture { border-left: 3px solid var(--accent, #3578e5); }
  .simulation-capture .facts dd { overflow-wrap: anywhere; }
  .simulation-effects { border-top: 1px solid #e1e5e8; margin-top: 14px; padding-top: 12px; }
  .simulation-effects h3, .simulation-effects h4 { margin: 0 0 8px; font-size: 0.82rem; letter-spacing: 0.04em; text-transform: uppercase; }
  .simulation-effects h4 { margin-top: 14px; }
</style>
