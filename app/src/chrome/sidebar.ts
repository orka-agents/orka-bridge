import { useEffect, useState } from 'react';

// The sidebar can fold to an icon rail. Narrow windows always show the rail, so the choice applies to wide ones;
// there the toggle opens the full sidebar over the page instead.
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
/** Whether the window is narrow, following resizes. Always false where matchMedia is missing (jsdom). */
export function useNarrowWindow() {
  const [narrow, setNarrow] = useState(narrowWindow);
  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return;
    const query = window.matchMedia(narrowQuery);
    let last: boolean | undefined;
    const change = () => {
      const now = query.matches;
      setNarrow(now);
      // Crossing the breakpoint restyles the whole window. WebKitGTK can keep the narrow layout's rules applied after
      // the window is widened again, until something else on the page changes; an inherited custom property on the
      // root changes every element's style, so all of them pick up the media rules that now match.
      if (last !== undefined && now !== last) document.documentElement.style.setProperty('--window-narrow', now ? '1' : '0');
      last = now;
    };
    change();
    // A resize is checked too: an engine that misses the query's change event still reports the new width.
    window.addEventListener('resize', change);
    if (typeof query.addEventListener === 'function') query.addEventListener('change', change);
    return () => { window.removeEventListener('resize', change); if (typeof query.removeEventListener === 'function') query.removeEventListener('change', change); };
  }, []);
  return narrow;
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
