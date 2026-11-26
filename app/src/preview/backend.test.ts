import { clearMocks } from '@tauri-apps/api/mocks';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readWorkspaceCatalog } from '../catalog';
import { clusterPlaces, selectCluster } from '../clusters/api';
import { sendPreflight } from '../clusters/send-api';
import { readAgents, readFiles, readPlaces } from '../overview/api';
import { chatCall } from '../designer/chat-api';
import { SAMPLE_DOCUMENT_ID } from '../designer/agent-api';
import { runCall } from '../designer/run-api';
import { connectModel } from '../setup/modelState';
import { readPreflight } from '../setup/preflight-state';
import { workspaceRequirements } from '../setup/requirements';
import { selectLocal, setupStatus } from '../setup/state';
import { upgradeCall } from '../setup/upgrade-api';
import { previewRequested } from './gate';

async function preview(query: string) {
  window.history.replaceState(null, '', `/?${query}&clean`);
  vi.resetModules();
  const module = await import('./backend');
  module.installPreview();
  return module;
}

afterEach(() => {
  clearMocks();
  delete (globalThis as { isTauri?: boolean }).isTauri;
  window.history.replaceState(null, '', '/');
  localStorage.clear();
});

describe('preview gate', () => {
  it('loads only in development with a scenario', () => {
    expect(previewRequested(true, '?scenario=fresh')).toBe(true);
    expect(previewRequested(false, '?scenario=fresh')).toBe(false);
    expect(previewRequested(true, '?guided=1')).toBe(false);
  });
});

// The preview is only useful while its replies satisfy the app's real native-reply validators.
describe('preview backend replies', () => {
  it('passes the setup, model and upgrade validators on a fresh computer', async () => {
    const { scenario } = await preview('scenario=fresh&speed=100');
    expect(scenario).toBe('fresh');
    const requirements = await workspaceRequirements();
    expect(requirements).not.toBeNull();
    expect(await readPreflight(requirements!.memoryBytes)).toMatchObject({ virtualization: 'ready' });
    expect(await setupStatus()).toMatchObject({ ok: true, status: { selected: null, localVm: 'idle' } });
    expect(await selectLocal()).toMatchObject({ ok: true, status: { selected: 'localVm' } });
    expect((await upgradeCall('status')).ok).toBe(true);
    // The workspace is still starting, so the AI cannot connect yet.
    expect(await connectModel()).toMatchObject({ ok: false, category: 'modelGuest' });
  });

  it('passes the run, chat and Overview validators once the AI is connected', async () => {
    await preview('scenario=connected');
    expect(await connectModel()).toMatchObject({ ok: true, connection: { kind: 'copilot' } });
    const idle = await runCall('agent_run_status');
    expect(idle).toMatchObject({ ok: true, snapshot: { state: 'idle', canRun: true } });
    expect(await runCall('agent_run', SAMPLE_DOCUMENT_ID, true, crypto.randomUUID())).toMatchObject({ ok: true, snapshot: { state: 'running' } });
    expect(await chatCall('agent_chat_status', SAMPLE_DOCUMENT_ID)).toMatchObject({ ok: true, snapshot: { canSend: true } });
    expect(await chatCall('agent_chat_send', SAMPLE_DOCUMENT_ID, 'Hello')).toMatchObject({ ok: true, snapshot: { busy: true } });
    expect((await readFiles()).state).toBe('complete');
    expect((await readAgents()).state).toBe('complete');
  });

  it('passes the cluster validators for a cluster connected in this run', async () => {
    await preview('scenario=clusterReady');
    const snapshot = await clusterPlaces();
    expect(snapshot).toMatchObject({ selectedKey: 'team-dev', persistenceState: 'complete', places: [{ key: 'team-dev', context: 'team-dev' }] });
    expect((await readPlaces())?.places.map(place => place.key)).toEqual(['team-dev']);
    const selected = await selectCluster('team-dev');
    expect(selected).toMatchObject({ target: { context_name: 'team-dev' }, providers: [{ ready: true }] });
    await expect(selectCluster('elsewhere')).rejects.toThrow();
    const remote = { targetId: selected.target.id, providerId: selected.providers[0].id };
    expect(await readWorkspaceCatalog(new AbortController().signal, remote)).toMatchObject({ models: { state: 'ready' } });
    expect(await sendPreflight(remote.targetId, remote.providerId, SAMPLE_DOCUMENT_ID)).toMatchObject({ canSend: false });
  });

  it('reports the signed-out GitHub CLI as a sign-in failure', async () => {
    await preview('scenario=ghSignedOut&speed=100');
    await selectLocal();
    await vi.waitFor(async () => expect(await setupStatus()).toMatchObject({ ok: true, status: { localVm: 'ready' } }), { timeout: 5000 });
    expect(await connectModel()).toMatchObject({ ok: false, category: 'modelCaptureFailed' });
  });
});
