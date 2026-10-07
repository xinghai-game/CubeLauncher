// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import {
  formatSize, instanceGameDir, isImported, shortPath, type Instance
} from '../src/types/api';

const instance = (overrides: Partial<Instance> = {}): Instance => ({
  id: 'alice',
  name: '原版存档',
  game_version: '1.20.1',
  loader: 'vanilla',
  loader_version: null,
  account_id: null,
  java_path: null,
  min_memory_mb: 1024,
  max_memory_mb: 4096,
  jvm_args: [],
  game_args: [],
  width: 1280,
  height: 720,
  fullscreen: false,
  installed: true,
  last_played: null,
  created_at: '2026-10-07T00:00:00Z',
  ...overrides
});

describe('game directories', () => {
  it('treats only a bound directory as imported', () => {
    expect(isImported(instance())).toBe(false);
    expect(isImported(instance({ game_dir: '/home/alice/.minecraft' }))).toBe(true);
    expect(isImported(null)).toBe(false);
  });

  it('mirrors the core rule for the managed directory', () => {
    // Same layout the Rust side resolves: `instances/<id>/.minecraft`.
    expect(instanceGameDir(instance(), '/tmp/cube-e2e')).toBe('/tmp/cube-e2e/instances/alice/.minecraft');
    expect(instanceGameDir(instance(), '/tmp/cube-e2e/')).toBe('/tmp/cube-e2e/instances/alice/.minecraft');
    // An imported directory wins over the managed one.
    expect(instanceGameDir(instance({ game_dir: '/home/alice/.minecraft' }), '/tmp/cube-e2e'))
      .toBe('/home/alice/.minecraft');
  });

  it('shortens long paths for lists without hiding the last folders', () => {
    expect(shortPath('/home/alice/.minecraft')).toBe('…/alice/.minecraft');
    expect(shortPath('/home/alice/games/minecraft/.minecraft')).toBe('…/minecraft/.minecraft');
    expect(shortPath('C:\\Users\\alice\\AppData\\Roaming\\.minecraft')).toBe('…/Roaming/.minecraft');
    // A path that is already short stays readable in full.
    expect(shortPath('/mnt/mc')).toBe('/mnt/mc');
    expect(shortPath('/mnt/mc', 3)).toBe('/mnt/mc');
  });

  it('formats sizes for the mod list', () => {
    expect(formatSize(512)).toBe('512 B');
    expect(formatSize(2048)).toBe('2 KiB');
    expect(formatSize(3 * 1024 * 1024)).toBe('3.0 MiB');
  });
});
