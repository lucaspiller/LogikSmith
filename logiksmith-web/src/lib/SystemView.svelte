<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import ExternalInputsSection from './ExternalInputsSection.svelte';
  import OperationsSection from './OperationsSection.svelte';
  import SignalsSection from './SignalsSection.svelte';
  import type { DisplaySnapshot, StreamStatus } from './dashboard-types';
  import { formatUtcOffset, formatValue } from './format';

  export let snapshot: DisplaySnapshot;
  export let stale = false;
  export let streamStatus: StreamStatus = 'connecting';

  const dispatch = createEventDispatcher<{ changed: void }>();

  $: host = snapshot.meta;
  $: unavailableCapabilities = host ? Object.entries(host.capabilities).filter(([, available]) => !available).map(([name]) => name.replaceAll('_', ' ')) : [];
</script>

<section class="panel system-view" aria-label="System">
  <div class="section-heading">
    <div>
      <p class="eyebrow">System</p>
      <h2>Host and runtime</h2>
      <p class="subtle">Readiness, service health, host limits, and site clock.</p>
    </div>
    <span class="status-pill {stale ? 'bad' : 'good'}">{stale ? 'stale data' : 'management ' + streamStatus}</span>
  </div>

  <div class="system-summary">
    <article>
      <h3>Host</h3>
      <dl class="facts">
        <dt>Kind</dt><dd>{host?.host ?? 'unknown'}</dd>
        {#if host}<dt>Programming mode</dt><dd>{host.programmingMode ? 'enabled' : 'disabled'}</dd><dt>Mutations</dt><dd>{host.mutationLocked ? 'locked' : 'available'}</dd>{/if}
        <dt>Runtime</dt><dd><span class="status-pill {host?.runtimeStatus === 'ready' || !host ? 'good' : 'bad'}">{host?.runtimeStatus ?? 'available'}</span>{#if host?.runtimeReason} · {host.runtimeReason}{/if}</dd>
        <dt>Storage</dt><dd><span class="status-pill {host?.storageStatus === 'ready' || !host ? 'good' : 'bad'}">{host?.storageStatus ?? 'available'}</span>{#if host?.storageReason} · {host.storageReason}{/if}</dd>
        {#if host}<dt>Host overload</dt><dd>{host.overload ? 'yes' : 'no'}</dd>{/if}
      </dl>
    </article>
    <article>
      <h3>Site clock</h3>
      {#if snapshot.siteTime}
        <dl class="facts">
          <dt>Timezone</dt><dd><code>{snapshot.siteTime.timezone}</code></dd>
          <dt>Local time</dt><dd>{snapshot.siteTime.localTime ?? 'unavailable'}</dd>
          <dt>UTC offset</dt><dd>{formatUtcOffset(snapshot.siteTime.utcOffsetSeconds)}</dd>
          <dt>Clock</dt><dd>{snapshot.siteTime.clockOk ? 'synchronised' : 'error'}</dd>
          <dt>Scheduler</dt><dd>{snapshot.siteTime.schedulerOk ? 'ready' : 'unavailable'}</dd>
          <dt>Coordinates</dt><dd>{snapshot.siteTime.coordinates ? `${snapshot.siteTime.coordinates.latitude.toFixed(4)}, ${snapshot.siteTime.coordinates.longitude.toFixed(4)}` : 'not configured'}</dd>
          <dt>Astronomy</dt><dd>{snapshot.siteTime.astronomy}{snapshot.siteTime.astronomyReason ? ` · ${snapshot.siteTime.astronomyReason}` : ''}</dd>
          <dt>Sun today</dt><dd>dawn {snapshot.siteTime.sun.dawn ?? '—'} · sunrise {snapshot.siteTime.sun.sunrise ?? '—'} · sunset {snapshot.siteTime.sun.sunset ?? '—'} · dusk {snapshot.siteTime.sun.dusk ?? '—'}</dd>
        </dl>
      {:else}
        <p class="empty">No site clock information is available.</p>
      {/if}
    </article>
    <article>
      <h3>Compatibility</h3>
      <dl class="facts">
        <dt>Active / saved structure</dt><dd>{snapshot.activeStructuralRevision ?? '—'} / {snapshot.savedStructuralRevision ?? '—'}</dd>
        <dt>Restart required</dt><dd>{snapshot.restartRequired ? 'yes' : 'no'}</dd>
        <dt>Observed input</dt><dd>{formatValue(snapshot.values.input.observed)}</dd>
        <dt>Observed output</dt><dd>{formatValue(snapshot.values.output.observed)}</dd>
        <dt>Requested output</dt><dd>{formatValue(snapshot.values.output.requested)}</dd>
        <dt>Write result</dt><dd>{snapshot.write.status}{snapshot.write.error ? ` · ${snapshot.write.error}` : ''}</dd>
      </dl>
      {#if unavailableCapabilities.length}
        <p class="subtle">Unavailable here: {unavailableCapabilities.join(', ')}.</p>
      {:else if !host}
        <p class="subtle">Detailed host capabilities aren't included in this snapshot.</p>
      {/if}
    </article>
  </div>
</section>

<OperationsSection {snapshot} {stale} on:changed={() => dispatch('changed')} />
<SignalsSection signals={snapshot.signals} />
<ExternalInputsSection externalInputs={snapshot.externalInputs} />
