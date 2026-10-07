import type { Settings } from '../types/api';

export const BMCLAPI_URL = 'https://bmclapi2.bangbang93.com';
export type DownloadSource = 'official' | 'bmclapi' | 'custom';
export type DownloadSourceDraft = { source: DownloadSource; customUrl: string };

/** Read older settings without replacing an existing custom mirror. */
export function downloadSourceDraft(value?: string | null): DownloadSourceDraft {
  const url = value?.trim() ?? '';
  if (!url) return { source: 'official', customUrl: '' };
  try {
    const parsed = new URL(url);
    if (parsed.origin === BMCLAPI_URL && /^\/+$/.test(parsed.pathname) && !parsed.search && !parsed.hash
      && !parsed.username && !parsed.password && !url.includes('?') && !url.includes('#')) {
      return { source: 'bmclapi', customUrl: '' };
    }
  } catch {
    // Keep invalid legacy values editable; validate when the user saves.
  }
  return { source: 'custom', customUrl: value?.trim() ?? '' };
}

/** Preset selection never changes the separately retained custom URL draft. */
export function downloadMirrorUrl(draft: DownloadSourceDraft): string | null {
  if (draft.source === 'official') return null;
  if (draft.source === 'bmclapi') return BMCLAPI_URL;
  return draft.customUrl.trim() || null;
}

function normalizeMirrorUrl(value: string | null | undefined, label: string): string | null {
  const url = value?.trim() || null;
  if (!url) return null;
  try {
    const parsed = new URL(url);
    if (/^https?:\/\//i.test(url) && ['http:', 'https:'].includes(parsed.protocol) && parsed.hostname
      && !parsed.username && !parsed.password && !url.includes('?') && !url.includes('#')) {
      return url.replace(/\/+$/, '');
    }
  } catch {
    // Report one actionable error for malformed URLs and unsupported protocols.
  }
  throw new Error(`${label}必须是完整的 http:// 或 https:// URL，且不能包含账号、查询参数或片段`);
}

/** Validate at the save boundary; do not silently clamp or round user input. */
export function normalizeDownloadSettings(settings: Settings): Settings {
  if (!Number.isInteger(settings.download_concurrency)
    || settings.download_concurrency < 1 || settings.download_concurrency > 16) {
    throw new Error('下载总连接上限必须是 1–16 的整数');
  }
  return {
    ...settings,
    mirror_base_url: normalizeMirrorUrl(settings.mirror_base_url, '游戏与依赖镜像地址'),
    java_mirror_base_url: normalizeMirrorUrl(settings.java_mirror_base_url, 'Java 镜像地址')
  };
}
