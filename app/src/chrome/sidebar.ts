// The sidebar can fold to an icon rail. Narrow windows always show the rail, so the choice applies to wide ones.
export const sidebarPreference = 'bridge.sidebar.v1';
export const narrowQuery = '(max-width: 820px)';

export function readSidebarCollapsed() {
  try { return localStorage.getItem(sidebarPreference) === 'collapsed'; } catch { return false; }
}
export function writeSidebarCollapsed(collapsed: boolean) {
  try { localStorage.setItem(sidebarPreference, collapsed ? 'collapsed' : 'expanded'); } catch { /* A presentation preference must not block Bridge. */ }
}
export function narrowWindow() {
  return typeof window.matchMedia === 'function' && window.matchMedia(narrowQuery).matches;
}

// The keyboard shortcut is a proposal; this is its only definition, so it can be dropped here.
const mac = (userAgent: string) => /\b(Macintosh|Mac OS X)\b/.test(userAgent);
export function sidebarShortcutLabel(userAgent = navigator.userAgent) {
  return mac(userAgent) ? '⌘B' : 'Ctrl+B';
}
export function sidebarShortcutKeys(userAgent = navigator.userAgent) {
  return mac(userAgent) ? 'Meta+B' : 'Control+B';
}
export function isSidebarShortcut(event: KeyboardEvent, userAgent = navigator.userAgent) {
  const modifier = mac(userAgent) ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
  return modifier && !event.altKey && !event.shiftKey && event.key.toLowerCase() === 'b';
}
