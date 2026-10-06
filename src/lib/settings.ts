import type { SettingsSection } from './state.svelte';

/** Level-2 entries of the settings page, each opening its own section page. */
export const settingsSections: { id: SettingsSection; title: string; blurb: string }[] = [
  { id: 'general', title: '通用与数据目录', blurb: '数据目录、离线策略与启动行为' },
  { id: 'download', title: '下载与镜像', blurb: '并行下载数、镜像地址与默认内存' },
  { id: 'java', title: 'Java 运行时', blurb: '扫描系统 Java、下载托管运行时' },
  { id: 'appearance', title: '外观', blurb: '主题与界面偏好' },
  { id: 'about', title: '关于', blurb: '版本、许可与数据位置' }
];

export function settingsTitle(section: SettingsSection): string {
  return settingsSections.find((entry) => entry.id === section)?.title ?? '设置';
}
