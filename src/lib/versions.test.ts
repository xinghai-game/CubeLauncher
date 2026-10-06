import { describe, expect, it } from 'vitest';
import type { CatalogVersion, Instance, VersionKind } from '../types/api';
import {
  countKinds, filterVersions, formatReleaseDate, installedIndex, phaseLabel, suggestInstanceName
} from './versions';

function version(id: string, kind: VersionKind): CatalogVersion {
  return {
    id,
    type: kind === 'release' ? 'release' : 'snapshot',
    kind,
    releaseTime: '2024-08-08T12:00:00+00:00',
    url: `https://example.invalid/${encodeURIComponent(id)}.json`,
    sha1: 'a'.repeat(40)
  };
}

function instance(id: string, gameVersion: string): Instance {
  return {
    id,
    name: id,
    game_version: gameVersion,
    loader: 'vanilla',
    loader_version: null,
    account_id: null,
    java_path: null,
    min_memory_mb: 1024,
    max_memory_mb: 4096,
    jvm_args: [],
    game_args: [],
    width: null,
    height: null,
    fullscreen: false,
    installed: true,
    last_played: null,
    created_at: '2024-08-08T12:00:00+00:00'
  };
}

const catalog: CatalogVersion[] = [
  version('1.21.1', 'release'),
  version('1.20.1', 'release'),
  version('26.4-snapshot-2', 'snapshot'),
  version('b1.8.1', 'old_beta'),
  version('a1.2.6', 'old_alpha'),
  version('24w14potato', 'april_fools')
];

describe('countKinds', () => {
  it('counts every category and the total', () => {
    expect(countKinds(catalog)).toEqual({
      release: 2, snapshot: 1, old_beta: 1, old_alpha: 1, april_fools: 1, all: 6
    });
  });

  it('reports zeroes for an empty catalog', () => {
    expect(countKinds([]).all).toBe(0);
    expect(countKinds([]).release).toBe(0);
  });
});

describe('filterVersions', () => {
  it('keeps the catalog order and only the requested category', () => {
    expect(filterVersions(catalog, 'release', '').map((entry) => entry.id))
      .toEqual(['1.21.1', '1.20.1']);
  });

  it('matches a case-insensitive substring', () => {
    expect(filterVersions(catalog, 'all', 'POTATO').map((entry) => entry.id))
      .toEqual(['24w14potato']);
  });

  it('trims the query and treats an empty one as "everything"', () => {
    expect(filterVersions(catalog, 'all', '   ')).toHaveLength(6);
  });

  it('returns nothing when the category and the query disagree', () => {
    expect(filterVersions(catalog, 'old_beta', '1.21')).toEqual([]);
  });
});

describe('formatReleaseDate', () => {
  it('prints the date without the time of day', () => {
    expect(formatReleaseDate('2024-08-08T12:34:56+00:00')).toMatch(/^2024-08-0[78]$/);
  });

  it('passes unparseable values through', () => {
    expect(formatReleaseDate('unknown')).toBe('unknown');
  });
});

describe('installedIndex', () => {
  it('groups instances by the game version they use', () => {
    const index = installedIndex([instance('a', '1.20.1'), instance('b', '1.20.1'), instance('c', '1.21.1')]);
    expect(index['1.20.1'].map((entry) => entry.id)).toEqual(['a', 'b']);
    expect(index['1.21.1']).toHaveLength(1);
    expect(index['1.19.2']).toBeUndefined();
  });
});

describe('suggestInstanceName', () => {
  it('keeps the plain version for vanilla and suffixes loaders', () => {
    expect(suggestInstanceName('1.20.1', 'vanilla')).toBe('1.20.1');
    expect(suggestInstanceName('1.20.1', 'fabric')).toBe('1.20.1-fabric');
  });
});

describe('phaseLabel', () => {
  it('translates the phases the core reports', () => {
    expect(phaseLabel('download')).toBe('下载文件');
    expect(phaseLabel('loader')).toBe('安装加载器');
  });

  it('passes unknown phases through instead of hiding them', () => {
    expect(phaseLabel('something-new')).toBe('something-new');
  });
});
