import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, expect, test, vi } from 'vitest';
import ts from 'typescript';
import { upgradeCall } from './upgrade-api';

const transaction = '10000000-0000-4000-8000-000000000001';
const status = { state: 'eligible', transaction: null, createdAt: null, sourceReleases: ['0.1.2'], targetRelease: '0.1.3', retainedAllocatedBytes: 1200, additionalBytes: 2400, credential: 'eligible', busy: false, error: null, repeated: false, canReplace: true, canRestore: false };
afterEach(() => { clearMocks(); vi.unstubAllGlobals(); });
function native(value: unknown) { vi.stubGlobal('isTauri', true); const ipc = vi.fn(() => value); mockIPC(ipc); return ipc; }

test('case-insensitive native hosts resolve the component, not the API client', () => {
  const files = Object.keys(import.meta.glob('./*.{ts,tsx}')).map(path => `/setup/${path.slice(2)}`.toLowerCase());
  const resolved = ts.resolveModuleName('./Upgrade', '/setup/Setup.tsx', { moduleResolution: ts.ModuleResolutionKind.Bundler }, {
    fileExists: path => files.includes(path.toLowerCase()), readFile: () => undefined,
  }).resolvedModule;
  expect(resolved?.resolvedFileName).toBe('/setup/Upgrade.tsx');
});
test('finite native eligibility and measured sizes survive decoding', async () => {
  native(status);
  expect(await upgradeCall('status')).toEqual({ ok: true, status });
});
test.each([
  { ...status, state: 'newerMaybe' }, { ...status, sourcePath: '/private' }, { ...status, credential: 'connected' },
  { ...status, transaction: 'not-an-id' }, { ...status, transaction: '00000000-0000-0000-0000-000000000000' },
  { ...status, retainedAllocatedBytes: -1 }, { ...status, additionalBytes: Number.MAX_SAFE_INTEGER + 1 },
  { ...status, createdAt: 1.5 }, { ...status, sourceReleases: [42] }, { ...status, busy: 1 },
  { ...status, error: 'private raw error' }, { ...status, canRestore: true },
  { ...status, state: 'conflict' }, { ...status, state: 'incomplete', transaction: null },
])('invalid or contradictory native replies never enable replacement: %j', async value => {
  native(value);
  expect((await upgradeCall('status')).ok).toBe(false);
});
test('recovery sends only the exact transaction, never a path or slot', async () => {
  const ipc = native({ ...status, state: 'consumed', transaction, createdAt: 1700000000, canReplace: false, canRestore: true });
  expect((await upgradeCall('restore', transaction)).ok).toBe(true);
  expect(ipc).toHaveBeenCalledExactlyOnceWith('workspace_upgrade_restore', { transaction });
});
test('missing or malformed transaction is refused before IPC', async () => {
  const ipc = native(status);
  expect((await upgradeCall('restore')).ok).toBe(false);
  expect((await upgradeCall('transfer', '../other')).ok).toBe(false);
  expect(ipc).not.toHaveBeenCalled();
});
