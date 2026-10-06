<script lang="ts">
  import { Trash2, X } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import { removeInstance } from '../lib/actions';

  const selected = $derived(app.data.instances.find((item) => item.id === ui.selectedId) ?? null);
</script>

{#if selected}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(event) => event.target === event.currentTarget && (ui.showDeleteConfirm = false)}
  >
    <section class="modal">
      <div class="modal-head">
        <div><span class="eyebrow">不可撤销</span><h2>删除 {selected.name}？</h2></div>
        <button class="close-button" onclick={() => (ui.showDeleteConfirm = false)}><X size={17}/></button>
      </div>
      <p class="help-text">这会删除该实例的存档、模组与日志。共享的游戏依赖与 Java 运行时不会被删除。</p>
      <div class="modal-actions">
        <button class="button ghost" onclick={() => (ui.showDeleteConfirm = false)}>取消</button>
        <button class="button danger" onclick={removeInstance}><Trash2 size={16}/>确认删除</button>
      </div>
    </section>
  </div>
{/if}
