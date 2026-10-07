<script lang="ts">
  import {
    Archive, Download, FolderInput, FolderOpen, LoaderCircle, Play, ShieldCheck,
    SlidersHorizontal, Square, TerminalSquare, Trash2, Wrench
  } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import { formatPlayed, isImported, loaderLabels } from '../types/api';
  import { phaseLabel } from '../lib/versions';
  import { installationState, installInFlight } from '../lib/install';
  import {
    activeAccount, go, goImport, installSelected, launchSelected, openFolder, selectInstance,
    setDetailTab, stopSelected, verifySelected
  } from '../lib/actions';
  import Breadcrumb from './Breadcrumb.svelte';
  import RunSettingsPanel from './RunSettingsPanel.svelte';
  import ResourcePanel from './ResourcePanel.svelte';
  import LogsPanel from './LogsPanel.svelte';

  const instanceId = $derived(ui.route.name === 'instance' ? ui.route.instanceId : '');
  const tab = $derived(ui.route.name === 'instance' ? ui.route.tab : 'overview');
  const selected = $derived(app.data.instances.find((item) => item.id === instanceId) ?? null);
  const running = $derived(new Set(app.data.running));
  const account = $derived(activeAccount());
  const enabledResources = $derived(app.resourceEntries.filter((resource) => resource.enabled).length);
  const installTask = $derived(
    app.task && !app.task.done && app.task.instance_id === selected?.id ? app.task : null
  );
  const installStatus = $derived(installationState(selected?.id ?? '', app.task, app.installCancellingId));
  const installBusy = $derived(installInFlight(selected?.id ?? '', app.task, app.installStartingId));
  const launching = $derived(Boolean(selected && app.launchingId === selected.id));
  const verifying = $derived(Boolean(selected && app.verifyingId === selected.id));
  const installPercent = $derived(installTask && installTask.total > 0
    ? Math.min(100, Math.round((installTask.current / installTask.total) * 100))
    : 0);

  // Breadcrumb links can enter this page without going through openInstance.
  $effect(() => {
    if (!instanceId || !selected) return;
    if (ui.selectedId !== instanceId || ui.detailTab !== tab) {
      ui.detailTab = tab;
      void selectInstance(instanceId);
    }
  });

  function selectTab(next: typeof tab) {
    void setDetailTab(next);
  }
</script>

<Breadcrumb
  items={[
    { label: '实例', to: { name: 'instances', tab: 'overview' } },
    { label: selected?.name ?? '未知实例' }
  ]}
/>

{#if !selected}
  <section class="panel install-panel">
    <div class="empty-state"><Archive size={25}/><strong>实例不存在</strong><span>它可能刚刚被删除，返回实例列表查看其他实例。</span></div>
    <div class="wizard-actions"><button class="button primary" onclick={() => go({ name: 'instances', tab: 'overview' })}>返回实例列表</button></div>
  </section>
{:else}
  <div class="page-head compact instance-detail-head">
    <div>
      <p class="eyebrow">{isImported(selected) ? '实例管理 · 导入目录' : '实例管理'}</p>
      <h1>{selected.name}</h1>
      <p class="subtitle">
        {selected.game_version} · {loaderLabels[selected.loader]}{selected.loader_version ? ` ${selected.loader_version}` : ''}
        · {account?.name ?? '未设置角色'} · 最近启动 {formatPlayed(selected.last_played)}
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
  </div>

  {#if !installTask && (!selected.installed || installStatus === 'cancelled' || installStatus === 'failed')}
    <p class="help-text">{installStatus === 'cancelled' ? '安装已取消。' : installStatus === 'failed' ? '上次安装失败。' : ''}可在这个实例页面中继续安装或重试，已校验文件与可恢复的下载进度会复用。</p>
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

  <div class="instance-tabs tabs">
    <button class:active={tab === 'overview'} onclick={() => selectTab('overview')}><SlidersHorizontal size={15}/>运行设置</button>
    <button class:active={tab === 'mods'} onclick={() => selectTab('mods')}><span class="tab-glyph">M</span>Mod {tab === 'mods' && app.resourceEntries.length ? `(${enabledResources}/${app.resourceEntries.length})` : ''}</button>
    <button class:active={tab === 'shaders'} onclick={() => selectTab('shaders')}><span class="tab-glyph">✦</span>光影</button>
    <button class:active={tab === 'projections'} onclick={() => selectTab('projections')}><span class="tab-glyph">▧</span>投影</button>
    <button class:active={tab === 'logs'} onclick={() => selectTab('logs')}><TerminalSquare size={15}/>日志</button>
  </div>

  {#if tab === 'overview'}
    <RunSettingsPanel/>
  {:else if tab === 'mods' || tab === 'shaders' || tab === 'projections'}
    <ResourcePanel kind={tab}/>
  {:else}
    <LogsPanel/>
  {/if}
{/if}
