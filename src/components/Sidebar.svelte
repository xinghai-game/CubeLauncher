<script lang="ts">
  import { ArrowUpRight, Box, CloudOff, Download, House, Library, Settings as SettingsIcon, ShieldCheck } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import { activeAccount, goAccounts, goHome, goInstances, goSettings, goVersions } from '../lib/actions';
  import { skinHeadStyle } from '../types/api';
  const account = $derived(activeAccount());
  const onVersions = $derived(['versions', 'wizard', 'install'].includes(ui.route.name));
  const onInstances = $derived(['instances', 'instance', 'import'].includes(ui.route.name));
</script>

<aside class="sidebar">
  <button class="sidebar-brand" onclick={goHome} aria-label="CubeLauncher 首页"><span class="brand-mark"><Box size={23}/></span><span>CubeLauncher<small>为每一次冒险</small></span></button>
  <nav aria-label="主要导航">
    <span class="nav-label">探索 / EXPLORE</span>
    <button class:active={ui.route.name === 'home'} aria-current={ui.route.name === 'home' ? 'page' : undefined} onclick={goHome}><House size={19}/>总览</button>
    <button class:active={onInstances} aria-current={onInstances ? 'page' : undefined} onclick={() => goInstances()}><Library size={19}/>游戏库<span class="nav-count">{app.data.instances.length}</span></button>
    <button class:active={onVersions} aria-current={onVersions ? 'page' : undefined} onclick={() => goVersions()}><Download size={19}/>探索版本</button>
    <span class="nav-label secondary">偏好 / PREFERENCES</span>
    <button class:active={['accounts', 'account'].includes(ui.route.name)} onclick={goAccounts}><ShieldCheck size={19}/>账户管理</button>
    <button class:active={ui.route.name === 'settings'} onclick={() => goSettings()}><SettingsIcon size={19}/>启动器设置</button>
  </nav>
  <div class="sidebar-bottom">
    <div class="sidebar-note"><span class="status-dot"></span>{app.data.settings.offline_mode ? '严格离线模式' : '离线优先，按需联网'}</div>
    <button class="profile-card" onclick={goAccounts}>
      {#if account && app.skins[account.id]}<span class="skin-head" style={skinHeadStyle(app.skins[account.id])}></span>{:else}<span class="avatar">{(account?.name ?? 'A').slice(0, 1).toUpperCase()}</span>{/if}
      <span class="profile-copy"><strong>{account?.name ?? '添加游戏角色'}</strong><small>{#if account?.kind === 'microsoft'}<ShieldCheck size={12}/>正版账户{:else}<CloudOff size={12}/>离线身份{/if}</small></span>
      <ArrowUpRight size={15} class="profile-chevron"/>
    </button>
    <div class="version-label">CUBE / JAVA <span>v0.1.0</span></div>
  </div>
</aside>
