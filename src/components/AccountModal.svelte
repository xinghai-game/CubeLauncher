<script lang="ts">
  import { Check, Plus, ShieldCheck, Trash2, X } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import { activeAccount, addAccount, chooseAccount, removeAccount } from '../lib/actions';

  const account = $derived(activeAccount());
</script>

<div
  class="modal-backdrop"
  role="presentation"
  onclick={(event) => event.target === event.currentTarget && (ui.showAccount = false)}
>
  <section class="modal">
    <div class="modal-head">
      <div><span class="eyebrow">离线身份</span><h2>角色管理</h2></div>
      <button class="close-button" onclick={() => (ui.showAccount = false)}><X size={17}/></button>
    </div>
    <div class="account-list">
      {#each app.data.accounts as entry (entry.id)}
        <div class="account-row" class:selected={entry.id === account?.id}>
          <button class="account-pick" onclick={() => chooseAccount(entry)}>
            <span class="avatar small">{entry.name.slice(0, 1).toUpperCase()}</span>
            <span><strong>{entry.name}</strong><small>{entry.uuid}</small></span>
            {#if entry.id === account?.id}<Check size={16}/>{/if}
          </button>
          <button class="close-button" title="删除角色" onclick={() => removeAccount(entry)}><Trash2 size={14}/></button>
        </div>
      {/each}
      {#if app.data.accounts.length === 0}
        <p class="help-text">还没有角色。添加后即可离线进入单人世界，UUID 在本地生成。</p>
      {/if}
    </div>
    <div class="add-account">
      <input
        placeholder="3–16 位字母、数字或下划线"
        bind:value={ui.accountName}
        onkeydown={(event) => event.key === 'Enter' && addAccount()}
      />
      <button class="button primary" onclick={addAccount}><Plus size={16}/>添加</button>
    </div>
    <p class="modal-footnote">
      <ShieldCheck size={14}/> UUID 由 “OfflinePlayer:角色名” 生成，与 Java 的 UUID.nameUUIDFromBytes 结果一致。
    </p>
  </section>
</div>
