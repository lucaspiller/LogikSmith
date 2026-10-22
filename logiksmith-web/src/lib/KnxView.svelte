<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import { formatExecutionTime, formatValue } from './format';
  import type { DisplayBlock, DisplayEndpoint, DisplayExecution, DisplaySnapshot } from './dashboard-types';

  export let snapshot: DisplaySnapshot;

  const dispatch = createEventDispatcher<{ selectBlock: string; selectExecution: DisplayExecution }>();
  let filter = '';

  type BindingRow = {
    groupAddress: string;
    blockId: string | null;
    endpoint: string;
    direction: DisplayEndpoint['direction'] | null;
    dpt: string;
  };
  type ExecutionReference = { execution: DisplayExecution; blockId: string | null };

  function connectionClass(state: DisplaySnapshot['connection']['state']): string {
    if (state === 'connected') return 'good';
    if (state === 'disconnected' || state === 'failed') return 'bad';
    return 'muted';
  }

  function connectionLabel(state: DisplaySnapshot['connection']['state']): string {
    return state === 'connected' ? 'connected' : state;
  }

  function displayTime(value: string): string {
    const date = new Date(value);
    return Number.isNaN(date.valueOf()) ? value : date.toLocaleTimeString();
  }

  function blockEndpoints(block: DisplayBlock): DisplayEndpoint[] {
    return [...block.inputs, ...block.outputs];
  }

  function endpointFor(block: DisplayBlock, name: string): DisplayEndpoint | undefined {
    return blockEndpoints(block).find((endpoint) => endpoint.name === name);
  }

  function blockBindingRows(block: DisplayBlock): BindingRow[] {
    return block.bindings.flatMap((binding) => {
      if (binding.kind && binding.kind !== 'knx') return [];
      const endpoint = endpointFor(block, binding.endpoint);
      if (endpoint?.bindingKind && endpoint.bindingKind !== 'knx') return [];
      const groupAddress = binding.groupAddress ?? (endpoint?.bindingKind === 'knx' ? endpoint.address : '');
      if (!groupAddress) return [];
      return [{
        groupAddress,
        blockId: block.id,
        endpoint: binding.endpoint,
        direction: endpoint?.direction ?? null,
        dpt: endpoint?.dpt ?? '—'
      }];
    });
  }

  function automationBindingRows(): BindingRow[] {
    if (snapshot.blocks.length || !snapshot.automation) return [];
    return snapshot.automation.bindings.flatMap((binding) => {
      if (binding.kind && binding.kind !== 'knx') return [];
      const endpoint = [...snapshot.automation!.inputs, ...snapshot.automation!.outputs].find((item) => item.name === binding.endpoint);
      const groupAddress = binding.groupAddress ?? (endpoint?.bindingKind === 'knx' ? endpoint.address : '');
      if (!groupAddress) return [];
      return [{
        groupAddress,
        blockId: null,
        endpoint: binding.endpoint,
        direction: endpoint?.direction ?? null,
        dpt: endpoint?.dpt ?? '—'
      }];
    });
  }

  function executionReferences(): ExecutionReference[] {
    const references: ExecutionReference[] = [];
    const seen = new Set<number>();
    for (const block of snapshot.blocks) {
      for (const execution of block.executions) {
        if (seen.has(execution.executionId)) continue;
        seen.add(execution.executionId);
        references.push({ execution, blockId: block.id });
      }
    }
    for (const execution of snapshot.executions) {
      if (seen.has(execution.executionId)) continue;
      seen.add(execution.executionId);
      references.push({ execution, blockId: execution.blockId });
    }
    return references.sort((left, right) => right.execution.timeMs - left.execution.timeMs);
  }

  function linkedExecutions(telegram: DisplaySnapshot['telegrams'][number]): ExecutionReference[] {
    if (telegram.timeMs === null) return [];
    return executionReferences().filter(({ execution }) => execution.timeMs === telegram.timeMs
      && execution.origin?.kind === 'knx'
      && execution.origin.groupAddress === telegram.destination);
  }

  function bindingsForTelegram(telegram: DisplaySnapshot['telegrams'][number]): BindingRow[] {
    return bindingRows.filter((binding) => binding.groupAddress === telegram.destination);
  }

  function telegramMatchesFilter(destination: string, source: string | null, query: string, bindings: BindingRow[], executions: ExecutionReference[]): boolean {
    if (!query) return true;
    const text = query.toLowerCase();
    return destination.toLowerCase().includes(text)
      || (source ?? '').toLowerCase().includes(text)
      || bindings.some((row) => row.groupAddress === destination && `${row.blockId ?? ''} ${row.endpoint}`.toLowerCase().includes(text))
      || executions.some((item) => item.execution.origin?.kind === 'knx' && item.execution.origin.groupAddress === destination && (item.blockId ?? '').toLowerCase().includes(text));
  }

  function selectExecution(reference: ExecutionReference): void {
    if (reference.blockId) dispatch('selectBlock', reference.blockId);
    dispatch('selectExecution', reference.execution);
  }

  $: bindingRows = [...snapshot.blocks.flatMap(blockBindingRows), ...automationBindingRows()]
    .sort((left, right) => left.groupAddress.localeCompare(right.groupAddress) || (left.blockId ?? '').localeCompare(right.blockId ?? '') || left.endpoint.localeCompare(right.endpoint));
  $: filteredBindings = bindingRows.filter((row) => !filter.trim() || `${row.groupAddress} ${row.blockId ?? ''} ${row.endpoint}`.toLowerCase().includes(filter.trim().toLowerCase()));
  $: executionRows = executionReferences();
  $: filteredTelegrams = snapshot.telegrams.filter((telegram) => telegramMatchesFilter(telegram.destination, telegram.source, filter.trim(), bindingRows, executionRows));
</script>

<section class="panel knx-view" aria-label="KNX">
  <div class="section-heading">
    <div>
      <h2>KNX</h2>
      <p class="subtle">Tunnel health, configured group addresses, and recent bus traffic.</p>
    </div>
    <span class="status-pill {connectionClass(snapshot.connection.state)}">{connectionLabel(snapshot.connection.state)}</span>
  </div>

  <dl class="facts runtime-facts">
    <dt>Connection</dt><dd>{snapshot.connection.state}</dd>
    <dt>Mapped addresses</dt><dd>{bindingRows.length}</dd>
    <dt>Recent telegrams</dt><dd>{snapshot.telegrams.length}</dd>
    <dt>Linked executions</dt><dd>{executionRows.filter(({ execution }) => execution.origin?.kind === 'knx' && execution.origin.groupAddress !== null).length}</dd>
  </dl>

  <label class="knx-filter">Filter addresses or blocks<input type="search" aria-label="Filter KNX addresses or blocks" bind:value={filter} placeholder="1/2/3 or block id" /></label>

  <div class="section-heading"><div><h3>Group address bindings</h3><p class="subtle">Each row is a configured KNX endpoint owned by a logic block.</p></div><span>{filteredBindings.length} / {bindingRows.length}</span></div>
  {#if filteredBindings.length}
    <div class="table-wrap"><table class="bindings-table"><thead><tr><th>Group address</th><th>Block</th><th>Endpoint</th><th>Direction</th><th>DPT</th></tr></thead><tbody>
      {#each filteredBindings as binding}
        <tr>
          <td><code>{binding.groupAddress}</code></td>
          <td>{#if binding.blockId}<button type="button" class="small-button" on:click={() => dispatch('selectBlock', binding.blockId!)}>{binding.blockId}</button>{:else}<span class="subtle">automation</span>{/if}</td>
          <td><code>{binding.endpoint}</code></td>
          <td>{binding.direction ?? '—'}</td>
          <td>{binding.dpt}</td>
        </tr>
      {/each}
    </tbody></table></div>
  {:else}
    <p class="empty">No KNX group address bindings are configured.</p>
  {/if}

  <div class="section-heading"><div><h3>Recent telegrams</h3><p class="subtle">Execution links match both the captured bus-event time and its group address.</p></div><span>{filteredTelegrams.length} / {snapshot.telegrams.length}</span></div>
  {#if filteredTelegrams.length}
    <div class="table-wrap"><table><thead><tr><th>Time</th><th>Source</th><th>Destination</th><th>Service</th><th>DPT</th><th>Value</th><th>Execution</th></tr></thead><tbody>
      {#each filteredTelegrams as telegram}
        {@const links = linkedExecutions(telegram)}
        {@const routes = bindingsForTelegram(telegram)}
        <tr>
          <td>{links[0] ? formatExecutionTime(links[0].execution.timeMs, links[0].execution.timeContext) : displayTime(telegram.time)}</td>
          <td>{telegram.source ?? '—'}</td>
          <td><code>{telegram.destination}</code></td>
          <td>{telegram.service}</td>
          <td>{telegram.dpt}</td>
          <td><span class="value">{formatValue(telegram.value)}</span></td>
          <td>{#if links.length}{#each links as link}<button type="button" class="small-button" on:click={() => selectExecution(link)}>{link.blockId ? `${link.blockId} · ` : ''}exec {link.execution.executionId}</button>{/each}{:else if routes.length}{#each routes as route}{#if route.blockId}<button type="button" class="small-button" on:click={() => dispatch('selectBlock', route.blockId!)}>{route.blockId} · <code>{route.endpoint}</code></button>{:else}<span class="subtle">automation · <code>{route.endpoint}</code></span>{/if}{/each}<span class="subtle"> · {telegram.service === 'group_value_write' && telegram.value !== null ? 'no execution recorded' : 'observation only'}</span>{:else}<span class="subtle">No mapped input</span>{/if}</td>
        </tr>
      {/each}
    </tbody></table></div>
  {:else}
    <p class="empty">{snapshot.telegrams.length ? 'No telegrams match this filter.' : 'No telegrams observed yet.'}</p>
  {/if}
</section>
