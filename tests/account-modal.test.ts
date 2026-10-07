// Drives the account dialog through a whole device code sign-in with stubbed
// commands. The core's own tests cover the HTTP chain; this covers what the
// user actually sees: the code, the polling loop, and the 正版 badge.
// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount } from 'svelte';
import type { Account, Bootstrap } from '../src/types/api';

const invocations = vi.hoisted(() => [] as Array<{ command: string; args: Record<string, unknown> }>);
const state = vi.hoisted(() => ({ polls: 0, skin: null as string | null }));

// `isTauri` is latched when `types/api.ts` is first evaluated, so the marker has
// to exist before the imports below run.
vi.hoisted(() => { (globalThis as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}; });

const STEVE: Account = {
  id: '069a79f444e94726a5befca90e38aaf5',
  name: 'Steve',
  uuid: '069a79f4-44e9-4726-a5be-fca90e38aaf5',
  kind: 'microsoft',
  created_at: '2026-10-07T00:00:00Z',
  microsoft: {
    xuid: '2535412345678901',
    owns_java: true,
    expires_at: '2026-10-08T00:00:00Z',
    token_valid: true,
    skin_url: 'http://textures.minecraft.net/texture/abc',
    last_login: '2026-10-07T00:00:00Z',
    refreshable: true
  }
};

const ALICE: Account = {
  id: '3d5cec06-bd15-31fa-982f-5dac8c06f1c7',
  name: 'Alice',
  uuid: '3d5cec06-bd15-31fa-982f-5dac8c06f1c7',
  kind: 'offline',
  created_at: '2026-10-07T00:00:00Z'
};

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (command: string, args: Record<string, unknown> = {}) => {
    invocations.push({ command, args });
    switch (command) {
      case 'start_microsoft_login':
        return {
          login_id: 'device-code-1', user_code: 'ABCD-EFGH',
          verification_uri: 'https://microsoft.com/link', message: '请在浏览器中完成登录',
          expires_in: 900, interval: 1
        };
      case 'poll_microsoft_login':
        state.polls += 1;
        // Microsoft says "not yet" once, then the user finishes.
        return state.polls < 2
          ? { state: 'pending', interval: 1 }
          : { state: 'ready', account: STEVE };
      case 'add_offline_account':
        return ALICE;
      case 'account_skin':
        return state.skin;
      case 'delete_account':
        return { ok: true, message: '角色已删除' };
      case 'bootstrap':
        return {
          settings: {}, accounts: [STEVE], instances: [], java: [], running: [], data_dir: '/tmp/cube'
        } as unknown as Bootstrap;
      default:
        return null;
    }
  })
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }));
vi.mock('@tauri-apps/plugin-opener', () => ({
  openPath: vi.fn(async () => {}),
  openUrl: vi.fn(async () => {})
}));

import { app, emptySettings, idleLogin, ui } from '../src/lib/state.svelte';
import { openAccounts } from '../src/lib/actions';
import AccountModal from '../src/components/AccountModal.svelte';

function mountModal() {
  const host = document.createElement('div');
  document.body.appendChild(host);
  mount(AccountModal, { target: host });
  flushSync();
  return host;
}

const button = (host: HTMLElement, label: string) =>
  [...host.querySelectorAll('button')].find((node) => node.textContent?.includes(label)) as HTMLButtonElement;

/** Click and let the handler's promise chain settle (timers are faked). */
async function click(target: HTMLButtonElement) {
  target.click();
  flushSync();
  await vi.advanceTimersByTimeAsync(0);
  flushSync();
}

async function type(input: HTMLInputElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  flushSync();
}

describe('account dialog', { timeout: 20000 }, () => {
  beforeEach(() => {
    vi.useFakeTimers();
    state.polls = 0;
    state.skin = null;
    invocations.length = 0;
    app.data = {
      settings: { ...emptySettings }, accounts: [], instances: [], java: [], running: [],
      data_dir: '/tmp/cube'
    };
    app.skins = {};
    ui.showAccount = true;
    ui.accountTab = 'microsoft';
    ui.accountName = '';
    ui.login = { ...idleLogin };
    ui.toast = '';
  });

  it('shows the device code and adds the account once Microsoft answers', async () => {
    const host = mountModal();
    expect(host.textContent).toContain('正版登录');

    await click(button(host, '使用 Microsoft 账户登录'));
    expect(invocations.map((call) => call.command)).toContain('start_microsoft_login');
    // The two things the user needs: the code and where to type it.
    expect(host.textContent).toContain('ABCD-EFGH');
    expect(host.textContent).toContain('microsoft.com/link');

    // First poll: still waiting, the dialog keeps the code on screen.
    await vi.advanceTimersByTimeAsync(1000);
    flushSync();
    expect(state.polls).toBe(1);
    expect(host.textContent).toContain('ABCD-EFGH');

    // Second poll: signed in. The account appears, the code is gone.
    await vi.advanceTimersByTimeAsync(1000);
    flushSync();
    expect(state.polls).toBe(2);
    expect(app.data.accounts.map((account) => account.name)).toEqual(['Steve']);
    expect(host.textContent).not.toContain('ABCD-EFGH');
    expect(host.textContent).toContain('正版');
    expect(host.textContent).toContain('登录有效至');
    expect(ui.toast).toContain('已登录正版账户 Steve');
    expect(ui.login.active).toBe(false);
  });

  it('renders the official skin head when the core can fetch it', async () => {
    state.skin = 'data:image/png;base64,iVBORw0KGgo=';
    app.data.accounts = [STEVE];
    const host = mountModal();
    // Opening the dialog is what asks the core for skin textures.
    openAccounts();
    await vi.advanceTimersByTimeAsync(0);
    flushSync();
    const head = host.querySelector('.skin-head') as HTMLElement;
    expect(head).not.toBeNull();
    expect(head.getAttribute('style')).toContain('background-image');
    // A 正版 row can be renewed in place; an offline one has nothing to renew.
    const titles = [...host.querySelectorAll('button')].map((node) => node.getAttribute('title'));
    expect(titles).toContain('刷新登录');
    expect(titles).toContain('删除角色');
  });

  it('stops polling when the user cancels', async () => {
    const host = mountModal();
    await click(button(host, '使用 Microsoft 账户登录'));
    expect(ui.login.active).toBe(true);

    await click(button(host, '取消登录'));
    expect(invocations.map((call) => call.command)).toContain('cancel_microsoft_login');
    expect(ui.login.active).toBe(false);

    await vi.advanceTimersByTimeAsync(5000);
    expect(state.polls).toBe(0);
  });

  it('adds an offline role from the other tab', async () => {
    const host = mountModal();
    await click(button(host, '离线角色'));
    await type(host.querySelector('input') as HTMLInputElement, 'Alice');
    await click(button(host, '添加'));

    const added = invocations.find((call) => call.command === 'add_offline_account');
    expect(added?.args).toEqual({ name: 'Alice' });
    expect(app.data.accounts.map((account) => account.name)).toEqual(['Alice']);
    expect(ui.toast).toContain('已添加离线角色 Alice');
  });
});
