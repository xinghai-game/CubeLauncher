<script lang="ts">
  import { Archive, ChevronRight, FolderInput, Plus } from 'lucide-svelte';
  import { app } from '../lib/state.svelte';
  import { formatPlayed, isImported, loaderColors, loaderLabels, shortPath } from '../types/api';
  import { goImport, goVersions, openInstance } from '../lib/actions';

  const running = $derived(new Set(app.data.running));
</script>

<div class="page-head compact">
  <div>
    <p class="eyebrow">实例管理</p>
    <h1>全部实例</h1>
    <p class="subtitle">每个实例都有独立的管理页面，可以分别配置运行参数、Mod、光影、投影和日志。</p>
  </div>
  <div class="head-actions">
    <button class="button ghost" onclick={() => goImport()}><FolderInput size={16}/>导入 .minecraft</button>
    <button class="button primary" onclick={() => goVersions()}><Plus size={17}/>安装新游戏</button>
  </div>
</div>

{#if app.data.instances.length === 0}
  <div class="empty-state big">
    <Archive size={26}/><strong>还没有实例</strong>
    <span>在版本目录里挑一个游戏版本安装，或者导入一个已经存在的 .minecraft 目录（不会复制文件）。</span>
    <div class="detail-actions">
      <button class="button primary" onclick={() => goVersions()}><Plus size={16}/>浏览版本目录</button>
      <button class="button ghost" onclick={() => goImport()}><FolderInput size={16}/>导入已有 .minecraft</button>
    </div>
  </div>
{:else}
  <div class="instance-list instance-list-page">
    {#each app.data.instances as instance (instance.id)}
      <button class="list-row instance-entry" onclick={() => openInstance(instance.id)}>
        <span class="loader-badge" style={`--loader-color:${loaderColors[instance.loader]}`}>
          {instance.loader === 'vanilla' ? '◇' : instance.loader === 'fabric' ? '✣' : '◈'}
        </span>
        <span class="list-main">
          <strong>{instance.name}</strong>
          <small>
            {instance.game_version} · {loaderLabels[instance.loader]} {instance.loader_version ?? ''}
            {#if isImported(instance)} · 外部目录 {shortPath(instance.game_dir ?? '')}{/if}
          </small>
        </span>
        <span class="instance-entry-resources">Mod · 光影 · 投影</span>
        {#if running.has(instance.id)}
          <span class="list-state running">运行中</span>
        {:else}
          <span class:ready={instance.installed} class="list-state">{instance.installed ? '已安装' : '需要安装'}</span>
        {/if}
        <span class="list-date">{formatPlayed(instance.last_played)}</span>
        <ChevronRight size={16} class="instance-chevron"/>
      </button>
    {/each}
  </div>

  <div class="instance-list-footnote">
    <span>选择一个实例进入专属页面</span>
    <span>每个实例的资源目录彼此隔离</span>
  </div>
{/if}
