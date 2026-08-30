<script lang="ts">
  import { formatAge, formatValue } from './format';
  import type { DisplayExternalHealth, DisplayExternalInputs, DisplayHttpPoll, DisplayWebhookInput } from './dashboard-types';

  export let externalInputs: DisplayExternalInputs;

  function healthClass(status: DisplayExternalHealth): string {
    return status === 'healthy' ? 'good' : status === 'failing' || status === 'stale' ? 'bad' : 'muted';
  }
  function timestamp(value: number | null): string {
    if (value === null) return '—';
    const date = new Date(value);
    return Number.isNaN(date.valueOf()) ? `${value} ms` : date.toLocaleString();
  }
  function nextAttempt(poll: DisplayHttpPoll): string {
    if (poll.nextAttemptAtMs === null) return '—';
    return timestamp(poll.nextAttemptAtMs);
  }
  function consumerList(consumers: { blockId: string; endpoint: string }[]): string {
    return consumers.length ? consumers.map((consumer) => `${consumer.blockId}.${consumer.endpoint}`).join(', ') : '—';
  }
</script>

<section class="panel external-inputs" aria-label="External inputs">
  <div class="section-heading">
    <div><h2>External inputs</h2><p class="subtle">Host-managed HTTP polls and webhook deliveries.</p></div>
    <span>{externalInputs.httpPolls.length + externalInputs.webhooks.length} source{externalInputs.httpPolls.length + externalInputs.webhooks.length === 1 ? '' : 's'}</span>
  </div>

  {#if !externalInputs.httpPolls.length && !externalInputs.webhooks.length}
    <p class="empty">No HTTP polls or webhooks configured.</p>
  {:else}
    <div class="external-source-list">
      {#each externalInputs.httpPolls as poll}
        <article class="external-source" aria-label={`HTTP poll ${poll.name}`}>
          <div class="section-heading"><div><h3><span class="external-kind">HTTP</span> <code>{poll.name}</code></h3><p class="subtle" title={poll.url}>{poll.url}</p></div><span class={`status-pill ${healthClass(poll.status)}`}>{poll.status}</span></div>
          <dl class="facts external-facts">
            <dt>Interval</dt><dd>{formatAge(poll.intervalMs)}</dd>
            <dt>Last attempt</dt><dd>{timestamp(poll.lastAttemptAtMs)}</dd>
            <dt>Last success</dt><dd>{timestamp(poll.lastSuccessAtMs)}</dd>
            <dt>Next attempt</dt><dd>{nextAttempt(poll)}</dd>
            <dt>Failures</dt><dd>{poll.consecutiveFailures}</dd>
            {#if poll.lastError}<dt>Last error</dt><dd class="external-error">{poll.lastError}</dd>{/if}
          </dl>
          {#if poll.values.length}
            <div class="table-wrap"><table class="external-values"><thead><tr><th>Value</th><th>DPT</th><th>Pointer</th><th>Current</th><th>Age</th><th>Consumers</th></tr></thead><tbody>
              {#each poll.values as value}
                <tr><td><code>{value.name}</code></td><td>{value.dpt}</td><td><code>{value.jsonPointer || '/'}</code></td><td><span class="value">{value.valid ? formatValue(value.value) : 'unknown'}</span></td><td>{formatAge(value.ageMs)}</td><td>{consumerList(value.consumers)}</td></tr>
              {/each}
            </tbody></table></div>
          {:else}<p class="empty">No extracted values.</p>{/if}
        </article>
      {/each}

      {#each externalInputs.webhooks as webhook}
        <article class="external-source" aria-label={`Webhook ${webhook.name}`}>
          <div class="section-heading"><div><h3><span class="external-kind">Webhook</span> <code>{webhook.name}</code></h3><p class="subtle"><code>{webhook.route}</code></p></div><span class={`status-pill ${healthClass(webhook.status)}`}>{webhook.status}</span></div>
          <dl class="facts external-facts">
            <dt>DPT</dt><dd>{webhook.dpt}</dd>
            <dt>JSON pointer</dt><dd><code>{webhook.jsonPointer || '/'}</code></dd>
            <dt>Authentication</dt><dd>{webhook.authenticationRequired ? webhook.authenticationConfigured ? 'bearer token configured' : 'required, not configured' : 'not required'}</dd>
            <dt>Last accepted</dt><dd>{timestamp(webhook.lastAcceptedAtMs)}</dd>
            <dt>Accepted / rejected</dt><dd>{webhook.acceptedCount} / {webhook.rejectedCount}</dd>
            <dt>Current</dt><dd><span class="value">{webhook.valid ? formatValue(webhook.value) : 'unknown'}</span> <span class="subtle">({formatAge(webhook.ageMs)})</span></dd>
            <dt>Consumers</dt><dd>{consumerList(webhook.consumers)}</dd>
          </dl>
        </article>
      {/each}
    </div>
  {/if}
</section>
