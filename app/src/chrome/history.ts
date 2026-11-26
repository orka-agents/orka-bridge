// Back and Forward move through the places the person went on purpose in this run. Nothing here is stored.

export type DesignerView = 'Agent' | 'Try' | 'File';
/** The Clusters page's views. A cluster's page carries its remembered key; Add a cluster has none. */
export type ClustersView = { kind: 'places' } | { kind: 'local' } | { kind: 'cluster'; key?: string };
export type Location =
  | { page: 'Home' }
  | { page: 'Overview' }
  | { page: 'Designer'; documentId?: string; view: DesignerView }
  | { page: 'Clusters'; view: ClustersView };
export type History = { entries: Location[]; index: number };

export const historyLimit = 50;

export function locationKey(location: Location): string {
  switch (location.page) {
    case 'Designer': return `Designer/${location.documentId ?? ''}/${location.view}`;
    case 'Clusters': return `Clusters/${location.view.kind}${location.view.kind === 'cluster' ? `/${location.view.key ?? ''}` : ''}`;
    default: return location.page;
  }
}
export const sameLocation = (a: Location, b: Location) => locationKey(a) === locationKey(b);

/** Whether two entries name the same agent or the same remembered cluster, so a missing one can be dropped everywhere. */
export function sameTarget(a: Location, b: Location) {
  if (a.page === 'Designer' && b.page === 'Designer') return !!a.documentId && a.documentId === b.documentId;
  if (a.page === 'Clusters' && b.page === 'Clusters' && a.view.kind === 'cluster' && b.view.kind === 'cluster') return !!a.view.key && a.view.key === b.view.key;
  return false;
}

export const startHistory = (location: Location): History => ({ entries: [location], index: 0 });
export const currentLocation = (history: History) => history.entries[history.index];

/** Going somewhere new: drops the forward entries and keeps at most `limit`, oldest first out. */
export function push(history: History, location: Location, limit = historyLimit): History {
  if (sameLocation(currentLocation(history), location)) return history;
  const entries = [...history.entries.slice(0, history.index + 1), location].slice(-Math.max(1, limit));
  return { entries, index: entries.length - 1 };
}

/** A move the person didn't ask for, or a step of the same move, takes the current entry's place. */
export function replace(history: History, location: Location): History {
  if (sameLocation(currentLocation(history), location)) return history;
  const entries = [...history.entries];
  entries[history.index] = location;
  return { entries, index: history.index };
}

// Steps skip entries equal to where the person is, so a replaced entry that now repeats its neighbour isn't a dead click.
function step(history: History, direction: -1 | 1): History | undefined {
  const here = currentLocation(history);
  for (let index = history.index + direction; index >= 0 && index < history.entries.length; index += direction) {
    if (!sameLocation(history.entries[index], here)) return { entries: history.entries, index };
  }
}
export const back = (history: History) => step(history, -1);
export const forward = (history: History) => step(history, 1);
export const canGoBack = (history: History | undefined) => !!history && !!back(history);
export const canGoForward = (history: History | undefined) => !!history && !!forward(history);

/** Drops entries that fail `keep`, except the one shown, then folds consecutive repeats. The shown entry stays current. */
export function prune(history: History, keep: (location: Location) => boolean): History {
  const entries: Location[] = [];
  let index = 0;
  history.entries.forEach((location, at) => {
    const shown = at === history.index;
    if (!shown && !keep(location)) return;
    const previous = entries.at(-1);
    if (previous && sameLocation(previous, location)) { if (shown) index = entries.length - 1; return; }
    entries.push(location);
    if (shown) index = entries.length - 1;
  });
  return { entries, index };
}

// Shortcuts: Alt+Left and Alt+Right on Windows and Linux; Cmd+[ and Cmd+] on macOS, where Alt+Arrow moves by word.
const mac = (userAgent: string) => /\b(Macintosh|Mac OS X)\b/.test(userAgent);
export function historyShortcutLabels(userAgent = navigator.userAgent) {
  return mac(userAgent) ? { back: '⌘[', forward: '⌘]' } : { back: 'Alt+Left', forward: 'Alt+Right' };
}
export function historyShortcutKeys(userAgent = navigator.userAgent) {
  return mac(userAgent) ? { back: 'Meta+[', forward: 'Meta+]' } : { back: 'Alt+ArrowLeft', forward: 'Alt+ArrowRight' };
}
/** Fields where Alt+Arrow already means something: moving the caret, or choosing in a list. */
export function editable(target: EventTarget | null) {
  if (!(target instanceof Element)) return false;
  if (target.closest('[contenteditable]:not([contenteditable="false"])')) return true;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
  return target instanceof HTMLInputElement && !['button', 'checkbox', 'radio', 'reset', 'submit', 'image', 'file', 'color', 'range'].includes(target.type);
}
export function historyShortcut(event: KeyboardEvent, userAgent = navigator.userAgent): -1 | 1 | undefined {
  if (event.shiftKey) return;
  if (mac(userAgent)) {
    if (!event.metaKey || event.ctrlKey || event.altKey) return;
    const key = event.key === '[' || event.code === 'BracketLeft' ? -1 : event.key === ']' || event.code === 'BracketRight' ? 1 : undefined;
    return key;
  }
  if (!event.altKey || event.ctrlKey || event.metaKey || editable(event.target)) return;
  return event.key === 'ArrowLeft' ? -1 : event.key === 'ArrowRight' ? 1 : undefined;
}
