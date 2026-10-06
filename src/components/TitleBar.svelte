<script lang="ts">
  import { Box, CircleHelp, X } from 'lucide-svelte';
  import { app } from '../lib/state.svelte';
  import { about, quit } from '../lib/actions';
</script>

<header class="titlebar" data-tauri-drag-region>
  <div class="brand">
    <span class="brand-mark"><Box size={17} strokeWidth={2.4} /></span>
    <span>Cube<span class="accent">Launcher</span></span>
  </div>
  <div class="titlebar-status">
    <span class="status-dot"></span>{app.data.instances.length} 个实例 · {app.data.accounts.length} 个角色
    <span class="divider"></span><span class="muted">{app.data.data_dir}</span>
  </div>
  <div class="window-actions">
    <button aria-label="关于" onclick={about}><CircleHelp size={16}/></button>
    <button aria-label="关闭" onclick={quit}><X size={15}/></button>
  </div>
</header>
