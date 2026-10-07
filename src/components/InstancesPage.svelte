<script lang="ts">
  import {
    Archive, Download, FolderInput, FolderOpen, ListTree, LoaderCircle, Play, Plus, ShieldCheck,
    SlidersHorizontal, Square, TerminalSquare, Trash2, Wrench
  } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import { formatPlayed, isImported, loaderColors, loaderLabels, shortPath } from '../types/api';
  import { phaseLabel } from '../lib/versions';
  import { installationState, installInFlight } from '../lib/install';
  import {
    activeAccount, go, goImport, goVersions, installSelected, launchSelected, openFolder,
    selectInstance, setDetailTab, stopSelected, verifySelected
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
  const installStatus = $derived(installationState(selected?.id ?? '', app.task, app.installCancellingId));
  // Only this instance's own install disables its actions: a task left behind by
  // a Java download or another instance used to freeze them permanently.
  const installBusy = $derived(installInFlight(selected?.id ?? '', app.task, app.installStartingId));
  const launching = $derived(Boolean(selected && app.launchingId === selected.id));
  const verifying = $derived(Boolean(selected && app.verifyingId === selected.id));
  const installPercent = $derived(installTask && installTask.total > 0
    ? Math.min(100, Math.round((installTask.current / installTask.total) * 100))
    : 0);
</script>

<div class="page-head compact">
  <div>
    <p class="eyebrow">实例管理</p>
    <h1>全部实例</h1>
    <p class="subtitle">每个实例拥有独立的存档、模组与运行参数，也可以直接指向已有的 .minecraft 目录。</p>
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
  <div class="instance-list">
    {#each app.data.instances as instance (instance.id)}
      <button class="list-row" class:selected={ui.selectedId === instance.id} onclick={() => selectInstance(instance.id)}>
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
      <span class="eyebrow">当前实例{isImported(selected) ? ' · 导入目录' : ''}</span>
      <h2>{selected.name}</h2>
      <p>
        {selected.game_version} · {loaderLabels[selected.loader]}{selected.loader_version ? ` ${selected.loader_version}` : ''}
        · {account?.name ?? '未设置角色'}
      </p>
    </div>
    <div class="detail-actions">
      {#if running.has(selected.id)}
        <button class="button danger" onclick={stopSelected}><Square size={15}/>停止</button>
      {:else if installTask}
        <button class="button primary" onclick={() => go({ name: 'install', instanceId: selected.id })}><LoaderCircle class="spin" size={16}/>查看安装进度</button>
      {:else if selected.installed}
        <button class="button primary" disabled={launching} onclick={launchSelected}>
          {#if launching}<LoaderCircle class="spin" size={16}/>{:else}<Play size={16}/>{/if}启动
        </button>
      {:else}
        <button class="button primary" disabled={installBusy} onclick={installSelected}><Download size={16}/>{installStatus === 'failed' ? '重试安装' : '继续安装'}</button>
      {/if}
      <button class="button ghost" disabled={verifying} onclick={verifySelected}>
        {#if verifying}<LoaderCircle class="spin" size={16}/>{:else}<ShieldCheck size={16}/>{/if}{verifying ? '校验中…' : '校验'}
      </button>
      <button class="button ghost" disabled={installBusy} onclick={installSelected}><Wrench size={16}/>修复/更新</button>
      <button class="button ghost" onclick={() => openFolder('game')}><FolderOpen size={16}/>游戏目录</button>
      <button class="button ghost" onclick={() => goImport(selected.id)}><FolderInput size={16}/>更换目录</button>
      <button class="button ghost" onclick={() => (ui.showDeleteConfirm = true)}><Trash2 size={16}/>删除</button>
    </div>
  </section>

  {#if !installTask && (!selected.installed || installStatus === 'cancelled' || installStatus === 'failed')}
    <p class="help-text">{installStatus === 'cancelled' ? '安装已取消。' : installStatus === 'failed' ? '上次安装失败。' : ''}可在原实例中继续安装或重试，已校验文件与可恢复的下载进度会复用，无需新建实例。</p>
  {/if}

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
