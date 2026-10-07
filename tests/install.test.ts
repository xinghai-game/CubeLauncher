import { describe, expect, it } from 'vitest';
import {
  activeDownloads, downloadHeadline, filePercent, fileName, installationState, installInFlight,
  taskPercent
} from '../src/lib/install';
import type { Task } from '../src/types/api';

const task: Task = {
  id: 'task-1', instance_id: 'existing-instance', phase: 'download',
  current: 2, total: 10, message: '下载中', done: false, error: null,
  downloaded_bytes: 0, total_bytes: 0, active: []
};

describe('installation recovery states', () => {
  it('keeps another instance or an absent task from changing this instance state', () => {
    expect(installationState('existing-instance', null)).toBe('idle');
    expect(installationState('another-instance', task)).toBe('idle');
  });

  it('waits for a terminal event after cancellation is requested', () => {
    expect(installationState('existing-instance', task)).toBe('running');
    expect(installationState('existing-instance', task, 'existing-instance')).toBe('cancelling');
    expect(installationState('existing-instance', task, 'another-instance')).toBe('running');
  });

  it('does not mistake cancellation without an error for successful installation', () => {
    const cancelled = { ...task, phase: 'cancelled', done: true };
    expect(installationState('existing-instance', cancelled)).toBe('cancelled');
    expect(installationState('existing-instance', cancelled, 'existing-instance')).toBe('cancelled');
  });

  it('uses the actual terminal result when cancellation races with completion or failure', () => {
    expect(installationState('existing-instance', { ...task, phase: 'done', done: true }, 'existing-instance')).toBe('done');
    expect(installationState('existing-instance', { ...task, done: true, error: '网络中断' }, 'existing-instance')).toBe('failed');
  });

  it('returns to running when the same instance is resumed with a fresh task', () => {
    expect(installationState('existing-instance', { ...task, id: 'retry-task' })).toBe('running');
  });
});

describe('per-instance install buttons', () => {
  it('is idle without a task, with an empty selection, and for other tasks', () => {
    expect(installInFlight('alice', null)).toBe(false);
    expect(installInFlight('', task)).toBe(false);
    // A Java download is a task too, but it does not belong to any instance.
    expect(installInFlight('alice', { ...task, instance_id: 'java' })).toBe(false);
    // Another instance installing must not freeze this one's buttons.
    expect(installInFlight('alice', { ...task, instance_id: 'bob' })).toBe(false);
    expect(installInFlight('alice', null, 'bob')).toBe(false);
  });

  it('disables only the instance that is actually installing', () => {
    expect(installInFlight('existing-instance', task)).toBe(true);
    expect(installInFlight('existing-instance', null, 'existing-instance')).toBe(true);
  });

  it('frees the buttons as soon as the install reaches a terminal state', () => {
    for (const finished of [
      { ...task, done: true, phase: 'done' },
      { ...task, done: true, phase: 'cancelled' },
      { ...task, done: true, phase: 'error', error: '网络中断' }
    ]) {
      expect(installInFlight('existing-instance', finished)).toBe(false);
    }
  });
});

describe('download progress display', () => {
  const transferring: Task = {
    ...task,
    current: 12, total: 100,
    downloaded_bytes: 45 * 1024 * 1024, total_bytes: 128 * 1024 * 1024,
    active: [
      { name: 'assets/objects/ab/abcdef0f', downloaded: 2000, total: 8192 },
      { name: '客户端 JAR', downloaded: 45 * 1024 * 1024, total: 128 * 1024 * 1024 },
      { name: 'fmlloader', downloaded: 1000, total: 2 * 1024 * 1024 },
      { name: '无大小文件', downloaded: 3000, total: 0 }
    ]
  };

  it('prefers bytes over file counts once the denominator is known', () => {
    expect(taskPercent(null)).toBe(0);
    expect(taskPercent({ ...task, total_bytes: 0 })).toBe(20); // 2/10 files
    expect(taskPercent({ ...task, total_bytes: 0, total: 0 })).toBe(0);
    expect(taskPercent(transferring)).toBe(35); // 45/128 MiB
    expect(taskPercent({ ...transferring, downloaded_bytes: 128 * 1024 * 1024 })).toBe(100);
  });

  it('keeps the head bar inside bounds while active files lack a size', () => {
    // Unknown sizes add received bytes without growing the denominator.
    const unknown = { ...transferring, active: [{ name: 'x', downloaded: 9000, total: 0 }] };
    expect(taskPercent({ ...unknown, downloaded_bytes: 10 * 1024 * 1024, total_bytes: 0 }))
      .toBe(12); // falls back to the file count (12/100)
    // Once bytes are the denominator, surplus received bytes clamp at 100%.
    expect(taskPercent({ ...unknown, downloaded_bytes: 5000, total_bytes: 1000 })).toBe(100);
  });

  it('trims the list to the rows worth showing, largest first, and reports the hidden count', () => {
    expect(activeDownloads(null)).toEqual({ rows: [], hidden: 0 });
    const { rows, hidden } = activeDownloads(transferring, 2);
    expect(rows.map((file) => file.name)).toEqual(['客户端 JAR', 'fmlloader']);
    expect(hidden).toBe(2);
  });

  it('shows per-file progress and falls back to file names for the headline', () => {
    expect(filePercent({ name: 'x', downloaded: 2048, total: 4096 })).toBe(50);
    expect(filePercent({ name: 'x', downloaded: 2048, total: 0 })).toBe(0);
    expect(fileName('assets/objects/ab/abcdef0f')).toBe('abcdef0f');
    expect(fileName('libraries\\net\\minecraft\\client.jar')).toBe('client.jar');
    expect(fileName('客户端 JAR')).toBe('客户端 JAR');
  });

  it('names the file worth watching while downloading, keeps stage messages otherwise', () => {
    expect(downloadHeadline(transferring, false)).toBe('正在下载 客户端 JAR 等 4 个文件');
    expect(downloadHeadline({ ...task, active: [{ name: 'Java 21', downloaded: 5, total: 10 }] }, false))
      .toBe('正在下载 Java 21');
    expect(downloadHeadline(task, false)).toBe('下载中');
    expect(downloadHeadline(task, true)).toBe('安装完成');
  });
});
