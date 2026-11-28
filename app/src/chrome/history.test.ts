import { describe, expect, it } from 'vitest';
import { back, canGoBack, canGoForward, currentLocation, editable, forward, type History, historyShortcut, historyShortcutLabels, type Location, prune, push, replace, sameLocation, sameTarget, startHistory } from './history';

const home: Location = { page: 'Home' };
const overview: Location = { page: 'Overview' };
const agent = (documentId: string, view: 'Agent' | 'Try' | 'File' = 'Agent'): Location => ({ page: 'Designer', documentId, view });
const cluster = (key?: string): Location => ({ page: 'Clusters', view: { kind: 'cluster', key } });
const places: Location = { page: 'Clusters', view: { kind: 'places' } };
const pages = (history: History) => history.entries.map(entry => JSON.stringify(entry));

describe('navigation history', () => {
  it('pushes new places and clears forward entries', () => {
    let history = push(push(startHistory(home), overview), agent('a'));
    expect(history.index).toBe(2);
    history = back(back(history)!)!;
    expect(currentLocation(history)).toEqual(home);
    expect(canGoForward(history)).toBe(true);
    history = push(history, places);
    expect(pages(history)).toEqual(pages({ entries: [home, places], index: 1 }));
    expect(canGoForward(history)).toBe(false);
  });

  it('back and forward restore entries without adding any', () => {
    const history = push(push(startHistory(home), overview), agent('a', 'File'));
    const earlier = back(history)!;
    expect(currentLocation(earlier)).toEqual(overview);
    expect(earlier.entries).toBe(history.entries);
    expect(currentLocation(forward(earlier)!)).toEqual(agent('a', 'File'));
    expect(back(startHistory(home))).toBeUndefined();
    expect(forward(history)).toBeUndefined();
    expect(canGoBack(undefined)).toBe(false);
  });

  it('collapses a push of the place already shown', () => {
    const history = push(startHistory(overview), overview);
    expect(history.entries).toHaveLength(1);
    expect(push(push(startHistory(home), agent('a')), agent('a', 'Agent')).entries).toHaveLength(2);
  });

  it('replace takes the current entry’s place and keeps forward entries', () => {
    let history = push(push(startHistory(home), overview), agent('a'));
    history = back(history)!;
    history = replace(history, places);
    expect(pages(history)).toEqual(pages({ entries: [home, places, agent('a')], index: 1 }));
    expect(replace(history, places)).toBe(history);
  });

  it('steps skip entries that repeat the place shown', () => {
    // A replaced entry can end up equal to its neighbour; Back moves past it instead of doing nothing.
    const history: History = { entries: [home, overview, overview], index: 2 };
    expect(currentLocation(back(history)!)).toEqual(home);
    expect(canGoBack({ entries: [overview, overview], index: 1 })).toBe(false);
    expect(canGoForward({ entries: [overview, overview], index: 0 })).toBe(false);
  });

  it('caps the stack, dropping the oldest entries', () => {
    let history = startHistory(agent('0'));
    for (let index = 1; index < 60; index++) history = push(history, agent(String(index)));
    expect(history.entries).toHaveLength(50);
    expect(currentLocation(history)).toEqual(agent('59'));
    expect(history.entries[0]).toEqual(agent('10'));
    expect(push(push(startHistory(home), overview, 2), places, 2).entries).toEqual([overview, places]);
  });

  it('prunes entries whose target is gone, keeps the place shown and folds repeats', () => {
    const history: History = { entries: [overview, cluster('gone'), overview, agent('a'), cluster('gone'), places, cluster('kept')], index: 5 };
    const pruned = prune(history, entry => !sameTarget(entry, cluster('gone')));
    expect(pages(pruned)).toEqual(pages({ entries: [overview, agent('a'), places, cluster('kept')], index: 2 }));
    expect(currentLocation(pruned)).toEqual(places);
    // The place shown stays even when it fails the test.
    const shown = prune({ entries: [overview, cluster('gone')], index: 1 }, entry => !sameTarget(entry, cluster('gone')));
    expect(pages(shown)).toEqual(pages({ entries: [overview, cluster('gone')], index: 1 }));
  });

  it('matches targets only by a named agent or remembered cluster', () => {
    expect(sameTarget(agent('a', 'File'), agent('a'))).toBe(true);
    expect(sameTarget(agent('a'), agent('b'))).toBe(false);
    expect(sameTarget(cluster(), cluster())).toBe(false);
    expect(sameTarget(places, places)).toBe(false);
    expect(sameTarget(home, home)).toBe(false);
    expect(sameTarget({ page: 'Designer', view: 'Agent', creating: true }, { page: 'Designer', view: 'Agent', creating: true })).toBe(false);
    expect(sameLocation({ page: 'Designer', documentId: 'a', view: 'File', creating: true }, { page: 'Designer', view: 'Agent', creating: true })).toBe(true);
  });
});

describe('history shortcuts', () => {
  const windows = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36 Edg/142.0.0.0';
  const mac = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)';
  const key = (init: KeyboardEventInit, target: EventTarget = document.body) => {
    const event = new KeyboardEvent('keydown', { bubbles: true, ...init });
    Object.defineProperty(event, 'target', { value: target });
    return event;
  };

  it('uses Alt+Left and Alt+Right on Windows and Linux', () => {
    expect(historyShortcut(key({ key: 'ArrowLeft', altKey: true }), windows)).toBe(-1);
    expect(historyShortcut(key({ key: 'ArrowRight', altKey: true }), windows)).toBe(1);
    expect(historyShortcut(key({ key: 'ArrowLeft', altKey: true, ctrlKey: true }), windows)).toBeUndefined();
    expect(historyShortcut(key({ key: 'ArrowLeft' }), windows)).toBeUndefined();
    expect(historyShortcut(key({ key: '[', metaKey: true }), windows)).toBeUndefined();
    expect(historyShortcutLabels(windows)).toEqual({ back: 'Alt+Left', forward: 'Alt+Right' });
  });

  it('leaves Alt+Arrow to text fields and lists', () => {
    const field = document.createElement('input');
    const area = document.createElement('textarea');
    const list = document.createElement('select');
    const button = document.createElement('button');
    for (const target of [field, area, list]) expect(historyShortcut(key({ key: 'ArrowLeft', altKey: true }, target), windows)).toBeUndefined();
    expect(historyShortcut(key({ key: 'ArrowLeft', altKey: true }, button), windows)).toBe(-1);
    expect(editable(Object.assign(document.createElement('input'), { type: 'checkbox' }))).toBe(false);
  });

  it('uses Cmd+[ and Cmd+] on macOS and never Alt+Arrow, which moves the caret by word', () => {
    expect(historyShortcut(key({ key: '[', metaKey: true }), mac)).toBe(-1);
    expect(historyShortcut(key({ key: ']', metaKey: true }), mac)).toBe(1);
    expect(historyShortcut(key({ key: '[', metaKey: true }, document.createElement('textarea')), mac)).toBe(-1);
    expect(historyShortcut(key({ key: 'ArrowLeft', altKey: true }), mac)).toBeUndefined();
    expect(historyShortcut(key({ key: '[', ctrlKey: true }), mac)).toBeUndefined();
    expect(historyShortcut(key({ key: '[', metaKey: true, shiftKey: true }), mac)).toBeUndefined();
    expect(historyShortcutLabels(mac)).toEqual({ back: '⌘[', forward: '⌘]' });
  });
});
