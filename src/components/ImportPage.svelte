<script lang="ts">
  import {
    Archive, Check, CircleAlert, Download, FolderOpen, HardDrive, LoaderCircle, Package,
    Search, Sparkles
  } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import {
    instanceGameDir, loaderColors, loaderLabels, shortPath, type GameDirVersion
  } from '../types/api';
  import {
    activeAccount, back, chooseImportVersion, confirmImport, goInstances, pickGameDir, scanGameDir
  } from '../lib/actions';
  import Breadcrumb from './Breadcrumb.svelte';

  const binding = $derived(ui.route.name === 'import' ? ui.route.instanceId : null);
  const target = $derived(binding ? app.data.instances.find((item) => item.id === binding) ?? null : null);
  const flow = $derived(ui.importFlow);
  const chosen = $derived(flow.scan?.versions.find((version) => version.id === flow.versionId) ?? null);
  const selectable = $derived((flow.scan?.versions ?? []).filter((version) => version.launchable));

  function hint(version: GameDirVersion): string {
    const parts = [
      version.game_version,
      `${loaderLabels[version.loader]}${version.loader_version ? ` ${version.loader_version}` : ''}`
    ];
    if (version.inherits_from) parts.push(`继承 ${version.inherits_from}`);
    if (!version.jar) parts.push('目录内没有同名 JAR');
    return parts.join(' · ');
  }
</script>

<Breadcrumb
  items={[
    { label: '实例管理', to: { name: 'instances', tab: 'overview' } },
    { label: binding ? '更换游戏目录' : '导入已有 .minecraft' }
  ]}
/>

<div class="page-head compact">
  <div>
    <p class="eyebrow">游戏目录 · {binding ? '重新绑定' : '导入'}</p>
    <h1>{binding ? `为 ${target?.name ?? '实例'} 更换 .minecraft` : '导入已有 .minecraft'}</h1>
    <p class="subtitle">
      直接使用官启、HMCL、PCL 等已有目录，不复制文件：存档、模组与依赖库都留在原地，
      启动时按该目录里的版本文件解析，离线也能进游戏。
    </p>
  </div>
  {#if !binding}
    <button class="button ghost" onclick={() => goInstances()}><Archive size={16}/>返回实例</button>
  {/if}
</div>

<section class="panel">
  <div class="panel-head">
    <div>
      <strong>1 · 选择目录</strong>
      <span>可以选择 .minecraft 本身，也可以选包含它的游戏文件夹</span>
    </div>
    <button class="button ghost small" onclick={pickGameDir}><FolderOpen size={14}/>浏览目录</button>
  </div>
  <div class="path-picker">
    <input
      bind:value={ui.importFlow.path}
      placeholder="/home/you/.minecraft 或 D:\\Minecraft"
      onkeydown={(event) => event.key === 'Enter' && scanGameDir()}
    />
    <button class="button primary" disabled={flow.scanning} onclick={() => scanGameDir()}>
      {#if flow.scanning}<LoaderCircle class="spin" size={15}/>{:else}<Search size={15}/>{/if}扫描
    </button>
  </div>
  {#if flow.error}
    <div class="inline-alert">
      <CircleAlert size={16}/>
      <div><strong>无法读取这个目录</strong><span>{flow.error}</span></div>
    </div>
  {/if}
  {#if flow.scan}
    <div class="confirm-grid">
      <div class="summary-row">
        <span>游戏目录</span><strong class="path">{flow.scan.game_dir}</strong>
      </div>
      <div class="summary-row">
        <span>读取方式</span>
        <strong>{flow.scan.nested ? '取用了内层 .minecraft' : '直接使用所选目录'}</strong>
      </div>
      <div class="summary-row">
        <span>可导入版本</span><strong>{selectable.length} / {flow.scan.versions.length}</strong>
      </div>
      <div class="summary-row">
        <span>依赖与资源</span>
        <strong>
          {flow.scan.has_libraries ? '自带 libraries' : '缺少 libraries'} ·
          {flow.scan.has_assets ? '自带 assets' : '缺少 assets'}
        </strong>
      </div>
      <div class="summary-row">
        <span>目录内内容</span>
        <strong>{flow.scan.mod_count} 个模组 · {flow.scan.save_count} 个存档</strong>
      </div>
      <div class="summary-row">
        <span>{binding ? '当前游戏目录' : '实例目录'}</span>
        <strong class="path">
          {binding && target
            ? instanceGameDir(target, app.data.data_dir)
            : `${app.data.data_dir}/instances`}
        </strong>
      </div>
    </div>
    {#if !flow.scan.has_libraries || !flow.scan.has_assets}
      <p class="help-text">
        该目录没有自带 libraries 或 assets：导入后仍可启动，缺失的依赖会下载到启动器数据目录，
        不会改写原目录里已有的文件。
      </p>
    {/if}
  {/if}
</section>

{#if flow.scan}
  <section class="panel">
    <div class="panel-head">
      <div>
        <strong>2 · 选择要导入的版本</strong>
        <span>列表来自该目录的 versions/ 文件夹，越靠上越新</span>
      </div>
    </div>
    {#if flow.scan.versions.length === 0}
      <div class="empty-state">
        <Package size={22}/>
        <strong>没有找到版本</strong>
        <span>versions/ 里没有可解析的 &lt;版本&gt;/&lt;版本&gt;.json。</span>
      </div>
    {:else}
      <div class="choice-list">
        {#each flow.scan.versions as version (version.id)}
          <button
            class="choice-row"
            class:chosen={flow.versionId === version.id}
            disabled={!version.launchable}
            onclick={() => chooseImportVersion(version.id)}
          >
            <span class="loader-badge" style={`--loader-color:${loaderColors[version.loader]}`}>
              {version.loader === 'vanilla' ? '◇' : version.loader === 'fabric' ? '✣' : '◈'}
            </span>
            <span class="choice-main">
              <strong>{version.id}</strong>
              <small>{hint(version)}</small>
              {#if version.note}<small class="recommend">{version.note}</small>{/if}
            </span>
            {#if version.imported}<span class="list-state">已导入</span>{/if}
            {#if version.jar}<span class="list-state ready">含 JAR</span>{:else}<span class="list-state">无 JAR</span>{/if}
            {#if !version.launchable}<span class="list-state">不可启动</span>{/if}
            {#if flow.versionId === version.id}<Check size={15}/>{/if}
          </button>
        {/each}
      </div>
    {/if}
  </section>

  {#if chosen}
    <section class="panel">
      <div class="panel-head">
        <div>
          <strong>3 · 确认</strong>
          <span>{binding ? '该实例会改用所选目录的这个版本' : '创建实例后即可直接启动，不需要下载游戏本体'}</span>
        </div>
      </div>
      {#if binding}
        <div class="inline-alert">
          <Sparkles size={16}/>
          <div>
            <strong>更换目录不会搬移文件</strong>
            <span>当前的存档与模组留在原目录；切换后该实例使用新目录里的存档与模组。</span>
          </div>
        </div>
      {:else}
        <label>实例名称
          <input bind:value={ui.importFlow.name} placeholder={chosen.id} />
        </label>
      {/if}
      <div class="confirm-grid">
        <div class="summary-row">
          <span>版本</span><strong>{chosen.id}</strong>
        </div>
        <div class="summary-row">
          <span>游戏版本</span><strong>{chosen.game_version}</strong>
        </div>
        <div class="summary-row">
          <span>加载器</span>
          <strong>
            {loaderLabels[chosen.loader]}{chosen.loader_version ? ` ${chosen.loader_version}` : ''}
          </strong>
        </div>
        <div class="summary-row">
          <span>使用角色</span>
          <strong>
            {binding
              ? app.data.accounts.find((item) => item.id === target?.account_id)?.name
                ?? activeAccount()?.name ?? '尚未添加角色'
              : app.data.accounts.find((item) => item.id === flow.accountId)?.name
                ?? activeAccount()?.name ?? '尚未添加角色'}
          </strong>
        </div>
        <div class="summary-row">
          <span>游戏目录</span><strong class="path">{flow.scan.game_dir}</strong>
        </div>
        <div class="summary-row">
          <span>目录简写</span><strong>{shortPath(flow.scan.game_dir)}</strong>
        </div>
      </div>
      <p class="help-text">
        导入后可用“校验”确认文件齐全，用“修复/更新”补齐缺失的依赖；这些操作不会删除原目录里的东西。
      </p>
      <div class="wizard-actions">
        <button class="button ghost" onclick={back}>返回</button>
        <button class="button primary" disabled={app.busy} onclick={confirmImport}>
          {#if app.busy}<LoaderCircle class="spin" size={16}/>{:else}<Download size={16}/>{/if}
          {binding ? '切换到该版本' : '导入为实例'}
        </button>
      </div>
      <div class="confirm-footnote">
        <HardDrive size={14}/>
        <span>文件保持原位：启动器只新增 instances/&lt;实例&gt;/ 下的原生库与日志。</span>
      </div>
    </section>
  {/if}
{/if}
