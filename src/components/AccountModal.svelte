<script lang="ts">
  import {
    Check, CloudOff, Copy, ExternalLink, LoaderCircle, Plus, RefreshCw, ShieldCheck, Trash2, X
  } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import {
    activeAccount, addAccount, cancelMicrosoftLogin, chooseAccount, copyUserCode,
    openVerificationPage, refreshAccount, removeAccount, startMicrosoftLogin
  } from '../lib/actions';
  import { formatDateTime, needsRefresh, skinHeadStyle, type Account } from '../types/api';

  const account = $derived(activeAccount());
  const login = $derived(ui.login);

  function close() {
    // Leaving a sign-in pending in the background would keep polling for a code
    // nobody is looking at.
    if (login.active) void cancelMicrosoftLogin();
    ui.showAccount = false;
  }

  function expiry(entry: Account): string {
    const microsoft = entry.microsoft;
    if (!microsoft) return entry.uuid;
    if (needsRefresh(entry)) {
      return microsoft.refreshable ? '登录已过期 · 启动时会自动刷新' : '登录已过期 · 需要重新登录';
    }
    return `登录有效至 ${formatDateTime(microsoft.expires_at)}`;
  }
</script>

<div
  class="modal-backdrop"
  role="presentation"
  onclick={(event) => event.target === event.currentTarget && close()}
>
  <section class="modal account-modal">
    <div class="modal-head">
      <div><span class="eyebrow">身份与登录</span><h2>账户管理</h2></div>
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
          </p>
          {#if login.error}<p class="help-text error-text">{login.error}</p>{/if}
          <button class="button primary" onclick={startMicrosoftLogin}><ShieldCheck size={16}/>使用 Microsoft 账户登录</button>
        </div>
      {/if}
    {:else}
      <div class="add-account">
        <input
          placeholder="3–16 位字母、数字或下划线"
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

    <p class="account-section-title">全部角色</p>
    <div class="account-list">
      {#each app.data.accounts as entry (entry.id)}
        <div class="account-row" class:selected={entry.id === account?.id}>
          <button class="account-pick" onclick={() => chooseAccount(entry)}>
            {#if app.skins[entry.id]}
              <span class="skin-head" style={skinHeadStyle(app.skins[entry.id])}></span>
            {:else}
              <span class="avatar small">{entry.name.slice(0, 1).toUpperCase()}</span>
            {/if}
            <span>
              <strong>
                {entry.name}
                <em class="account-badge" class:online={entry.kind === 'microsoft'}>
                  {entry.kind === 'microsoft' ? '正版' : '离线'}
                </em>
              </strong>
              <small>{expiry(entry)}</small>
            </span>
            {#if entry.id === account?.id}<Check size={16}/>{/if}
          </button>
          {#if entry.kind === 'microsoft'}
            <button
              class="close-button"
              title="刷新登录"
              onclick={() => refreshAccount(entry)}
            ><RefreshCw size={14}/></button>
          {/if}
          <button class="close-button" title="删除角色" onclick={() => removeAccount(entry)}>
            <Trash2 size={14}/>
          </button>
        </div>
      {/each}
      {#if app.data.accounts.length === 0}
        <p class="help-text">还没有角色：登录正版账户，或添加一个离线角色。</p>
      {/if}
    </div>

    <p class="modal-footnote">
      <ShieldCheck size={14}/>
      登录令牌只保存在本机 accounts.json（Unix 下权限 0600），界面不会接触到令牌本身。
    </p>
  </section>
</div>
