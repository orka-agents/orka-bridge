import { useCallback, useEffect, useRef, useState } from 'react';
import { back, canGoBack, canGoForward, currentLocation, editable, forward, type History, historyShortcut, type Location, locationKey, prune, push, replace, sameTarget, startHistory } from './history';

// A move counts as going somewhere on purpose when the person pressed, clicked or chose something since the last
// recorded move; anything else (the launch page, Setup opening Designer after a failed first test, the later steps
// of one move) takes the current entry's place, so Back never lands on a page the app chose and bounces forward again.
export function useNavigationHistory({ enabled, location, restore, exists }: {
  /** False during the guided first run; history starts when the full app first appears. */
  enabled: boolean;
  location: Location;
  /** Goes to an entry the way the person would by hand. */
  restore: (location: Location) => void;
  /** Whether an entry's agent or cluster still exists. Unknown counts as existing; restoring then fails as by hand. */
  exists: (location: Location) => Promise<boolean>;
}) {
  const history = useRef<History | undefined>(undefined);
  const [, setVersion] = useState(0);
  const changed = () => setVersion(value => value + 1);
  const input = useRef(false);
  const shown = useRef(location);
  const handlers = useRef({ restore, exists });
  useEffect(() => { shown.current = location; handlers.current = { restore, exists }; });
  useEffect(() => {
    const mark = (event: Event) => {
      // Typing in a field isn't a choice of place; Enter there can be (a form that opens something).
      if (event instanceof KeyboardEvent && (editable(event.target) && event.key !== 'Enter' || ['Shift', 'Control', 'Alt', 'Meta'].includes(event.key))) return;
      input.current = true;
    };
    window.addEventListener('pointerdown', mark, true);
    window.addEventListener('keydown', mark, true);
    return () => { window.removeEventListener('pointerdown', mark, true); window.removeEventListener('keydown', mark, true); };
  }, []);
  const key = locationKey(location);
  useEffect(() => {
    const was = history.current;
    if (!enabled) { history.current = undefined; if (was) changed(); return; }
    history.current = !was ? startHistory(shown.current) : input.current ? push(was, shown.current) : replace(was, shown.current);
    input.current = false;
    if (history.current !== was) changed();
  }, [enabled, key]);
  /** Marks the next moves as the app's own until the person does something. */
  const automatic = useCallback(() => { input.current = false; }, []);
  const go = useCallback(async (direction: -1 | 1, fromKeyboard = false) => {
    const start = history.current;
    if (!start) return;
    let state = start;
    for (;;) {
      const next = direction < 0 ? back(state) : forward(state);
      if (!next) { if (state !== start) { history.current = state; changed(); } return; }
      const target = currentLocation(next);
      let found = true;
      try { found = await handlers.current.exists(target); } catch { /* Unknown: navigate as by hand. */ }
      // The person went somewhere else meanwhile; that move wins.
      if (history.current !== start) return;
      if (found) {
        history.current = next; input.current = false; changed();
        handlers.current.restore(target);
        if (fromKeyboard) keepFocus();
        return;
      }
      // Skip a deleted agent or a removed cluster everywhere in the history, not only this once.
      state = prune(state, entry => !sameTarget(entry, target));
    }
  }, []);
  useEffect(() => {
    if (!enabled) return;
    const key = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.repeat) return;
      const direction = historyShortcut(event);
      if (!direction) return;
      event.preventDefault();
      void go(direction, true);
    };
    // Mouse buttons 4 and 5 (back and forward), where the system delivers them to the page.
    const mouse = (event: MouseEvent) => {
      if (event.button !== 3 && event.button !== 4) return;
      event.preventDefault();
      void go(event.button === 3 ? -1 : 1);
    };
    window.addEventListener('keydown', key);
    window.addEventListener('mouseup', mouse);
    return () => { window.removeEventListener('keydown', key); window.removeEventListener('mouseup', mouse); };
  }, [enabled, go]);
  return {
    canBack: canGoBack(history.current), canForward: canGoForward(history.current),
    back: useCallback(() => { void go(-1); }, [go]), forward: useCallback(() => { void go(1); }, [go]), automatic,
  };
}

// A shortcut leaves focus where it was, as choosing a page in the sidebar does; when that control was on the page
// just hidden, focus moves to the new page's heading where it takes focus, or else to Back.
function keepFocus() {
  requestAnimationFrame(() => requestAnimationFrame(() => {
    const active = document.activeElement;
    if (active instanceof HTMLElement && active !== document.body && active.getClientRects().length && !active.closest('[hidden]')) return;
    const heading = document.getElementById('page-title');
    if (heading?.hasAttribute('tabindex') && heading.getClientRects().length) { heading.focus(); return; }
    document.querySelector<HTMLButtonElement>('.history-back:not(:disabled), .history-forward:not(:disabled)')?.focus();
  }));
}
