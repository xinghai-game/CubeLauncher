<script lang="ts">
  import { ArrowUpRight, Cpu, Download, FolderInput, HardDrive, LoaderCircle, Play, Plus, ShieldCheck, Sparkles, Square } from 'lucide-svelte';
  import WorldArt from './WorldArt.svelte';
  import { app } from '../lib/state.svelte';
  import { loaderColors, loaderLabels, formatPlayed, isImported, shortPath } from '../types/api';
  import { activeAccount, goImport, goInstances, goSettings, goVersions, installInstance, launchSelected, openInstance, selectInstance, stopSelected } from '../lib/actions';

  const account = $derived(activeAccount());
  const running = $derived(new Set(app.data.running));
  const installed = $derived(app.data.instances.filter((item) => item.installed).length);
  const recent = $derived(app.data.instances[0] ?? null);
  const launching = $derived(Boolean(recent && app.launchingId === recent.id));
</script>

<div class="page-head home-heading">
  <div>
    <p class="eyebrow">你的游戏空间 / OVERVIEW</p>
    <h1>你好，{account?.name ?? '冒险家'}<span class="greeting-dot">.</span></h1>
    <p class="subtitle">一切就绪时，新的冒险只需一次点击。</p>
  </div>
  <button class="button ghost" onclick={() => goImport()}><FolderInput size={16}/>导入游戏</button>
</div>

<div class="home-dashboard">
  <section class="hero-card launch-hero">
    <div class="hero-glow"></div>
    <div class="hero-copy">
      <div class="hero-kicker"><Sparkles size={14}/>{recent ? '继续你的旅程' : '下一个世界，等你创造'}</div>
      <h2>{recent ? recent.name : '从一个方块，\n到无限可能。'}</h2>
      <p>{recent ? `${recent.game_version} · ${loaderLabels[recent.loader]}${recent.loader_version ? ` ${recent.loader_version}` : ''}` : '选择喜欢的版本，剩下的交给 CubeLauncher。'}</p>
      <div class="hero-buttons">
        {#if recent}
          {#if running.has(recent.id)}
            <button class="button hero-primary" onclick={() => { void selectInstance(recent.id); return stopSelected(); }}><Square size={16}/>停止游戏</button>
          {:else}
            <button class="button hero-primary" disabled={launching} onclick={() => { void selectInstance(recent.id); return recent.installed ? launchSelected() : installInstance(recent.id); }}>
              {#if launching}<LoaderCircle class="spin" size={17}/>{:else if recent.installed}<Play size={17}/>{:else}<Download size={17}/>{/if}
              {launching ? '正在启动' : recent.installed ? '启动游戏' : '安装实例'}
            </button>
          {/if}
          <button class="hero-link" onclick={() => openInstance(recent.id)}>管理实例 <ArrowUpRight size={16}/></button>
        {:else}
          <button class="button hero-primary" onclick={() => goVersions()}><Plus size={17}/>探索游戏版本</button>
        {/if}
      </div>
    </div>
    <WorldArt />
    <div class="hero-footer"><span class="status-dot"></span>{recent ? formatPlayed(recent.last_played) : '自动准备 Java · 依赖 · 游戏资源'}<span class="hero-edition">JAVA EDITION</span></div>
  </section>

  <aside class="home-rail" aria-label="启动环境">
    <div class="rail-heading"><span>启动环境</span><span class="rail-label">SYSTEM</span></div>
    <div class="environment-item"><span class="environment-icon"><ShieldCheck size={19}/></span><div><strong>离线也能畅玩</strong><small>已安装游戏，随时出发</small></div></div>
    <div class="environment-item"><span class="environment-icon"><Cpu size={19}/></span><div><strong>{app.data.java.length} 个 Java 运行时</strong><small>根据游戏版本自动选择</small></div></div>
    <div class="environment-item"><span class="environment-icon"><HardDrive size={19}/></span><div><strong>{app.data.settings.download_concurrency} 路并行下载</strong><small>下载后自动校验完整性</small></div></div>
    <button class="rail-settings" onclick={() => goSettings()}>管理启动器 <ArrowUpRight size={15}/></button>
  </aside>
</div>

<div class="section-title library-heading">
  <div><p class="eyebrow">YOUR COLLECTION</p><h3>游戏库 <span class="collection-count">{app.data.instances.length}</span></h3></div>
  <button class="text-button" onclick={() => goInstances()}>查看全部 <ArrowUpRight size={15}/></button>
</div>
<section class="home-library" aria-label="游戏实例">
  {#each app.data.instances.slice(0, 3) as instance (instance.id)}
    <button class="instance-card library-card" onclick={() => openInstance(instance.id)}>
      <div class="instance-top"><span class="loader-badge" style={`--loader-color:${loaderColors[instance.loader]}`}>{instance.loader === 'vanilla' ? '◇' : instance.loader === 'fabric' ? '✣' : '◈'}</span><span class="install-status" class:ready={instance.installed}>{running.has(instance.id) ? '运行中' : instance.installed ? '已安装' : '未安装'}</span></div>
      <div class="instance-card-name">{instance.name}</div>
      <div class="instance-card-meta">{instance.game_version} <span>·</span> {loaderLabels[instance.loader]}</div>
      {#if isImported(instance)}<div class="instance-card-meta imported-path" title={instance.game_dir ?? ''}>{shortPath(instance.game_dir ?? '')}</div>{/if}
      <div class="card-bottom"><span class="played">{formatPlayed(instance.last_played)}</span><ArrowUpRight size={16}/></div>
    </button>
  {/each}
  <button class="library-add" onclick={() => goVersions()}><span class="add-orbit"><Plus size={24}/></span><strong>{app.data.instances.length ? '新的冒险' : '创建你的第一个世界'}</strong><small>浏览原版与模组加载器</small><span class="add-link">安装新游戏 <ArrowUpRight size={14}/></span></button>
</section>
<div class="library-footnote"><span>{installed} 个已安装 · {running.size} 个运行中</span><span>你的世界，由你掌控。</span></div>
