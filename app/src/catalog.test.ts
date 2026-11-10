import { beforeEach, expect, test, vi } from 'vitest';
import { readWorkspaceCatalog } from './catalog';
const ipc = vi.hoisted(() => ({ invoke: vi.fn(), native: true }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: ipc.invoke, isTauri: () => ipc.native }));
const fixture = { defaultModel: 'm:exact', models: { state: 'ready', items: [{ id: 'm:exact' }, { id: 'other', pickerEnabled: false, chatSupported: true }] }, tools: { state: 'ready', items: [{ kind: 'builtin', name: 'web_fetch', description: 'Fetch' }, { kind: 'tool', name: 'custom', description: '<script>plain text</script>', available: false }] } };
beforeEach(() => { ipc.invoke.mockReset(); ipc.native = true; });
const read = () => readWorkspaceCatalog(new AbortController().signal);
test('reads exact metadata without supplying a target or losing unknown capabilities', async () => {
  ipc.invoke.mockResolvedValue(fixture);
  expect(await read()).toEqual(fixture);
  expect(ipc.invoke).toHaveBeenCalledExactlyOnceWith('workspace_catalog');
});
test('keeps independently unavailable and successful empty catalogs distinct', async () => {
  const value = { defaultModel: 'm', models: { state: 'ready', items: [] }, tools: { state: 'unavailable' } };
  ipc.invoke.mockResolvedValue(value); expect(await read()).toEqual(value);
});
test.each([
  { ...fixture, token: 'synthetic-private' },
  { ...fixture, defaultModel: 'auto' },
  { ...fixture, models: { state: 'invented', items: [] } },
  { ...fixture, models: { state: 'unavailable', items: [] } },
  { ...fixture, models: { state: 'ready', items: [{ id: 'm', chatSupported: null }] } },
  { ...fixture, models: { state: 'ready', items: [{ id: 'm' }, { id: 'm' }] } },
  { ...fixture, models: { state: 'ready', items: [{ id: 'm'.repeat(257) }] } },
  { ...fixture, tools: { state: 'ready', items: [{ kind: 'tool', name: 'custom', description: 'desc', url: 'http://private.invalid' }] } },
  { ...fixture, tools: { state: 'ready', items: [{ kind: 'tool', name: 'custom', description: 'é'.repeat(4097) }] } },
  { ...fixture, tools: { state: 'ready', items: [{ kind: 'mcp', name: 'x', description: 'x' }] } },
])('rejects malformed or over-budget reply without exposing supplied values', async value => {
  ipc.invoke.mockResolvedValue(value); expect(await read()).toBeUndefined();
});
test('a workspace change aborts an outstanding read and a pre-aborted read never invokes', async () => {
  const controller = new AbortController();
  let finish!: (value: unknown) => void;
  ipc.invoke.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const pending = readWorkspaceCatalog(controller.signal);
  controller.abort(); finish(fixture); expect(await pending).toBeUndefined();
  ipc.invoke.mockClear(); expect(await readWorkspaceCatalog(controller.signal)).toBeUndefined();
  expect(ipc.invoke).not.toHaveBeenCalled();
});
test('preview and native failure return no untrusted error text', async () => {
  ipc.native = false; expect(await read()).toBeUndefined(); expect(ipc.invoke).not.toHaveBeenCalled();
  ipc.native = true; ipc.invoke.mockRejectedValue({ message: 'synthetic-private', token: 'secret' });
  expect(await read()).toBeUndefined();
});
