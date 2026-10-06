// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import { applyTheme, normalizeTheme, readCachedTheme, THEME_KEY } from '../src/lib/theme';

describe('theme switching', () => {
  beforeEach(() => {
    localStorage.clear();
    delete document.documentElement.dataset.theme;
  });

  it('treats anything other than "light" as the dark default', () => {
    expect(normalizeTheme('light')).toBe('light');
    expect(normalizeTheme('dark')).toBe('dark');
    expect(normalizeTheme(undefined)).toBe('dark');
    expect(normalizeTheme('system')).toBe('dark');
  });

  it('applies the palette by setting <html data-theme>', () => {
    expect(applyTheme('light')).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(document.documentElement.style.colorScheme).toBe('light');

    expect(applyTheme('dark')).toBe('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(document.documentElement.style.colorScheme).toBe('dark');
  });

  it('remembers the choice so the next launch can paint it before the bundle runs', () => {
    applyTheme('light');
    expect(localStorage.getItem(THEME_KEY)).toBe('light');
    expect(readCachedTheme()).toBe('light');

    applyTheme('dark');
    expect(readCachedTheme()).toBe('dark');
  });

  it('falls back to dark when storage is unavailable', () => {
    expect(readCachedTheme()).toBe('dark');
  });
});
