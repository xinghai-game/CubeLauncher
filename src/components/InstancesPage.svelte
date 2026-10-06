<script lang="ts">
  import {
    Archive, Download, FolderOpen, ListTree, LoaderCircle, Play, Plus, ShieldCheck,
    SlidersHorizontal, Square, TerminalSquare, Trash2, Wrench
  } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import { formatPlayed, loaderColors, loaderLabels } from '../types/api';
  import { phaseLabel } from '../lib/versions';
  import {
    activeAccount, goVersions, installSelected, launchSelected, openFolder, selectInstance,
    setDetailTab, stopSelected, verifySelected
  } from '../lib/actions';
  import RunSettingsPanel from './RunSettingsPanel.svelte';
  import ModsPanel from './ModsPanel.svelte';
  import LogsPanel from './LogsPanel.svelte';

  const selected = $derived(app.data.instances.find((item) => item.id === ui.selectedId) ?? null);
  const running = $derived(new Set(app.data.running));
  const account = $derived(activeAccount());
  const enabledMods = $derived(app.mods.filter((mod) => mod.enabled).length);
  const installTask = $derived(
    app.task && !app.task.done && app.task.instance_id === selected?.id ? app.task : null
  );
  const installPercent = $derived(installTask && installTask.total > 0
    ? Math.min(100, Math.round((installTask.current / installTask.total) * 100))
    : 0);
</script>

<div class="page-head compact">
  <div>
    <p class="eyebrow">实例管理</p>
    <h1>全部实例</h1>
    <p class="subtitle">每个实例拥有独立的存档、模组与运行参数。</p>
  </div>
  <button class="button primary" onclick={() => goVersions()}><Plus size={17}/>安装新游戏</button>
</div>

{#if app.data.instances.length === 0}
  <div class="empty-state big">
    <Archive size={26}/><strong>还没有实例</strong>
    <span>在版本目录里挑一个游戏版本，安装向导会自动创建实例并开始下载。</span>
  </div>
{:else}
  <div class="instance-list">
    {#each app.data.instances as instance (instance.id)}
      <button class="list-row" class:selected={ui.selectedId === instance.id} onclick={() => selectInstance(instance.id)}>
        <span class="loader-badge" style={`--loader-color:${loaderColors[instance.loader]}`}>
          {instance.loader === 'vanilla' ? '◇' : instance.loader === 'fabric' ? '✣' : '◈'}
        </span>
        <span class="list-main">
          <strong>{instance.name}</strong>
          <small>{instance.game_version} · {loaderLabels[instance.loader]} {instance.loader_version ?? ''}</small>
        </span>
        {#if running.has(instance.id)}
          <span class="list-state running">运行中</span>
        {:else}
          <span class:ready={instance.installed} class="list-state">{instance.installed ? '已安装' : '需要安装'}</span>
        {/if}
        <span class="list-date">{formatPlayed(instance.last_played)}</span>
      </button>
    {/each}
  </div>
{/if}

{#if selected}
  <section class="detail-panel">
    <div>
      <span class="eyebrow">当前实例</span>
      <h2>{selected.name}</h2>
      <p>
        {selected.game_version} · {loaderLabels[selected.loader]}{selected.loader_version ? ` ${selected.loader_version}` : ''}
        · {account?.name ?? '未设置角色'}
      </p>
    </div>
    <div class="detail-actions">
      {#if running.has(selected.id)}
        <button class="button danger" onclick={stopSelected}><Square size={15}/>停止</button>
      {:else if selected.installed}
        <button class="button primary" disabled={app.busy} onclick={launchSelected}>
          {#if app.busy}<LoaderCircle class="spin" size={16}/>{:else}<Play size={16}/>{/if}启动
        </button>
      {:else}
        <button class="button primary" onclick={installSelected}><Download size={16}/>安装</button>
      {/if}
      <button class="button ghost" onclick={verifySelected}><ShieldCheck size={16}/>校验</button>
      <button class="button ghost" onclick={installSelected}><Wrench size={16}/>修复/更新</button>
      <button class="button ghost" onclick={() => openFolder('instance')}><FolderOpen size={16}/>目录</button>
      <button class="button ghost" onclick={() => (ui.showDeleteConfirm = true)}><Trash2 size={16}/>删除</button>
    </div>
  </section>

  {#if installTask}
    <div class="row-progress">
      <div class="row-progress-head">
        <span><LoaderCircle class="spin" size={14}/> {installTask.message}</span>
        <span>{phaseLabel(installTask.phase)} · {installTask.current}/{installTask.total || '—'} · {installPercent}%</span>
      </div>
      <div class="progress-track"><span style={`width:${installPercent}%`}></span></div>
    </div>
  {/if}

  <div class="tabs">
    <button class:active={ui.detailTab === 'overview'} onclick={() => setDetailTab('overview')}>
      <SlidersHorizontal size={15}/>运行设置
    </button>
    <button class:active={ui.detailTab === 'mods'} onclick={() => setDetailTab('mods')}>
      <ListTree size={15}/>模组 {app.mods.length ? `(${enabledMods}/${app.mods.length})` : ''}
    </button>
    <button class:active={ui.detailTab === 'logs'} onclick={() => setDetailTab('logs')}>
      <TerminalSquare size={15}/>日志
    </button>
  </div>

  {#if ui.detailTab === 'overview'}
    <RunSettingsPanel/>
  {:else if ui.detailTab === 'mods'}
    <ModsPanel/>
  {:else}
    <LogsPanel/>
  {/if}
{/if}
