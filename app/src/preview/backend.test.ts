import { clearMocks } from '@tauri-apps/api/mocks';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readWorkspaceCatalog } from '../catalog';
import { ClusterRequestError, clusterPlaces, connectCluster, contexts, discover, previewServiceAccount, reconnectCluster, reloadContexts, selectCluster, serviceAccounts } from '../clusters/api';
import { sendCall, sendPreflight } from '../clusters/send-api';
import { readAgents, readFiles, readPlaces } from '../overview/api';
import { chatCall } from '../designer/chat-api';
import { agentCall, SAMPLE_DOCUMENT_ID } from '../designer/agent-api';
import { jobCall, runCall } from '../designer/run-api';
import { isTestDraft } from '../designer/test-definitions';
import { checkCopilot, connectKey, connectModel, resetModel } from '../setup/modelState';
import { readPreflight } from '../setup/preflight-state';
import { workspaceRequirements } from '../setup/requirements';
import { selectLocal, setupStatus } from '../setup/state';
import { startFresh } from '../setup/fresh';
import { removeModel } from '../setup/modelState';
import { upgradeCall } from '../setup/upgrade-api';
import { previewRequested } from './gate';
import { parseReply } from '../home/cards';
import { navigatorCall } from '../home/navigator-api';

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

  it('reports this computer as not integrated on a build without the local cluster, keeping an earlier choice as native does', async () => {
    await preview('scenario=noWorkspace&speed=100');
    expect(await workspaceRequirements()).toBeNull();
    expect(await setupStatus()).toEqual({ ok: true, status: { selected: null, localVm: 'notIntegrated', localVmError: null, model: 'locked', modelSupported: false } });
    await preview('scenario=noWorkspaceChosen&speed=100');
    // Native keeps the choice; the app reads it as no choice, so nothing waits for this computer.
    expect(await setupStatus()).toMatchObject({ ok: true, status: { selected: null, localVm: 'notIntegrated' } });
    expect(await clusterPlaces()).toMatchObject({ places: [{ context: 'team-dev', stale: true }] });
    expect(await reconnectCluster('team-dev/control/orka')).toMatchObject({ context_name: 'team-dev' });
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
    expect(snapshot).toMatchObject({ selectedKey: 'team-dev', persistenceState: 'complete', accessLost: false, places: [{ key: 'team-dev', context: 'team-dev' }] });
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

  it('runs Try a task and the Navigator on a cluster connected in this run', async () => {
    await preview('scenario=clusterReady&speed=1000');
    const selected = await selectCluster('team-dev');
    const place = { targetId: selected.target.id, providerId: selected.providers[0].id };
    expect(await chatCall('agent_chat_send', SAMPLE_DOCUMENT_ID, 'Read https://example.com', place)).toMatchObject({ ok: true, snapshot: { busy: true, denied: null, keptOnCluster: false, readOnly: false } });
    await new Promise(resolve => setTimeout(resolve, 20));
    // Each place answers with its own conversation: this computer's is still empty.
    expect(await chatCall('agent_chat_status', SAMPLE_DOCUMENT_ID)).toMatchObject({ ok: true, snapshot: { versions: [] } });
    const task = await chatCall('agent_chat_status', SAMPLE_DOCUMENT_ID, undefined, place);
    expect(task).toMatchObject({ ok: true, snapshot: { versions: [{ turns: [{ phase: 'Succeeded', tools: [{ name: 'web_fetch' }, { name: 'web_fetch' }] }] }] } });
    expect(await navigatorCall('agent_chat_send', '', 'What is a Task?', place)).toMatchObject({ ok: true, thinksWith: { kind: 'cluster' } });
    await new Promise(resolve => setTimeout(resolve, 20));
    // The second message continues the same conversation on the same cluster.
    expect(await navigatorCall('agent_chat_send', '', 'And a Session?', place)).toMatchObject({ ok: true });
    await new Promise(resolve => setTimeout(resolve, 20));
    const nav = await navigatorCall('agent_chat_status', '', undefined, place);
    expect(nav.ok && nav.snapshot.versions.map(version => version.turns.length)).toEqual([2]);
    const here = await navigatorCall('agent_chat_status', '');
    expect(here.ok && here.snapshot.versions).toEqual([]);
  });

  it('refuses a cluster conversation before the cluster is connected in this run', async () => {
    await preview('scenario=connected');
    const place = { targetId: '00000000-0000-4000-8000-000000000001', providerId: '00000000-0000-4000-8000-000000000002' };
    expect(await chatCall('agent_chat_send', SAMPLE_DOCUMENT_ID, 'Read it', place)).toEqual({ ok: false, message: 'Bridge couldn’t reach the selected cluster. Reconnect it in Clusters, then try again.' });
  });

  it('opens memoryLow with insufficient installed memory', async () => {
    await preview('scenario=memoryLow');
    expect(await readPreflight(6 * 1024 ** 3)).toMatchObject({ memory: 'insufficient', virtualization: 'ready' });
    expect(await setupStatus()).toMatchObject({ ok: true, status: { selected: null, localVm: 'idle' } });
  });

  it('opens checkAtStart with a build that checks this computer only when the workspace starts', async () => {
    await preview('scenario=checkAtStart');
    expect(await readPreflight(6 * 1024 ** 3)).toBe('atStart');
    expect(await setupStatus()).toMatchObject({ ok: true, status: { selected: null, localVm: 'idle' } });
  });

  it('drafts, runs and reviews a saved job under its own id, with native\'s refusals', async () => {
    const job = '5f0c2a1e-7b3d-4c8e-9a61-2d4f8b0e6c13';
    await preview('scenario=ready&speed=100');
    expect(await jobCall('job_run_start', job, 'Tell me the title of example.com')).toEqual({ ok: false, message: 'Connect the AI before running the job.' });
    await preview('scenario=connected&speed=100');
    for (const [id, prompt] of [[SAMPLE_DOCUMENT_ID, 'Read it'], [job, ' padded '], [job, ''], [job, 'a\u0007b'], [job, 'x'.repeat(8 * 1024 + 1)]]) {
      expect(await jobCall('job_run_start', id, prompt)).toEqual({ ok: false, message: 'The job is invalid.' });
    }
    expect(await jobCall('job_run_start', job, 'Tell me the title of example.com')).toMatchObject({ ok: true, snapshot: { state: 'running', busy: { name: 'Your job' }, report: null, job: { stage: 'draft', instructions: null, review: null, finishedAt: null } } });
    await vi.waitFor(async () => expect(await jobCall('agent_run_status', job)).toMatchObject({ ok: true, snapshot: { state: 'complete', report: null, error: null, job: { stage: 'review', tools: ['web_fetch'], used: ['web_fetch'], review: { verdict: 'done' } } } }), { timeout: 3000 });
    // An agent's run is untouched by the job's.
    expect(await runCall('agent_run_status')).toMatchObject({ ok: true, snapshot: { state: 'idle' } });
    await jobCall('job_run_start', job, 'Tell me the title of example.com');
    expect(await jobCall('agent_run_cancel', job)).toMatchObject({ ok: true, snapshot: { state: 'complete', cancellation: 'cancelled', job: { review: null } } });
  });

  it('opens stopped and failed workspaces, and restarts the ones that can restart', async () => {
    await preview('scenario=wsStopped&speed=100');
    expect(await setupStatus()).toMatchObject({ ok: true, status: { selected: 'localVm', localVm: 'cancelled' } });
    expect(await selectLocal()).toMatchObject({ ok: true, status: { localVm: 'starting' } });
    for (const [query, failure] of [['wsCouldntStart', { operation: 'start', cause: 'guestConnection' }], ['wsForced', { operation: 'stop', cause: 'forcedStop' }], ['wsExited', { operation: 'serve', cause: 'exited' }]] as const) {
      await preview(`scenario=${query}&speed=100`);
      expect(await setupStatus()).toMatchObject({ ok: true, status: { localVm: 'failed', workspaceFailure: failure } });
      expect((await readAgents()).state).toBe('unavailable');
      expect((await upgradeCall('status')).ok).toBe(true);
    }
    for (const query of ['wsCouldntStart', 'wsExited']) {
      await preview(`scenario=${query}&speed=100`);
      expect(await selectLocal()).toMatchObject({ ok: true, status: { localVm: 'starting' } });
    }
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
    expect(await connectModel(undefined, 'gpt-6.1-terra')).toMatchObject({ ok: true, connection: { kind: 'copilot', model: 'gpt-6.1-terra' } });
  });

  it('lists the Copilot catalog through the model validators and connects only with a chosen model', async () => {
    await preview('scenario=ready&speed=100');
    const check = await checkCopilot();
    expect(check).toMatchObject({ ok: true, catalog: { account: 'octocat', preselected: 'gpt-5.6-terra' } });
    expect(check.ok && check.catalog.models.length).toBeGreaterThan(3);
    const choice = await connectModel();
    expect(choice).toMatchObject({ ok: false, category: 'modelCopilotChooseModel', catalog: { preselected: 'gpt-5.6-terra' } });
    expect(await connectModel(undefined, 'gpt-5.6-sol')).toMatchObject({ ok: true, connection: { kind: 'copilot', model: 'gpt-5.6-sol' } });
    expect(await readWorkspaceCatalog(new AbortController().signal)).toMatchObject({ defaultModel: 'gpt-5.6-sol' });
  });

  it.each([
    ['copilotDenied', 'modelCopilotAccessDenied'],
    ['copilotUnreachable', 'modelCopilotUnreachable'],
  ])('%s refuses the check and the connect with a typed outcome', async (name, category) => {
    await preview(`scenario=${name}&speed=100`);
    expect(await checkCopilot()).toMatchObject({ ok: false, category, account: 'octocat' });
  });

  it('copilotModelFallback, copilotNoModel and copilotChoiceGone shape the picker', async () => {
    await preview('scenario=copilotModelFallback&speed=100');
    const fallback = await checkCopilot();
    expect(fallback).toMatchObject({ ok: true, catalog: { preselected: 'gpt-6.1-terra' } });
    expect(fallback.ok && fallback.catalog.models.some(m => m.id === 'gpt-5.6-terra')).toBe(false);
    await preview('scenario=copilotNoModel&speed=100');
    expect(await checkCopilot()).toEqual({ ok: true, catalog: { account: 'octocat', models: [] } });
    await preview('scenario=copilotChoiceGone&speed=100');
    expect(await checkCopilot()).toMatchObject({ ok: true, catalog: { preselected: 'gpt-6.1-terra' } });
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

  it('passes the Navigator validator in every Navigator scenario, with valid cards', async () => {
    const expected = { navThinking: undefined, navReplyFailed: undefined, navExplain: 'explain', navTask: 'task', navTaskFails: 'task', navAgent: 'agent' } as const;
    for (const [name, kind] of Object.entries(expected)) {
      await preview(`scenario=${name}`);
      const status = await navigatorCall('agent_chat_status', '');
      expect(status).toMatchObject({ ok: true, thinksWith: { kind: 'copilot', model: 'gpt-5.6-terra' } });
      const turn = status.ok ? status.snapshot.versions.at(-1)!.turns[0] : undefined;
      expect(turn?.answer ? parseReply(turn.answer).card?.kind : undefined).toBe(kind);
    }
    await preview('scenario=navThinking');
    expect(await navigatorCall('agent_chat_status', '')).toMatchObject({ ok: true, snapshot: { busy: true, canSend: false } });
    await preview('scenario=navReplyFailed');
    expect(await navigatorCall('agent_chat_status', '')).toMatchObject({ ok: true, snapshot: { busy: false, versions: [{ turns: [{ phase: 'Failed', answer: null }] }] } });
    await preview('scenario=navNoModel');
    expect(await navigatorCall('agent_chat_status', '')).toMatchObject({ ok: true, thinksWith: null, snapshot: { canSend: false } });
  });

  it('answers the Navigator with scripted replies and admits one turn at a time', async () => {
    await preview('scenario=connected&speed=1000');
    expect(await navigatorCall('agent_chat_send', '', 'Review new pull requests on my repo automatically.')).toMatchObject({ ok: true, snapshot: { busy: true } });
    // While the Navigator answers, a task cannot start, as native authoring admits one turn.
    expect(await chatCall('agent_chat_send', SAMPLE_DOCUMENT_ID, 'Read https://example.com')).toMatchObject({ ok: false });
    await new Promise(resolve => setTimeout(resolve, 20));
    const done = await navigatorCall('agent_chat_status', '');
    const answer = done.ok ? done.snapshot.versions[0].turns[0].answer! : '';
    expect(parseReply(answer)).toMatchObject({ card: undefined, sources: ['orka:guides/repository-monitors.md'] });
    expect(await navigatorCall('agent_chat_new', '')).toMatchObject({ ok: true });
    expect(await navigatorCall('agent_chat_send', '', 'Read https://example.com')).toMatchObject({ ok: true });
    await new Promise(resolve => setTimeout(resolve, 20));
    const proposal = await navigatorCall('agent_chat_status', '');
    expect(proposal.ok && proposal.snapshot.versions).toHaveLength(2);
    expect(proposal.ok && parseReply(proposal.snapshot.versions[1].turns[0].answer!).card).toEqual({ kind: 'task', agent: 'web-reader', task: 'Read https://example.com' });
  });

  it('reports the signed-out GitHub CLI as a sign-in failure', async () => {
    await preview('scenario=ghSignedOut&speed=100');
    await selectLocal();
    await vi.waitFor(async () => expect(await setupStatus()).toMatchObject({ ok: true, status: { localVm: 'ready' } }), { timeout: 5000 });
    expect(await connectModel()).toMatchObject({ ok: false, category: 'modelCaptureFailed' });
  });
  it('refuses cleanup until a fresh workspace replaces the stuck one, then sets up as a first install', async () => {
    await preview('scenario=aiStuck&speed=100');
    expect(await connectModel()).toMatchObject({ ok: false, category: 'modelReconnect', cleanup: ['disconnectRetainedModel', 'refused'] });
    expect(await removeModel()).toMatchObject({ ok: false, category: 'modelReconnect' });
    const replaced = await startFresh();
    expect(replaced).toMatchObject({ state: 'replaced', ai: 'removed', cleaning: true });
    expect(replaced.state === 'replaced' && replaced.reclaimable[0]).toMatch(/\\discarded-[0-9a-f-]{36}$/);
    // The ordinary first-setup screen: the workspace getting ready, the AI card free to check Copilot early.
    const getting = await setupStatus();
    expect(getting).toMatchObject({ ok: true, status: { selected: 'localVm', workspaceFresh: { state: 'replaced' } } });
    expect(getting.ok && ['preparing', 'installing'].includes(getting.status.localVm)).toBe(true);
    expect(await checkCopilot()).toMatchObject({ ok: true });
    await vi.waitFor(async () => expect(await setupStatus()).toMatchObject({ ok: true, status: { workspaceFresh: { cleaning: false, reclaimable: [] } } }), { timeout: 5000 });
  });

  it('reports a fresh workspace refused when the stop is unconfirmed, and one whose old folder is left', async () => {
    await preview('scenario=freshStopUnconfirmed&speed=100');
    expect(await startFresh()).toEqual({ state: 'refused', reason: 'stopUnconfirmed', restarted: false });
    expect(await setupStatus()).toMatchObject({ ok: true, status: { localVm: 'failed', workspaceFailure: { cause: 'stopUnconfirmed' }, workspaceFresh: { state: 'refused' } } });
    await preview('scenario=freshReclaimable&speed=100');
    const replaced = await startFresh();
    await vi.waitFor(async () => expect(await setupStatus()).toMatchObject({ ok: true, status: { workspaceFresh: { cleaning: false, reclaimable: replaced.state === 'replaced' ? replaced.reclaimable : [] } } }), { timeout: 5000 });
    await preview('scenario=wsVersionUnsupported&speed=100');
    expect(await setupStatus()).toMatchObject({ ok: true, status: { localVm: 'failed', workspaceFailure: { cause: 'workspaceVersionUnsupported' } } });
    expect(await startFresh()).toMatchObject({ state: 'replaced' });
  });

  it('reports the plan, an unfinished fresh workspace and folders Bridge did not expect', async () => {
    await preview('scenario=freshDownload&speed=100');
    expect(await setupStatus()).toMatchObject({ ok: true, status: { workspaceFreshPlan: { image: { kind: 'download', bytes: 1_967_572_914 } } } });
    await preview('scenario=freshBlocked&speed=100');
    expect(await setupStatus()).toMatchObject({ ok: true, status: { workspaceFreshPlan: { blocked: 'updateHistory' } } });
    expect(await connectModel()).toMatchObject({ ok: false, cleanup: ['disconnectRetainedModel', 'refused'] });
    await preview('scenario=freshInterrupted&speed=100');
    expect(await setupStatus()).toMatchObject({ ok: true, status: { localVm: 'failed', workspaceFresh: { state: 'interrupted' } } });
    expect(await startFresh()).toMatchObject({ state: 'replaced' });
    await preview('scenario=freshConflict&speed=100');
    const conflict = await setupStatus();
    expect(conflict).toMatchObject({ ok: true, status: { localVm: 'failed', workspaceFresh: { state: 'conflict' } } });
    expect(await startFresh()).toMatchObject({ state: 'conflict' });
  });
});
