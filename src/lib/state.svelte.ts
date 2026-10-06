import type {
  Bootstrap, Instance, JavaRequirement, LaunchPreview, Loader, ModEntry, Settings, Task,
  VersionCatalog
} from '../types/api';
import type { KindFilter } from './versions';
import { applyTheme, readCachedTheme } from './theme';

export type DetailTab = 'overview' | 'mods' | 'logs';
export type SettingsSection = 'general' | 'download' | 'java' | 'appearance' | 'about';

/**
 * Pages are hierarchical, like HMCL's: a top-level page (version list, settings)
 * opens a deeper one (install wizard, settings section), and each deeper page
 * knows the route it came from so it can go back.
 */
export type Route =
  | { name: 'home' }
  | { name: 'instances'; tab: DetailTab }
  | { name: 'versions'; kind: KindFilter; query: string }
  | { name: 'wizard'; version: string; step: 1 | 2 | 3 }
  | { name: 'install'; instanceId: string }
  | { name: 'settings'; section: SettingsSection | null };

export const emptySettings: Settings = {
  schema_version: 1, data_dir: 'CubeLauncher', theme: 'dark', locale: 'zh-CN',
  download_concurrency: 4, default_memory_mb: 4096, offline_mode: false,
  mirror_base_url: null, java_mirror_base_url: null, close_launcher_after_launch: false
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
  liveLogs: [] as string[],
  fileLogs: [] as string[],
  mods: [] as ModEntry[],
  preview: null as LaunchPreview | null,
  busy: false
});

/** Everything the user is currently looking at or editing. */
export const ui = $state({
  route: { name: 'home' } as Route,
  history: [] as Route[],
  selectedId: null as string | null,
  detailTab: 'overview' as DetailTab,
  visibleVersions: 60,
  showAccount: false,
  showDeleteConfirm: false,
  accountName: '',
  settingsDraft: { ...emptySettings, theme: readCachedTheme() } as Settings,
  instanceDraft: null as Instance | null,
  form: {
    name: '',
    version: '',
    loader: 'vanilla' as Loader,
    loaderVersion: ''
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
