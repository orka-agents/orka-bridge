import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, expect, test, vi } from 'vitest';
import { contexts, discover, connectCluster, reloadContexts } from './api';

afterEach(() => { clearMocks(); vi.unstubAllGlobals(); });
const id = '00000000-0000-4000-8000-000000000001';
test.each([null, {}, [{ id, name: 'team', token: 'SECRET_MARKER' }], [{ id: 'endpoint', name: 'team' }], [{ id, name: 'line\nfeed' }]])('invalid context reply is not exported: %j', async value => {
  vi.stubGlobal('isTauri', true); mockIPC(() => value);
  await expect(contexts()).rejects.toThrow('Cluster information could not be read');
});
test('native diagnostics cannot be echoed through the adapter', async () => {
  vi.stubGlobal('isTauri', true); mockIPC(() => Promise.reject({ token: 'SECRET_MARKER', message: 'SECRET_MARKER' }));
  await expect(discover(id)).rejects.toThrow('Cluster information could not be read');
});
test('a different context target cannot be selected', async () => {
  vi.stubGlobal('isTauri', true); mockIPC(() => [{ id, context_id: '00000000-0000-4000-8000-000000000002', context_name: 'team', service_name: 'orka', service_namespace: 'control', watched_namespace: 'agents', image: 'image:v1' }]);
  await expect(discover(id)).rejects.toThrow();
});
test.each([{ status: 'connected', providers: [] }, { status: 'needServiceAccount', providers: [{ id, name: 'model', ready: true }] }, { status: 'invented', providers: [] }])('invalid access is not a pass: %j', async value => {
  vi.stubGlobal('isTauri', true); mockIPC(() => value); await expect(connectCluster(id, null)).rejects.toThrow();
});
test('reload invokes the offline native command', async () => {
  vi.stubGlobal('isTauri', true); const ipc = vi.fn(() => [{ id, name: 'fixed' }]); mockIPC(ipc);
  await expect(reloadContexts()).resolves.toEqual([{ id, name: 'fixed' }]);
  expect(ipc).toHaveBeenCalledWith('cluster_reload', {});
});
test('finite failure retains its check but private diagnostics are not exported', async () => {
  vi.stubGlobal('isTauri', true); mockIPC(() => Promise.reject({ check: 'providers', result: 'permissionDenied' }));
  await expect(discover(id)).rejects.toMatchObject({ failure: { check: 'providers', result: 'permissionDenied' } });
  mockIPC(() => Promise.reject({ check: 'providers', result: 'permissionDenied', diagnostic: 'SECRET_MARKER' }));
  await expect(discover(id)).rejects.toMatchObject({ failure: { check: 'clusterApi', result: 'unknown' } });
});
test.each([
  { status: 'connected', providers: [{ id, name: 'model', ready: true }], failure: { check: 'providers', result: 'unknown' } },
  { status: 'unavailable', providers: [], failure: null },
  { status: 'denied', providers: [], failure: { check: 'secret', result: 'permissionDenied' } },
])('inconsistent failure metadata is rejected: %j', async value => {
  vi.stubGlobal('isTauri', true); mockIPC(() => value); await expect(connectCluster(id, null)).rejects.toThrow();
});
test('valid failures and non-failures carry consistent metadata', async () => {
  vi.stubGlobal('isTauri', true);
  for (const value of [
    { status: 'connected', providers: [{ id, name: 'model', ready: true }], failure: null },
    { status: 'needServiceAccount', providers: [], failure: null },
    { status: 'unavailable', providers: [], failure: { check: 'providers', result: 'noProviders' } },
  ]) { mockIPC(() => value); await expect(connectCluster(id, null)).resolves.toEqual(value); }
});
test('browser preview does not invoke native commands', async () => {
  vi.stubGlobal('isTauri', false); const ipc = vi.fn(); mockIPC(ipc);
  await expect(contexts()).rejects.toThrow('desktop app'); expect(ipc).not.toHaveBeenCalled();
});
