import type { ActiveDownload, Task } from '../types/api';

export type InstallationState = 'idle' | 'running' | 'cancelling' | 'cancelled' | 'failed' | 'done';

/**
 * The core reports a missing install as a single entry instead of a file list
 * (see `LauncherCore::verify_instance`), so the UI can tell "nothing installed
 * yet" apart from "a few files are missing".
 */
export const NOT_INSTALLED = '实例尚未安装';

/**
 * True while *this* instance is starting or running an install.
 *
 * Deliberately per-instance: a task left behind by a Java download, by another
 * instance, or by a terminal event that never arrived must not freeze these
 * buttons forever — a stuck `app.task` is what made “校验/修复更新” look dead.
 * Duplicate installs of the same instance are still refused by the backend.
 */
export function installInFlight(
  instanceId: string,
  task: Task | null,
  startingInstanceId: string | null = null
): boolean {
  if (!instanceId) return false;
  if (startingInstanceId === instanceId) return true;
  return Boolean(task && !task.done && task.instance_id === instanceId);
}

/** Cancellation is terminal even when the backend reports no error. */
export function installationState(
  instanceId: string,
  task: Task | null,
  cancellingInstanceId: string | null = null
): InstallationState {
  if (!task || task.instance_id !== instanceId) return 'idle';
  if (!task.done) return cancellingInstanceId === instanceId ? 'cancelling' : 'running';
  if (task.phase === 'cancelled') return 'cancelled';
  return task.error ? 'failed' : 'done';
}

/* ---------------------------------------------------------------- download */

/**
 * Relative progress of the head bar. Bytes are the honest measure once the
 * backend knows them (one large file moves a count bar in one jump); the file
 * count remains the fallback for batches without published sizes.
 */
export function taskPercent(task: Task | null | undefined): number {
  if (!task) return 0;
  if (task.total_bytes > 0) {
    return Math.min(100, Math.round((task.downloaded_bytes / task.total_bytes) * 100));
  }
  return task.total > 0 ? Math.min(100, Math.round((task.current / task.total) * 100)) : 0;
}

/** Progress of one in-flight file; `0` while its size is unknown. */
export function filePercent(file: ActiveDownload): number {
  return file.total > 0 ? Math.min(100, Math.round((file.downloaded / file.total) * 100)) : 0;
}

/**
 * The rows worth showing, and how many more are transferring behind them.
 * Largest files first, independent of the backend's ordering, so the rows the
 * user watches are the ones that take the longest.
 */
export function activeDownloads(
  task: Task | null | undefined,
  limit = 4
): { rows: ActiveDownload[]; hidden: number } {
  const all = task?.active ?? [];
  const rows = [...all].sort(
    (a, b) => b.total - a.total || a.name.localeCompare(b.name)
  );
  return { rows: rows.slice(0, limit), hidden: Math.max(0, rows.length - limit) };
}

/** Last path segment of a label that fell back to a destination path. */
export function fileName(pathOrLabel: string): string {
  const segments = pathOrLabel.split(/[\\/]/);
  return segments[segments.length - 1] || pathOrLabel;
}

/**
 * Headline while a download phase runs: name the file worth watching — the
 * largest one in flight. Phases without transfers keep the backend's own
 * message (preparing, processors…).
 */
export function downloadHeadline(task: Task | null | undefined, done: boolean): string {
  if (!task || done) return done ? '安装完成' : '';
  const { rows } = activeDownloads(task, 1);
  const [head] = rows;
  if (!head) return task.message;
  const rest = task.active.length > 1 ? ` 等 ${task.active.length} 个文件` : '';
  return `正在下载 ${fileName(head.name)}${rest}`;
}
