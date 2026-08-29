<script lang="ts">
  import type { AutomationSchedule } from './automation';

  export let schedules: AutomationSchedule[] = [];
  export let onChange: (schedules: AutomationSchedule[]) => void = () => {};

  const kinds: AutomationSchedule['kind'][] = ['fixed', 'interval', 'astronomical'];
  const anchors: NonNullable<AutomationSchedule['anchor']>[] = ['dawn', 'sunrise', 'sunset', 'dusk'];

  function template(name: string, kind: AutomationSchedule['kind'] = 'fixed'): AutomationSchedule {
    if (kind === 'interval') return { name, enabled: true, kind, every: '1h' };
    if (kind === 'astronomical') return { name, enabled: true, kind, anchor: 'sunset', offset: '0s' };
    return { name, enabled: true, kind, at: '12:00' };
  }

  function edit(index: number, patch: Partial<AutomationSchedule>): void {
    onChange(schedules.map((schedule, current) => current === index ? { ...schedule, ...patch } : schedule));
  }

  function changeKind(index: number, kind: AutomationSchedule['kind']): void {
    const current = schedules[index];
    if (current) edit(index, template(current.name, kind));
  }

  function add(): void {
    if (schedules.length >= 32) return;
    const names = new Set(schedules.map((schedule) => schedule.name));
    let suffix = 1;
    let name = `schedule_${suffix}`;
    while (names.has(name)) { suffix += 1; name = `schedule_${suffix}`; }
    onChange([...schedules, template(name)]);
  }

  function remove(index: number): void { onChange(schedules.filter((_, current) => current !== index)); }
  function weekdays(value: string): AutomationSchedule['weekdays'] {
    const parsed = value.split(',').map((item) => item.trim().toLowerCase()).filter(Boolean) as AutomationSchedule['weekdays'];
    return parsed?.length ? parsed : undefined;
  }
</script>

<div class="schedule-authoring"><div class="section-heading"><h3>Schedule definitions</h3><button type="button" class="small-button" disabled={schedules.length >= 32} on:click={add}>Add schedule</button></div><p class="subtle">Definitions belong to this block. Fixed and interval changes are saved with the block; astronomical rules use the desktop site-time context.</p>
  {#if schedules.length}
    {#each schedules as schedule, index}
      <fieldset class="schedule-editor"><legend><code>{schedule.name}</code></legend><div class="schedule-fields">
        <label>Name<input aria-label={`Schedule ${index + 1} name`} value={schedule.name} on:input={(event) => edit(index, { name: (event.currentTarget as HTMLInputElement).value.trim().toLowerCase() })} /></label>
        <label>Kind<select aria-label={`Schedule ${index + 1} kind`} value={schedule.kind} on:change={(event) => changeKind(index, (event.currentTarget as HTMLSelectElement).value as AutomationSchedule['kind'])}>{#each kinds as kind}<option value={kind}>{kind}</option>{/each}</select></label>
        <label class="checkbox-label"><input aria-label={`Schedule ${index + 1} enabled`} type="checkbox" checked={schedule.enabled} on:change={(event) => edit(index, { enabled: (event.currentTarget as HTMLInputElement).checked })} /> Enabled</label>
        {#if schedule.kind === 'fixed'}
          <label>At (local)<input aria-label={`Schedule ${index + 1} local time`} placeholder="HH:MM" value={schedule.at ?? ''} on:input={(event) => edit(index, { at: (event.currentTarget as HTMLInputElement).value })} /></label>
        {:else if schedule.kind === 'interval'}
          <label>Every<input aria-label={`Schedule ${index + 1} interval`} placeholder="1h30m" value={schedule.every ?? ''} on:input={(event) => edit(index, { every: (event.currentTarget as HTMLInputElement).value })} /></label>
          <label>Offset<input aria-label={`Schedule ${index + 1} interval offset`} placeholder="0s" value={schedule.offset ?? ''} on:input={(event) => edit(index, { offset: (event.currentTarget as HTMLInputElement).value || undefined })} /></label>
        {:else}
          <label>Anchor<select aria-label={`Schedule ${index + 1} astronomical anchor`} value={schedule.anchor ?? 'sunset'} on:change={(event) => edit(index, { anchor: (event.currentTarget as HTMLSelectElement).value as AutomationSchedule['anchor'] })}>{#each anchors as anchor}<option value={anchor}>{anchor}</option>{/each}</select></label>
          <label>Offset<input aria-label={`Schedule ${index + 1} astronomical offset`} placeholder="-1h30m" value={schedule.offset ?? ''} on:input={(event) => edit(index, { offset: (event.currentTarget as HTMLInputElement).value })} /></label>
          <label>Earliest (local)<input aria-label={`Schedule ${index + 1} earliest time`} placeholder="05:30" value={schedule.earliest ?? ''} on:input={(event) => edit(index, { earliest: (event.currentTarget as HTMLInputElement).value || undefined })} /></label>
          <label>Latest (local)<input aria-label={`Schedule ${index + 1} latest time`} placeholder="23:30" value={schedule.latest ?? ''} on:input={(event) => edit(index, { latest: (event.currentTarget as HTMLInputElement).value || undefined })} /></label>
        {/if}
        <label>Weekdays<input aria-label={`Schedule ${index + 1} weekdays`} placeholder="mon, tue, wed" value={(schedule.weekdays ?? []).join(', ')} on:input={(event) => edit(index, { weekdays: weekdays((event.currentTarget as HTMLInputElement).value) })} /></label>
        <button type="button" class="small-button danger-button" on:click={() => remove(index)}>Remove</button>
      </div></fieldset>
    {/each}
  {:else}<p class="empty">No schedule definitions. Add one to author a block-local schedule.</p>{/if}
</div>

<style>
  .schedule-authoring { margin: 1rem 0; padding: 1rem; border: 1px solid var(--border, #d7dce5); border-radius: 0.5rem; background: var(--panel, #fff); }
  .schedule-editor { margin: 0.75rem 0; padding: 0.75rem; border: 1px solid var(--border, #d7dce5); border-radius: 0.4rem; }
  .schedule-fields { display: flex; flex-wrap: wrap; align-items: end; gap: 0.75rem; }
  .schedule-fields label { display: flex; min-width: 9rem; flex-direction: column; gap: 0.25rem; }
  .schedule-fields .checkbox-label { min-width: auto; flex-direction: row; align-items: center; }
  .schedule-fields input, .schedule-fields select { min-height: 2rem; }
  .danger-button { color: #a12626; }
</style>
