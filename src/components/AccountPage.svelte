<script lang="ts">
  import {
    Check, ChevronRight, Copy, FolderOpen, KeyRound, LoaderCircle,
    RefreshCw, ShieldCheck, Trash2, UserRound, Users
  } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import {
    activeAccount, chooseAccount, copyAccountUuid, goAccounts, goInstances, loadSkin, openAccounts,
    openInstance, refreshAccount, removeAccount
  } from '../lib/actions';
  import {
    formatDateTime, formatPlayed, loaderColors, loaderLabels, needsRefresh, skinHeadStyle, type Account
  } from '../types/api';
  import Breadcrumb from './Breadcrumb.svelte';

  const accountId = $derived(ui.route.name === 'account' ? ui.route.accountId : '');
  const account = $derived(app.data.accounts.find((entry) => entry.id === accountId) ?? null);
  const instances = $derived(account
    ? app.data.instances.filter((instance) => instance.account_id === account.id)
    : []);
  const current = $derived(Boolean(account && activeAccount()?.id === account.id));
  let refreshing = $state(false);
  let confirmingDelete = $state(false);

  $effect(() => {
    if (account) void loadSkin(account);
  });

  function status(entry: Account): string {
    if (entry.kind !== 'microsoft') return '本机离线角色，无需登录或令牌';
    if (needsRefresh(entry)) {
      return entry.microsoft?.refreshable ? '登录已过期，可以使用本机刷新令牌恢复' : '登录已过期，需要重新登录';
    }
    return 'Microsoft 登录有效，可以进行正版验证';
  }

  function expiry(entry: Account): string {
    if (entry.kind !== 'microsoft' || !entry.microsoft) return '离线角色无需登录期限';
    if (needsRefresh(entry)) return entry.microsoft.refreshable ? '已过期 · 可自动刷新' : '已过期 · 需要重新登录';
    return `有效至 ${formatDateTime(entry.microsoft.expires_at)}`;
  }

  function instanceLabel(instance: typeof app.data.instances[number]): string {
    return `${instance.game_version} · ${loaderLabels[instance.loader]}${instance.loader_version ? ` ${instance.loader_version}` : ''}`;
  }

  async function renew() {
    if (!account || refreshing) return;
    refreshing = true;
    try {
      await refreshAccount(account);
    } finally {
      refreshing = false;
    }
  }

  async function deleteCurrentAccount() {
    if (!account) return;
    const id = account.id;
    confirmingDelete = false;
    await removeAccount(account);
    if (!app.data.accounts.some((entry) => entry.id === id)) goAccounts();
  }
</script>

<Breadcrumb
  items={[
    { label: '角色管理', to: { name: 'accounts' } },
    { label: account?.name ?? '角色详情' }
  ]}
>
  {#snippet actions()}
    {#if account}
      <button class="button ghost small" onclick={openAccounts}><Users size={14}/>管理角色</button>
    {/if}
  {/snippet}
</Breadcrumb>

{#if !account}
  <section class="panel account-missing-panel">
    <div class="empty-state">
      <UserRound size={28}/>
      <strong>角色不存在</strong>
      <span>这个角色可能刚刚被删除，返回角色管理查看其他角色。</span>
      <button class="button primary" onclick={goAccounts}><Users size={16}/>返回角色管理</button>
    </div>
  </section>
{:else}
  <div class="page-head compact account-detail-head">
    <div class="account-detail-title">
      {#if app.skins[account.id]}
        <span class="skin-head account-detail-avatar" style={skinHeadStyle(app.skins[account.id])}></span>
      {:else}
        <span class="avatar account-detail-avatar">{account.name.slice(0, 1).toUpperCase()}</span>
      {/if}
      <div>
        <p class="eyebrow">角色详情 · {account.kind === 'microsoft' ? '正版账户' : '离线角色'}</p>
        <h1>{account.name}</h1>
        <p class="subtitle">{status(account)} · 绑定 {instances.length} 个实例</p>
      </div>
    </div>
    <div class="detail-actions account-detail-actions">
      <button class="button ghost" onclick={() => copyAccountUuid(account)}><Copy size={15}/>复制 UUID</button>
      <button class="button primary" class:account-active-button={current} onclick={() => chooseAccount(account)}>
        {#if current}<Check size={15}/>{:else}<ShieldCheck size={15}/>{/if}{current ? '当前角色' : '用于当前实例'}
      </button>
      {#if account.kind === 'microsoft'}
        <button class="button ghost" disabled={refreshing} onclick={renew}>
          {#if refreshing}<LoaderCircle class="spin" size={15}/>{:else}<RefreshCw size={15}/>{/if}刷新登录
        </button>
      {/if}
      <button class="button ghost account-delete-trigger" onclick={() => (confirmingDelete = !confirmingDelete)}>
        <Trash2 size={15}/>删除
      </button>
    </div>
  </div>

  {#if confirmingDelete}
    <div class="inline-alert account-delete-alert">
      <Trash2 size={16}/>
      <div><strong>确认删除“{account.name}”？</strong><span>绑定实例会保留，但这个角色和本机登录令牌会被删除。</span></div>
      <button class="button ghost small" onclick={() => (confirmingDelete = false)}>取消</button>
      <button class="button danger small" onclick={deleteCurrentAccount}>确认删除</button>
    </div>
  {/if}

  <div class="account-detail-grid">
    <section class="panel account-detail-card">
      <div class="panel-head">
        <div><strong>身份信息</strong><span>启动游戏时会使用这组身份数据</span></div>
        <span class="account-detail-icon"><UserRound size={16}/></span>
      </div>
      <div class="account-detail-values">
        <div class="account-detail-value account-detail-value-wide">
          <span>UUID</span>
          <strong>{account.uuid}</strong>
          <button class="icon-button" title="复制 UUID" aria-label="复制 UUID" onclick={() => copyAccountUuid(account)}><Copy size={14}/></button>
        </div>
        <div class="account-detail-value">
          <span>角色类型</span>
          <strong><em class="account-badge" class:online={account.kind === 'microsoft'}>{account.kind === 'microsoft' ? '正版账户' : '离线角色'}</em></strong>
        </div>
        <div class="account-detail-value">
          <span>创建时间</span>
          <strong>{formatDateTime(account.created_at)}</strong>
        </div>
      </div>
    </section>

    <section class="panel account-detail-card">
      <div class="panel-head">
        <div><strong>{account.kind === 'microsoft' ? '登录与安全' : '离线身份说明'}</strong><span>{account.kind === 'microsoft' ? '令牌状态只显示摘要，不会暴露凭证' : '离线角色只在本机生成身份'}</span></div>
        <span class="account-detail-icon"><KeyRound size={16}/></span>
      </div>
      <div class="account-detail-values">
        <div class="account-detail-value account-detail-value-wide">
          <span>当前状态</span>
          <strong class:account-warning={needsRefresh(account)}>{expiry(account)}</strong>
        </div>
        {#if account.kind === 'microsoft'}
          <div class="account-detail-value">
            <span>上次登录</span>
            <strong>{formatDateTime(account.microsoft?.last_login)}</strong>
          </div>
          <div class="account-detail-value">
            <span>Java 权限</span>
            <strong>{account.microsoft?.owns_java ? '已拥有' : '未检测到'}</strong>
          </div>
          <div class="account-detail-value">
            <span>自动续期</span>
            <strong>{account.microsoft?.refreshable ? '可用' : '不可用'}</strong>
          </div>
        {:else}
          <div class="account-detail-value account-detail-value-wide">
            <span>适用范围</span>
            <strong>单人世界与允许离线登录的服务器</strong>
          </div>
        {/if}
      </div>
    </section>
  </div>

  <section class="panel account-bound-instances">
    <div class="panel-head">
      <div><strong>绑定实例</strong><span>使用这个角色启动的实例会显示在这里</span></div>
      <button class="button ghost small" onclick={() => goInstances()}><FolderOpen size={14}/>管理实例</button>
    </div>
    {#if instances.length === 0}
      <div class="account-instance-empty">
        <span class="account-detail-icon"><FolderOpen size={16}/></span>
        <div><strong>还没有绑定实例</strong><small>打开实例详情，在角色选择中将实例绑定到 {account.name}。</small></div>
        <button class="button ghost small" onclick={() => goInstances()}>查看实例</button>
      </div>
    {:else}
      <div class="account-instance-list">
        {#each instances as instance (instance.id)}
          <button class="account-instance-row" onclick={() => openInstance(instance.id)}>
            <span class="loader-badge" style={`--loader-color:${loaderColors[instance.loader]}`}>
              {instance.loader === 'vanilla' ? '◇' : instance.loader === 'fabric' ? '✣' : '◈'}
            </span>
            <span class="account-instance-copy"><strong>{instance.name}</strong><small>{instanceLabel(instance)} · {formatPlayed(instance.last_played)}</small></span>
            <span class:ready={instance.installed} class="list-state">{instance.installed ? '已安装' : '需要安装'}</span>
            <ChevronRight size={16}/>
          </button>
        {/each}
      </div>
    {/if}
  </section>

  <p class="account-detail-footnote"><ShieldCheck size={14}/>令牌只保存在本机 accounts.json；角色详情页不会显示或传输令牌。</p>
{/if}
