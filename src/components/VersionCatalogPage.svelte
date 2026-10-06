<script lang="ts">
  import { Archive, CircleAlert, Download, LoaderCircle, RefreshCw, Search, Sparkles } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import { formatDateTime, kindLabels, loaderLabels, manifestSourceLabels } from '../types/api';
  import {
    countKinds, filterOrder, filterLabels, filterVersions, formatReleaseDate, installedIndex,
    type KindFilter
  } from '../lib/versions';
  import { goWizard, refreshCatalog } from '../lib/actions';

  const versions = $derived(app.catalog?.versions ?? []);
  const counts = $derived(countKinds(versions));
  const kind = $derived(ui.route.name === 'versions' ? ui.route.kind : 'release');
  const query = $derived(ui.route.name === 'versions' ? ui.route.query : '');
  const filtered = $derived(filterVersions(versions, kind, query));
  const visible = $derived(filtered.slice(0, ui.visibleVersions));
  const installed = $derived(installedIndex(app.data.instances));
  const sourceLabel = $derived(app.catalog ? manifestSourceLabels[app.catalog.source] : '—');

  function setKind(next: KindFilter) {
    if (ui.route.name !== 'versions') return;
    ui.route.kind = next;
    ui.visibleVersions = 60;
  }

  function setQuery(value: string) {
    if (ui.route.name !== 'versions') return;
    ui.route.query = value;
    ui.visibleVersions = 60;
  }
</script>

<div class="page-head compact">
  <div>
    <p class="eyebrow">官方清单 · version_manifest_v2</p>
    <h1>版本目录</h1>
    <p class="subtitle">
      版本列表按需从官方清单动态获取，并按 HMCL 的分类方式整理；选中版本后进入安装向导。
    </p>
  </div>
  <div class="head-actions">
    <button class="icon-button" title="刷新版本目录" onclick={() => refreshCatalog(true)}>
      <RefreshCw size={16} class={app.catalogLoading ? 'spin' : ''}/>
    </button>
  </div>
</div>

<div class="catalog-status">
  <span class="status-chip" class:live={app.catalog?.source !== 'cache'}>
    {#if app.catalogLoading}<LoaderCircle class="spin" size={13}/>{:else}<Download size={13}/>{/if}
    {sourceLabel}
  </span>
  {#if app.catalog}
    <span>共 <b>{app.catalog.total}</b> 个版本</span>
    <span class="divider"></span>
    <span>最新正式版 <b>{app.catalog.latest.release}</b></span>
    <span class="divider"></span>
    <span>最新快照 <b>{app.catalog.latest.snapshot}</b></span>
    {#if app.catalog.fetched_at}
      <span class="divider"></span>
      <span class="muted">{app.catalog.cached ? '缓存于' : '更新于'} {formatDateTime(app.catalog.fetched_at)}</span>
    {/if}
  {:else if !app.catalogLoading}
    <span class="muted">还没有版本列表</span>
  {/if}
</div>

{#if app.catalogError}
  <div class="inline-alert">
    <CircleAlert size={16}/>
    <div><strong>读取版本列表失败</strong><span>{app.catalogError}</span></div>
    <button class="button ghost small" onclick={() => refreshCatalog(true)}>重试</button>
  </div>
{/if}

<div class="filter-bar">
  <div class="kind-tabs">
    {#each filterOrder as option (option)}
      <button class:active={kind === option} onclick={() => setKind(option)}>
        {filterLabels[option]}<span class="kind-count">{counts[option]}</span>
      </button>
    {/each}
  </div>
  <label class="search-field">
    <Search size={15}/>
    <input placeholder="搜索版本号，例如 1.20.1" value={query} oninput={(event) => setQuery(event.currentTarget.value)} />
  </label>
</div>

<p class="filter-hint">{filterLabels[kind]} · {filtered.length} 个版本 · 按发布时间从新到旧</p>

<div class="version-table">
  <div class="table-head catalog-head"><span>版本</span><span>类型</span><span>发布时间</span><span>状态</span><span></span></div>
  {#if app.catalogLoading && versions.length === 0}
    <div class="empty-state"><LoaderCircle class="spin" size={25}/><strong>正在获取版本列表</strong><span>首次获取会读取官方清单，之后直接使用本地缓存。</span></div>
  {:else if visible.length === 0}
    <div class="empty-state">
      <Archive size={25}/><strong>{query ? '没有匹配的版本' : '这个分类暂时没有版本'}</strong>
      <span>{query ? `试试其他关键字，或切换到“全部”。` : '点击右上角刷新，从官方清单重新获取。'}</span>
    </div>
  {:else}
    {#each visible as version (version.id)}
      <button class="table-row catalog-row" onclick={() => goWizard(version.id)}>
        <strong>{version.id}</strong>
        <span class={`kind-pill k-${version.kind}`}>{kindLabels[version.kind]}</span>
        <span>{formatReleaseDate(version.releaseTime)}</span>
        <span class="row-state">
          {#if installed[version.id]?.length}
            {#each installed[version.id].slice(0, 1) as instance (instance.id)}
              <span class="installed-badge">已安装 · {instance.name}</span>
            {/each}
          {:else}
            <span class="muted">未安装</span>
          {/if}
        </span>
        <span class="row-action">安装 <span>→</span></span>
      </button>
    {/each}
    {#if filtered.length > visible.length}
      <button class="load-more" onclick={() => (ui.visibleVersions += 120)}>
        显示更多（还有 {filtered.length - visible.length} 个版本）
      </button>
    {/if}
  {/if}
</div>

<div class="catalog-footnote">
  <Sparkles size={14}/>
  <span>
    原版与四种加载器的版本都会在安装向导里动态查询：
    {Object.values(loaderLabels).join(' / ')}。
  </span>
</div>
