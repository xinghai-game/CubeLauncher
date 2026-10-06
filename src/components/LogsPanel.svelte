<script lang="ts">
  import { RefreshCw, TerminalSquare, Trash2 } from 'lucide-svelte';
  import { app } from '../lib/state.svelte';
  import { clearLogs, loadFileLogs } from '../lib/actions';
</script>

<section class="panel">
  <div class="panel-head">
    <div><strong>运行日志</strong><span>实时输出与磁盘中最近 300 行</span></div>
    <div class="detail-actions">
      <button class="button ghost" onclick={loadFileLogs}><RefreshCw size={15}/>刷新</button>
      <button class="button ghost" onclick={clearLogs}><Trash2 size={15}/>清空</button>
    </div>
  </div>
  <div class="log-view">
    {#if app.liveLogs.length === 0 && app.fileLogs.length === 0}
      <div class="log-empty"><TerminalSquare size={24}/><span>启动游戏后，Java 输出会显示在这里。</span></div>
    {:else}
      {#each app.fileLogs as line}<div><span class="log-time">记录</span><span>{line}</span></div>{/each}
      {#each app.liveLogs as line}<div><span class="log-time">实时</span><span>{line}</span></div>{/each}
    {/if}
  </div>
</section>
