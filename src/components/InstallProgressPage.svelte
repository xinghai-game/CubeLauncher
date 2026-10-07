<script lang="ts">
  import { Check, CircleAlert, CircleCheck, LoaderCircle, Play, Rocket, Square } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import { formatSize, loaderLabels } from '../types/api';
  import { phaseLabel } from '../lib/versions';
  import {
    activeDownloads, downloadHeadline, filePercent, fileName, installationState, taskPercent
  } from '../lib/install';
  import Breadcrumb from './Breadcrumb.svelte';
  import { cancelInstall, goHome, goInstances, installInstance, launchSelected, selectInstance } from '../lib/actions';

  const instanceId = $derived(ui.route.name === 'install' ? ui.route.instanceId : '');
  const instance = $derived(app.data.instances.find((item) => item.id === instanceId) ?? null);
  const task = $derived(app.task && app.task.instance_id === instanceId ? app.task : null);
  const status = $derived(installationState(instanceId, task, app.installCancellingId));
  const running = $derived(status === 'running' || status === 'cancelling');
  const done = $derived(status === 'done');
  const retryDisabled = $derived(!instance || Boolean(app.installStartingId || (app.task && !app.task.done)));
  const percent = $derived(taskPercent(task));
  const phases = ['prepare', 'download', 'loader', 'done'];
  const phaseIndex = $derived(task ? Math.max(0, phases.indexOf(task.phase)) : 0);
  const { rows: files, hidden } = $derived(activeDownloads(task));
  const bytesKnown = $derived(Boolean(task && task.total_bytes > 0));

  let now = $state(Date.now());
  $effect(() => {
    if (!running) return;
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  const elapsed = $derived.by(() => {
    if (!app.taskStartedAt) return '';
    const seconds = Math.max(0, Math.floor((now - app.taskStartedAt) / 1000));
    return `${String(Math.floor(seconds / 60)).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`;
  });

  async function openInstance() {
    await goInstances();
    if (instanceId) await selectInstance(instanceId);
  }

  async function launchNow() {
    await openInstance();
    await launchSelected();
  }
</script>

<Breadcrumb
  items={[
    { label: '实例', to: { name: 'instances', tab: 'overview' } },
    { label: instance?.name ?? '未知实例', to: { name: 'instances', tab: 'overview' } },
    { label: '安装' }
  ]}
/>

<div class="page-head compact">
  <div>
    <p class="eyebrow">{running ? (status === 'cancelling' ? '正在取消安装' : '安装进行中') : done ? '安装完成' : '实例安装'}</p>
    <h1>{instance?.name ?? '实例安装'}</h1>
    <p class="subtitle">
      {instance ? `${instance.game_version} · ${loaderLabels[instance.loader]}${instance.loader_version ? ` ${instance.loader_version}` : ''}` : '实例已不在列表中'}
    </p>
  </div>
</div>

{#if !task}
  <section class="panel install-panel">
    <div class="empty-state">
      <Rocket size={24}/><strong>当前没有进行中的安装</strong>
      <span>{instance?.installed ? '实例已安装，可返回实例启动游戏。' : '中断后可在此继续安装，将复用原实例、已校验文件与可恢复的下载进度。'}</span>
    </div>
    <div class="wizard-actions">
      <button class="button ghost" onclick={openInstance}>返回实例</button>
      {#if instance && !instance.installed}
        <button class="button primary" disabled={retryDisabled} onclick={() => installInstance(instanceId)}>继续安装</button>
      {/if}
    </div>
  </section>
{:else if status === 'failed' || status === 'cancelled'}
  <section class="panel install-panel">
    <div class="install-head">
      <span class="install-badge bad"><CircleAlert size={18}/></span>
      <div><strong>{status === 'cancelled' ? '安装已取消' : '安装失败'}</strong><small>{task.error || task.message}</small></div>
    </div>
    <div class="inline-alert">
      <CircleAlert size={16}/>
      <div><strong>{status === 'cancelled' ? '可随时继续安装' : '可以重试安装'}</strong><span>原实例与临时文件已保留。继续时会复用已校验文件，并尝试断点续传；服务器不支持续传时会重新下载相应文件。</span></div>
    </div>
    <div class="wizard-actions">
      <button class="button ghost" onclick={openInstance}>返回实例</button>
      <button class="button primary" disabled={retryDisabled} onclick={() => installInstance(instanceId)}>{status === 'cancelled' ? '继续安装' : '重试安装'}</button>
    </div>
  </section>
{:else}
  <section class="panel install-panel">
    <div class="install-head">
      <span class="install-badge" class:good={done}>{#if done}<CircleCheck size={18}/>{:else}<LoaderCircle class="spin" size={18}/>{/if}</span>
      <div>
        <strong>{done ? '安装完成' : status === 'cancelling' ? '正在取消，保留下载进度…' : downloadHeadline(task, done)}</strong>
        <small>
          {#if done}所有文件已通过校验，可以离线启动
          {:else}{phaseLabel(task.phase)} · 已用 {elapsed || '00:00'}{/if}
        </small>
      </div>
      <span class="install-percent">{percent}%</span>
    </div>

    <div class="progress-track big"><span style={`width:${percent}%`}></span></div>

    {#if running && files.length > 0}
      <div class="download-summary">
        {#if bytesKnown}
          <span>{formatSize(task.downloaded_bytes)} / {formatSize(task.total_bytes)}</span>
        {:else}
          <span>已下载 {formatSize(task.downloaded_bytes)}</span>
        {/if}
        <span class="meta-sep">·</span>
        <span>{task.current}/{task.total || '—'} 个文件</span>
      </div>
      <div class="download-list">
        {#each files as file (file.name)}
          <div class="download-row">
            <span class="download-name" title={file.name}>{fileName(file.name)}</span>
            <span class="download-amount">
              {formatSize(file.downloaded)}{file.total ? ` / ${formatSize(file.total)}` : ''}
            </span>
            <span class="download-track"><i style={`width:${filePercent(file)}%`}></i></span>
          </div>
        {/each}
        {#if hidden > 0}
          <p class="help-text">还有 {hidden} 个文件正在并行下载</p>
        {/if}
      </div>
    {/if}

    <div class="phase-track">
      {#each phases as phase, index (phase)}
        <div class="phase-node" class:done={phaseIndex > index || done} class:active={phaseIndex === index && !done}>
          <span class="phase-dot">{#if phaseIndex > index || done}<Check size={11}/>{/if}</span>
          <span>{phaseLabel(phase)}</span>
        </div>
      {/each}
    </div>

    <p class="help-text">
      {#if done}安装已完成，可返回实例管理或直接启动游戏。
      {:else if status === 'cancelling'}正在等待下载停止；完成取消后即可继续安装。
      {:else}离开此页会在后台继续下载。取消或中断后临时文件会保留，下次在原实例中继续安装即可尝试断点续传。{/if}
    </p>

    <div class="wizard-actions">
      <button class="button ghost" onclick={openInstance}>返回实例</button>
      {#if done}
        <button class="button primary" onclick={launchNow}><Play size={15}/>启动游戏</button>
      {:else}
        <button class="button ghost" disabled={status === 'cancelling' || Boolean(app.installStartingId)} onclick={() => cancelInstall(instanceId)}><Square size={15}/>{status === 'cancelling' ? '正在取消…' : '取消安装（保留进度）'}</button>
        <button class="button ghost" onclick={goHome}>后台安装</button>
      {/if}
    </div>
  </section>
{/if}
