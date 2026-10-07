// Regression test for “校验 / 修复更新 点不动”: the instance action row must react
// to every click and must not stay disabled because of somebody else's task.
// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount } from 'svelte';
import type { Instance, Task } from '../src/types/api';

// `isTauri` is latched when `types/api.ts` is first evaluated, so the marker has
// to exist before the imports below run; the calls are recorded from there on.
const invocations = vi.hoisted(() => [] as Array<{ command: string; args: Record<string, unknown> }>);
vi.hoisted(() => { (globalThis as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}; });

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (command: string, args: Record<string, unknown> = {}) => {
    invocations.push({ command, args });
    if (command === 'verify_instance') return [];
    if (command === 'install_instance') return 'install-task-1';
    return null;
  })
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }));
vi.mock('@tauri-apps/plugin-opener', () => ({ openPath: vi.fn(async () => {}) }));

import { app, emptySettings, ui } from '../src/lib/state.svelte';
import InstancesPage from '../src/components/InstancesPage.svelte';

const instance = (overrides: Partial<Instance> = {}): Instance => ({
  id: 'alice', name: '原版存档', game_version: '1.20.1', loader: 'vanilla',
  loader_version: null, account_id: null, java_path: null,
  min_memory_mb: 1024, max_memory_mb: 4096, jvm_args: [], game_args: [],
  width: 1280, height: 720, fullscreen: false, installed: true,
  last_played: null, created_at: '2026-10-07T00:00:00Z',
  ...overrides
});

const runningTask = (instanceId: string): Task => ({
  id: 'install-task', instance_id: instanceId, phase: 'download', current: 1, total: 10,
  message: '下载中', done: false, error: null
});

/** Render the page for the selected instance in the given app state. */
function mountPage(task: Task | null = null, startingId: string | null = null) {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  invocations.length = 0;
  app.data = {
    settings: { ...emptySettings },
    accounts: [], instances: [instance()], java: [], running: [], data_dir: '/tmp/cube'
  };
  ui.selectedId = 'alice';
  app.task = task;
  app.installStartingId = startingId;
  app.installCancellingId = null;
  app.verifyingId = null;
  app.launchingId = null;
  ui.route = { name: 'instances', tab: 'overview' };
  ui.history = [];
  const host = document.createElement('div');
  document.body.appendChild(host);
  mount(InstancesPage, { target: host });
  flushSync();
  return host;
}

const button = (host: HTMLElement, label: string) =>
  [...host.querySelectorAll('button')].find((node) => node.textContent?.includes(label)) as HTMLButtonElement;

/** Click and let the handler's promise chain settle. */
async function click(target: HTMLButtonElement) {
  target.click();
  flushSync();
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

describe('instance action buttons', { timeout: 20000 }, () => {
  beforeEach(() => { invocations.length = 0; });

  it('checks files on click and shows the running check', async () => {
    const host = mountPage();
    const verify = button(host, '校验');

    verify.click();
    // The scan takes a moment: the button says so instead of looking dead.
    flushSync();
    expect(app.verifyingId).toBe('alice');
    expect(verify.disabled).toBe(true);
    expect(verify.textContent).toContain('校验中');

    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
    expect(invocations.map((call) => call.command)).toContain('verify_instance');
    expect(app.verifyingId).toBeNull();
    expect(verify.disabled).toBe(false);
    expect(ui.toast).toContain('校验通过');
  });

  it('repairs the instance even when another task never finished', async () => {
    // A Java download used to leave such a task behind; it froze 修复/更新 for good.
    const host = mountPage(runningTask('java'));
    const repair = button(host, '修复/更新');
    expect(repair.disabled).toBe(false);

    await click(repair);
    expect(invocations.map((call) => call.command)).toContain('install_instance');
    expect(app.task?.instance_id).toBe('alice');
  });

  it('keeps 修复/更新 disabled only while this instance installs', () => {
    const host = mountPage(runningTask('alice'));
    expect(button(host, '修复/更新').disabled).toBe(true);
    // 校验 stays available during an install: files can be checked at any time.
    expect(button(host, '校验').disabled).toBe(false);
  });

  it('disables the row while this instance is starting and frees it afterwards', () => {
    const host = mountPage(null, 'alice');
    expect(button(host, '修复/更新').disabled).toBe(true);
    app.installStartingId = null;
    flushSync();
    expect(button(host, '修复/更新').disabled).toBe(false);
  });

  it('starts an install on a repair click and leaves the page only then', async () => {
    const host = mountPage();
    expect(ui.route.name).toBe('instances');

    await click(button(host, '修复/更新'));
    expect(invocations.map((call) => call.command)).toContain('install_instance');
    expect(ui.route).toEqual({ name: 'install', instanceId: 'alice' });
  });
});
