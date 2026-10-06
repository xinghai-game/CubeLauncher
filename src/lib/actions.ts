import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { openPath } from '@tauri-apps/plugin-opener';
import {
  invoke, isTauri, subscribe,
  type Account, type Bootstrap, type Instance, type JavaRequirement, type LaunchPreview,
  type Loader, type ModEntry, type Runtime, type Task, type VersionCatalog
} from '../types/api';
import {
  app, loaderKey, resetWizardForm, ui, type DetailTab, type Route, type SettingsSection
} from './state.svelte';
import { suggestInstanceName, type KindFilter } from './versions';
import { normalizeTheme, type Theme } from './theme';

/* ------------------------------------------------------------------ messaging */

export function notify(message: string, error = false) {
  ui.toast = message;
  ui.toastError = error;
  setTimeout(() => { if (ui.toast === message) ui.toast = ''; }, error ? 7000 : 3600);
}

export function guard(error: unknown) {
  notify(error instanceof Error ? error.message : String(error), true);
}

/* ----------------------------------------------------------------- navigation */

const HISTORY_LIMIT = 20;

/** Go one level deeper; the current page stays reachable with `back()`. */
export function go(route: Route) {
  ui.history = [...ui.history.slice(-HISTORY_LIMIT), ui.route];
  ui.route = route;
}

/** Replace the current page, e.g. when a wizard hands over to the install page. */
export function replaceRoute(route: Route) {
  ui.route = route;
}

/** Jump to a top-level page, dropping the breadcrumb trail. */
export function reset(route: Route) {
  ui.history = [];
  ui.route = route;
}

export function back() {
  const previous = ui.history[ui.history.length - 1];
  if (!previous) return;
  ui.history = ui.history.slice(0, -1);
  ui.route = previous;
}

export function goHome() {
  reset({ name: 'home' });
}

export function goInstances(tab: DetailTab = 'overview') {
  reset({ name: 'instances', tab });
}

export function goSettings(section: SettingsSection | null = null) {
  if (section) go({ name: 'settings', section });
  else reset({ name: 'settings', section: null });
}

/** Version list: the entry point for installing anything, like HMCL's 下载 page. */
export async function goVersions(kind: KindFilter = 'release', query = '') {
  reset({ name: 'versions', kind, query });
  ui.visibleVersions = 60;
  if (!app.catalog) await refreshCatalog(false);
}

/** Open the install wizard for one game version (level 3 under the version list). */
export function goWizard(version: string) {
  resetWizardForm(version);
  app.loaderVersions = {};
  go({ name: 'wizard', version, step: 1 });
}

/* ---------------------------------------------------------------- bootstrap */

export async function connectEvents() {
  try {
    return await subscribe({
      onTask: (update) => {
        app.task = update;
        if (update.done) {
          if (update.error) notify(update.error, true);
          else notify(update.message);
          void refreshAll();
        }
      },
      onLog: (line) => { app.liveLogs = [...app.liveLogs.slice(-500), `[${line.stream}] ${line.line}`]; },
      onState: (state) => {
        app.data.running = state.state === 'starting'
          ? [...new Set([...app.data.running, state.instance_id])]
          : app.data.running.filter((id) => id !== state.instance_id);
        if (state.state === 'stopped' && state.exit_code && state.exit_code !== 0) {
          notify(`游戏退出码 ${state.exit_code}，可查看日志定位原因`, true);
        }
      }
    });
  } catch {
    // Live events are a bonus; the launcher still works without them.
    return () => {};
  }
}

export async function bootstrap() {
  try {
    app.data = await invoke<Bootstrap>('bootstrap');
    ui.settingsDraft = { ...app.data.settings };
    if (app.data.instances[0]) await selectInstance(app.data.instances[0].id);
  } catch {
    // Browser preview keeps the empty shell.
  }
}

export async function refreshAll() {
  try {
    const next = await invoke<Bootstrap>('bootstrap');
    app.data = next;
    if (ui.selectedId && !next.instances.some((item) => item.id === ui.selectedId)) {
      ui.selectedId = next.instances[0]?.id ?? null;
    }
    const current = next.instances.find((item) => item.id === ui.selectedId) ?? null;
    if (current) ui.instanceDraft = { ...current };
  } catch (error) { guard(error); }
}

/* ------------------------------------------------------------- version list */

export async function refreshCatalog(force: boolean) {
  app.catalogLoading = true;
  app.catalogError = '';
  try {
    const catalog = await invoke<VersionCatalog>('version_catalog', { force });
    app.catalog = catalog;
    if (force) {
      notify(`版本目录已更新：共 ${catalog.total} 个版本，最新正式版 ${catalog.latest.release}`);
    }
  } catch (error) {
    app.catalogError = error instanceof Error ? error.message : String(error);
    if (force) guard(error);
  } finally {
    app.catalogLoading = false;
  }
}

/** Java requirement of the version being configured in the wizard. */
export async function loadJavaRequirement(versionId: string) {
  app.javaLoading = true;
  try {
    app.javaRequirement = await invoke<JavaRequirement>('version_java', { versionId });
  } catch (error) {
    app.javaRequirement = null;
    guard(error);
  } finally {
    app.javaLoading = false;
  }
}

/**
 * Loader version list for a game version, cached per (game, loader) pair. The
 * result carries its own loading/error state so the wizard can show why a loader
 * is unavailable instead of silently offering nothing.
 */
export async function loadLoaderVersions(gameVersion: string, loader: Loader, force = false) {
  const key = loaderKey(gameVersion, loader);
  const cached = app.loaderVersions[key];
  if (cached && !force && (cached.versions.length > 0 || cached.error)) return cached;
  app.loaderVersions[key] = { loading: true, error: '', versions: [] };
  try {
    const versions = await invoke<string[]>('loader_versions', { gameVersion, loader });
    app.loaderVersions[key] = { loading: false, error: '', versions };
  } catch (error) {
    app.loaderVersions[key] = {
      loading: false,
      error: error instanceof Error ? error.message : String(error),
      versions: []
    };
  }
  return app.loaderVersions[key];
}

/* --------------------------------------------------------------- instances */

export function activeAccount(): Account | null {
  const selected = app.data.instances.find((item) => item.id === ui.selectedId) ?? null;
  return app.data.accounts.find((item) => item.id === selected?.account_id)
    ?? app.data.accounts[0]
    ?? null;
}

export async function selectInstance(id: string) {
  ui.selectedId = id;
  const instance = app.data.instances.find((item) => item.id === id) ?? null;
  ui.instanceDraft = instance ? { ...instance } : null;
  app.preview = null;
  app.liveLogs = [];
  app.mods = [];
  app.fileLogs = [];
  if (!instance) return;
  if (ui.detailTab === 'mods') await loadMods();
  if (ui.detailTab === 'logs') await loadFileLogs();
}

export async function setDetailTab(tab: DetailTab) {
  ui.detailTab = tab;
  if (tab === 'mods') await loadMods();
  if (tab === 'logs') await loadFileLogs();
}

export async function addAccount() {
  if (!ui.accountName.trim()) return;
  try {
    const account = await invoke<Account>('add_offline_account', { name: ui.accountName.trim() });
    app.data.accounts = [...app.data.accounts, account];
    ui.accountName = '';
    notify(`已添加离线角色 ${account.name}`);
  } catch (error) { guard(error); }
}

export async function chooseAccount(account: Account) {
  const selected = app.data.instances.find((item) => item.id === ui.selectedId) ?? null;
  if (!selected || !ui.instanceDraft) { notify(`已选择 ${account.name}`); return; }
  ui.instanceDraft = { ...ui.instanceDraft, account_id: account.id };
  await saveInstanceDraft();
  notify(`已为 ${selected.name} 选择 ${account.name}`);
}

export async function removeAccount(account: Account) {
  try {
    await invoke('delete_account', { id: account.id });
    app.data.accounts = app.data.accounts.filter((item) => item.id !== account.id);
    notify(`已删除角色 ${account.name}`);
  } catch (error) { guard(error); }
}

export async function saveInstanceDraft() {
  if (!ui.instanceDraft) return;
  try {
    const updated = await invoke<Instance>('update_instance', { instance: ui.instanceDraft });
    app.data.instances = app.data.instances.map((item) => (item.id === updated.id ? updated : item));
    ui.instanceDraft = { ...updated };
    notify('实例设置已保存');
  } catch (error) { guard(error); }
}

export async function removeInstance() {
  const selected = app.data.instances.find((item) => item.id === ui.selectedId) ?? null;
  if (!selected) return;
  try {
    await invoke('delete_instance', { id: selected.id });
    ui.showDeleteConfirm = false;
    const remaining = app.data.instances.filter((item) => item.id !== selected.id);
    app.data.instances = remaining;
    ui.selectedId = remaining[0]?.id ?? null;
    await selectInstance(ui.selectedId ?? '');
    notify('实例已删除');
  } catch (error) { guard(error); }
}

/* ---------------------------------------------------------- install & launch */

export async function installInstance(instanceId: string) {
  try {
    await invoke<string>('install_instance', { instanceId });
    app.task = {
      id: '', instance_id: instanceId, phase: 'prepare', current: 0, total: 0,
      message: '准备安装', done: false
    };
    app.taskStartedAt = Date.now();
  } catch (error) { guard(error); }
}

export async function installSelected() {
  if (!ui.selectedId) return;
  await installInstance(ui.selectedId);
  notify('已开始安装，可在安装页面查看进度');
}

/**
 * Wizard hand-off: create the instance, open the install page (replacing the
 * wizard so "back" returns to the version list) and start downloading.
 */
export async function createAndInstall() {
  const version = ui.form.version;
  const loader = ui.form.loader;
  const name = ui.form.name.trim() || suggestInstanceName(version, loader);
  const account = activeAccount();
  app.busy = true;
  try {
    const instance = await invoke<Instance>('create_instance', {
      input: {
        name,
        game_version: version,
        loader,
        loader_version: loader === 'vanilla' || !ui.form.loaderVersion ? null : ui.form.loaderVersion,
        account_id: account?.id ?? null
      }
    });
    app.data.instances = [instance, ...app.data.instances];
    ui.selectedId = instance.id;
    ui.instanceDraft = { ...instance };
    replaceRoute({ name: 'install', instanceId: instance.id });
    await installInstance(instance.id);
    notify('实例已创建，正在安装');
  } catch (error) {
    guard(error);
  } finally {
    app.busy = false;
  }
}

export async function verifySelected() {
  if (!ui.selectedId) return;
  try {
    const missing = await invoke<string[]>('verify_instance', { instanceId: ui.selectedId });
    if (missing.length === 0) notify('校验通过：所有文件齐全，可以离线启动');
    else notify(`缺少 ${missing.length} 项，可点击“修复/更新”：${missing.slice(0, 3).join('、')}`, true);
  } catch (error) { guard(error); }
}

export async function launchSelected() {
  if (!ui.selectedId) return;
  app.busy = true;
  try {
    const result = await invoke<{ ok: boolean; message: string }>('launch_instance', { instanceId: ui.selectedId });
    notify(result.message, !result.ok);
  } catch (error) { guard(error); } finally { app.busy = false; await refreshAll(); }
}

export async function stopSelected() {
  if (!ui.selectedId) return;
  try {
    const result = await invoke<{ message: string }>('stop_instance', { instanceId: ui.selectedId });
    notify(result.message);
  } catch (error) { guard(error); }
}

export async function showPreview() {
  if (!ui.selectedId) return;
  try {
    app.preview = await invoke<LaunchPreview>('launch_preview', { instanceId: ui.selectedId });
  } catch (error) { app.preview = null; guard(error); }
}

/* -------------------------------------------------------------------- mods */

export async function loadMods() {
  if (!ui.selectedId) return;
  try { app.mods = await invoke<ModEntry[]>('list_mods', { instanceId: ui.selectedId }); }
  catch (error) { guard(error); }
}

export async function addMod() {
  if (!ui.selectedId) return;
  if (!isTauri) { notify('浏览器预览模式无法选择文件', true); return; }
  try {
    const picked = await openDialog({ multiple: true, filters: [{ name: '模组', extensions: ['jar'] }] });
    const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
    for (const path of paths) await invoke('add_mod', { instanceId: ui.selectedId, path });
    if (paths.length) notify(`已添加 ${paths.length} 个模组文件`);
    await loadMods();
  } catch (error) { guard(error); }
}

export async function toggleMod(mod: ModEntry) {
  if (!ui.selectedId) return;
  try {
    await invoke('toggle_mod', { instanceId: ui.selectedId, fileName: mod.file_name, enabled: !mod.enabled });
    await loadMods();
  } catch (error) { guard(error); }
}

export async function removeMod(mod: ModEntry) {
  if (!ui.selectedId) return;
  try {
    await invoke('delete_mod', { instanceId: ui.selectedId, fileName: mod.file_name });
    await loadMods();
    notify(`已删除 ${mod.display_name}`);
  } catch (error) { guard(error); }
}

export async function openFolder(kind: 'instance' | 'mods') {
  if (!ui.selectedId) return;
  try {
    const path = await invoke<string>(kind === 'mods' ? 'mods_dir' : 'instance_dir', { instanceId: ui.selectedId });
    if (isTauri) await openPath(path); else notify(path);
  } catch (error) { guard(error); }
}

/* -------------------------------------------------------------------- logs */

export async function loadFileLogs() {
  if (!ui.selectedId) return;
  try { app.fileLogs = await invoke<string[]>('read_log', { instanceId: ui.selectedId, maxLines: 300 }); }
  catch (error) { guard(error); }
}

export async function clearLogs() {
  if (!ui.selectedId) return;
  try {
    await invoke('clear_log', { instanceId: ui.selectedId });
    app.fileLogs = [];
    app.liveLogs = [];
    notify('日志已清空');
  } catch (error) { guard(error); }
}

/* ---------------------------------------------------------------- settings */

/**
 * Switch theme immediately, then persist just that field, so unsaved edits on
 * the settings page survive the round trip. The document attribute itself is
 * mirrored from `ui.settingsDraft.theme` by the effect in state.svelte.ts.
 */
export async function chooseTheme(theme: Theme) {
  if (ui.settingsDraft.theme === theme) return;
  const previous = normalizeTheme(ui.settingsDraft.theme);
  ui.settingsDraft = { ...ui.settingsDraft, theme };
  const message = theme === 'light' ? '已切换到浅色模式' : '已切换到深色模式';
  if (!isTauri) { notify(message); return; }
  try {
    app.data = await invoke<Bootstrap>('save_settings', { settings: { ...app.data.settings, theme } });
    ui.settingsDraft = { ...ui.settingsDraft, theme: normalizeTheme(app.data.settings.theme) };
    notify(message);
  } catch (error) {
    ui.settingsDraft = { ...ui.settingsDraft, theme: previous };
    guard(error);
  }
}

export async function saveSettings() {
  try {
    app.data = await invoke<Bootstrap>('save_settings', { settings: ui.settingsDraft });
    ui.settingsDraft = { ...app.data.settings };
    notify('启动器设置已保存');
  } catch (error) { guard(error); }
}

export function resetSettingsDraft() {
  ui.settingsDraft = { ...app.data.settings };
  notify('已恢复为当前设置');
}

export async function installJava(major: number) {
  app.busy = true;
  try {
    const runtime = await invoke<Runtime>('install_java', { major });
    app.data.java = await invoke<Runtime[]>('list_java');
    notify(`已准备 Java ${runtime.major}（${runtime.architecture}）`);
  } catch (error) { guard(error); } finally { app.busy = false; }
}

export async function rescanJava() {
  try {
    app.data.java = await invoke<Runtime[]>('list_java');
    notify(`发现 ${app.data.java.length} 个 Java 运行时`);
  } catch (error) { guard(error); }
}

export async function quit() {
  if (isTauri) await invoke('quit');
  else notify('浏览器预览模式');
}

export function about() {
  notify('CubeLauncher 0.1.0 · 离线优先的 Minecraft Java 启动器');
}
