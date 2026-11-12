import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, expect, test, vi } from 'vitest';
import { isLiveAgents, isSavedFiles, readAgents, readFiles } from './api';
const installation = '12345678-1234-1234-1234-123456789abc';
const complete = { state: 'complete', installation, agents: [], observedAt: 123, observation: null, runActive: false, reason: null };
afterEach(() => { clearMocks(); vi.unstubAllGlobals(); });
test('native reads take no renderer selector, source, namespace or path', async () => {
  vi.stubGlobal('isTauri', true);
  mockIPC((command, args) => {
    expect(args).toEqual({});
    return command === 'overview_files' ? { state: 'complete', files: [], runActive: false } : complete;
  });
  expect((await readFiles()).state).toBe('complete'); expect(await readAgents()).toEqual(complete);
});
test('malformed and incomplete metadata is unavailable, not an empty success', async () => {
  vi.stubGlobal('isTauri', true); mockIPC(() => ({ ...complete, observedAt: null }));
  expect((await readAgents()).state).toBe('unavailable');
  expect(isLiveAgents({ ...complete, agents: [{ name: 'agent', namespace: 'ns' }] })).toBe(false);
  expect(isLiveAgents({ ...complete, agents: [{ name: 'x'.repeat(1048577), namespace: 'ns', uid: 'uid' }] })).toBe(false);
  expect(isLiveAgents({ ...complete, observation: { installation, documentId: installation, source: { agent: 'no', tests: 'no' } } })).toBe(false);
  expect(isSavedFiles({ state: 'complete', files: [], runActive: 'false' })).toBe(false);
});
test('supported long declarations, empty optional metadata and large registries are not discarded', () => {
  const file = { documentId: installation, name: 'reader', state: 'readable', source: null, summary: { name: '', description: 'x'.repeat(5000), model: 'model', tools: [] }, reason: null };
  expect(isSavedFiles({ state: 'complete', runActive: false, files: [file] })).toBe(true);
  expect(isSavedFiles({ state: 'complete', runActive: false, files: Array(1025).fill(file) })).toBe(true);
  expect(isLiveAgents({ ...complete, agents: Array(1025).fill({ name: 'agent', namespace: 'ns', uid: 'uid' }) })).toBe(true);
});
test('native rejection and browser preview never expose diagnostics', async () => {
  vi.stubGlobal('isTauri', true); mockIPC(() => { throw Error('/private/path with credential'); });
  expect(JSON.stringify(await readFiles())).not.toMatch(/private|credential/);
  expect(JSON.stringify(await readAgents())).not.toMatch(/private|credential/);
  vi.stubGlobal('isTauri', false); expect((await readAgents()).state).toBe('unavailable');
});
