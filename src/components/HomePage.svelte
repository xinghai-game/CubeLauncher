<script lang="ts">
  import {
    Cpu, Download, FolderInput, HardDrive, LoaderCircle, Play, Plus, ShieldCheck, Sparkles, Square
  } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import { loaderColors, loaderLabels, formatPlayed, isImported, shortPath } from '../types/api';
  import {
    activeAccount, goImport, goInstances, goVersions, goWizard, installInstance, launchSelected,
    openInstance, selectInstance, stopSelected
  } from '../lib/actions';

  const account = $derived(activeAccount());
  const running = $derived(new Set(app.data.running));
  const installed = $derived(app.data.instances.filter((item) => item.installed).length);
  const recent = $derived(app.data.instances[0] ?? null);
  // Waiting for the game process to exit must not disable the page: only the
  // moment between the click and the process state event counts as launching.
  const launching = $derived(Boolean(recent && app.launchingId === recent.id));

  async function openRecent(id: string) {
    await openInstance(id);
  }
</script>

<div class="page-head">
  <div>
    <p class="eyebrow">离线优先 · 低占用</p>
    <h1>你好，<span class="headline-accent">{account?.name ?? '冒险家'}</span></h1>
    <p class="subtitle">安装一次之后，断网也能直接进入世界。</p>
  </div>
  <div class="head-actions">
    <button class="button ghost" onclick={() => goImport()}><FolderInput size={16}/>导入 .minecraft</button>
    <button class="button primary" onclick={() => goVersions()}><Plus size={17}/>安装新游戏</button>
  </div>
</div>

{#if app.data.instances.length === 0}
  <section class="hero-card">
    <div class="hero-glow"></div>
    <div class="hero-copy">
      <div class="hero-kicker"><Sparkles size={14}/> 从零开始</div>
      <h2>安装第一个游戏</h2>
      <p>从版本目录挑一个游戏版本，再选加载器，启动器会自动准备 Java、依赖与资源。</p>
      <div class="hero-buttons">
        <button class="button hero-primary" onclick={() => goVersions()}><Plus size={16}/>浏览版本目录</button>
        <button class="button ghost" onclick={() => goImport()}><FolderInput size={16}/>导入已有 .minecraft</button>
      </div>
      <p class="help-text">已经用官方启动器、HMCL 或 PCL 玩过？导入那个目录即可，文件不会被复制。</p>
    </div>
    <div class="hero-art">
      <div class="sun"></div><div class="mountain mountain-back"></div>
      <div class="mountain mountain-front"></div><div class="pixel-tree tree-one"></div>
      <div class="pixel-tree tree-two"></div><div class="pixel-ground"></div>
    </div>
  </section>
{:else if recent}
  <section class="hero-card">
    <div class="hero-glow"></div>
    <div class="hero-copy">
      <div class="hero-kicker"><Sparkles size={14}/> 最近游玩</div>
      <h2>{recent.name}</h2>
      <p>{recent.game_version} · {loaderLabels[recent.loader]}{recent.loader_version ? ` ${recent.loader_version}` : ''}</p>
      <div class="hero-buttons">
        {#if running.has(recent.id)}
          <button class="button hero-primary" onclick={() => { void selectInstance(recent.id); return stopSelected(); }}>
            <Square size={15}/>停止游戏
          </button>
        {:else}
          <button
            class="button hero-primary"
            disabled={launching}
            onclick={() => {
              void selectInstance(recent.id);
              return recent.installed ? launchSelected() : installInstance(recent.id);
            }}
          >
            {#if launching}<LoaderCircle class="spin" size={16}/>
            {:else if recent.installed}<Play size={16}/>
            {:else}<Download size={16}/>{/if}
            {recent.installed ? '启动游戏' : '安装实例'}
          </button>
        {/if}
        <button class="button ghost" onclick={() => openRecent(recent.id)}>管理实例 <span>→</span></button>
      </div>
    </div>
    <div class="hero-art">
      <div class="sun"></div><div class="mountain mountain-back"></div>
      <div class="mountain mountain-front"></div><div class="pixel-tree tree-one"></div>
      <div class="pixel-tree tree-two"></div><div class="pixel-ground"></div>
    </div>
  </section>
{/if}

<div class="section-title">
  <div>
    <h3>你的实例</h3>
    <span>{installed} 个已安装 · 共 {app.data.instances.length} 个</span>
  </div>
  <button class="text-button" onclick={() => goInstances()}>查看全部 <span>→</span></button>
</div>

<section class="instance-grid">
  {#each app.data.instances.slice(0, 3) as instance (instance.id)}
    <button class="instance-card" class:selected={ui.selectedId === instance.id} onclick={() => openRecent(instance.id)}>
      <div class="instance-top">
        <span class="loader-badge" style={`--loader-color:${loaderColors[instance.loader]}`}>
          {instance.loader === 'vanilla' ? '◇' : instance.loader === 'fabric' ? '✣' : '◈'}
        </span>
        <span class:ready={instance.installed} class="install-status">
          {running.has(instance.id) ? '运行中' : instance.installed ? '已安装' : '未安装'}
        </span>
      </div>
      <div class="instance-card-name">{instance.name}</div>
      <div class="instance-card-meta">
        <span>{instance.game_version}</span><span class="meta-sep">·</span><span>{loaderLabels[instance.loader]}</span>
      </div>
      {#if isImported(instance)}
        <div class="instance-card-meta">
          <span class="played">外部目录 {shortPath(instance.game_dir ?? '')}</span>
        </div>
      {/if}
      <div class="card-bottom">
        <span class="played">{formatPlayed(instance.last_played)}</span>
        <span class="played">{instance.max_memory_mb} MB</span>
      </div>
    </button>
  {/each}
  <button class="new-card" onclick={() => goVersions()}>
    <span><Plus size={20}/></span><strong>安装新游戏</strong>
    <small>原版、Fabric、Forge 或 NeoForge</small>
  </button>
  <button class="new-card" onclick={() => goImport()}>
    <span><FolderInput size={20}/></span><strong>导入 .minecraft</strong>
    <small>直接使用已有目录，不复制文件</small>
  </button>
</section>

<section class="quick-stats">
  <div>
    <span class="stat-icon green"><ShieldCheck size={17}/></span>
    <div><strong>离线校验</strong><small>启动前检查缺失文件</small></div>
  </div>
  <div>
    <span class="stat-icon orange"><Cpu size={17}/></span>
    <div><strong>{app.data.java.length} 个运行时</strong><small>按版本自动选择 Java</small></div>
  </div>
  <div>
    <span class="stat-icon purple"><HardDrive size={17}/></span>
    <div><strong>{app.data.settings.download_concurrency} 路下载</strong><small>校验通过才写入缓存</small></div>
  </div>
</section>
