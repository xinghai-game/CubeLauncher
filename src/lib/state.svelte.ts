import type {
  Bootstrap, GameDirScan, Instance, JavaRequirement, LaunchPreview, Loader, ResourceEntry, Settings,
  Task, VersionCatalog
} from '../types/api';
import type { KindFilter } from './versions';
import { applyTheme, readCachedTheme } from './theme';
import { downloadSourceDraft } from './downloads';

export type DetailTab = 'overview' | 'mods' | 'shaders' | 'projections' | 'logs';
export type SettingsSection = 'general' | 'account' | 'download' | 'java' | 'appearance' | 'about';

/**
 * Pages are hierarchical, like HMCL's: a top-level page (version list, settings)
 * opens a deeper one (install wizard, settings section), and each deeper page
 * knows the route it came from so it can go back.
 */
export type Route =
  | { name: 'home' }
  | { name: 'accounts' }
  | { name: 'account'; accountId: string }
  | { name: 'instances'; tab: DetailTab }
  | { name: 'instance'; instanceId: string; tab: DetailTab }
  | { name: 'versions'; kind: KindFilter; query: string }
  | { name: 'wizard'; version: string; step: 1 | 2 | 3 }
  | { name: 'install'; instanceId: string }
  /** Import an existing `.minecraft`, or point an existing instance at one. */
  | { name: 'import'; instanceId: string | null }
  | { name: 'settings'; section: SettingsSection | null };

export const emptySettings: Settings = {
  schema_version: 1, data_dir: 'CubeLauncher', theme: 'dark', locale: 'zh-CN',
  download_concurrency: 4, default_memory_mb: 4096, offline_mode: false,
  mirror_base_url: null, java_mirror_base_url: null, close_launcher_after_launch: false,
  // The draft keeps a string (inputs cannot bind to null); saving turns "" into null.
  microsoft_client_id: ''
};

const demo: Bootstrap = {
  settings: emptySettings, accounts: [], instances: [], java: [], running: [], data_dir: '浏览器预览模式'
};

export type LoaderVersionState = { loading: boolean; error: string; versions: string[] };

/** Everything that mirrors backend data or a running job. */
export const app = $state({
  data: demo as Bootstrap,
  catalog: null as VersionCatalog | null,
  catalogLoading: false,
  catalogError: '',
  javaRequirement: null as JavaRequirement | null,
  javaLoading: false,
  /** Loader version lists, keyed `game|loader`, so switching back is instant. */
  loaderVersions: {} as Record<string, LoaderVersionState>,
  task: null as Task | null,
  taskStartedAt: 0,
  installStartingId: null as string | null,
  installCancellingId: null as string | null,
  /** Instance whose file check is running, so “校验” can show it immediately. */
  verifyingId: null as string | null,
  /**
   * Instance whose launch request is still in flight. It is not `busy`: the
   * launch command only resolves when the game exits, so tying a global flag to
   * it would silently disable buttons for the whole play session.
   */
  launchingId: null as string | null,
  liveLogs: [] as string[],
  fileLogs: [] as string[],
  resourceEntries: [] as ResourceEntry[],
  resourceLoading: false,
  preview: null as LaunchPreview | null,
  /** Skin heads already fetched, keyed by account id (data URLs, per session). */
  skins: {} as Record<string, string>,
  busy: false
});

/** A 正版 sign-in in flight: the device code, its state, and the failure count. */
export type LoginFlow = {
  active: boolean;
  starting: boolean;
  loginId: string;
  userCode: string;
  verificationUri: string;
  message: string;
  error: string;
  /** Seconds between two polls; Microsoft may ask for more. */
  interval: number;
  /** Local deadline, so a forgotten dialog stops polling on its own. */
  expiresAt: number;
  /** Consecutive transport failures; three in a row end the attempt. */
  failures: number;
};

export const idleLogin: LoginFlow = {
  active: false, starting: false, loginId: '', userCode: '', verificationUri: '',
  message: '', error: '', interval: 5, expiresAt: 0, failures: 0
};

/** Everything the user is currently looking at or editing. */
export const ui = $state({
  route: { name: 'home' } as Route,
  history: [] as Route[],
  selectedId: null as string | null,
  detailTab: 'overview' as DetailTab,
  visibleVersions: 60,
  showAccount: false,
  showDeleteConfirm: false,
  /** Which account the user chose when no instance is selected. */
  accountSelectionId: null as string | null,
  /** Which half of the account dialog is showing. */
  accountTab: 'microsoft' as 'microsoft' | 'offline',
  accountName: '',
  login: { ...idleLogin },
  settingsDraft: { ...emptySettings, theme: readCachedTheme() } as Settings,
  downloadSourceDraft: downloadSourceDraft(emptySettings.mirror_base_url),
  instanceDraft: null as Instance | null,
  form: {
    name: '',
    version: '',
    loader: 'vanilla' as Loader,
    loaderVersion: ''
  },
  /** Import page: the picked folder, what the scan found, and the draft name. */
  importFlow: {
    path: '',
    scanning: false,
    scan: null as GameDirScan | null,
    error: '',
    versionId: '',
    name: '',
    accountId: null as string | null
  },
  toast: '',
  toastError: false
});

export function loaderKey(gameVersion: string, loader: Loader): string {
  return `${gameVersion}|${loader}`;
}

/**
 * The theme lives on `<html data-theme>`, not in Svelte, so mirror the draft onto
 * the document. The theme buttons only assign `draft.theme`; bootstrap/save/reset
 * all reassign `settingsDraft`, so every path lands here.
 */
$effect.root(() => {
  $effect(() => { applyTheme(ui.settingsDraft.theme); });
});

/** Drop per-version drafts when leaving the install flow. */
export function resetWizardForm(version: string) {
  ui.form = { name: '', version, loader: 'vanilla', loaderVersion: '' };
  app.javaRequirement = null;
}

/** Start with a clean import page; `instanceId` re-binds that instance instead. */
export function resetImportFlow() {
  ui.importFlow = {
    path: '', scanning: false, scan: null, error: '', versionId: '', name: '', accountId: null
  };
}
