<script lang="ts">
  import {
    Check, CircleAlert, Coffee, Download, Layers, LoaderCircle, Package, RefreshCw, Search
  } from 'lucide-svelte';
  import { app, loaderKey, ui } from '../lib/state.svelte';
  import { loaderColors, loaderLabels, type Loader } from '../types/api';
  import { suggestInstanceName } from '../lib/versions';
  import Breadcrumb from './Breadcrumb.svelte';
  import {
    activeAccount, back, createAndInstall, go, loadJavaRequirement, loadLoaderVersions,
    replaceRoute
  } from '../lib/actions';

  const route = $derived(ui.route.name === 'wizard' ? ui.route : { name: 'wizard' as const, version: '', step: 1 as const });
  const version = $derived(route.version);
  const step = $derived(route.step);
  const loaders = $derived(Object.keys(loaderLabels) as Loader[]);
  const loaderState = $derived(app.loaderVersions[loaderKey(version, ui.form.loader)]);
  // "版本目录" in the crumb trail returns to the tab the user came from.
  const versionsRoute = $derived.by(() => {
    const previous = [...ui.history].reverse().find((entry) => entry.name === 'versions');
    return previous ?? { name: 'versions' as const, kind: 'release' as const, query: '' };
  });

  let versionQuery = $state('');

  const hints: Record<Loader, string> = {
    vanilla: '最纯净的官方版本，不需要额外组件',
    fabric: '轻量、更新快，适合贴近原版的模组',
    forge: '生态最完整的模组加载器，1.12.2 起',
    neoforge: 'Forge 的社区分支，1.20.2 起'
  };

  function stateOf(loader: Loader) {
    return app.loaderVersions[loaderKey(version, loader)];
  }

  function statusOf(loader: Loader): { text: string; tone: 'ok' | 'bad' | 'idle' | 'busy' } {
    if (loader === 'vanilla') return { text: '无需加载器，立即可用', tone: 'ok' };
    const state = stateOf(loader);
    if (!state) return { text: '点击查询可用版本', tone: 'idle' };
    if (state.loading) return { text: '正在查询版本列表…', tone: 'busy' };
    if (state.error) return { text: state.error, tone: 'bad' };
    if (state.versions.length === 0) return { text: '没有适用于该版本的版本', tone: 'bad' };
    return { text: `${state.versions.length} 个可用版本 · 最新 ${state.versions[0]}`, tone: 'ok' };
  }

  function pickLoader(loader: Loader) {
    ui.form.loader = loader;
    ui.form.loaderVersion = '';
    if (loader !== 'vanilla') void loadLoaderVersions(version, loader);
  }

  async function probeAll() {
    for (const loader of loaders) {
      if (loader !== 'vanilla') void loadLoaderVersions(version, loader);
    }
  }

  function loaderReady(): boolean {
    if (ui.form.loader === 'vanilla') return true;
    const state = stateOf(ui.form.loader);
    return Boolean(state && !state.loading && (state.versions.length > 0 || state.error));
  }

  function nextFromLoader() {
    const next = ui.form.loader === 'vanilla' ? 3 : 2;
    go({ name: 'wizard', version, step: next as 1 | 2 | 3 });
    if (next === 3) void loadJavaRequirement(version);
  }

  function nextFromVersion() {
    go({ name: 'wizard', version, step: 3 });
    void loadJavaRequirement(version);
  }

  function backToStep1() {
    replaceRoute({ name: 'wizard', version, step: 1 });
  }

  const filteredLoaderVersions = $derived.by(() => {
    const list = loaderState?.versions ?? [];
    const needle = versionQuery.trim().toLowerCase();
    if (!needle) return list;
    return list.filter((entry) => entry.toLowerCase().includes(needle));
  });

  const stepTitle = $derived(step === 1 ? '选择加载器' : step === 2 ? '选择加载器版本' : '确认并安装');
  const javaLabel = $derived.by(() => {
    if (app.javaLoading) return '读取中…';
    if (!app.javaRequirement) return '未知（安装时按版本元数据决定）';
    return `Java ${app.javaRequirement.major}`;
  });
  const javaNote = $derived(app.javaRequirement?.source === 'metadata'
    ? '来自该版本的元数据'
    : '来自启动器的回退表，安装时会以元数据为准');
</script>

<Breadcrumb
  items={[
    { label: '版本目录', to: versionsRoute },
    { label: version, to: { name: 'wizard', version, step: 1 } },
    { label: stepTitle }
  ]}
/>

<div class="page-head compact">
  <div>
    <p class="eyebrow">安装向导 · 第 {step} / 3 步</p>
    <h1>{version}</h1>
    <p class="subtitle">加载器版本来自各自官方 Meta API 与 Maven，查询结果会缓存到本地。</p>
  </div>
</div>

<div class="wizard-steps">
  {#each [1, 2, 3] as index (index)}
    <div class="wizard-step" class:done={step > index} class:active={step === index}>
      <span class="step-index">{#if step > index}<Check size={13}/>{:else}{index}{/if}</span>
      <span>{index === 1 ? '选择加载器' : index === 2 ? '选择版本' : '确认并安装'}</span>
    </div>
  {/each}
</div>

{#if step === 1}
  <section class="panel">
    <div class="panel-head">
      <div><strong>加载器</strong><span>原版不需要加载器；其余加载器会按游戏版本查询可用版本</span></div>
      <button class="button ghost small" onclick={probeAll}><RefreshCw size={14}/>查询全部加载器</button>
    </div>
    <div class="loader-grid">
      {#each loaders as loader (loader)}
        {@const status = statusOf(loader)}
        <button
          class="loader-option"
          class:chosen={ui.form.loader === loader}
          onclick={() => pickLoader(loader)}
        >
          <span class="loader-mark" style={`--loader-color:${loaderColors[loader]}`}>
            {loader === 'vanilla' ? '◇' : loader === 'fabric' ? '✣' : '◈'}
          </span>
          <span class="loader-copy">
            <strong>{loaderLabels[loader]}</strong>
            <small>{hints[loader]}</small>
          </span>
          <span class={`loader-status tone-${status.tone}`}>
            {#if status.tone === 'busy'}<LoaderCircle class="spin" size={12}/>{/if}{status.text}
          </span>
        </button>
      {/each}
    </div>
    {#if ui.form.loader === 'neoforge'}<p class="help-text">NeoForge 从 Minecraft 1.20.2 起提供；1.20.1 请选择 Forge。</p>{/if}
    {#if ui.form.loader !== 'vanilla' && stateOf(ui.form.loader)?.error}
      <div class="inline-alert">
        <CircleAlert size={16}/>
        <div><strong>无法自动获取版本列表</strong><span>{stateOf(ui.form.loader)?.error}</span></div>
        <button class="button ghost small" onclick={() => loadLoaderVersions(version, ui.form.loader, true)}>重试</button>
      </div>
    {/if}
    <div class="wizard-actions">
      <button class="button primary" disabled={!loaderReady()} onclick={nextFromLoader}>
        下一步{ui.form.loader === 'vanilla' ? '（确认）' : ''}
      </button>
    </div>
  </section>

{:else if step === 2}
  <section class="panel">
    <div class="panel-head">
      <div>
        <strong>{loaderLabels[ui.form.loader]} 版本</strong>
        <span>按新到旧排列，第一项即官方最新版</span>
      </div>
      <label class="search-field small">
        <Search size={14}/>
        <input placeholder="筛选版本号" bind:value={versionQuery} />
      </label>
    </div>
    {#if loaderState?.loading}
      <div class="empty-state"><LoaderCircle class="spin" size={24}/><strong>正在查询</strong><span>读取 {loaderLabels[ui.form.loader]} 的官方版本列表。</span></div>
    {:else if loaderState?.error}
      <div class="inline-alert">
        <CircleAlert size={16}/>
        <div><strong>查询失败</strong><span>{loaderState.error}</span></div>
        <button class="button ghost small" onclick={() => loadLoaderVersions(version, ui.form.loader, true)}>重试</button>
      </div>
      <label>手动填写版本号
        <input bind:value={ui.form.loaderVersion} placeholder="例如 47.4.26" />
      </label>
      <p class="help-text">镜像的加载器列表可能滞后；手动填写的版本号仍会按官方哈希校验。</p>
    {:else}
      <div class="choice-list">
        <button class="choice-row" class:chosen={ui.form.loaderVersion === ''} onclick={() => (ui.form.loaderVersion = '')}>
          <span class="choice-main"><strong>自动选择最新</strong><small>留给启动器挑选当前最新的稳定版本</small></span>
          {#if ui.form.loaderVersion === ''}<Check size={15}/>{/if}
        </button>
        {#each filteredLoaderVersions as entry, index (entry)}
          <button class="choice-row" class:chosen={ui.form.loaderVersion === entry} onclick={() => (ui.form.loaderVersion = entry)}>
            <span class="choice-main">
              <strong>{entry}</strong>
              {#if index === 0 && !versionQuery}<small class="recommend">最新</small>{/if}
            </span>
            {#if ui.form.loaderVersion === entry}<Check size={15}/>{/if}
          </button>
        {/each}
        {#if filteredLoaderVersions.length === 0}
          <div class="empty-state"><Package size={22}/><span>没有匹配的版本号。</span></div>
        {/if}
      </div>
    {/if}
    <div class="wizard-actions">
      <button class="button ghost" onclick={back}>上一步</button>
      <button class="button primary" disabled={loaderState?.error ? !ui.form.loaderVersion : false} onclick={nextFromVersion}>下一步</button>
    </div>
  </section>

{:else}
  <section class="panel">
    <div class="panel-head">
      <div><strong>确认安装</strong><span>创建实例后会立即开始下载，期间可以离开此页面</span></div>
    </div>
    <label>实例名称
      <input bind:value={ui.form.name} placeholder={suggestInstanceName(version, ui.form.loader)} />
    </label>
    <div class="confirm-grid">
      <div class="summary-row"><span>游戏版本</span><strong>{version}</strong></div>
      <div class="summary-row">
        <span>加载器</span>
        <strong>{loaderLabels[ui.form.loader]}{ui.form.loader !== 'vanilla' && ui.form.loaderVersion ? ` ${ui.form.loaderVersion}` : ''}</strong>
      </div>
      <div class="summary-row">
        <span>需要 Java</span>
        <strong class="with-icon"><Coffee size={14}/>{javaLabel}</strong>
      </div>
      <div class="summary-row"><span>使用角色</span><strong>{activeAccount()?.name ?? '尚未添加角色'}</strong></div>
      <div class="summary-row"><span>安装位置</span><strong class="path">{app.data.data_dir}/instances</strong></div>
    </div>
    <p class="help-text">{javaNote}。安装完成后实例独立保存存档与模组。</p>
    <div class="wizard-actions">
      {#if ui.form.loader === 'vanilla'}
        <button class="button ghost" onclick={back}>上一步</button>
      {:else}
        <button class="button ghost" onclick={backToStep1}>上一步</button>
      {/if}
      <button class="button primary" disabled={app.busy} onclick={createAndInstall}>
        {#if app.busy}<LoaderCircle class="spin" size={16}/>{:else}<Download size={16}/>{/if}创建并安装
      </button>
    </div>
    <div class="confirm-footnote">
      <Layers size={14}/>
      <span>安装会按官方元数据下载客户端、依赖库与资源，并在需要时准备 Java 运行时。</span>
    </div>
  </section>
{/if}
