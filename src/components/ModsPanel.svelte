<script lang="ts">
  import { FolderOpen, ListTree, Plus, Trash2 } from 'lucide-svelte';
  import { app } from '../lib/state.svelte';
  import { formatSize } from '../types/api';
  import { addMod, openFolder, removeMod, toggleMod } from '../lib/actions';

  const enabled = $derived(app.mods.filter((mod) => mod.enabled).length);
</script>

<section class="panel">
  <div class="panel-head">
    <div><strong>本地模组</strong><span>{enabled} 个已启用 / 共 {app.mods.length} 个文件</span></div>
    <div class="detail-actions">
      <button class="button ghost" onclick={() => openFolder('mods')}><FolderOpen size={15}/>打开目录</button>
      <button class="button primary" onclick={addMod}><Plus size={15}/>添加模组</button>
    </div>
  </div>
  {#if app.mods.length === 0}
    <div class="empty-state">
      <ListTree size={24}/><strong>还没有模组</strong>
      <span>添加 .jar 文件后会显示在这里，可随时启用或停用。</span>
    </div>
  {:else}
    <div class="mod-list">
      {#each app.mods as mod (mod.file_name)}
        <div class="mod-row" class:disabled={!mod.enabled}>
          <span class="mod-name"><strong>{mod.display_name}</strong><small>{mod.file_name} · {formatSize(mod.size)}</small></span>
          <button class="button ghost small" onclick={() => toggleMod(mod)}>{mod.enabled ? '停用' : '启用'}</button>
          <button class="button ghost small" onclick={() => removeMod(mod)}><Trash2 size={14}/></button>
        </div>
      {/each}
    </div>
  {/if}
</section>
