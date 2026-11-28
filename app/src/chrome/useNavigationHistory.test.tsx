import { act, cleanup, fireEvent, renderHook } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import type { Location } from './history';
import { useNavigationHistory } from './useNavigationHistory';

afterEach(cleanup);
const home: Location = { page: 'Home' };
const overview: Location = { page: 'Overview' };
const clusters: Location = { page: 'Clusters', view: { kind: 'places' } };
const designer: Location = { page: 'Designer', documentId: 'a', view: 'Agent' };

function setup(start: Location = clusters, enabled = true) {
  const restore = vi.fn();
  const view = renderHook(({ location, on }) => useNavigationHistory({ enabled: on, location, restore, exists: async () => true }), { initialProps: { location: start, on: enabled } });
  const go = (location: Location, on = true) => view.rerender({ location, on });
  return { ...view, go, restore };
}
const press = () => fireEvent.pointerDown(document.body);

test('moves the person makes push; the app’s own moves replace', async () => {
  const { result, go, restore } = setup();
  // The launch page decision comes without any input: it replaces the first entry.
  go(home);
  expect(result.current.canBack).toBe(false);
  press(); go(overview);
  expect(result.current.canBack).toBe(true);
  // The later steps of the same move (Designer opening, then its document) are one entry.
  press(); go({ page: 'Designer', view: 'Agent' }); go(designer);
  act(() => result.current.back());
  await vi.waitFor(() => expect(restore).toHaveBeenCalledWith(overview));
  expect(result.current.canForward).toBe(true);
});

test('a move marked automatic replaces even right after the person did something', async () => {
  const { result, go, restore } = setup(home);
  press(); go(overview);
  // Setup opens Designer when the first test fails, while the person may be typing elsewhere.
  press(); act(() => result.current.automatic()); go(designer);
  act(() => result.current.back());
  await vi.waitFor(() => expect(restore).toHaveBeenCalledWith(home));
});

test('typing in a field is not a choice of place', () => {
  const { result, go } = setup(home);
  const field = document.body.appendChild(document.createElement('input'));
  fireEvent.keyDown(field, { key: 'a' });
  go(overview);
  expect(result.current.canBack).toBe(false);
  field.remove();
});

test('history is off while disabled and starts when enabled', () => {
  const { result, go } = setup(clusters, false);
  press(); go(overview, false);
  expect(result.current.canBack).toBe(false);
  go(home, true);
  expect(result.current.canBack).toBe(false);
  press(); go(overview, true);
  expect(result.current.canBack).toBe(true);
});

test('a missing target is skipped and Back lands on the entry before it', async () => {
  const restore = vi.fn();
  const exists = vi.fn(async (location: Location) => location.page !== 'Designer');
  const { result, rerender } = renderHook(({ location }) => useNavigationHistory({ enabled: true, location, restore, exists }), { initialProps: { location: home as Location } });
  for (const location of [overview, designer, { ...designer, view: 'File' } as Location, clusters]) { press(); rerender({ location }); }
  act(() => result.current.back());
  await vi.waitFor(() => expect(restore).toHaveBeenCalledWith(overview));
  expect(restore).toHaveBeenCalledTimes(1);
  rerender({ location: overview });
  // Both Designer entries are gone, so Forward goes straight back to Clusters.
  act(() => result.current.forward());
  await vi.waitFor(() => expect(restore).toHaveBeenLastCalledWith(clusters));
});

test('a failing existence check never throws; the entry is restored as by hand', async () => {
  const restore = vi.fn();
  const { result, rerender } = renderHook(({ location }) => useNavigationHistory({ enabled: true, location, restore, exists: () => Promise.reject(Error('unreadable')) }), { initialProps: { location: home as Location } });
  press(); rerender({ location: designer });
  act(() => result.current.back());
  await vi.waitFor(() => expect(restore).toHaveBeenCalledWith(home));
});
