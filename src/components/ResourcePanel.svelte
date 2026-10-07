<script lang="ts">
  import { FolderOpen, ListTree, LoaderCircle, Plus, Trash2 } from 'lucide-svelte';
  import { app } from '../lib/state.svelte';
  import type { ResourceKind } from '../types/api';
  import { formatSize } from '../types/api';
  import { addResource, openFolder, removeResource, toggleResource } from '../lib/actions';

  let { kind }: { kind: ResourceKind } = $props();

  const configs: Record<ResourceKind, {
    title: string;
    description: string;
    empty: string;
    hint: string;
    extensions: string;
  }> = {
    mods: {
      title: '本地模组',
      description: '启用的 .jar 会在下次启动时加载',
      empty: '还没有模组',
      hint: '添加 .jar 文件后会显示在这里，可随时启用或停用。',
      extensions: '.jar'
    },
    shaders: {
      title: '光影包',
      description: '光影包保存在 shaderpacks 目录',
      empty: '还没有光影包',
      hint: '支持 .zip 或 .jar 光影文件，停用后仍会保留在实例目录。',
      extensions: '.zip / .jar'
    },
    projections: {
      title: '投影文件',
      description: '投影文件保存在 schematics 目录',
      empty: '还没有投影文件',
      hint: '支持 Litematica、WorldEdit 常用的 .litematic、.schem、.schematic 文件。',
      extensions: '.litematic / .schem / .schematic'
    }
  };

  const config = $derived(configs[kind]);
  const enabled = $derived(app.resourceEntries.filter((resource) => resource.enabled).length);
</script>

<section class="panel resource-panel">
  <div class="panel-head">
    <div>
      <strong>{config.title}</strong>
      <span>{enabled} 个已启用 / 共 {app.resourceEntries.length} 个文件 · {config.description}</span>
    </div>
    <div class="detail-actions">
      <button class="button ghost" onclick={() => openFolder(kind)}><FolderOpen size={15}/>打开目录</button>
      <button class="button primary" onclick={() => addResource(kind)}><Plus size={15}/>添加文件</button>
    </div>
  </div>

  {#if app.resourceLoading}
    <div class="empty-state resource-loading"><LoaderCircle class="spin" size={24}/><span>正在读取 {config.title}…</span></div>
  {:else if app.resourceEntries.length === 0}
    <div class="empty-state">
      <ListTree size={24}/><strong>{config.empty}</strong>
      <span>{config.hint}</span>
    </div>
  {:else}
    <div class="mod-list">
      {#each app.resourceEntries as resource (resource.file_name)}
        <div class="mod-row" class:disabled={!resource.enabled}>
          <span class="resource-type-icon"><ListTree size={15}/></span>
          <span class="mod-name">
            <strong>{resource.display_name}</strong>
            <small>{resource.file_name} · {formatSize(resource.size)}</small>
          </span>
          <span class="resource-extension">{config.extensions}</span>
          <button class="button ghost small" onclick={() => toggleResource(kind, resource)}>{resource.enabled ? '停用' : '启用'}</button>
          <button class="button ghost small" title="删除" onclick={() => removeResource(kind, resource)}><Trash2 size={14}/></button>
        </div>
      {/each}
    </div>
  {/if}
</section>
