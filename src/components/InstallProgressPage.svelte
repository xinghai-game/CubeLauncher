<script lang="ts">
  import { Check, CircleAlert, CircleCheck, LoaderCircle, Play, Rocket } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import { loaderLabels } from '../types/api';
  import { phaseLabel } from '../lib/versions';
  import Breadcrumb from './Breadcrumb.svelte';
  import { goHome, goInstances, installInstance, launchSelected, selectInstance } from '../lib/actions';

  const instanceId = $derived(ui.route.name === 'install' ? ui.route.instanceId : '');
  const instance = $derived(app.data.instances.find((item) => item.id === instanceId) ?? null);
  const task = $derived(app.task && app.task.instance_id === instanceId ? app.task : null);
  const running = $derived(Boolean(task && !task.done && !task.error));
  const done = $derived(Boolean(task?.done && !task.error));
  const failed = $derived(Boolean(task?.done && task.error));
  const percent = $derived(task && task.total > 0 ? Math.min(100, Math.round((task.current / task.total) * 100)) : 0);
  const phases = ['prepare', 'download', 'loader', 'done'];
  const phaseIndex = $derived(task ? Math.max(0, phases.indexOf(task.phase)) : 0);

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
    <p class="eyebrow">安装进行中</p>
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
      <span>可以返回实例列表查看状态，或从版本目录重新安装。</span>
    </div>
    <div class="wizard-actions">
      <button class="button primary" onclick={openInstance}>返回实例</button>
    </div>
  </section>
{:else if failed}
  <section class="panel install-panel">
    <div class="install-head">
      <span class="install-badge bad"><CircleAlert size={18}/></span>
      <div><strong>安装失败</strong><small>{task.message}</small></div>
    </div>
    <div class="inline-alert">
      <CircleAlert size={16}/>
      <div><strong>可以重试</strong><span>已下载并通过校验的文件会直接复用，不会重新下载。</span></div>
    </div>
    <div class="wizard-actions">
      <button class="button ghost" onclick={openInstance}>返回实例</button>
      <button class="button primary" onclick={() => installInstance(instanceId)}>重试安装</button>
    </div>
  </section>
{:else}
  <section class="panel install-panel">
    <div class="install-head">
      <span class="install-badge" class:good={done}>{#if done}<CircleCheck size={18}/>{:else}<LoaderCircle class="spin" size={18}/>{/if}</span>
      <div>
        <strong>{done ? '安装完成' : task.message}</strong>
        <small>
          {#if done}所有文件已通过校验，可以离线启动
          {:else}{phaseLabel(task.phase)} · 已用 {elapsed || '00:00'}{/if}
        </small>
      </div>
      <span class="install-percent">{percent}%</span>
    </div>

    <div class="progress-track big"><span style={`width:${percent}%`}></span></div>

    <div class="phase-track">
      {#each phases as phase, index (phase)}
        <div class="phase-node" class:done={phaseIndex > index || done} class:active={phaseIndex === index && !done}>
          <span class="phase-dot">{#if phaseIndex > index || done}<Check size={11}/>{/if}</span>
          <span>{phaseLabel(phase)}</span>
        </div>
      {/each}
    </div>

    <p class="help-text">
      安装期间可以离开这个页面，下载会在后台继续；完成后实例会显示在实例列表中。
    </p>

    <div class="wizard-actions">
      <button class="button ghost" onclick={openInstance}>返回实例</button>
      {#if done}
        <button class="button primary" onclick={launchNow}><Play size={15}/>启动游戏</button>
      {:else}
        <button class="button ghost" onclick={goHome}>后台安装</button>
      {/if}
    </div>
  </section>
{/if}
