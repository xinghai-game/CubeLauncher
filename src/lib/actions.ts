import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { openPath, openUrl } from '@tauri-apps/plugin-opener';
import {
  invoke, isTauri, subscribe,
  type Account, type Bootstrap, type DeviceCodePrompt, type GameDirScan, type Instance,
  type JavaRequirement, type LaunchPreview, type Loader, type LoginStatus, type ResourceEntry,
  type ResourceKind, type Runtime, type Task, type VersionCatalog
} from '../types/api';
import {
  app, idleLogin, loaderKey, resetImportFlow, resetWizardForm, ui, type DetailTab, type Route,
  type SettingsSection
} from './state.svelte';
import { suggestInstanceName, type KindFilter } from './versions';
import { installInFlight, NOT_INSTALLED } from './install';
import { normalizeTheme, type Theme } from './theme';
import { downloadMirrorUrl, downloadSourceDraft, normalizeDownloadSettings } from './downloads';

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

export function goAccounts() {
  reset({ name: 'accounts' });
}

export function openAccount(id: string) {
  ui.showAccount = false;
  const account = app.data.accounts.find((item) => item.id === id);
  if (account) void loadSkin(account);
  go({ name: 'account', accountId: id });
}

export function goInstances(tab: DetailTab = 'overview') {
  ui.detailTab = tab;
  reset({ name: 'instances', tab });
}

/** Open one instance as its own page, preserving the selected resource tab. */
export async function openInstance(id: string, tab: DetailTab = 'overview') {
  ui.detailTab = tab;
  await selectInstance(id);
  go({ name: 'instance', instanceId: id, tab });
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
          if (app.installCancellingId === update.instance_id) app.installCancellingId = null;
          if (update.phase === 'cancelled') notify('安装已取消，已保留下载进度，可继续安装');
          else if (update.error) notify(update.error, true);
          else notify(update.message);
          void refreshAll();
        }
      },
      onLog: (line) => { app.liveLogs = [...app.liveLogs.slice(-500), `[${line.stream}] ${line.line}`]; },
      onState: (state) => {
        // The launch command stays pending until the game exits. Release the
        // "launching" flag as soon as the process state is known, otherwise the
        // rest of the launcher would look broken for the whole play session.
        if (app.launchingId === state.instance_id) app.launchingId = null;
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
    ui.downloadSourceDraft = downloadSourceDraft(app.data.settings.mirror_base_url);
    if (app.data.instances[0]) await selectInstance(app.data.instances[0].id);
  } catch {
    // Browser preview keeps the empty shell.
  }
}

export async function refreshAll() {
  try {
    const next = await invoke<Bootstrap>('bootstrap');
    app.data = next;
    if (ui.accountSelectionId && !next.accounts.some((item) => item.id === ui.accountSelectionId)) {
      ui.accountSelectionId = null;
    }
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
    ?? app.data.accounts.find((item) => item.id === ui.accountSelectionId)
    ?? app.data.accounts[0]
    ?? null;
}

export function isResourceTab(tab: DetailTab): tab is ResourceKind {
  return tab === 'mods' || tab === 'shaders' || tab === 'projections';
}

let resourceRequest = 0;

export async function selectInstance(id: string) {
  ui.selectedId = id;
  resourceRequest += 1;
  app.resourceLoading = false;
  const instance = app.data.instances.find((item) => item.id === id) ?? null;
  ui.instanceDraft = instance ? { ...instance } : null;
  app.preview = null;
  app.liveLogs = [];
  app.resourceEntries = [];
  app.fileLogs = [];
  if (!instance) return;
  if (isResourceTab(ui.detailTab)) await loadResources(ui.detailTab);
  if (ui.detailTab === 'logs') await loadFileLogs();
}

export async function setDetailTab(tab: DetailTab) {
  ui.detailTab = tab;
  resourceRequest += 1;
  app.resourceLoading = false;
  if (ui.route.name === 'instance') ui.route = { ...ui.route, tab };
  if (isResourceTab(tab)) await loadResources(tab);
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

/* ------------------------------------------------------------- 正版登录 */

/** Open on the tab that matches the identity currently in use. */
export function openAccounts() {
  ui.showAccount = true;
  ui.accountTab = activeAccount()?.kind === 'offline' ? 'offline' : 'microsoft';
  void loadSkins();
}

let loginTimer: ReturnType<typeof setTimeout> | null = null;

function clearLoginTimer() {
  if (loginTimer !== null) { clearTimeout(loginTimer); loginTimer = null; }
}

function schedulePoll(seconds: number) {
  clearLoginTimer();
  loginTimer = setTimeout(() => { void pollMicrosoftLogin(); }, Math.max(1, seconds) * 1000);
}

function failLogin(message: string, error = true) {
  clearLoginTimer();
  ui.login = { ...ui.login, active: false, starting: false, error: message };
  notify(message, error);
}

/** Start the device code flow and open Microsoft's page in the browser. */
export async function startMicrosoftLogin() {
  if (ui.login.active || ui.login.starting) return;
  clearLoginTimer();
  ui.login = { ...idleLogin, starting: true };
  try {
    const prompt = await invoke<DeviceCodePrompt>('start_microsoft_login');
    ui.login = {
      ...idleLogin,
      active: true,
      loginId: prompt.login_id,
      userCode: prompt.user_code,
      verificationUri: prompt.verification_uri,
      message: prompt.message,
      interval: Math.max(1, prompt.interval),
      expiresAt: Date.now() + Math.max(60, prompt.expires_in) * 1000
    };
    // Opening the page immediately is the whole point of the flow: the code is
    // already on screen, so the user only has to paste it.
    if (isTauri) { try { await openUrl(prompt.verification_uri); } catch { /* shown as a link */ } }
    schedulePoll(ui.login.interval);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    ui.login = { ...idleLogin, error: message };
    guard(error);
  }
}

/** One poll; the loop re-schedules itself until the sign-in ends. */
export async function pollMicrosoftLogin() {
  if (!ui.login.active || !ui.login.loginId) return;
  if (Date.now() > ui.login.expiresAt) {
    failLogin('设备代码已过期，请重新发起登录');
    return;
  }
  try {
    const status = await invoke<LoginStatus>('poll_microsoft_login', { loginId: ui.login.loginId });
    ui.login = { ...ui.login, failures: 0, error: '' };
    switch (status.state) {
      case 'pending':
        schedulePoll(ui.login.interval);
        return;
      case 'slow_down':
        ui.login = { ...ui.login, interval: status.interval ?? ui.login.interval + 5 };
        schedulePoll(ui.login.interval);
        return;
      case 'ready': {
        const account = status.account;
        clearLoginTimer();
        ui.login = { ...idleLogin };
        if (account) {
          app.data.accounts = [...app.data.accounts.filter((item) => item.id !== account.id), account];
          notify(`已登录正版账户 ${account.name}`);
          void loadSkin(account);
        } else {
          notify('登录成功');
        }
        await refreshAll();
        return;
      }
      case 'expired':
        failLogin('设备代码已过期，请重新发起登录');
        return;
      case 'declined':
        failLogin('已取消登录', false);
        return;
      default:
        failLogin(status.message ?? '登录失败', true);
    }
  } catch (error) {
    // A dropped connection is not a failed sign-in: keep trying, but do not
    // spin forever on a real outage.
    const failures = ui.login.failures + 1;
    if (failures >= 3) {
      failLogin(error instanceof Error ? error.message : String(error));
      return;
    }
    ui.login = { ...ui.login, failures, error: error instanceof Error ? error.message : String(error) };
    schedulePoll(ui.login.interval);
  }
}

/** Give up on a pending sign-in and tell the core to forget it. */
export async function cancelMicrosoftLogin() {
  const loginId = ui.login.loginId;
  clearLoginTimer();
  ui.login = { ...idleLogin };
  if (loginId && isTauri) {
    try { await invoke('cancel_microsoft_login', { loginId }); } catch { /* already gone */ }
  }
}

export async function copyUserCode() {
  const code = ui.login.userCode;
  if (!code) return;
  try {
    await navigator.clipboard.writeText(code);
    notify(`已复制 ${code}`);
  } catch {
    notify('无法访问剪贴板，请手动抄写代码', true);
  }
}

export async function openVerificationPage() {
  if (!ui.login.verificationUri) return;
  if (!isTauri) { notify(ui.login.verificationUri); return; }
  try { await openUrl(ui.login.verificationUri); } catch (error) { guard(error); }
}

/** Skin heads are decoration: a failure silently keeps the letter avatar. */
export async function loadSkin(account: Account) {
  if (!account.microsoft?.skin_url || app.skins[account.id]) return;
  try {
    const data = await invoke<string | null>('account_skin', { id: account.id });
    if (data) app.skins = { ...app.skins, [account.id]: data };
  } catch { /* letter avatar */ }
}

export async function loadSkins() {
  await Promise.all(app.data.accounts.map((account) => loadSkin(account)));
}

/** Renew a 正版 session without asking the user for anything. */
export async function refreshAccount(account: Account) {
  try {
    const updated = await invoke<Account>('refresh_account', { id: account.id });
    app.data.accounts = app.data.accounts.map((item) => (item.id === updated.id ? updated : item));
    notify(`已刷新 ${updated.name} 的登录`);
    void loadSkin(updated);
  } catch (error) { guard(error); }
}

export async function chooseAccount(account: Account) {
  ui.accountSelectionId = account.id;
  const selected = app.data.instances.find((item) => item.id === ui.selectedId) ?? null;
  if (!selected || !ui.instanceDraft) { notify(`已选择 ${account.name}`); return; }
  ui.instanceDraft = { ...ui.instanceDraft, account_id: account.id };
  await saveInstanceDraft();
  notify(`已为 ${selected.name} 选择 ${account.name}`);
}

export async function copyAccountUuid(account: Account) {
  try {
    await navigator.clipboard.writeText(account.uuid);
    notify(`已复制 ${account.name} 的 UUID`);
  } catch {
    notify('无法访问剪贴板，请手动复制 UUID', true);
  }
}

export async function removeAccount(account: Account) {
  try {
    await invoke('delete_account', { id: account.id });
    app.data.accounts = app.data.accounts.filter((item) => item.id !== account.id);
    const skins = { ...app.skins };
    delete skins[account.id];
    app.skins = skins;
    if (ui.accountSelectionId === account.id) ui.accountSelectionId = null;
    // The deleted account may have been the one the dialog was signing in as.
    if (ui.login.active) await cancelMicrosoftLogin();
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

export async function installInstance(instanceId: string): Promise<boolean> {
  // Only this instance's own install blocks a retry. A task left behind by a
  // Java download or by another instance used to freeze the button for good.
  if (installInFlight(instanceId, app.task, app.installStartingId)) {
    notify('该实例正在安装，可在安装页查看进度或取消');
    return false;
  }
  app.installStartingId = instanceId;
  app.installCancellingId = null;
  // Set the placeholder before invoking: progress events can arrive before the response.
  app.task = {
    id: '', instance_id: instanceId, phase: 'prepare', current: 0, total: 0,
    message: '准备安装，检查可复用与可续传的文件', done: false,
    downloaded_bytes: 0, total_bytes: 0, active: []
  };
  app.taskStartedAt = Date.now();
  try {
    const taskId = await invoke<string>('install_instance', { instanceId });
    if (app.task?.instance_id === instanceId && !app.task.id) app.task.id = taskId;
    return true;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (app.task?.instance_id === instanceId && !app.task.id) {
      app.task = { ...app.task, phase: 'failed', done: true, message, error: message };
    }
    guard(error);
    return false;
  } finally {
    app.installStartingId = null;
  }
}

/**
 * “修复/更新” and “继续安装”. A click must always answer: while this instance is
 * already installing we simply show its progress, and we only leave the page once
 * the install really started — landing on an empty progress page for a request
 * that was refused looked exactly like a dead button.
 */
export async function installSelected() {
  const instanceId = ui.selectedId;
  if (!instanceId) { notify('请先选择一个实例', true); return; }
  if (installInFlight(instanceId, app.task, app.installStartingId)) {
    go({ name: 'install', instanceId });
    return;
  }
  if (await installInstance(instanceId)) go({ name: 'install', instanceId });
}

export async function cancelInstall(instanceId: string) {
  if (app.installStartingId || app.installCancellingId || !app.task
    || app.task.instance_id !== instanceId || app.task.done) return;
  app.installCancellingId = instanceId;
  try {
    await invoke('cancel_install', { instanceId });
    // Keep the pending state until the worker publishes its terminal event.
  } catch (error) {
    if (app.installCancellingId === instanceId) app.installCancellingId = null;
    guard(error);
  }
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
  } catch (error) {
    guard(error);
  } finally {
    app.busy = false;
  }
}

/**
 * Check that every file the instance needs is on disk. The scan can take a few
 * seconds on a full installation, so the button switches to a busy state; the
 * result is always reported instead of leaving the click unanswered.
 */
export async function verifySelected() {
  const instanceId = ui.selectedId;
  if (!instanceId) { notify('请先选择一个实例', true); return; }
  if (app.verifyingId) { notify('正在校验，请稍候'); return; }
  app.verifyingId = instanceId;
  try {
    const missing = await invoke<string[]>('verify_instance', { instanceId });
    if (missing.length === 0) {
      notify('校验通过：所有文件齐全，可以离线启动');
    } else if (missing.length === 1 && missing[0] === NOT_INSTALLED) {
      notify(`实例尚未安装，请点击“${selectedInstalled() ? '修复/更新' : '继续安装'}”准备文件`, true);
    } else {
      notify(`校验完成：缺少 ${missing.length} 项（${missing.slice(0, 3).join('、')}），可点击“修复/更新”补齐`, true);
    }
  } catch (error) { guard(error); }
  finally { if (app.verifyingId === instanceId) app.verifyingId = null; }
}

/** Whether the selected instance already has an install record. */
function selectedInstalled(): boolean {
  return app.data.instances.find((item) => item.id === ui.selectedId)?.installed ?? false;
}

export async function launchSelected() {
  const instanceId = ui.selectedId;
  if (!instanceId) { notify('请先选择一个实例', true); return; }
  if (app.launchingId) { notify('正在启动游戏，请稍候'); return; }
  app.launchingId = instanceId;
  try {
    const result = await invoke<{ ok: boolean; message: string }>('launch_instance', { instanceId });
    notify(result.message, !result.ok);
  } catch (error) { guard(error); }
  finally {
    if (app.launchingId === instanceId) app.launchingId = null;
    await refreshAll();
  }
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

/* ---------------------------------------------------------- instance resources */

const resourceLabels: Record<ResourceKind, string> = {
  mods: '模组',
  shaders: '光影',
  projections: '投影'
};

const resourceFilters: Record<ResourceKind, { name: string; extensions: string[] }> = {
  mods: { name: '模组', extensions: ['jar'] },
  shaders: { name: '光影', extensions: ['zip', 'jar'] },
  projections: { name: '投影文件', extensions: ['litematic', 'schematic', 'schem'] }
};

export async function loadResources(kind: ResourceKind = 'mods') {
  const instanceId = ui.selectedId;
  if (!instanceId) return;
  const request = ++resourceRequest;
  app.resourceLoading = true;
  try {
    const entries = await invoke<ResourceEntry[]>('list_resources', { instanceId, kind });
    if (request === resourceRequest && ui.selectedId === instanceId) {
      app.resourceEntries = entries;
    }
  } catch (error) { guard(error); }
  finally {
    if (request === resourceRequest) app.resourceLoading = false;
  }
}

export async function addResource(kind: ResourceKind) {
  if (!ui.selectedId) return;
  if (!isTauri) { notify('浏览器预览模式无法选择文件', true); return; }
  try {
    const picked = await openDialog({ multiple: true, filters: [resourceFilters[kind]] });
    const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
    for (const path of paths) {
      await invoke('add_resource', { instanceId: ui.selectedId, kind, path });
    }
    if (paths.length) notify(`已添加 ${paths.length} 个${resourceLabels[kind]}文件`);
    await loadResources(kind);
  } catch (error) { guard(error); }
}

export async function toggleResource(kind: ResourceKind, resource: ResourceEntry) {
  if (!ui.selectedId) return;
  try {
    await invoke('toggle_resource', {
      instanceId: ui.selectedId, kind, fileName: resource.file_name, enabled: !resource.enabled
    });
    await loadResources(kind);
  } catch (error) { guard(error); }
}

export async function removeResource(kind: ResourceKind, resource: ResourceEntry) {
  if (!ui.selectedId) return;
  try {
    await invoke('delete_resource', {
      instanceId: ui.selectedId, kind, fileName: resource.file_name
    });
    await loadResources(kind);
    notify(`已删除 ${resource.display_name}`);
  } catch (error) { guard(error); }
}

/** Compatibility wrappers for callers that still use the old Mod API. */
export async function loadMods() { await loadResources('mods'); }
export async function addMod() { await addResource('mods'); }
export async function toggleMod(resource: ResourceEntry) { await toggleResource('mods', resource); }
export async function removeMod(resource: ResourceEntry) { await removeResource('mods', resource); }

export async function openFolder(kind: 'instance' | 'mods' | 'shaders' | 'projections' | 'game') {
  if (!ui.selectedId) return;
  const command = kind === 'game' ? 'game_dir' : kind === 'instance' ? 'instance_dir' : 'resource_dir';
  try {
    const args = command === 'resource_dir'
      ? { instanceId: ui.selectedId, kind }
      : { instanceId: ui.selectedId };
    const path = await invoke<string>(command, args);
    if (isTauri) await openPath(path); else notify(path);
  } catch (error) { guard(error); }
}

/* ------------------------------------------------------- game directories */

/**
 * Open the import page: pick an existing `.minecraft`, see what is inside and
 * register one of its versions. With `instanceId` the same page re-binds an
 * existing instance to another directory instead of creating one.
 */
export function goImport(instanceId: string | null = null) {
  resetImportFlow();
  ui.importFlow.accountId = activeAccount()?.id ?? null;
  go({ name: 'import', instanceId });
}

/** Instance the import page is re-binding, when it was opened from an instance. */
export function importBindingId(): string | null {
  return ui.route.name === 'import' ? ui.route.instanceId : null;
}

/** Ask for a folder, then look inside it. */
export async function pickGameDir() {
  if (!isTauri) { notify('浏览器预览模式无法选择目录', true); return; }
  try {
    const picked = await openDialog({ directory: true, multiple: false, title: '选择 .minecraft 目录' });
    const path = Array.isArray(picked) ? picked[0] : picked;
    if (!path) return;
    ui.importFlow.path = path;
    await scanGameDir(path);
  } catch (error) { guard(error); }
}

/** Read one directory and offer the versions it holds. */
export async function scanGameDir(path = ui.importFlow.path) {
  const target = path.trim();
  ui.importFlow.path = target;
  ui.importFlow.error = '';
  if (!target) { ui.importFlow.scan = null; notify('请先选择一个 .minecraft 目录', true); return; }
  ui.importFlow.scanning = true;
  try {
    const scan = await invoke<GameDirScan>('scan_game_dir', { path: target });
    ui.importFlow.scan = scan;
    // A single version is what the user came for: preselect it and name the
    // instance after the folder, the way the other launcher labelled it.
    const first = scan.versions.find((version) => version.launchable) ?? scan.versions[0] ?? null;
    ui.importFlow.versionId = first?.id ?? '';
    if (!importBindingId()) ui.importFlow.name = first?.id ?? '';
    if (scan.versions.length === 0) notify('这个目录里没有可用的版本文件', true);
  } catch (error) {
    ui.importFlow.scan = null;
    ui.importFlow.error = error instanceof Error ? error.message : String(error);
    guard(error);
  } finally {
    ui.importFlow.scanning = false;
  }
}

export function chooseImportVersion(id: string) {
  ui.importFlow.versionId = id;
  if (!importBindingId()) ui.importFlow.name = id;
}

/** Create an instance from the scanned directory, or re-bind the current one. */
export async function confirmImport() {
  const flow = ui.importFlow;
  const version = flow.scan?.versions.find((entry) => entry.id === flow.versionId) ?? null;
  if (!flow.scan) { notify('请先扫描一个 .minecraft 目录', true); return; }
  if (!version) { notify('请选择要导入的版本', true); return; }
  if (!version.launchable) { notify('该版本文件缺少 mainClass，无法启动', true); return; }
  const binding = ui.route.name === 'import' ? ui.route.instanceId : null;
  app.busy = true;
  try {
    if (binding) {
      const instance = await invoke<Instance>('bind_instance_game_dir', {
        instanceId: binding, path: flow.scan.game_dir, versionId: version.id
      });
      app.data.instances = app.data.instances.map((item) => (item.id === instance.id ? instance : item));
      await selectInstance(instance.id);
      notify(`${instance.name} 已切换到 ${version.id}`);
      reset({ name: 'instances', tab: 'overview' });
      return;
    }
    const name = flow.name.trim() || version.id;
    const instance = await invoke<Instance>('import_instance', {
      input: {
        name,
        path: flow.scan.game_dir,
        version_id: version.id,
        account_id: flow.accountId ?? activeAccount()?.id ?? null
      }
    });
    app.data.instances = [instance, ...app.data.instances];
    await selectInstance(instance.id);
    reset({ name: 'instances', tab: 'overview' });
    notify(`已导入 ${instance.name}：${version.id}，存档与模组留在原目录`);
  } catch (error) {
    guard(error);
  } finally {
    app.busy = false;
  }
}

/** Send an instance back to the launcher-managed game directory. */
export async function unbindGameDir() {
  if (!ui.selectedId) return;
  try {
    const instance = await invoke<Instance>('bind_instance_game_dir', {
      instanceId: ui.selectedId, path: null, versionId: null
    });
    app.data.instances = app.data.instances.map((item) => (item.id === instance.id ? instance : item));
    ui.instanceDraft = { ...instance };
    notify('已恢复为启动器管理的游戏目录，需要重新安装');
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
    const settings = normalizeDownloadSettings({
      ...ui.settingsDraft,
      mirror_base_url: downloadMirrorUrl(ui.downloadSourceDraft),
      // Blank means "use the built-in application id", which is `null`, not "".
      microsoft_client_id: ui.settingsDraft.microsoft_client_id?.trim() || null
    });
    app.data = await invoke<Bootstrap>('save_settings', { settings });
    ui.settingsDraft = { ...app.data.settings };
    const sourceDraft = downloadSourceDraft(app.data.settings.mirror_base_url);
    ui.downloadSourceDraft = {
      ...sourceDraft,
      customUrl: sourceDraft.source === 'custom' ? sourceDraft.customUrl : ui.downloadSourceDraft.customUrl
    };
    notify('启动器设置已保存');
  } catch (error) { guard(error); }
}

export function resetSettingsDraft() {
  ui.settingsDraft = { ...app.data.settings };
  ui.downloadSourceDraft = downloadSourceDraft(app.data.settings.mirror_base_url);
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
