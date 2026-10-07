<script lang="ts">
  import { Check, CircleHelp, LoaderCircle } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import { phaseLabel } from '../lib/versions';
  import { downloadHeadline, taskPercent } from '../lib/install';
  import { formatSize } from '../types/api';

  const task = $derived(app.task);
  const percent = $derived(taskPercent(task));
  const headline = $derived(downloadHeadline(task, task?.done ?? false));
  const detail = $derived(
    !task || task.done
      ? ''
      : task.total_bytes > 0
        ? `${formatSize(task.downloaded_bytes)} / ${formatSize(task.total_bytes)}`
        : `${task.current}/${task.total || '—'} 个文件`
  );
  // The install page shows progress in full, so the floating card stays out of the way.
  const showTask = $derived(Boolean(task && !task.done && ui.route.name !== 'install'));
</script>

{#if showTask && task}
  <div class="task-toast">
    <div class="task-head">
      <span class="task-spinner"><LoaderCircle size={16}/></span>
      <div>
        <strong>{headline}</strong>
        <small>{phaseLabel(task.phase)} · {detail}</small>
      </div>
      <span class="task-percent">{percent}%</span>
    </div>
    <div class="progress-track"><span style={`width:${percent}%`}></span></div>
  </div>
{/if}

{#if ui.toast}
  <div class="toast" class:error={ui.toastError}>
    {#if ui.toastError}<CircleHelp size={16}/>{:else}<Check size={16}/>{/if}{ui.toast}
  </div>
{/if}