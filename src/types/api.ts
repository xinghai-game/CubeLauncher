import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type Loader = 'vanilla' | 'fabric' | 'forge' | 'neoforge';

export type Account = { id: string; name: string; uuid: string; kind: string; created_at: string };

export type Instance = {
  id: string;
  name: string;
  game_version: string;
  loader: Loader;
  loader_version?: string | null;
  account_id?: string | null;
  java_path?: string | null;
  min_memory_mb: number;
  max_memory_mb: number;
  jvm_args: string[];
  game_args: string[];
  width?: number | null;
  height?: number | null;
  fullscreen: boolean;
  installed: boolean;
  last_played?: string | null;
  created_at: string;
};

export type Runtime = {
  path: string;
  major: number;
  architecture: string;
  vendor?: string | null;
  source: string;
};

export type Settings = {
  schema_version: number;
  data_dir: string;
  theme: string;
  locale: string;
  download_concurrency: number;
  default_memory_mb: number;
  default_java?: string | null;
  offline_mode: boolean;
  mirror_base_url?: string | null;
  java_mirror_base_url?: string | null;
  close_launcher_after_launch: boolean;
};

export type VersionKind = 'release' | 'snapshot' | 'old_beta' | 'old_alpha' | 'april_fools';
export type ManifestSource = 'cache' | 'official' | 'mirror';

/** One manifest entry with the category the core derived for it. */
export type CatalogVersion = {
  id: string;
  type: string;
  kind: VersionKind;
  releaseTime: string;
  url: string;
  sha1: string;
};

/** The version list as the core hands it over: categories plus provenance. */
export type VersionCatalog = {
  latest: { release: string; snapshot: string };
  total: number;
  versions: CatalogVersion[];
  cached: boolean;
  source: ManifestSource;
  fetched_at?: string | null;
};

/** Java major a version needs; `source` says how it was determined. */
export type JavaRequirement = { major: number; source: string };

export type Task = {
  id: string;
  instance_id: string;
  phase: string;
  current: number;
  total: number;
  message: string;
  done: boolean;
  error?: string | null;
};

export type LogLine = { stream: string; line: string; timestamp: string };

export type ModEntry = {
  file_name: string;
  display_name: string;
  enabled: boolean;
  size: number;
  modified?: string | null;
};

export type Bootstrap = {
  settings: Settings;
  accounts: Account[];
  instances: Instance[];
  java: Runtime[];
  running: string[];
  data_dir: string;
};

export type LaunchPreview = {
  java: string;
  args: string[];
  working_dir: string;
  classpath_entries: number;
  natives_dir: string;
  missing_files: string[];
};

export type InstanceStateEvent = { instance_id: string; state: string; exit_code?: number | null };
export type CommandResult = { ok: boolean; message: string };

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) throw new Error('浏览器预览模式无法调用桌面核心');
  return tauriInvoke<T>(command, args);
}

type Handlers = {
  onTask?: (task: Task) => void;
  onLog?: (line: LogLine) => void;
  onState?: (state: InstanceStateEvent) => void;
};

/** Subscribe to launcher events; resolves to an unsubscribe function. */
export async function subscribe(handlers: Handlers): Promise<() => void> {
  if (!isTauri) return () => {};
  const unlisten = await Promise.all([
    listen<Task>('task_updated', (event) => handlers.onTask?.(event.payload)),
    listen<LogLine>('log_batch', (event) => handlers.onLog?.(event.payload)),
    listen<InstanceStateEvent>('instance_state_changed', (event) => handlers.onState?.(event.payload))
  ]);
  return () => unlisten.forEach((off) => off());
}

export const loaderLabels: Record<Loader, string> = {
  vanilla: '原版',
  fabric: 'Fabric',
  forge: 'Forge',
  neoforge: 'NeoForge'
};

export const loaderColors: Record<Loader, string> = {
  vanilla: '#8b9a91',
  fabric: '#62b890',
  forge: '#f0a660',
  neoforge: '#d47c8f'
};

export const kindLabels: Record<VersionKind, string> = {
  release: '正式版',
  snapshot: '快照',
  old_beta: '远古 Beta',
  old_alpha: '远古 Alpha',
  april_fools: '愚人节'
};

export const manifestSourceLabels: Record<ManifestSource, string> = {
  cache: '本地缓存',
  official: '官方源',
  mirror: '镜像'
};

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KiB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
}

export function formatPlayed(value?: string | null): string {
  if (!value) return '还没有启动过';
  const minutes = Math.max(1, Math.floor((Date.now() - new Date(value).getTime()) / 60000));
  if (minutes < 60) return `${minutes} 分钟前启动`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)} 小时前启动`;
  return `${Math.floor(minutes / 1440)} 天前启动`;
}

/** ISO timestamp → `2026-10-06 18:37`, used for catalog refresh times. */
export function formatDateTime(value?: string | null): string {
  if (!value) return '未知';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '未知';
  const pad = (input: number) => String(input).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}
