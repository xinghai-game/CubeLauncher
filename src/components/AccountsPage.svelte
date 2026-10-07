<script lang="ts">
  import { ArrowUpRight, CloudOff, Plus, Search, ShieldCheck, Users } from 'lucide-svelte';
  import { app } from '../lib/state.svelte';
  import { formatDateTime, needsRefresh, skinHeadStyle, type Account } from '../types/api';
  import { goHome, openAccount, openAccounts, loadSkins } from '../lib/actions';
  import Breadcrumb from './Breadcrumb.svelte';

  let query = $state('');
  const accounts = $derived(app.data.accounts);
  const microsoftCount = $derived(accounts.filter((entry) => entry.kind === 'microsoft').length);
  const offlineCount = $derived(accounts.filter((entry) => entry.kind !== 'microsoft').length);
  const attentionCount = $derived(accounts.filter((entry) => needsRefresh(entry)).length);
  const filteredAccounts = $derived(
    [...accounts]
      .filter((entry) => {
        const keyword = query.trim().toLocaleLowerCase();
        return !keyword
          || entry.name.toLocaleLowerCase().includes(keyword)
          || entry.uuid.toLocaleLowerCase().includes(keyword);
      })
      .sort((left, right) => accountTime(right) - accountTime(left))
  );

  $effect(() => {
    if (accounts.length > 0) void loadSkins();
  });

  function accountTime(entry: Account): number {
    const value = entry.microsoft?.last_login ?? entry.created_at;
    const time = new Date(value).getTime();
    return Number.isNaN(time) ? 0 : time;
  }

  function status(entry: Account): string {
    if (entry.kind !== 'microsoft') return '本机离线角色';
    if (needsRefresh(entry)) {
      return entry.microsoft?.refreshable ? '登录已过期，可刷新' : '登录已过期，需重新登录';
    }
    return '正版登录有效';
  }

  function detail(entry: Account): string {
    if (entry.kind === 'microsoft' && entry.microsoft?.last_login) {
      return `上次登录 ${formatDateTime(entry.microsoft.last_login)}`;
    }
    return `创建于 ${formatDateTime(entry.created_at)}`;
  }
</script>

<Breadcrumb
  items={[
    { label: '总览', to: { name: 'home' } },
    { label: '角色管理' }
  ]}
/>

<div class="page-head compact account-directory-head">
  <div>
    <p class="eyebrow">身份中心 · {accounts.length} 个角色</p>
    <h1>角色管理</h1>
    <p class="subtitle">每个角色都有独立详情页，可以查看登录状态、绑定实例和身份信息。</p>
  </div>
  <div class="head-actions">
    <button class="button ghost" onclick={goHome}>返回总览</button>
    <button class="button primary" onclick={openAccounts}><Plus size={16}/>添加角色</button>
  </div>
</div>

<div class="account-directory-stats account-stats">
  <div class="account-stat">
    <span>全部角色</span>
    <strong>{accounts.length}</strong>
    <small>点击卡片进入独立页面</small>
  </div>
  <div class="account-stat account-stat-online">
    <span>正版账户</span>
    <strong>{microsoftCount}</strong>
    <small>皮肤、正版验证和自动续期</small>
  </div>
  <div class="account-stat account-stat-offline">
    <span>离线角色</span>
    <strong>{offlineCount}</strong>
    <small>无需联网即可使用</small>
  </div>
  <div class="account-stat account-stat-alert">
    <span>需要处理</span>
    <strong>{attentionCount}</strong>
    <small>{attentionCount ? '有角色需要刷新或重新登录' : '当前登录状态正常'}</small>
  </div>
</div>

<div class="account-directory-toolbar">
  <div>
    <h2>全部角色</h2>
    <span>{filteredAccounts.length === accounts.length ? `共 ${accounts.length} 个` : `显示 ${filteredAccounts.length} / ${accounts.length} 个`}</span>
  </div>
  <label class="search-field account-directory-search">
    <Search size={15}/>
    <input aria-label="搜索角色" placeholder="搜索名称或 UUID" value={query} oninput={(event) => (query = event.currentTarget.value)} />
  </label>
</div>

{#if accounts.length === 0}
  <section class="empty-state big account-directory-empty">
    <Users size={28}/>
    <strong>还没有角色</strong>
    <span>登录 Microsoft 正版账户，或添加一个只在本机使用的离线角色。</span>
    <button class="button primary" onclick={openAccounts}><Plus size={16}/>添加第一个角色</button>
  </section>
{:else if filteredAccounts.length === 0}
  <section class="empty-state big account-directory-empty">
    <Search size={28}/>
    <strong>没有匹配的角色</strong>
    <span>试试其他名称或 UUID。</span>
    <button class="button ghost" onclick={() => (query = '')}>清除搜索</button>
  </section>
{:else}
  <section class="account-directory-grid">
    {#each filteredAccounts as entry (entry.id)}
      <button class="account-directory-card" onclick={() => openAccount(entry.id)}>
        <div class="account-directory-card-head">
          {#if app.skins[entry.id]}
            <span class="skin-head account-directory-avatar" style={skinHeadStyle(app.skins[entry.id])}></span>
          {:else}
            <span class="avatar account-directory-avatar">{entry.name.slice(0, 1).toUpperCase()}</span>
          {/if}
          <span class="account-directory-card-arrow"><ArrowUpRight size={16}/></span>
        </div>
        <div class="account-directory-name">{entry.name}</div>
        <div class="account-directory-kind">
          <span class:online={entry.kind === 'microsoft'} class="account-badge">
            {#if entry.kind === 'microsoft'}<ShieldCheck size={11}/>{:else}<CloudOff size={11}/>{/if}
            {entry.kind === 'microsoft' ? '正版账户' : '离线角色'}
          </span>
          {#if needsRefresh(entry)}<span class="account-directory-warning">需处理</span>{/if}
        </div>
        <div class="account-directory-status">{status(entry)}</div>
        <div class="account-directory-meta">{detail(entry)}</div>
      </button>
    {/each}
  </section>
{/if}

<p class="account-directory-footnote"><ShieldCheck size={14}/>登录令牌仍只保存在本机账户文件中，角色详情页不会显示任何令牌。</p>
