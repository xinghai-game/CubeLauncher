<script lang="ts">
  import {
    ChevronRight, Coffee, FolderOpen, HardDrive, Info, Moon, ShieldCheck, SlidersHorizontal, Users
  } from 'lucide-svelte';
  import { app, ui, type SettingsSection } from '../lib/state.svelte';
  import { settingsSections } from '../lib/settings';
  import { saveSettings, goSettings } from '../lib/actions';

  const draft = $derived(ui.settingsDraft);

  const icons: Record<SettingsSection, typeof FolderOpen> = {
    general: FolderOpen,
    account: Users,
    download: HardDrive,
    java: Coffee,
    appearance: Moon,
    about: Info
  };

  function summary(section: SettingsSection): string {
    switch (section) {
      case 'general':
        return `${draft.data_dir} · ${draft.offline_mode ? '严格离线' : '按需联网'}`;
      case 'account': {
        const microsoft = app.data.accounts.filter((account) => account.kind === 'microsoft').length;
        const offline = app.data.accounts.length - microsoft;
        return `${microsoft} 个正版 · ${offline} 个离线角色`;
      }
      case 'download':
        return `${draft.download_concurrency} 路并行 · ${draft.mirror_base_url || '官方源'}`;
      case 'java':
        return `${app.data.java.length} 个运行时 · 默认最大内存 ${draft.default_memory_mb} MB`;
      case 'appearance':
        return draft.theme === 'dark' ? '深色主题' : '浅色主题';
      default:
        return 'CubeLauncher 0.1.0 · MIT 许可';
    }
  }
</script>

<div class="page-head compact">
  <div>
    <p class="eyebrow">偏好与路径</p>
    <h1>启动器设置</h1>
    <p class="subtitle">设置按用途分组；进入某一组后可以返回这里继续调整其他内容。</p>
  </div>
  <button class="button primary" onclick={saveSettings}><ShieldCheck size={16}/>保存全部设置</button>
</div>

<div class="settings-categories">
  {#each settingsSections as section (section.id)}
    {@const Icon = icons[section.id]}
    <button class="category-card" onclick={() => goSettings(section.id)}>
      <span class="category-icon"><Icon size={17}/></span>
      <span class="category-copy">
        <strong>{section.title}</strong>
        <small>{section.blurb}</small>
        <span class="category-value">{summary(section.id)}</span>
      </span>
      <ChevronRight size={16} class="category-arrow"/>
    </button>
  {/each}
</div>

<div class="catalog-footnote">
  <SlidersHorizontal size={14}/>
  <span>设置保存在数据目录的 settings.json；修改数据目录后，已有实例仍保留在原位置。</span>
</div>
