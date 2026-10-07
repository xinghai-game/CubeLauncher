import type { SettingsSection } from './state.svelte';

/** Level-2 entries of the settings page, each opening its own section page. */
export const settingsSections: { id: SettingsSection; title: string; blurb: string }[] = [
  { id: 'general', title: '通用与数据目录', blurb: '数据目录、离线策略与启动行为' },
  { id: 'account', title: '账户与登录', blurb: '正版登录状态、令牌保存位置与应用 ID' },
  { id: 'download', title: '下载与镜像', blurb: '多线程下载、官方源 / BMCLAPI 与自定义镜像' },
  { id: 'java', title: 'Java 运行时', blurb: '扫描系统 Java、下载托管运行时' },
  { id: 'appearance', title: '外观', blurb: '主题与界面偏好' },
  { id: 'about', title: '关于', blurb: '版本、许可与数据位置' }
];

export function settingsTitle(section: SettingsSection): string {
  return settingsSections.find((entry) => entry.id === section)?.title ?? '设置';
}
