import { StrictMode } from 'react';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import { armFirstTest, retireFirstTest, useFirstTest } from './first-test';

const sample = '9b2b93c4-375c-4815-9717-39852a96e411';
const pair = { agent: '---\nname: web-reader\n---\nRead a page.\n', tests: 'A checksum test.\n' };
const idle = { documentId: sample, busy: null, state: 'idle', canRun: true, modelConnected: true, report: null, acknowledged: [], pending: null, error: null };
function native({ changed = false, crlf = false, unreadable = false, rejected = false } = {}) {
  vi.stubGlobal('isTauri', true);
  let done = false; let held = false;
  const ipc = vi.fn((command: string, args?: unknown) => {
    if (command === 'setup_example') return pair;
    if (command === 'agent_open') return { ...pair, documentId: sample, name: 'web-reader', agent: changed ? 'Edited instructions' : crlf ? pair.agent.replaceAll('\n', '\r\n') : pair.agent };
    if (command === 'agent_run') { if (unreadable) throw Error('lost submission'); return rejected ? { ...idle, error: 'session' } : { ...idle, state: 'running', canRun: false, busy: { documentId: sample, name: 'web-reader' } }; }
    if (command === 'agent_run_status') return done
      ? { ...idle, state: 'unknown', busy: held ? { documentId: sample, name: 'web-reader' } : null, canRun: !held }
      : ipc.mock.calls.some(([c]) => c === 'agent_run') ? { ...idle, state: 'running', canRun: false, busy: { documentId: sample, name: 'web-reader' } } : idle;
    throw Error(`Unexpected ${command} ${JSON.stringify(args)}`);
  });
  mockIPC(ipc);
  return { ipc, finish: (busy = false) => { done = true; held = busy; } };
}
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); vi.restoreAllMocks(); localStorage.clear(); });

// A missing one-shot claim would submit twice on StrictMode/reconnect/reopen.
test('first completion submits the sample once, waits for release, then opens its result', async () => {
  const { ipc, finish } = native({ crlf: true }); const onOpen = vi.fn(); armFirstTest();
  const view = renderHook(({ ready }) => useFirstTest({ ready, canOpen: true, sampleDirty: false, onOpen }), { initialProps: { ready: false }, wrapper: StrictMode });
  expect(ipc).not.toHaveBeenCalled();
  view.rerender({ ready: true });
  await waitFor(() => expect(view.result.current).toBe('running'));
  expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toEqual([['agent_run', { documentId: sample }]]);
  expect(onOpen).not.toHaveBeenCalled();
  finish(true);
  await new Promise(resolve => setTimeout(resolve, 550));
  expect(onOpen).not.toHaveBeenCalled();
  finish();
  await waitFor(() => {
    expect(onOpen).toHaveBeenCalledOnce();
    expect(view.result.current).toBe('unknown');
  });
  view.rerender({ ready: false }); view.rerender({ ready: true });
  view.unmount();
  renderHook(() => useFirstTest({ ready: true, canOpen: true, sampleDirty: false, onOpen }));
  armFirstTest();
  await new Promise(resolve => setTimeout(resolve, 550));
  expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toHaveLength(1);
});

test('observing an existing ready setup or retiring intent never submits a test', async () => {
  const { ipc } = native(); const onOpen = vi.fn();
  const view = renderHook(() => useFirstTest({ ready: true, canOpen: true, sampleDirty: false, onOpen }));
  await act(async () => {});
  expect(ipc).not.toHaveBeenCalled(); view.unmount();
  armFirstTest(); retireFirstTest(); armFirstTest();
  renderHook(() => useFirstTest({ ready: true, canOpen: true, sampleDirty: false, onOpen }));
  await act(async () => {});
  expect(ipc).not.toHaveBeenCalled();
});

test('finishing without navigation ownership updates status without pulling the creator away', async () => {
  const { finish } = native(); const onOpen = vi.fn(); armFirstTest();
  const view = renderHook(({ canOpen }) => useFirstTest({ ready: true, canOpen, sampleDirty: false, onOpen }), { initialProps: { canOpen: true } });
  await waitFor(() => expect(view.result.current).toBe('running'));
  view.rerender({ canOpen: false }); finish();
  await waitFor(() => expect(view.result.current).toBe('unknown'));
  expect(onOpen).not.toHaveBeenCalled();
});

test.each([{ unreadable: true }, { rejected: true }])('uncertain or rejected submission ends without replay: %j', async options => {
  const { ipc } = native(options); const onOpen = vi.fn(); armFirstTest();
  const view = renderHook(() => useFirstTest({ ready: true, canOpen: true, sampleDirty: false, onOpen }));
  await waitFor(() => expect(onOpen).toHaveBeenCalledOnce());
  view.unmount();
  renderHook(() => useFirstTest({ ready: true, canOpen: true, sampleDirty: false, onOpen }));
  await act(async () => {});
  expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toHaveLength(1);
});

test('edited saved sample is not overwritten or automatically submitted', async () => {
  const { ipc } = native({ changed: true }); const onOpen = vi.fn(); armFirstTest();
  renderHook(() => useFirstTest({ ready: true, canOpen: true, sampleDirty: false, onOpen }));
  await waitFor(() => expect(onOpen).toHaveBeenCalledOnce());
  expect(ipc.mock.calls.some(([c]) => c === 'agent_run' || c === 'agent_save')).toBe(false);
});

test('losing readiness ends first-test observation without leaving a running step on reconnect', async () => {
  const { ipc } = native(); armFirstTest();
  const view = renderHook(({ ready }) => useFirstTest({ ready, canOpen: false, sampleDirty: false, onOpen: vi.fn() }), { initialProps: { ready: true } });
  await waitFor(() => expect(view.result.current).toBe('running'));
  view.rerender({ ready: false });
  await waitFor(() => expect(view.result.current).toBe('unknown'));
  view.rerender({ ready: true });
  expect(view.result.current).toBe('unknown');
  expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toHaveLength(1);
});

test('unsaved sample edits hold the automatic action and storage failure never admits submission', async () => {
  const { ipc } = native(); armFirstTest();
  const view = renderHook(() => useFirstTest({ ready: true, canOpen: true, sampleDirty: true, onOpen: vi.fn() }));
  await act(async () => {}); expect(ipc).not.toHaveBeenCalled(); view.unmount();
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw Error('storage unavailable'); });
  renderHook(() => useFirstTest({ ready: true, canOpen: true, sampleDirty: false, onOpen: vi.fn() }));
  await act(async () => {});
  expect(ipc.mock.calls.some(([c]) => c === 'agent_run')).toBe(false);
});
