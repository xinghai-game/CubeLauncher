import { describe, expect, it } from 'vitest';
import {
  BMCLAPI_URL, downloadMirrorUrl, downloadSourceDraft, normalizeDownloadSettings
} from '../src/lib/downloads';
import type { Settings } from '../src/types/api';

const settings: Settings = {
  schema_version: 1, data_dir: 'CubeLauncher', theme: 'dark', locale: 'zh-CN',
  download_concurrency: 4, default_memory_mb: 4096, offline_mode: false,
  mirror_base_url: null, java_mirror_base_url: null, close_launcher_after_launch: false
};

describe('download source compatibility', () => {
  it.each([undefined, null, '', '   '])('reads an absent or blank legacy mirror as official: %s', (url) => {
    const draft = downloadSourceDraft(url);
    expect(draft.source).toBe('official');
    expect(downloadMirrorUrl(draft)).toBeNull();
  });

  it.each([BMCLAPI_URL, `${BMCLAPI_URL}/`, ` ${BMCLAPI_URL}/// `])('recognizes the BMCLAPI preset: %s', (url) => {
    const draft = downloadSourceDraft(url);
    expect(draft.source).toBe('bmclapi');
    expect(downloadMirrorUrl(draft)).toBe(BMCLAPI_URL);
  });

  it.each(['https://mirror.example/minecraft', `${BMCLAPI_URL}/custom`, 'http://localhost:8080/base', '/', 'not a URL'])('keeps legacy custom values editable: %s', (url) => {
    const draft = downloadSourceDraft(` ${url} `);
    expect(draft).toEqual({ source: 'custom', customUrl: url });
    expect(downloadMirrorUrl(draft)).toBe(url);
  });

  it('retains the custom address while the user switches through both presets', () => {
    const draft = downloadSourceDraft('https://mirror.example/base');
    draft.source = 'official';
    expect(downloadMirrorUrl(draft)).toBeNull();
    draft.source = 'bmclapi';
    expect(downloadMirrorUrl(draft)).toBe(BMCLAPI_URL);
    draft.source = 'custom';
    expect(downloadMirrorUrl(draft)).toBe('https://mirror.example/base');
  });

  it('normalizes an empty custom input to the official source', () => {
    expect(downloadMirrorUrl({ source: 'custom', customUrl: '  ' })).toBeNull();
  });
});

describe('download settings save boundary', () => {
  it('preserves official/4 defaults and the existing schema', () => {
    expect(normalizeDownloadSettings(settings)).toEqual(settings);
  });

  it('normalizes both mirror fields without modifying the draft or unrelated settings', () => {
    const draft = { ...settings, mirror_base_url: ' https://mirror.example/base/// ', java_mirror_base_url: '  ' };
    expect(normalizeDownloadSettings(draft)).toEqual({
      ...settings, mirror_base_url: 'https://mirror.example/base', java_mirror_base_url: null
    });
    expect(draft.mirror_base_url).toBe(' https://mirror.example/base/// ');
    expect(normalizeDownloadSettings({ ...settings, mirror_base_url: 'http://localhost:8080/cache/' }).mirror_base_url)
      .toBe('http://localhost:8080/cache');
  });

  it.each(['mirror_base_url', 'java_mirror_base_url'] as const)('rejects unsupported URLs in %s', (field) => {
    for (const url of ['mirror.example', '/relative', 'https://', 'https:mirror.example', 'ftp://mirror.example',
      'file:///tmp/mirror', 'https://user:pass@mirror.example', 'https://mirror.example?token=1',
      'https://mirror.example#download', 'https://mirror.example?', 'https://mirror.example#']) {
      expect(() => normalizeDownloadSettings({ ...settings, [field]: url })).toThrow('http:// 或 https://');
    }
  });

  it.each([1, 4, 8, 16])('accepts valid total connection limits: %s', (download_concurrency) => {
    expect(normalizeDownloadSettings({ ...settings, download_concurrency }).download_concurrency)
      .toBe(download_concurrency);
  });

  it.each([0, -1, 17, 4.5, NaN, Infinity, undefined, null, '4'])('rejects invalid limits instead of silently coercing: %s', (value) => {
    expect(() => normalizeDownloadSettings({ ...settings, download_concurrency: value as number }))
      .toThrow('1–16 的整数');
  });
});
