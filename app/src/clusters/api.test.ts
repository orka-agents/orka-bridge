import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, expect, test, vi } from 'vitest';
import { contexts, discover, connectCluster } from './api';

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
test('browser preview does not invoke native commands', async () => {
  vi.stubGlobal('isTauri', false); const ipc = vi.fn(); mockIPC(ipc);
  await expect(contexts()).rejects.toThrow('desktop app'); expect(ipc).not.toHaveBeenCalled();
});
