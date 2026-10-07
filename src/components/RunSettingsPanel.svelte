<script lang="ts">
  import {
    Check, ChevronDown, Cpu, FolderInput, FolderOpen, RotateCcw, ShieldCheck, SlidersHorizontal,
    TerminalSquare
  } from 'lucide-svelte';
  import { app, ui } from '../lib/state.svelte';
  import { instanceGameDir, isImported, shortPath } from '../types/api';
  import { goImport, openFolder, saveInstanceDraft, showPreview, unbindGameDir } from '../lib/actions';

  const previewArgs = $derived(app.preview ? app.preview.args.join(' ') : '');
  const imported = $derived(isImported(ui.instanceDraft));
  const gameDir = $derived(
    ui.instanceDraft ? instanceGameDir(ui.instanceDraft, app.data.data_dir) : ''
  );
</script>

{#if ui.instanceDraft}
  {@const draft = ui.instanceDraft}
  <div class="settings-grid">
    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon green"><Cpu size={17}/></span>
        <div><strong>内存与 Java</strong><small>内存由启动器统一管理，避免与元数据参数冲突</small></div>
      </div>
      <label>最小内存 (MB)<input type="number" min="512" step="256" bind:value={draft.min_memory_mb} /></label>
      <label>最大内存 (MB)<input type="number" min="512" step="512" bind:value={draft.max_memory_mb} /></label>
      <label>Java 运行时
        <span class="select-wrap">
          <select bind:value={draft.java_path}>
            <option value={null}>自动选择</option>
            {#each app.data.java as runtime (runtime.path)}
              <option value={runtime.path}>Java {runtime.major} · {runtime.architecture} · {runtime.source}</option>
            {/each}
          </select><ChevronDown size={15}/>
        </span>
      </label>
      <p class="help-text">已发现 {app.data.java.length} 个运行时，缺少时可在“启动器设置 → Java 运行时”里下载。</p>
    </section>

    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon purple"><FolderInput size={17}/></span>
        <div>
          <strong>游戏目录</strong>
          <small>{imported ? '导入的 .minecraft：文件留在原处，不会复制' : '启动器管理的独立 .minecraft'}</small>
        </div>
      </div>
      <div class="summary-row">
        <span>{imported ? '外部 .minecraft' : '启动器管理'}</span>
        <strong class="path">{gameDir}</strong>
      </div>
      {#if imported}
        <div class="summary-row">
          <span>版本文件夹</span>
          <strong>{draft.version_id ?? draft.game_version}</strong>
        </div>
        <div class="summary-row">
          <span>简写</span>
          <strong>{shortPath(gameDir)}</strong>
        </div>
      {/if}
      <div class="detail-actions wrap">
        <button class="button ghost" onclick={() => openFolder('game')}><FolderOpen size={15}/>打开目录</button>
        <button class="button ghost" onclick={() => goImport(draft.id)}>
          <FolderInput size={15}/>{imported ? '更换目录' : '绑定已有 .minecraft'}
        </button>
        {#if imported}
          <button class="button ghost" onclick={unbindGameDir}><RotateCcw size={15}/>恢复默认目录</button>
        {/if}
      </div>
      <p class="help-text">
        存档、模组与截图都在这个目录里；导入的目录由原启动器与 CubeLauncher 共用，
        删除实例不会删除它。
      </p>
    </section>

    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon orange"><SlidersHorizontal size={17}/></span>
        <div><strong>窗口与参数</strong><small>留空即使用默认值</small></div>
      </div>
      <div class="inline-fields">
        <label>窗口宽度<input type="number" min="640" step="10" bind:value={draft.width} /></label>
        <label>窗口高度<input type="number" min="480" step="10" bind:value={draft.height} /></label>
      </div>
      <label class="toggle-row">
        <span>全屏启动<small>等价于附加 --fullscreen</small></span>
        <input type="checkbox" bind:checked={draft.fullscreen} /><i></i>
      </label>
      <label>附加 JVM 参数
        <input
          placeholder="-XX:+UseG1GC"
          value={draft.jvm_args.join(' ')}
          oninput={(event) => (draft.jvm_args = event.currentTarget.value.split(/\s+/).filter(Boolean))}
        />
      </label>
      <label>附加游戏参数
        <input
          placeholder="--server example.com"
          value={draft.game_args.join(' ')}
          oninput={(event) => (draft.game_args = event.currentTarget.value.split(/\s+/).filter(Boolean))}
        />
      </label>
    </section>

    <section class="settings-card">
      <div class="settings-card-head">
        <span class="stat-icon purple"><ShieldCheck size={17}/></span>
        <div><strong>启动预览</strong><small>先看清真正执行的命令，再决定启动</small></div>
      </div>
      <div class="detail-actions wrap">
        <button class="button ghost" onclick={showPreview}><TerminalSquare size={15}/>生成命令</button>
        <button class="button primary" onclick={saveInstanceDraft}><Check size={15}/>保存设置</button>
      </div>
      {#if app.preview}
        <div class="preview-block">
          <div><strong>Java</strong><span>{app.preview.java}</span></div>
          <div><strong>工作目录</strong><span>{app.preview.working_dir}</span></div>
          <div><strong>原生库</strong><span>{app.preview.natives_dir}</span></div>
          <div><strong>classpath</strong><span>{app.preview.classpath_entries} 个条目</span></div>
          {#if app.preview.missing_files.length}
            <div class="warn"><strong>缺失</strong><span>{app.preview.missing_files.slice(0, 4).join('、')}</span></div>
          {:else}
            <div class="ok"><strong>文件</strong><span>完整</span></div>
          {/if}
          <code>{previewArgs}</code>
        </div>
      {/if}
    </section>
  </div>
{/if}
