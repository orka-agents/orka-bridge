import { clearMocks } from '@tauri-apps/api/mocks';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readWorkspaceCatalog } from '../catalog';
import { ClusterRequestError, clusterPlaces, connectCluster, contexts, discover, previewServiceAccount, reconnectCluster, reloadContexts, selectCluster, serviceAccounts } from '../clusters/api';
import { sendCall, sendPreflight } from '../clusters/send-api';
import { readAgents, readFiles, readPlaces } from '../overview/api';
import { chatCall } from '../designer/chat-api';
import { agentCall, SAMPLE_DOCUMENT_ID } from '../designer/agent-api';
import { runCall } from '../designer/run-api';
import { isTestDraft } from '../designer/test-definitions';
import { connectKey, connectModel, resetModel } from '../setup/modelState';
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

  it('runs each task in a fresh conversation with its own start and finish times', async () => {
    await preview('scenario=connected&speed=1000');
    await connectModel();
    const task = 'Read https://example.com and summarise it.';
    for (const round of [1, 2]) {
      expect(await chatCall('agent_chat_new', SAMPLE_DOCUMENT_ID)).toMatchObject({ ok: true });
      const sent = await chatCall('agent_chat_send', SAMPLE_DOCUMENT_ID, task);
      expect(sent).toMatchObject({ ok: true, snapshot: { busy: true } });
      expect(sent.ok && sent.snapshot.versions.at(-1)!.turns[0]).toMatchObject({ message: task, finishedAt: null });
      await new Promise(resolve => setTimeout(resolve, 20));
      const done = await chatCall('agent_chat_status', SAMPLE_DOCUMENT_ID);
      expect(done.ok && done.snapshot.versions).toHaveLength(round);
      const turn = done.ok ? done.snapshot.versions.at(-1)!.turns : [];
      expect(turn).toHaveLength(1);
      expect(turn[0]).toMatchObject({ phase: 'Succeeded', tools: [{ name: 'web_fetch', phase: 'ToolCallStarted' }, { name: 'web_fetch', phase: 'ToolCallCompleted' }] });
      expect(turn[0].finishedAt).toBeGreaterThanOrEqual(turn[0].startedAt);
    }
  });

  it('reports an idle Send in the original scenarios', async () => {
    for (const name of ['ready', 'connected', 'runFails', 'restart']) {
      await preview(`scenario=${name}`);
      expect(await sendCall('send_status')).toEqual({ documentId: SAMPLE_DOCUMENT_ID, view: null, busy: null });
    }
  });

  it('passes the cluster validators for a cluster connected in this run', async () => {
    await preview('scenario=clusterReady');
    const snapshot = await clusterPlaces();
    expect(snapshot).toMatchObject({ selectedKey: 'team-dev', persistenceState: 'complete', places: [{ key: 'team-dev', context: 'team-dev' }] });
    // Native observation times are Unix milliseconds, as RelativeTime reads them.
    expect(Math.abs(Date.now() - snapshot!.places[0].checkedAt!)).toBeLessThan(60_000);
    expect((await readPlaces())?.places.map(place => place.key)).toEqual(['team-dev']);
    const selected = await selectCluster('team-dev');
    expect(selected).toMatchObject({ target: { context_name: 'team-dev' }, providers: [{ ready: true }] });
    await expect(selectCluster('elsewhere')).rejects.toThrow();
    const remote = { targetId: selected.target.id, providerId: selected.providers[0].id };
    expect(await readWorkspaceCatalog(new AbortController().signal, remote)).toMatchObject({ models: { state: 'ready' } });
    expect(await sendPreflight(remote.targetId, remote.providerId, SAMPLE_DOCUMENT_ID)).toMatchObject({ canSend: false });
  });

  it('opens memoryLow with insufficient installed memory', async () => {
    await preview('scenario=memoryLow');
    expect(await readPreflight(6 * 1024 ** 3)).toMatchObject({ memory: 'insufficient', virtualization: 'ready' });
    expect(await setupStatus()).toMatchObject({ ok: true, status: { selected: null, localVm: 'idle' } });
  });

  it('opens stopped and failed workspaces, and restarts the ones that can restart', async () => {
    await preview('scenario=wsStopped&speed=100');
    expect(await setupStatus()).toMatchObject({ ok: true, status: { selected: 'localVm', localVm: 'cancelled' } });
    expect(await selectLocal()).toMatchObject({ ok: true, status: { localVm: 'starting' } });
    for (const [query, failure] of [['wsCouldntStart', { operation: 'start', cause: 'guestConnection' }], ['wsForced', { operation: 'stop', cause: 'forcedStop' }]] as const) {
      await preview(`scenario=${query}&speed=100`);
      expect(await setupStatus()).toMatchObject({ ok: true, status: { localVm: 'failed', workspaceFailure: failure } });
      expect((await readAgents()).state).toBe('unavailable');
      expect((await upgradeCall('status')).ok).toBe(true);
    }
    await preview('scenario=wsCouldntStart&speed=100');
    expect(await selectLocal()).toMatchObject({ ok: true, status: { localVm: 'starting' } });
  });

  it('passes the upgrade validator for update, restore and recovery', async () => {
    await preview('scenario=wsUpdate&speed=100');
    expect(await setupStatus()).toMatchObject({ ok: true, status: { localVm: 'failed', workspaceFailure: { cause: 'workspaceUpdateRequired' } } });
    expect(await upgradeCall('status')).toMatchObject({ ok: true, status: { state: 'eligible', canReplace: true, credential: 'eligible' } });
    expect(await upgradeCall('replace')).toMatchObject({ ok: true, outcome: 'completed', status: { state: 'consumed', canRestore: true } });
    expect(await setupStatus()).toMatchObject({ ok: true, status: { localVm: 'ready', workspaceReplaced: true } });
    const consumed = await upgradeCall('status');
    expect(consumed.ok && await upgradeCall('transfer', consumed.status.transaction!)).toMatchObject({ ok: true, status: { credential: 'transferred' } });
    expect(await upgradeCall('cancel')).toMatchObject({ ok: true, outcome: 'notCancellable' });

    await preview('scenario=wsRestore&speed=100');
    expect(await setupStatus()).toMatchObject({ ok: true, status: { localVm: 'failed', workspaceReplaced: true, workspaceFailure: { cause: 'preparation' } } });
    const retained = await upgradeCall('status');
    expect(retained).toMatchObject({ ok: true, status: { state: 'sourceRetained', canReplace: true, canRestore: true } });
    expect(retained.ok && await upgradeCall('restore', retained.status.transaction!)).toMatchObject({ ok: true, status: { state: 'restored', canRestore: false } });

    await preview('scenario=wsRecovery&speed=100');
    const incomplete = await upgradeCall('status');
    expect(incomplete).toMatchObject({ ok: true, status: { state: 'incomplete', repeated: true } });
    expect(incomplete.ok && await upgradeCall('retry', incomplete.status.transaction!)).toMatchObject({ ok: true, outcome: 'completed' });
  });

  it('rejects a key and reports an earlier setup through the model validator', async () => {
    await preview('scenario=keyRejected&speed=100');
    expect(await connectKey('openai', 'sk-preview')).toMatchObject({ ok: false, category: 'modelCredentialRejected', vendor: 'openai' });
    expect(await connectKey('anthropic', 'sk-preview')).toMatchObject({ ok: false, category: 'modelKeyInvalid' });
    await preview('scenario=aiMismatch&speed=100');
    const mismatch = await connectModel();
    expect(mismatch).toMatchObject({ ok: false, category: 'modelInstallationMismatch' });
    expect(!mismatch.ok && mismatch.recovery).toBeTruthy();
    expect(await resetModel()).toMatchObject({ ok: true });
    expect(await connectModel()).toMatchObject({ ok: true, connection: { kind: 'copilot' } });
  });

  it('passes the cluster validators for denied and unreachable clusters', async () => {
    await preview('scenario=clusterDenied&speed=100');
    const places = await clusterPlaces();
    expect(places).toMatchObject({ selectedKey: null, places: [{ context: 'team-dev', stale: true }] });
    expect(Math.abs(Date.now() - 2 * 86_400_000 - places!.places[0].checkedAt!)).toBeLessThan(60_000);
    const target = await reconnectCluster(places!.places[0].key);
    expect(await connectCluster(target.id, null)).toMatchObject({ status: 'denied', failure: { check: 'permissions', result: 'permissionDenied' } });
    expect((await readPlaces())?.places).toHaveLength(1);

    await preview('scenario=clusterUnreachable&speed=100');
    const key = (await clusterPlaces())!.places[0].key;
    const failure = await reconnectCluster(key).catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(ClusterRequestError);
    expect((failure as ClusterRequestError).failure).toEqual({ check: 'clusterApi', result: 'connectionFailed' });
  });

  async function connectedCluster() {
    const places = await clusterPlaces();
    const target = await reconnectCluster(places!.places[0].key);
    const access = await connectCluster(target.id, null);
    expect(access).toMatchObject({ status: 'connected' });
    expect(await clusterPlaces()).toMatchObject({ selectedKey: places!.places[0].key });
    expect(await selectCluster(places!.places[0].key)).toMatchObject({ target: { id: target.id } });
    expect(await readWorkspaceCatalog(new AbortController().signal, { targetId: target.id, providerId: access.providers[0].id })).toBeDefined();
    return { targetId: target.id, providerId: access.providers[0].id };
  }

  it('blocks Send on destination checks', async () => {
    await preview('scenario=sendBlocked&speed=100');
    const { targetId, providerId } = await connectedCluster();
    expect(await sendPreflight(targetId, providerId)).toMatchObject({ canSend: false, action: 'blocked' });
    expect(await sendCall('send_status')).toMatchObject({ view: null, busy: null });
  });

  it('runs Send on the cluster and keeps it running until cancelled', async () => {
    await preview('scenario=sendRunning&speed=100');
    const { targetId, providerId } = await connectedCluster();
    const check = await sendPreflight(targetId, providerId);
    expect(check).toMatchObject({ canSend: true, action: 'create' });
    expect(await sendCall('send_start', { targetId, providerId, expectedSource: check.source })).toMatchObject({ view: { state: 'running', stage: 'observe' }, busy: { name: 'web-reader' } });
    await new Promise(resolve => setTimeout(resolve, 300));
    const running = await sendCall('send_status');
    expect(running).toMatchObject({ view: { state: 'running' } });
    expect(await sendCall('send_cancel', { sendId: running.view!.id })).toMatchObject({ view: { state: 'complete', cancellation: 'confirmed', report: { verdict: 'cancelled' } } });
  });

  it('finishes Send with a failing test', async () => {
    await preview('scenario=sendFailed&speed=100');
    const { targetId, providerId } = await connectedCluster();
    const check = await sendPreflight(targetId, providerId);
    await sendCall('send_start', { targetId, providerId, expectedSource: check.source });
    await vi.waitFor(async () => expect(await sendCall('send_status')).toMatchObject({ view: { state: 'complete', report: { verdict: 'failed' } } }), { timeout: 3000 });
    expect(await sendPreflight(targetId, providerId)).toMatchObject({ action: 'unchanged' });
    expect((await readPlaces())?.places[0].agents).toHaveLength(1);
  });

  it('records a finished local run for Overview and lets Stop cancel a running one', async () => {
    await preview('scenario=connected&speed=1000');
    await connectModel();
    expect(await runCall('agent_run', SAMPLE_DOCUMENT_ID, true, crypto.randomUUID())).toMatchObject({ ok: true, snapshot: { state: 'running', canCancel: true, model: 'gpt-5.6-terra' } });
    await vi.waitFor(async () => expect(await runCall('agent_run_status')).toMatchObject({ ok: true, snapshot: { state: 'complete', report: { verdict: 'passed' } } }), { timeout: 3000 });
    const file = (await readFiles()).files.find(f => f.documentId === SAMPLE_DOCUMENT_ID);
    expect(file?.localResult).toMatchObject({ state: 'complete', documentId: SAMPLE_DOCUMENT_ID, report: { verdict: 'passed', passed: 1, total: 1 } });
    // Wall-clock milliseconds, as RelativeTime reads them.
    expect(Math.abs(Date.now() - file!.localResult!.observedAt)).toBeLessThan(60_000);

    await preview('scenario=connected');
    await connectModel();
    await runCall('agent_run', SAMPLE_DOCUMENT_ID, true, crypto.randomUUID());
    expect(await runCall('agent_run_cancel')).toMatchObject({ ok: true, snapshot: { state: 'complete', cancellation: 'cancelled', report: { verdict: 'cancelled' } } });
    expect((await readFiles()).files[0].localResult).toMatchObject({ report: { verdict: 'cancelled' } });
  });

  it('opens runStale with a result from files that changed since', async () => {
    await preview('scenario=runStale');
    const run = await runCall('agent_run_status');
    const pair = await agentCall('agent_open', undefined, SAMPLE_DOCUMENT_ID);
    expect(run).toMatchObject({ ok: true, snapshot: { state: 'complete', report: { verdict: 'passed' } } });
    expect(run.ok && run.snapshot.source?.agent).not.toBe(pair.source?.agent);
    expect(run.ok && run.snapshot.source?.tests).toBe(pair.source?.tests);
  });

  it('opens overviewEmpty with no saved agents', async () => {
    await preview('scenario=overviewEmpty');
    expect(await readFiles()).toMatchObject({ state: 'complete', files: [] });
    expect((await readPlaces())?.places).toEqual([]);
  });

  it('finishes Send with every test passing, and fails the local test in sendLocalFailed', async () => {
    await preview('scenario=sendPassed&speed=100');
    const { targetId, providerId } = await connectedCluster();
    const check = await sendPreflight(targetId, providerId);
    await sendCall('send_start', { targetId, providerId, expectedSource: check.source });
    await vi.waitFor(async () => expect(await sendCall('send_status')).toMatchObject({ view: { state: 'complete', agentAction: 'create', report: { verdict: 'passed', cases: [{ verdict: 'passed' }] } } }), { timeout: 3000 });

    await preview('scenario=sendLocalFailed&speed=1000');
    const local = await connectedCluster();
    expect(await sendPreflight(local.targetId, local.providerId)).toMatchObject({ canSend: true });
    await runCall('agent_run', SAMPLE_DOCUMENT_ID, true, crypto.randomUUID());
    await vi.waitFor(async () => expect(await runCall('agent_run_status')).toMatchObject({ ok: true, snapshot: { report: { verdict: 'failed' } } }), { timeout: 3000 });
  });

  it('lists kubectl contexts with one, two, no and a ServiceAccount-gated Orka installation', async () => {
    await preview('scenario=clusterContexts&speed=100');
    const listed = await contexts();
    expect(listed.map(context => context.name)).toEqual(['team-dev', 'team-shared', 'kind-scratch', 'team-restricted']);
    expect(await reloadContexts()).toEqual(listed);
    const [dev, shared, kind, restricted] = listed;
    expect(await discover(kind.id)).toEqual([]);
    expect((await discover(shared.id)).map(target => `${target.service_namespace}/${target.service_name} · ${target.watched_namespace}`)).toEqual(['control/orka · agents', 'staging/orka · agents-staging']);
    const [gated] = await discover(restricted.id);
    expect(await connectCluster(gated.id, null)).toMatchObject({ status: 'needServiceAccount', providers: [] });
    expect(await serviceAccounts(gated.id)).toMatchObject({ names: ['bridge-user', 'default', 'orka-reader'], truncated: false });
    expect(await previewServiceAccount(gated.id, 'bridge-user')).toMatchObject({ status: 'allowed' });
    expect(await clusterPlaces()).toMatchObject({ places: [], selectedKey: null });
    const [target] = await discover(dev.id);
    const access = await connectCluster(target.id, null);
    expect(access).toMatchObject({ status: 'connected', providers: [{ name: 'team-model', ready: true }] });
    const remembered = await clusterPlaces();
    expect(remembered).toMatchObject({ places: [{ context: 'team-dev', service: 'control/orka' }] });
    expect(await selectCluster(remembered!.selectedKey!)).toMatchObject({ target: { id: target.id } });
    expect((await readPlaces())?.places).toHaveLength(1);
  });

  it('opens a first setup with the workspace starting', async () => {
    const { scenario } = await preview('scenario=wsStarting');
    expect(scenario).toBe('wsStarting');
    expect(localStorage.getItem('bridge.firstTest.v1')).toBe('pending');
    const status = await setupStatus();
    expect(status).toMatchObject({ ok: true, status: { selected: 'localVm', localVm: 'starting', workspaceProgress: { phase: 'starting' } } });
    expect(status.ok && status.status.workspaceProgress!.elapsedMs).toBeGreaterThanOrEqual(12_000);
    expect(await connectModel()).toMatchObject({ ok: false, category: 'modelGuest' });
  });

  it('opens a created agent whose tests are the draft skeleton', async () => {
    await preview('scenario=testIncomplete');
    const files = await readFiles();
    const draft = files.files.find(file => file.name === 'my-agent');
    expect(draft).toMatchObject({ draft: true, summary: null });
    const pair = await agentCall('agent_open', undefined, draft!.documentId);
    expect(isTestDraft(pair.tests)).toBe(true);
    expect(await sendCall('send_status', { documentId: draft!.documentId })).toMatchObject({ view: null });
  });

  it('reports the signed-out GitHub CLI as a sign-in failure', async () => {
    await preview('scenario=ghSignedOut&speed=100');
    await selectLocal();
    await vi.waitFor(async () => expect(await setupStatus()).toMatchObject({ ok: true, status: { localVm: 'ready' } }), { timeout: 5000 });
    expect(await connectModel()).toMatchObject({ ok: false, category: 'modelCaptureFailed' });
  });
});
