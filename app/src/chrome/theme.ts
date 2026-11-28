// The theme follows the system unless the person picks Light or Dark. Only the page changes:
// the native window keeps the system theme.
export type Theme = 'system' | 'light' | 'dark';
export const themePreference = 'bridge.theme.v1';
export const themes: readonly Theme[] = ['system', 'light', 'dark'];

export function readTheme(): Theme {
  try {
    const value = localStorage.getItem(themePreference);
    return value === 'light' || value === 'dark' ? value : 'system';
  } catch { return 'system'; }
}
export function writeTheme(theme: Theme) {
  try {
    if (theme === 'system') localStorage.removeItem(themePreference);
    else localStorage.setItem(themePreference, theme);
  } catch { /* A presentation preference must not block Bridge. */ }
}
export function applyTheme(theme: Theme) {
  if (theme === 'system') document.documentElement.removeAttribute('data-theme');
  else document.documentElement.setAttribute('data-theme', theme);
}
export function nextTheme(theme: Theme): Theme {
  return themes[(themes.indexOf(theme) + 1) % themes.length];
}
