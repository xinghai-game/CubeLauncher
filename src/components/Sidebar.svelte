<script lang="ts">
  import { ChevronDown, CloudOff, Download, House, Library, Settings as SettingsIcon, ShieldCheck, Zap } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import { activeAccount, goHome, goInstances, goSettings, goVersions, openAccounts } from '../lib/actions';
  import { skinHeadStyle } from '../types/api';

  const account = $derived(activeAccount());
  // Deeper pages keep their parent highlighted, like HMCL's nested navigation.
  const onVersions = $derived(['versions', 'wizard', 'install'].includes(ui.route.name));
  const onInstances = $derived(['instances', 'import'].includes(ui.route.name));
</script>

<aside class="sidebar">
  <button class="profile-card" onclick={openAccounts}>
    {#if account && app.skins[account.id]}
      <span class="skin-head" style={skinHeadStyle(app.skins[account.id])}></span>
    {:else}
      <span class="avatar">{(account?.name ?? 'A').slice(0, 1).toUpperCase()}</span>
    {/if}
    <span class="profile-copy">
      <strong>{account?.name ?? '添加角色'}</strong>
      {#if account?.kind === 'microsoft'}
        <small><ShieldCheck size={12}/> 正版账户</small>
      {:else}
        <small><CloudOff size={12}/> 离线身份</small>
      {/if}
    </span>
    <ChevronDown size={15} class="profile-chevron" />
  </button>
  <nav>
    <span class="nav-label">工作台</span>
    <button class:active={ui.route.name === 'home'} onclick={goHome}><House size={17}/>总览</button>
    <button class:active={onInstances} onclick={() => goInstances()}>
      <Library size={17}/>实例<span class="nav-count">{app.data.instances.length}</span>
    </button>
    <button class:active={onVersions} onclick={() => goVersions()}>
      <Download size={17}/>版本目录<span class="nav-count">{app.catalog?.total ?? '—'}</span>
    </button>
    <span class="nav-label secondary">管理</span>
    <button class:active={ui.route.name === 'settings'} onclick={() => goSettings()}>
      <SettingsIcon size={17}/>启动器设置
    </button>
  </nav>
  <div class="sidebar-bottom">
    <div class="resource-card">
      <div class="resource-icon"><Zap size={16}/></div>
      <div>
        <strong>{app.data.java.length} 个 Java 运行时</strong>
        <span>{app.data.settings.offline_mode ? '严格离线模式' : '仅按需联网'}</span>
      </div>
    </div>
    <div class="version-label">CUBELAUNCHER <span>0.1.0</span></div>
  </div>
</aside>
