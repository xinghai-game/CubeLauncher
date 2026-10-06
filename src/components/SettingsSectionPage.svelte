<script lang="ts">
  import { Check, CloudOff, Coffee, Cpu, FolderOpen, HardDrive, Moon, RefreshCw, Sparkles, Undo2 } from 'lucide-svelte';
  import { app, ui, type SettingsSection } from '../lib/state.svelte';
  import { settingsTitle } from '../lib/settings';
  import { chooseTheme, installJava, rescanJava, resetSettingsDraft, saveSettings } from '../lib/actions';
  import Breadcrumb from './Breadcrumb.svelte';

  const section = $derived((ui.route.name === 'settings' ? ui.route.section : null) ?? 'general') as SettingsSection;
  const draft = $derived(ui.settingsDraft);
</script>

<Breadcrumb
  items={[
    { label: '启动器设置', to: { name: 'settings', section: null } },
    { label: settingsTitle(section) }
  ]}
/>

<div class="page-head compact">
  <div>
    <p class="eyebrow">设置 · {settingsTitle(section)}</p>
    <h1>{settingsTitle(section)}</h1>
    <p class="subtitle">改动会在保存后写入 settings.json。</p>
  </div>
  <div class="head-actions">
    <button class="button ghost" onclick={resetSettingsDraft}><Undo2 size={16}/>还原</button>
    <button class="button primary" onclick={saveSettings}><Check size={17}/>保存设置</button>
  </div>
</div>

{#if section === 'general'}
  <div class="settings-grid">
    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon green"><FolderOpen size={17}/></span>
        <div><strong>数据目录</strong><small>实例、依赖与 Java 的存放位置</small></div>
      </div>
      <label>当前目录<input bind:value={draft.data_dir} /></label>
      <p class="help-text">修改后新实例写入新位置，已有实例仍保留在原目录。</p>
    </section>
    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon orange"><CloudOff size={17}/></span>
        <div><strong>离线与启动行为</strong><small>控制启动器是否发起网络请求</small></div>
      </div>
      <label class="toggle-row">
        <span>严格离线模式<small>启动器不发起网络请求，缺文件时直接列出</small></span>
        <input type="checkbox" bind:checked={draft.offline_mode} /><i></i>
      </label>
      <label class="toggle-row">
        <span>启动游戏后关闭启动器<small>进一步降低常驻内存</small></span>
        <input type="checkbox" bind:checked={draft.close_launcher_after_launch} /><i></i>
      </label>
    </section>
    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon purple"><HardDrive size={17}/></span>
        <div><strong>当前数据位置</strong><small>只读信息，便于排查问题</small></div>
      </div>
      <div class="summary-row"><span>生效目录</span><strong class="path">{app.data.data_dir}</strong></div>
      <div class="summary-row"><span>实例目录</span><strong class="path">{app.data.data_dir}/instances</strong></div>
      <div class="summary-row"><span>元数据缓存</span><strong class="path">{app.data.data_dir}/metadata</strong></div>
    </section>
  </div>

{:else if section === 'download'}
  <div class="settings-grid">
    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon orange"><HardDrive size={17}/></span>
        <div><strong>下载</strong><small>并发过高可能触发限速，4–8 较稳妥</small></div>
      </div>
      <label>并行下载数
        <div class="range-line">
          <input type="range" min="1" max="16" bind:value={draft.download_concurrency} />
          <b>{draft.download_concurrency}</b>
        </div>
      </label>
      <label>默认最大内存 (MB)<input type="number" min="1024" step="512" bind:value={draft.default_memory_mb} /></label>
      <p class="help-text">新建实例时会使用这个内存上限，之后可在实例的“运行设置”里单独修改。</p>
    </section>
    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon green"><Sparkles size={17}/></span>
        <div><strong>镜像</strong><small>留空使用官方源</small></div>
      </div>
      <label>游戏与依赖镜像（可选）<input placeholder="https://bmclapi2.bangbang93.com" bind:value={draft.mirror_base_url} /></label>
      <p class="help-text">镜像会改写版本清单、依赖库与资源地址；没有上游校验值的文件不会被标记为已校验。</p>
      <label>Java 镜像（可选）<input placeholder="https://mirrors.tuna.tsinghua.edu.cn/Adoptium" bind:value={draft.java_mirror_base_url} /></label>
      <p class="help-text">用于下载 Adoptium 运行时；SHA-256 仍按官方接口校验，失败会自动回退上游。</p>
    </section>
    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon purple"><Cpu size={17}/></span>
        <div><strong>版本列表缓存</strong><small>清单按需刷新，过期时自动重新获取</small></div>
      </div>
      <div class="summary-row"><span>缓存版本</span><strong>{app.catalog ? `${app.catalog.total} 个` : '尚未获取'}</strong></div>
      <div class="summary-row"><span>来源</span><strong>{app.catalog?.source ?? '—'}</strong></div>
      <p class="help-text">严格离线模式下无法刷新清单，但已有缓存仍可用于安装与启动。</p>
    </section>
  </div>

{:else if section === 'java'}
  <div class="settings-grid">
    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon purple"><Cpu size={17}/></span>
        <div><strong>Java 运行时</strong><small>按游戏版本自动选择，也可以指定固定路径</small></div>
      </div>
      <div class="java-list">
        {#each app.data.java as runtime (runtime.path)}
          <div class="java-row">
            <strong>Java {runtime.major}</strong>
            <span>{runtime.architecture} · {runtime.source}</span>
            <small>{runtime.path}</small>
          </div>
        {/each}
        {#if app.data.java.length === 0}
          <div class="java-row"><strong>未发现运行时</strong><span>可下载 Adoptium JRE，会安装到启动器私有目录。</span></div>
        {/if}
      </div>
      <div class="detail-actions wrap">
        <button class="button ghost" disabled={app.busy} onclick={() => installJava(8)}>下载 Java 8</button>
        <button class="button ghost" disabled={app.busy} onclick={() => installJava(17)}>下载 Java 17</button>
        <button class="button ghost" disabled={app.busy} onclick={() => installJava(21)}>下载 Java 21</button>
        <button class="button ghost" onclick={rescanJava}><RefreshCw size={15}/>重新扫描</button>
      </div>
    </section>
    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon green"><Coffee size={17}/></span>
        <div><strong>选择规则</strong><small>没有指定路径时按版本元数据决定</small></div>
      </div>
      <div class="summary-row"><span>1.12.2 及更早</span><strong>Java 8</strong></div>
      <div class="summary-row"><span>1.17</span><strong>Java 16</strong></div>
      <div class="summary-row"><span>1.18 – 1.20.4</span><strong>Java 17</strong></div>
      <div class="summary-row"><span>1.20.5 及更新</span><strong>Java 21</strong></div>
      <p class="help-text">表只用于旧版元数据缺省时的兜底；元数据声明了 javaVersion 时一律以元数据为准。</p>
    </section>
  </div>

{:else if section === 'appearance'}
  <div class="settings-grid">
    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon green"><Moon size={17}/></span>
        <div><strong>主题</strong><small>界面随系统字体与缩放</small></div>
      </div>
      <div class="theme-options">
        <button class:chosen={draft.theme !== 'light'} aria-pressed={draft.theme !== 'light'} onclick={() => chooseTheme('dark')}><Moon size={17}/>深色</button>
        <button class:chosen={draft.theme === 'light'} aria-pressed={draft.theme === 'light'} onclick={() => chooseTheme('light')}><Sparkles size={17}/>浅色</button>
      </div>
      <p class="help-text">深色主题是默认值；浅色主题仍在打磨中。</p>
    </section>
  </div>

{:else}
  <div class="settings-grid">
    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon green"><Coffee size={17}/></span>
        <div><strong>CubeLauncher 0.1.0</strong><small>离线优先的 Minecraft Java 启动器</small></div>
      </div>
      <div class="summary-row"><span>许可</span><strong>MIT</strong></div>
      <div class="summary-row"><span>数据目录</span><strong class="path">{app.data.data_dir}</strong></div>
      <div class="summary-row"><span>界面</span><strong>Svelte 5 + Tauri WebView</strong></div>
      <div class="summary-row"><span>核心</span><strong>launcher-core (Rust)</strong></div>
    </section>
    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon orange"><CloudOff size={17}/></span>
        <div><strong>身份说明</strong><small>本启动器只支持离线身份</small></div>
      </div>
      <p class="help-text">
        离线身份可以进入单人世界与允许离线登录的服务器，不提供 Realms、正版验证服务器登录或在线皮肤。
        游戏与模组自身的联网行为不受启动器控制。
      </p>
    </section>
  </div>
{/if}
