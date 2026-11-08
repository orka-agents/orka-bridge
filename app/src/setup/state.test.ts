import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, expect, test, vi } from 'vitest';
import { selectLocal, setupStatus } from './state';
const snapshot = { selected: null, localVm: 'notIntegrated', model: 'locked' };
afterEach(() => { clearMocks(); vi.unstubAllGlobals(); });

test.each([null, [], {}, 'SECRET_MARKER', { ...snapshot, selected: 'existingCluster' },
  { ...snapshot, model: 'ready' }, { ...snapshot, localVm: 'ready' },
  { ...snapshot, credential: 'SECRET_MARKER' }])('rejects unsupported status %j', async value => {
  vi.stubGlobal('isTauri', true); mockIPC(() => value);
  const result = await setupStatus();
  expect(result.ok).toBe(false);
  expect(JSON.stringify(result)).not.toContain('SECRET_MARKER');
});
test.each(['idle', 'preparing', 'installing', 'starting', 'ready', 'closing', 'closed', 'failed'])('accepts finite native runtime phase %s without unlocking models', async localVm => {
  vi.stubGlobal('isTauri', true);
  const value = { ...snapshot, selected: 'localVm', localVm };
  mockIPC(() => value);
  expect(await setupStatus()).toEqual({ ok: true, status: value });
});
test.each([null, 'Starting the local VM failed: Runtime operation failed: Busy',
  'Stopping the local VM failed: Runtime operation failed: RetainedState'])('preserves native failure reason %j on reads and selection', async localVmError => {
  vi.stubGlobal('isTauri', true);
  const value = { ...snapshot, selected: 'localVm', localVm: 'failed', localVmError };
  mockIPC(() => value);
  expect(await setupStatus()).toEqual({ ok: true, status: value });
  expect(await selectLocal()).toEqual({ ok: true, status: value });
});
test.each([42, false, {}, ['native failure']])('rejects malformed native failure reason %j', async localVmError => {
  vi.stubGlobal('isTauri', true);
  mockIPC(() => ({ ...snapshot, selected: 'localVm', localVmError }));
  expect((await setupStatus()).ok).toBe(false);
  expect((await selectLocal()).ok).toBe(false);
});
test('no-argument selection requires an acknowledgement of that selection', async () => {
  vi.stubGlobal('isTauri', true);
  const ipc = vi.fn(() => snapshot); mockIPC(ipc);
  expect((await selectLocal()).ok).toBe(false);
  expect(ipc).toHaveBeenCalledExactlyOnceWith('setup_select_local', {});
});
test('IPC errors cannot leak rejected messages or claim availability', async () => {
  vi.stubGlobal('isTauri', true); mockIPC(() => Promise.reject('SECRET_MARKER'));
  expect(await setupStatus()).toEqual({ ok: false, message: 'The setup choice could not be read or saved.' });
});
