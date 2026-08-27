<script lang="ts">
  import { onMount } from 'svelte';
  import { DashboardClient } from './lib/api';
  import {
    countdownMs,
    formatCountdown,
    formatValue,
    initialDashboardState,
    reduceDashboardState,
    type DashboardState
  } from './lib/state';

  let state: DashboardState = initialDashboardState;
  $: snapshot = state.snapshot;
  $: countdown = countdownMs(snapshot, state.nowMs);
  $: connectionState = snapshot?.connection.state ?? 'starting';

  const dispatch = (action: Parameters<typeof reduceDashboardState>[1]) => {
    state = reduceDashboardState(state, action);
  };

  onMount(() => {
    const client = new DashboardClient({
      handlers: {
        onSnapshot: (next) => dispatch({ type: 'snapshot_loaded', snapshot: next, nowMs: Date.now() }),
        onEvent: (event) => dispatch({ type: 'event_received', event }),
        onStreamOpen: () => dispatch({ type: 'stream_open' }),
        onStreamLost: (error) => dispatch({ type: 'stream_lost', error }),
        onError: (error) => dispatch({ type: 'stream_error', error: error.message })
      }
    });
    void client.start();
    const interval = window.setInterval(() => dispatch({ type: 'tick', nowMs: Date.now() }), 100);
    return () => {
      client.stop();
      window.clearInterval(interval);
    };
  });

  function displayTime(value: string): string {
    const date = new Date(value);
    return Number.isNaN(date.valueOf()) ? value : date.toLocaleTimeString();
  }

  function displayFields(fields: Record<string, string | number | boolean | null>): string {
    return Object.entries(fields).map(([key, value]) => `${key}=${String(value)}`).join(' ');
  }
</script>

<svelte:head>
  <meta name="description" content="Live LogikSmith KNX runtime dashboard" />
</svelte:head>

<main>
  <header class="page-header">
    <div>
      <p class="eyebrow">LogikSmith / runtime</p>
      <h1>KNX dashboard</h1>
    </div>
    <div class="status-cluster" aria-live="polite">
      <span class:good={connectionState === 'connected'} class:bad={connectionState === 'failed'} class="status-pill">
        KNX {connectionState}
      </span>
      <span class:good={state.streamStatus === 'connected'} class="status-pill muted">
        Browser stream {state.streamStatus}
      </span>
      {#if state.stale}
        <span class="stale" title="The displayed snapshot is retained while the browser stream reconnects.">stale data</span>
      {/if}
    </div>
  </header>

  {#if state.error}
    <div class="alert" role="alert">{state.error}</div>
  {/if}

  {#if snapshot}
    <section class="grid two-columns" aria-label="Configuration and current values">
      <article class="panel">
        <h2>Configured POC</h2>
        <dl class="facts">
          <dt>Input</dt><dd><code>{snapshot.config.input.address}</code> <small>{snapshot.config.input.dpt}</small></dd>
          <dt>Output</dt><dd><code>{snapshot.config.output.address}</code> <small>{snapshot.config.output.dpt}</small></dd>
          <dt>Off delay</dt><dd>{snapshot.config.offDelayMs} ms</dd>
        </dl>
      </article>

      <article class="panel">
        <h2>Values and writes</h2>
        <dl class="facts">
          <dt>Input observed</dt><dd><span class="value">{formatValue(snapshot.values.input.observed)}</span></dd>
          <dt>Output observed</dt><dd><span class="value">{formatValue(snapshot.values.output.observed)}</span></dd>
          <dt>Output requested</dt><dd><span class="value requested">{formatValue(snapshot.values.output.requested)}</span></dd>
          <dt>Write status</dt><dd>{snapshot.write.status}{snapshot.write.error ? ` — ${snapshot.write.error}` : ''}</dd>
        </dl>
      </article>
    </section>

    <section class="panel timer-panel" aria-label="Timer state">
      <div>
        <h2>Off timer</h2>
        <p class="timer-state">{snapshot.timer.state}</p>
      </div>
      <div class="countdown" aria-live="polite">{formatCountdown(countdown)}</div>
      <div class="timer-detail">
        {#if snapshot.timer.state === 'pending'}
          {#if snapshot.timer.deadlineMs !== null}<span>deadline {snapshot.timer.deadlineMs} ms</span>{/if}
          {#if snapshot.timer.remainingMs !== null}<span>server remaining {snapshot.timer.remainingMs} ms</span>{/if}
        {:else}
          <span>No pending off request</span>
        {/if}
      </div>
    </section>

    <section class="panel" aria-label="Recent KNX telegrams">
      <div class="section-heading"><h2>Recent telegrams</h2><span>{snapshot.telegrams.length}</span></div>
      {#if snapshot.telegrams.length}
        <div class="table-wrap">
          <table>
            <thead><tr><th>Time</th><th>Source</th><th>Destination</th><th>Service</th><th>DPT</th><th>Value</th></tr></thead>
            <tbody>
              {#each snapshot.telegrams as telegram}
                <tr>
                  <td>{displayTime(telegram.time)}</td><td>{telegram.source ?? '—'}</td><td><code>{telegram.destination}</code></td>
                  <td>{telegram.service}</td><td>{telegram.dpt}</td><td>{formatValue(telegram.value)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else}
        <p class="empty">No telegrams observed yet.</p>
      {/if}
    </section>

    <section class="panel" aria-label="Recent runtime logs">
      <div class="section-heading"><h2>Recent logs</h2><span>{snapshot.logs.length}</span></div>
      {#if snapshot.logs.length}
        <div class="table-wrap">
          <table>
            <thead><tr><th>Time</th><th>Level</th><th>Target</th><th>Message</th><th>Fields</th></tr></thead>
            <tbody>
              {#each snapshot.logs as log}
                <tr>
                  <td>{displayTime(log.time)}</td><td><span class="level level-{log.level.toLowerCase()}">{log.level}</span></td>
                  <td>{log.target}</td><td>{log.message}</td><td class="fields">{displayFields(log.fields) || '—'}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else}
        <p class="empty">No runtime logs captured yet.</p>
      {/if}
    </section>
  {:else}
    <section class="panel loading" aria-live="polite">Loading runtime snapshot…</section>
  {/if}

  <footer>Snapshot revision {state.revision}</footer>
</main>
