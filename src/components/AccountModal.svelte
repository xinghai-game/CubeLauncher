<script lang="ts">
  import {
    Check, CircleAlert, CloudOff, Copy, ExternalLink, LoaderCircle, Plus, RefreshCw, Search,
    ShieldCheck, Trash2, X
  } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import {
    activeAccount, addAccount, cancelMicrosoftLogin, chooseAccount, copyAccountUuid,
    copyUserCode, openAccount, openVerificationPage, refreshAccount, removeAccount, startMicrosoftLogin
  } from '../lib/actions';
  import { formatDateTime, needsRefresh, skinHeadStyle, type Account } from '../types/api';

  type AccountFilter = 'all' | 'microsoft' | 'offline' | 'attention';
  type AccountSort = 'recent' | 'name';

  const account = $derived(activeAccount());
  const login = $derived(ui.login);
  let query = $state('');
  let filter = $state<AccountFilter>('all');
  let sort = $state<AccountSort>('recent');
  let pendingDelete = $state<string | null>(null);
  let refreshingId = $state<string | null>(null);

  const filterOptions: Array<{ value: AccountFilter; label: string }> = [
    { value: 'all', label: '全部' },
    { value: 'microsoft', label: '正版' },
    { value: 'offline', label: '离线' },
    { value: 'attention', label: '需处理' }
  ];

  const counts = $derived({
    all: app.data.accounts.length,
    microsoft: app.data.accounts.filter((entry) => entry.kind === 'microsoft').length,
    offline: app.data.accounts.filter((entry) => entry.kind !== 'microsoft').length,
    attention: app.data.accounts.filter((entry) => needsRefresh(entry)).length
  });

  const visibleAccounts = $derived(
    [...app.data.accounts]
      .filter((entry) => {
        const keyword = query.trim().toLocaleLowerCase();
        const matchesQuery = !keyword
          || entry.name.toLocaleLowerCase().includes(keyword)
          || entry.uuid.toLocaleLowerCase().includes(keyword);
        const matchesFilter = filter === 'all'
          || (filter === 'microsoft' && entry.kind === 'microsoft')
          || (filter === 'offline' && entry.kind !== 'microsoft')
          || (filter === 'attention' && needsRefresh(entry));
        return matchesQuery && matchesFilter;
      })
      .sort((left, right) => sort === 'name'
        ? left.name.localeCompare(right.name, 'zh-CN')
        : accountTime(right) - accountTime(left))
  );

  function accountTime(entry: Account): number {
    const value = entry.microsoft?.last_login ?? entry.created_at;
    const time = new Date(value).getTime();
    return Number.isNaN(time) ? 0 : time;
  }

  function filterCount(value: AccountFilter): number {
    return counts[value];
  }

  function status(entry: Account): string {
    if (entry.kind !== 'microsoft') return '本机离线身份，可直接用于单人世界';
    if (needsRefresh(entry)) {
      return entry.microsoft?.refreshable ? '登录已过期，可尝试自动刷新' : '登录已过期，需要重新登录';
    }
    return '正版登录有效';
  }

  function expiry(entry: Account): string {
    const microsoft = entry.microsoft;
    if (!microsoft) return `UUID ${entry.uuid}`;
    if (needsRefresh(entry)) {
      return microsoft.refreshable ? '登录已过期 · 启动时会自动刷新' : '登录已过期 · 需要重新登录';
    }
    return `登录有效至 ${formatDateTime(microsoft.expires_at)}`;
  }

  function secondaryMeta(entry: Account): string {
    if (entry.kind === 'microsoft' && entry.microsoft?.last_login) {
      return `上次登录 ${formatDateTime(entry.microsoft.last_login)}`;
    }
    return `创建于 ${formatDateTime(entry.created_at)}`;
  }

  function close() {
    // Leaving a sign-in pending in the background would keep polling for a code
    // nobody is looking at.
    if (login.active) void cancelMicrosoftLogin();
    ui.showAccount = false;
  }

  function requestDelete(entry: Account) {
    pendingDelete = pendingDelete === entry.id ? null : entry.id;
  }

  async function confirmRemove(entry: Account) {
    pendingDelete = null;
    await removeAccount(entry);
  }

  async function renew(entry: Account) {
    refreshingId = entry.id;
    try {
      await refreshAccount(entry);
    } finally {
      if (refreshingId === entry.id) refreshingId = null;
    }
  }
</script>

<div
  class="modal-backdrop"
  role="presentation"
  onclick={(event) => event.target === event.currentTarget && close()}
>
  <section class="modal account-modal" aria-label="账户管理">
    <div class="modal-head">
      <div>
        <span class="eyebrow">身份与登录</span>
        <h2>账户管理</h2>
        <p class="modal-subtitle">统一管理正版登录、离线角色和当前使用身份</p>
      </div>
      <button class="close-button" onclick={close} title="关闭"><X size={17}/></button>
    </div>

    <div class="tabs">
      <button class:active={ui.accountTab === 'microsoft'} onclick={() => (ui.accountTab = 'microsoft')}>
        <ShieldCheck size={15}/>正版登录
      </button>
      <button class:active={ui.accountTab === 'offline'} onclick={() => (ui.accountTab = 'offline')}>
        <CloudOff size={15}/>离线角色
      </button>
    </div>

    {#if ui.accountTab === 'microsoft'}
      {#if login.active}
        <div class="device-panel">
          <p class="device-step">
            <b>1</b> 在浏览器中打开
            <button class="link-button" onclick={openVerificationPage}>
              {login.verificationUri || 'https://microsoft.com/link'}<ExternalLink size={12}/>
            </button>
          </p>
          <p class="device-step"><b>2</b> 输入下面的代码</p>
          <div class="device-code">
            <code>{login.userCode}</code>
            <button class="button ghost" onclick={copyUserCode}><Copy size={15}/>复制</button>
          </div>
          <p class="help-text device-wait">
            <LoaderCircle class="spin" size={13}/>
            {login.error || '等待你在 Microsoft 页面完成登录，启动器会自动继续……'}
          </p>
          <div class="detail-actions">
            <button class="button ghost" onclick={openVerificationPage}><ExternalLink size={15}/>重新打开页面</button>
            <button class="button ghost" onclick={cancelMicrosoftLogin}>取消登录</button>
          </div>
        </div>
      {:else}
        <div class="login-intro">
          <p class="help-text">
            用 Microsoft 账户登录即可使用正版身份：进入正版验证服务器、显示自己的皮肤与名称。
            启动器不会接触你的密码，也不会把令牌发给第三方：登录在 Microsoft 自己的页面上完成。
            页面可能把授权应用显示为“其他启动器”，这是微软登记的应用名称；按页面输入代码并同意授权即可。
          </p>
          {#if login.error}<p class="help-text error-text">{login.error}</p>{/if}
          <button class="button primary" onclick={startMicrosoftLogin}><ShieldCheck size={16}/>使用 Microsoft 账户登录</button>
        </div>
      {/if}
    {:else}
      <div class="add-account">
        <input
          placeholder="3–16 位字母、数字或下划线"
          aria-label="离线角色名称"
          bind:value={ui.accountName}
          onkeydown={(event) => event.key === 'Enter' && addAccount()}
        />
        <button class="button primary" onclick={addAccount}><Plus size={16}/>添加</button>
      </div>
      <p class="help-text">
        离线角色的 UUID 由 “OfflinePlayer:角色名” 生成，与 Java 的 UUID.nameUUIDFromBytes 结果一致，
        可以进入单人世界与允许离线登录的服务器。
      </p>
    {/if}

    <div class="account-stats" aria-label="角色概览">
      <div class="account-stat">
        <span>全部角色</span>
        <strong>{counts.all}</strong>
        <small>{account ? `当前 · ${account.name}` : '尚未选择'}</small>
      </div>
      <div class="account-stat account-stat-online">
        <span>正版账户</span>
        <strong>{counts.microsoft}</strong>
        <small>{counts.microsoft ? '支持皮肤与正版验证' : '登录后即可添加'}</small>
      </div>
      <div class="account-stat account-stat-offline">
        <span>离线角色</span>
        <strong>{counts.offline}</strong>
        <small>{counts.offline ? '仅保存在本机' : '可随时添加'}</small>
      </div>
      <div class="account-stat account-stat-alert">
        <span>需要处理</span>
        <strong>{counts.attention}</strong>
        <small>{counts.attention ? '令牌已过期' : '登录状态正常'}</small>
      </div>
    </div>

    <p class="account-section-title">角色列表</p>
    <div class="account-toolbar">
      <div class="kind-tabs account-filters">
        {#each filterOptions as option (option.value)}
          <button class:active={filter === option.value} onclick={() => (filter = option.value)}>
            {option.label}<span class="kind-count">{filterCount(option.value)}</span>
          </button>
        {/each}
      </div>
      <label class="select-wrap account-sort">
        <span class="sr-only">角色排序</span>
        <select value={sort} aria-label="角色排序" onchange={(event) => (sort = event.currentTarget.value as AccountSort)}>
          <option value="recent">最近使用</option>
          <option value="name">名称排序</option>
        </select>
      </label>
    </div>
    <label class="search-field account-search">
      <Search size={15}/>
      <input
        aria-label="搜索角色"
        placeholder="搜索名称或 UUID"
        value={query}
        oninput={(event) => (query = event.currentTarget.value)}
      />
      {#if query}
        <button class="search-clear" title="清除搜索" aria-label="清除搜索" onclick={() => (query = '')}><X size={13}/></button>
      {/if}
    </label>
    <div class="account-list-head">
      <p>{visibleAccounts.length === app.data.accounts.length
        ? `共 ${app.data.accounts.length} 个角色`
        : `显示 ${visibleAccounts.length} / ${app.data.accounts.length} 个角色`}</p>
      {#if query || filter !== 'all'}
        <button class="text-button" onclick={() => { query = ''; filter = 'all'; }}>清除筛选</button>
      {/if}
    </div>

    <div class="account-list">
      {#each visibleAccounts as entry (entry.id)}
        <div class="account-entry">
          <div class="account-row" class:selected={entry.id === account?.id}>
            <button class="account-pick" onclick={() => chooseAccount(entry)} title={`使用 ${entry.name}`}>
              {#if app.skins[entry.id]}
                <span class="skin-head" style={skinHeadStyle(app.skins[entry.id])}></span>
              {:else}
                <span class="avatar small">{entry.name.slice(0, 1).toUpperCase()}</span>
              {/if}
              <span class="account-row-copy">
                <strong>
                  {entry.name}
                  {#if entry.id === account?.id}<em class="account-current">当前使用</em>{/if}
                  <em class="account-badge" class:online={entry.kind === 'microsoft'}>
                    {entry.kind === 'microsoft' ? '正版' : '离线'}
                  </em>
                </strong>
                <small class="account-status" class:attention={needsRefresh(entry)}>{expiry(entry)}</small>
                <small class="account-secondary">{secondaryMeta(entry)} · {status(entry)}</small>
              </span>
              {#if entry.id === account?.id}<Check size={16}/>{/if}
            </button>
            <div class="account-row-actions">
              <button class="close-button" title="打开角色页面" aria-label={`打开 ${entry.name} 的角色页面`} onclick={() => openAccount(entry.id)}>
                <ExternalLink size={13}/>
              </button>
              <button class="close-button" title="复制 UUID" aria-label={`复制 ${entry.name} 的 UUID`} onclick={() => copyAccountUuid(entry)}>
                <Copy size={13}/>
              </button>
              {#if entry.kind === 'microsoft'}
                <button
                  class="close-button"
                  title="刷新登录"
                  aria-label={`刷新 ${entry.name} 的登录`}
                  disabled={refreshingId === entry.id}
                  onclick={() => renew(entry)}
                ><RefreshCw size={14} class={refreshingId === entry.id ? 'spin' : ''}/></button>
              {/if}
              <button
                class="close-button"
                class:danger-action={pendingDelete === entry.id}
                title={pendingDelete === entry.id ? '取消删除' : '删除角色'}
                aria-label={pendingDelete === entry.id ? '取消删除' : `删除 ${entry.name}`}
                onclick={() => requestDelete(entry)}
              ><Trash2 size={14}/></button>
            </div>
          </div>
          {#if pendingDelete === entry.id}
            <div class="account-delete-banner">
              <CircleAlert size={15}/>
              <div><strong>确认删除 {entry.name}？</strong><span>相关登录令牌也会从本机账户文件中移除。</span></div>
              <button class="button ghost small" onclick={() => (pendingDelete = null)}>取消</button>
              <button class="button danger small" onclick={() => confirmRemove(entry)}>确认删除</button>
            </div>
          {/if}
        </div>
      {/each}
      {#if app.data.accounts.length === 0}
        <p class="help-text account-empty">还没有角色：登录正版账户，或添加一个离线角色。</p>
      {:else if visibleAccounts.length === 0}
        <p class="help-text account-empty">没有符合当前筛选条件的角色，试试其他名称或筛选。</p>
      {/if}
    </div>

    <p class="modal-footnote">
      <ShieldCheck size={14}/>
      登录令牌只保存在本机 accounts.json（Unix 下权限 0600），界面不会接触到令牌本身。
    </p>
  </section>
</div>
