/**
 * Theme plumbing shared by the launcher shell.
 *
 * The colour palette lives in styles.css and is keyed off `<html data-theme>`,
 * so switching themes is a DOM attribute change. `main.ts`/index.html paint the
 * cached theme before the bundle runs; everything after that goes through here.
 */
export const THEME_KEY = 'cubelauncher:theme';

export type Theme = 'dark' | 'light';

/** Anything that is not an explicit `light` falls back to the dark default. */
export function normalizeTheme(value: unknown): Theme {
  return value === 'light' ? 'light' : 'dark';
}

/** Last theme we painted; lets the launcher start without flashing the wrong palette. */
export function readCachedTheme(): Theme {
  try { return normalizeTheme(localStorage.getItem(THEME_KEY)); } catch { return 'dark'; }
}

/** Apply a theme to the document and remember it for the next launch. */
export function applyTheme(value: unknown): Theme {
  const theme = normalizeTheme(value);
  if (typeof document !== 'undefined') {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
    const meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.setAttribute('content', theme === 'light' ? '#f5f5fa' : '#101116');
  }
  try { localStorage.setItem(THEME_KEY, theme); } catch { /* storage may be unavailable */ }
  return theme;
}
