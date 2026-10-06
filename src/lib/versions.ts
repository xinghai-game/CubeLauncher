import type { CatalogVersion, Instance, Task, VersionKind } from '../types/api';

/** Categories in the order the version list shows them, HMCL-style. */
export const kindOrder: VersionKind[] = ['release', 'snapshot', 'old_beta', 'old_alpha', 'april_fools'];

export type KindFilter = VersionKind | 'all';

export const filterOrder: KindFilter[] = ['release', 'snapshot', 'old_beta', 'old_alpha', 'april_fools', 'all'];

export const filterLabels: Record<KindFilter, string> = {
  release: '正式版',
  snapshot: '快照',
  old_beta: '远古 Beta',
  old_alpha: '远古 Alpha',
  april_fools: '愚人节',
  all: '全部'
};

export const filterHints: Record<KindFilter, string> = {
  release: '可以长期游玩的稳定版本',
  snapshot: '开发中的预览版本，存档可能不兼容',
  old_beta: '2010–2011 年的 Beta 版本，需要旧版 Java',
  old_alpha: '2009–2010 年的 Alpha 版本，需要旧版 Java',
  april_fools: 'Mojang 的愚人节玩笑版本，仅供体验',
  all: '清单中的每一个版本'
};

/** How many versions each tab would show. */
export function countKinds(versions: CatalogVersion[]): Record<KindFilter, number> {
  const counts: Record<KindFilter, number> = {
    release: 0, snapshot: 0, old_beta: 0, old_alpha: 0, april_fools: 0, all: versions.length
  };
  for (const version of versions) {
    if (version.kind in counts) counts[version.kind] += 1;
  }
  return counts;
}

/**
 * Versions of one category matching a search query. The catalog arrives newest
 * first and keeps that order; an empty query matches everything.
 */
export function filterVersions(
  versions: CatalogVersion[],
  kind: KindFilter,
  query: string
): CatalogVersion[] {
  const needle = query.trim().toLowerCase();
  return versions.filter((version) => {
    if (kind !== 'all' && version.kind !== kind) return false;
    if (!needle) return true;
    return version.id.toLowerCase().includes(needle);
  });
}

/** Release date as `2024-08-08`, without the time of day. */
export function formatReleaseDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const pad = (input: number) => String(input).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** Human label for the install phases the core reports. */
export function phaseLabel(phase: string): string {
  const labels: Record<string, string> = {
    prepare: '准备安装',
    download: '下载文件',
    loader: '安装加载器',
    java: '准备 Java 运行时',
    done: '安装完成',
    error: '安装失败'
  };
  return labels[phase] ?? phase;
}

/**
 * Instances that already use a game version, so the list can mark it as installed
 * instead of offering an install that would duplicate it.
 */
export function installedIndex(instances: Instance[]): Record<string, Instance[]> {
  const index: Record<string, Instance[]> = {};
  for (const instance of instances) {
    (index[instance.game_version] ??= []).push(instance);
  }
  return index;
}

/** Default instance name offered by the wizard, e.g. `1.20.1` or `1.20.1-fabric`. */
export function suggestInstanceName(version: string, loader: string): string {
  return loader === 'vanilla' ? version : `${version}-${loader}`;
}

/** True while an install is running for one instance. */
export function isInstalling(task: Task | null, instanceId: string | null): boolean {
  return Boolean(task && !task.done && instanceId && task.instance_id === instanceId);
}
