// Development-only browser preview. It answers native commands with scripted
// replies so the real UI can be walked in a browser without a VM. main.tsx loads
// it only from `npm run dev` with a `?scenario=` query; release builds never
// include it.
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import agentSource from '../../src-tauri/example-agent/web-reader.agent.md?raw';
import testSource from '../../src-tauri/example-agent/web-reader.agent.test.md?raw';
import { SAMPLE_DOCUMENT_ID } from '../designer/agent-api';
import { orkaPermissions } from '../clusters/api';
import { sidebarPreference } from '../chrome/sidebar';
import { themePreference } from '../chrome/theme';
import {
  clusterCatalog, type Credential, contextTargets, createdSeed, deniedAccess, kubeContexts, failingClusterPlace, failingClusterProvider, failingClusterTarget, type FailureScenario,
  installationMismatch, keyInvalid, keyRejected, modelGuest, sendRows, unreachableCluster, upgradeStatus, workspaceFailures,
} from './fixtures';

export const scenarios = {
  fresh: 'First run on a new computer',
  ready: 'Workspace ready, AI not connected',
  connected: 'Workspace ready, AI connected',
  restart: 'Reopened while the workspace starts',
  ghMissing: 'GitHub CLI not installed',
  ghSignedOut: 'GitHub CLI installed, signed out',
  runFails: 'Example test fails',
  vmFailed: 'Virtualization turned off',
  clusterReady: 'A cluster connected in this run',
  memoryLow: 'Not enough memory at setup',
  wsStopped: 'Workspace stopped',
  wsCouldntStart: "Workspace couldn't start",
  wsForced: 'Workspace forcibly stopped',
  wsUpdate: 'Workspace update needed',
  wsRestore: 'Interrupted update, restore offered',
  wsRecovery: 'Interrupted update, setup incomplete',
  keyRejected: 'API key not accepted',
  aiMismatch: 'Saved AI connection from an earlier setup',
  clusterDenied: 'Cluster access denied',
  clusterUnreachable: "Can't reach the cluster",
  sendBlocked: 'Send blocked by destination checks',
  sendRunning: 'Send running on the cluster',
  sendFailed: 'Send finished with a failing test',
  sendPassed: 'Send finished with every test passing',
  sendLocalFailed: 'Local test failed, cluster ready to send',
  testIncomplete: 'First test is incomplete',
  runStale: 'Local result from before your last edit',
  overviewEmpty: 'No saved agents yet',
  clusterContexts: 'Add a cluster with kubectl contexts',
  wsStarting: 'First setup, the workspace starting',
} as const;
export type Scenario = keyof typeof scenarios;
type Gh = 'missing' | 'signedOut' | 'signedIn';

const GiB = 1024 ** 3;
const imageBytes = 3_334_537_216;
// Seconds per phase, shaped like a first run on a typical Windows computer.
const timing = { verify: 3, copy: 8, start: 90, reopen: 30 };
const runSeconds = 20;
const chatSeconds = 3;

const params = new URLSearchParams(location.search);
const requested = params.get('scenario') ?? 'fresh';
export const scenario: Scenario = requested in scenarios ? requested as Scenario : 'fresh';
const speed = Math.max(0.1, Number(params.get('speed')) || 1);
let gh: Gh = (['missing', 'signedOut', 'signedIn'] as const).find(value => value === params.get('gh'))
  ?? (scenario === 'ghMissing' ? 'missing' : scenario === 'ghSignedOut' ? 'signedOut' : 'signedIn');

const now = () => Date.now() * speed;
const fresh = scenario === 'fresh' || scenario === 'vmFailed' || scenario === 'ghMissing' || scenario === 'ghSignedOut' || scenario === 'memoryLow';
let selected: 'localVm' | null = fresh ? null : 'localVm';
// wsStarting opens a first setup 12 seconds into starting, after the files were checked and installed.
let startedAt: number | null = scenario === 'restart' ? now() : scenario === 'wsStarting' ? now() - (timing.verify + timing.copy + 12) * 1000 : null;
let reopened = scenario === 'restart';
let stopped: 'cancelled' | 'closed' | null = null;
let modelConnected = scenario === 'connected' || scenario === 'runFails' || scenario === 'clusterReady';

// Deterministic 64-hex digest; the preview only needs stable, distinct source ids.
function digest(text: string) {
  let h = 0x811c9dc5;
  let out = '';
  for (let round = 0; round < 8; round++) {
    for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i) ^ round, 16777619) >>> 0;
    out += h.toString(16).padStart(8, '0');
  }
  return out;
}
const uuid = (seed: string) => { const d = digest(seed); return `${d.slice(0, 8)}-${d.slice(8, 12)}-4${d.slice(13, 16)}-8${d.slice(17, 20)}-${d.slice(20, 32)}`; };
const installation = uuid('installation');

type Doc = { name: string; agent: string; tests: string };
const docs = new Map<string, Doc>([[SAMPLE_DOCUMENT_ID, { name: 'web-reader', agent: agentSource, tests: testSource }]]);
let selectedDocument = SAMPLE_DOCUMENT_ID;
const source = (doc: Doc) => ({ agent: digest(doc.agent), tests: digest(doc.tests) });
const blankTests = (name: string) => `---\nschemaVersion: 1\n---\n\n# ${name} tests\n`;
function blankAgent(name: string, model = 'gpt-5.6-terra') {
  return `---\nname: ${name}\ndescription: ""\nmodel: ${model}\nkmx:\n  schemaVersion: 1\n  runtime: orka\n  tools: []\n---\n`;
}

type Phase = { localVm: 'idle' | 'preparing' | 'installing' | 'starting' | 'ready' | 'failed' | 'cancelled' | 'closed'; progress: object | null };
function phase(): Phase {
  if (!selected) return { localVm: 'idle', progress: null };
  if (stopped) return { localVm: stopped, progress: null };
  if (scenario === 'vmFailed') return { localVm: 'failed', progress: null };
  if (startedAt === null) return { localVm: 'ready', progress: null };
  const s = (now() - startedAt) / 1000;
  const ms = (value: number) => Math.max(0, Math.round(value * 1000 / speed));
  if (!reopened) {
    if (s < timing.verify) return { localVm: s < 1 ? 'preparing' : 'installing', progress: { phase: 'verifying', bytes: null, totalBytes: null, elapsedMs: ms(s), remainingMs: null } };
    const copied = s - timing.verify;
    if (copied < timing.copy) return { localVm: 'installing', progress: { phase: 'materializing', bytes: Math.round(imageBytes * copied / timing.copy), totalBytes: imageBytes, elapsedMs: ms(copied), remainingMs: ms(timing.copy - copied) } };
  }
  const booting = s - (reopened ? 0 : timing.verify + timing.copy);
  if (booting < (reopened ? timing.reopen : timing.start)) return { localVm: 'starting', progress: { phase: 'starting', bytes: null, totalBytes: null, elapsedMs: ms(booting), remainingMs: null } };
  startedAt = null; reopened = false;
  return { localVm: 'ready', progress: null };
}
const ready = () => phase().localVm === 'ready';

function status() {
  const p = phase();
  return {
    selected, localVm: p.localVm, localVmError: null, model: p.localVm === 'ready' ? 'available' : 'locked', modelSupported: true,
    workspaceProgress: p.progress,
    ...(p.localVm === 'failed' ? { preflightFailure: 'virtualizationDisabled' } : {}),
  };
}

// A run captures the files it started from, as native does, so an edit afterwards makes its result an earlier one.
type Run = { documentId: string; startedAt: number; attemptId: string | null; count: number; source: { agent: string; tests: string } | null; cancelledAt: number | null };
let run: Run | null = null;
let runCount = 0;
const runModel = 'gpt-5.6-terra';
function runSnapshot(documentId: string) {
  const doc = docs.get(documentId);
  const canRun = ready() && modelConnected;
  const base = { documentId, busy: null, modelConnected, source: doc ? source(doc) : null, pending: null, error: null };
  if (!run || run.documentId !== documentId) return { ...base, attemptId: null, state: 'idle', stage: null, canRun, report: null, acknowledged: [] };
  const elapsed = ((run.cancelledAt ?? now()) - run.startedAt) / 1000;
  const acknowledged = [{ kind: 'Agent', name: doc?.name ?? 'agent', uid: uuid(`agent${run.count}`) }, { kind: 'Task', name: `${doc?.name ?? 'agent'}-test-${run.count}`, uid: uuid(`task${run.count}`) }];
  const captured = { ...base, source: run.source, model: runModel, attemptId: run.attemptId, acknowledged };
  if (run.cancelledAt !== null) return { ...captured, state: 'complete', stage: 'done', canRun, cancellation: 'cancelled', report: { verdict: 'cancelled', cases: [{
    id: 'published-checksum', title: 'Published checksum', phase: 'Cancelled', question: 'Open the download page and report the SHA-256 checksum it publishes.', answer: null, completedTools: [], assertions: [], verdict: 'cancelled',
  }] } };
  if (elapsed < runSeconds) return { ...captured, busy: { documentId, name: doc?.name ?? 'agent' }, state: 'running', stage: 'observe', canRun: false, canCancel: true, report: null };
  const pass = scenario !== 'runFails' && scenario !== 'sendLocalFailed';
  const checksum = 'cf699c56340dc775230fde4ef84237d27563ea6ef52164c7d078072b586c3918';
  return { ...captured, state: 'complete', stage: 'done', canRun, report: { verdict: pass ? 'passed' : 'failed', cases: [{
    id: 'published-checksum', title: 'Published checksum', phase: 'Succeeded',
    question: 'Open the download page and report the SHA-256 checksum it publishes.',
    answer: pass ? `The page publishes this SHA-256 checksum:\n\n${checksum}` : "I couldn't fetch that page, so I can't tell you the checksum.",
    completedTools: pass ? ['web_fetch'] : [],
    assertions: [
      { id: 'checksum-from-page', kind: 'contains', expected: checksum, verdict: pass ? 'passed' : 'failed' },
      { id: 'fetched-page', kind: 'toolCalled', expected: 'web_fetch', verdict: pass ? 'passed' : 'failed' },
    ],
    verdict: pass ? 'passed' : 'failed',
  }] } };
}

// One scripted task per conversation, as Try a task uses it: New, then Send. Phases advance with time; web-reader
// fetches the page it was given. Stop deletes the task, as the native path does, and leaves its phase as last seen.
type Turn = { message: string; answer: string | null; phase: string; tools: { name: string; callId: string; phase: string }[]; contextRetained: boolean; terminationUnconfirmed: boolean; stopUnconfirmed: boolean; sentAt: number; startedAt: number; finishedAt: number | null; stopped: boolean };
type Chat = { versions: Turn[][]; fresh: boolean };
const chats = new Map<string, Chat>();
const chatPhases = [[0.1, 'Pending'], [0.3, 'Scheduled'], [0.5, 'Running'], [0.85, 'Finalizing']] as const;
function chatSnapshot(documentId: string) {
  const chat = chats.get(documentId) ?? { versions: [], fresh: true };
  const turns = chat.versions.flat();
  for (const turn of turns) {
    if (turn.answer !== null || turn.stopped) continue;
    const progress = (now() - turn.sentAt) / 1000 / chatSeconds;
    const fetches = docs.get(documentId)?.name === 'web-reader' && /https?:\/\//.test(turn.message);
    if (progress >= 1) {
      const failed = scenario === 'runFails';
      turn.phase = failed ? 'Failed' : 'Succeeded'; turn.finishedAt = turn.startedAt + Math.round(chatSeconds * 1000 / speed);
      turn.answer = failed ? "I couldn't fetch that page, so I can't answer from it." : reply(documentId, turn.message);
      turn.contextRetained = true;
      if (fetches) turn.tools = [{ name: 'web_fetch', callId: 'preview-call', phase: 'ToolCallStarted' }, { name: 'web_fetch', callId: 'preview-call', phase: failed ? 'ToolCallFailed' : 'ToolCallCompleted' }];
    } else {
      turn.phase = [...chatPhases].reverse().find(([at]) => progress >= at)?.[1] ?? '';
      if (fetches && progress >= 0.5) turn.tools = [{ name: 'web_fetch', callId: 'preview-call', phase: 'ToolCallStarted' }];
    }
  }
  const working = turns.some(turn => turn.answer === null && !turn.stopped);
  const canSend = ready() && modelConnected && !working;
  return {
    documentId, state: working ? 'working' : 'ready', busy: working, canSend, error: null,
    versions: chat.versions.map(version => ({ turns: version.map(turn => ({ message: turn.message, answer: turn.answer, phase: turn.phase, tools: turn.tools, contextRetained: turn.contextRetained, terminationUnconfirmed: turn.terminationUnconfirmed, stopUnconfirmed: turn.stopUnconfirmed, startedAt: turn.startedAt, finishedAt: turn.finishedAt })) })),
  };
}
function reply(documentId: string, message: string) {
  const doc = docs.get(documentId);
  if (doc?.name === 'web-reader') {
    const url = /https?:\/\/\S+/.exec(message)?.[0]?.replace(/[.,)]+$/, '');
    return url ? `I read ${url}.\n\n- It is a placeholder page kept for documentation examples.\n- It has one heading and one paragraph.\n- It links to an explanation of why the domain is reserved.`
      : 'Give me a link and I will read the page and tell you what it says.';
  }
  return `This preview answers with scripted text. In the desktop app, ${doc?.name ?? 'your agent'} answers with the AI in your workspace.`;
}

// The last finished local run, recorded for Overview the way native local-results does.
function localResult(documentId: string) {
  if (!run || run.documentId !== documentId) return null;
  const snapshot = runSnapshot(documentId);
  if (snapshot.state !== 'complete' || !snapshot.report || !run.source) return null;
  const cases = snapshot.report.cases;
  const finished = run.cancelledAt ?? run.startedAt + runSeconds * 1000;
  return {
    installation, documentId, source: run.source, namespace: 'bridge-runs', agentName: snapshot.acknowledged[0].name, agentUid: snapshot.acknowledged[0].uid,
    // Native times are Unix milliseconds; the preview clock runs at speed, so convert back to wall time.
    observedAt: Math.round(finished / speed), state: 'complete',
    report: { verdict: snapshot.report.verdict, passed: cases.filter(c => c.verdict === 'passed').length, total: cases.length },
  };
}
function savedFiles() {
  return {
    state: 'complete', runActive: !!run && run.cancelledAt === null && (now() - run.startedAt) / 1000 < runSeconds, historyState: 'complete', runAgents: [],
    files: scenario === 'overviewEmpty' ? [] : [...docs].map(([documentId, doc]) => {
      const description = /^description: (.+)$/m.exec(doc.agent)?.[1]?.replace(/^"|"$/g, '') || null;
      const model = /^model: (.+)$/m.exec(doc.agent)?.[1] ?? 'gpt-5.6-terra';
      return { documentId, name: doc.name, state: 'readable', source: source(doc), summary: { name: doc.name, description, model, tools: doc.name === 'web-reader' ? ['web_fetch'] : [] }, reason: null, localResult: localResult(documentId) };
    }),
  };
}

const wait = (seconds: number) => new Promise(resolve => setTimeout(resolve, seconds * 1000 / speed));
function pair(documentId: string) {
  const doc = docs.get(documentId);
  if (!doc) throw 'Your agent folder could not be opened.';
  return { documentId, name: doc.name, agent: doc.agent, tests: doc.tests, source: source(doc), path: `C:\\Users\\you\\Orka\\agents\\${doc.name}\\${doc.name}.agent.md`, storage: 'bridge' };
}
const documentId = (args?: Record<string, unknown>) => typeof args?.documentId === 'string' ? args.documentId : SAMPLE_DOCUMENT_ID;
const upgradeNone = { state: 'none', transaction: null, createdAt: null, sourceReleases: [], targetRelease: null, retainedAllocatedBytes: null, additionalBytes: null, credential: 'absent', busy: false, error: null, repeated: false, canReplace: false, canRestore: false, canCancel: false };

// clusterReady: one remembered cluster, team-dev, whose access this run already confirmed, so the
// switcher can select it without reconnecting.
function clusterReply(command: string, args?: Record<string, unknown>): unknown {
  const key = 'team-dev';
  // Native observation times are Unix milliseconds.
  const place = { key, context: 'team-dev', service: 'control/orka', namespace: 'agents', stale: false, observedAt: Date.now(), checkedAt: Date.now(), agents: [] };
  const target = { id: uuid('team-dev-target'), context_id: uuid('team-dev-context'), context_name: 'team-dev', service_name: 'orka', service_namespace: 'control', watched_namespace: 'agents', image: 'ghcr.io/orka-agents/orka:preview' };
  const provider = { id: uuid('team-dev-provider'), name: 'team-model', ready: true };
  switch (command) {
    case 'cluster_places': return { places: [place], selectedKey: key, persistenceState: 'complete' };
    case 'overview_places': return { places: [place], historyState: 'complete', unobserved: [] };
    case 'cluster_select': if (args?.key !== key) throw { check: 'configuration', result: 'unknown' }; return { target, providers: [provider] };
    case 'cluster_catalog': return { defaultModel: null, models: { state: 'ready', items: [{ id: 'gpt-5.6-terra' }] }, tools: { state: 'ready', items: [{ kind: 'builtin', name: 'web_fetch', description: 'Fetch a web page and return its text.' }] } };
    case 'send_preflight': {
      const id = documentId(args);
      const doc = docs.get(id);
      return { documentId: id, targetId: target.id, providerId: provider.id, source: doc ? source(doc) : null, model: 'gpt-5.6-terra', canSend: false,
        rows: [{ label: 'Destination', status: 'allowed', message: 'team-dev is connected' }, { label: 'Send', status: 'cannotCheck', message: 'The preview does not send agents' }] };
    }
  }
}

// Scenarios that open in states the ordinary walk can't reach. Replies come from ./fixtures; a command
// a scenario doesn't own falls through to the ordinary script in handle().
const unscripted = Symbol('unscripted');
const added = !['fresh', 'ready', 'connected', 'restart', 'ghMissing', 'ghSignedOut', 'runFails', 'vmFailed', 'clusterReady', 'clusterContexts', 'wsStarting'].includes(scenario);
const upgradeScenario = scenario === 'wsUpdate' || scenario === 'wsRestore' || scenario === 'wsRecovery';
const sendScenario = ['sendBlocked', 'sendRunning', 'sendFailed', 'sendPassed', 'sendLocalFailed'].includes(scenario);
const remoteScenario = sendScenario || scenario === 'clusterDenied' || scenario === 'clusterUnreachable';
let failed: FailureScenario | null = Object.hasOwn(workspaceFailures, scenario) ? scenario as FailureScenario : null;
let replaced = scenario === 'wsRestore' || scenario === 'wsRecovery';
let credential: Credential = scenario === 'wsUpdate' ? 'eligible' : 'absent';
let upgrade = upgradeScenario ? upgradeStatus(scenario === 'wsUpdate' ? 'eligible' : scenario === 'wsRestore' ? 'sourceRetained' : 'incomplete', uuid('upgrade'), credential) : null;
let mismatch = scenario === 'aiMismatch';
let clusterKnown = true;
let clusterChecked = false;
let sent = false;
let remoteSend: { id: string; documentId: string; startedAt: number; count: number; created: boolean; cancelled: boolean } | null = null;
let sendCount = 0;
if (scenario === 'wsStopped') stopped = 'cancelled';
if (sendScenario || scenario === 'testIncomplete' || scenario === 'runStale') modelConnected = true;
// runStale: a finished run on files that have changed since, so its result is an earlier one.
if (scenario === 'runStale') {
  const earlier = { agent: digest(`${agentSource}\nAn earlier edit.`), tests: digest(testSource) };
  run = { documentId: SAMPLE_DOCUMENT_ID, startedAt: now() - (runSeconds + 300) * 1000, attemptId: null, count: ++runCount, source: earlier, cancelledAt: null };
}
const draftDocument = uuid('doc-my-agent');
if (scenario === 'testIncomplete') {
  docs.set(draftDocument, { name: 'my-agent', ...createdSeed('my-agent') });
  selectedDocument = draftDocument;
}
const clusterTarget = failingClusterTarget(uuid);
const clusterProvider = failingClusterProvider(uuid);
const sendAgent = { name: `bridge-run-${digest('send-agent').slice(0, 32)}`, uid: uuid('send-agent') };
function clusterPlace() {
  const place = failingClusterPlace(uuid, clusterChecked);
  return sent ? { ...place, agents: [{ ...sendAgent, namespace: place.namespace, agentType: 'ai', model: 'gpt-5.6-terra', tools: ['web_fetch'], record: null }] } : place;
}
function failedStatus(name: FailureScenario) {
  const { localVmError, workspaceFailure } = workspaceFailures[name];
  return { selected: 'localVm', localVm: 'failed', localVmError, model: 'locked', modelSupported: true, workspaceFailure, ...(replaced ? { workspaceReplaced: true } : {}) };
}
const modelOf = (doc: Doc) => (/^model: (.+)$/m.exec(doc.agent)?.[1] ?? 'gpt-5.6-terra').replace(/^"|"$/g, '');
const toolsOf = (doc: Doc) => doc.name === 'web-reader' ? ['web_fetch'] : [];
function sendReport(cancelled: boolean) {
  const base = { id: 'published-checksum', title: 'Published checksum', question: 'Open the download page and report the SHA-256 checksum it publishes.' };
  if (cancelled) return { verdict: 'cancelled', cases: [{ ...base, phase: 'Cancelled', answer: null, completedTools: [], assertions: [], verdict: 'cancelled' }] };
  if (scenario === 'sendPassed') {
    const checksum = 'cf699c56340dc775230fde4ef84237d27563ea6ef52164c7d078072b586c3918';
    return { verdict: 'passed', cases: [{
      ...base, phase: 'Succeeded', answer: `The page publishes this SHA-256 checksum:\n\n${checksum}`, completedTools: ['web_fetch'],
      assertions: [{ id: 'checksum-from-page', kind: 'contains', expected: checksum, verdict: 'passed' }, { id: 'fetched-page', kind: 'toolCalled', expected: 'web_fetch', verdict: 'passed' }],
      verdict: 'passed',
    }] };
  }
  return { verdict: 'failed', cases: [{
    ...base, phase: 'Succeeded', answer: "I couldn't fetch that page, so I can't tell you the checksum.", completedTools: [],
    assertions: [
      { id: 'checksum-from-page', kind: 'contains', expected: 'cf699c56340dc775230fde4ef84237d27563ea6ef52164c7d078072b586c3918', verdict: 'failed' },
      { id: 'fetched-page', kind: 'toolCalled', expected: 'web_fetch', verdict: 'failed' },
    ],
    verdict: 'failed',
  }] };
}
function sendStatus(id: string) {
  const s = remoteSend;
  const doc = s && docs.get(s.documentId);
  if (!s || !doc) return { documentId: id, view: null, busy: null };
  // sendRunning stays on the cluster until cancelled, so the running state can be studied.
  const running = !s.cancelled && (scenario === 'sendRunning' || (now() - s.startedAt) / 1000 < runSeconds);
  const view = {
    documentId: s.documentId, id: s.id, targetId: clusterTarget.id, providerId: clusterProvider.id, targetName: clusterTarget.context_name,
    source: source(doc), model: modelOf(doc), agentName: sendAgent.name, state: running ? 'running' : 'complete', stage: running ? 'observe' : 'done',
    error: null, storageError: false, cancellation: s.cancelled ? 'confirmed' : null, agentAction: s.created ? 'create' : 'unchanged',
    acknowledged: [{ kind: 'Agent', ...sendAgent }, { kind: 'Task', name: `bridge-task-${digest(`send-task${s.count}`).slice(0, 32)}`, uid: uuid(`send-task${s.count}`) }],
    pending: null, report: running ? null : sendReport(s.cancelled),
  };
  return { documentId: id, view: s.documentId === id ? view : null, busy: running ? { documentId: s.documentId, name: doc.name } : null };
}

// clusterContexts: kubeconfig lists contexts whose clusters have one, two or no Orka installations, and one
// whose installation needs a ServiceAccount. A cluster used in this run is remembered, as native does.
const contexts = kubeContexts(uuid);
const targets = contextTargets(uuid, contexts);
const contextProvider = { id: uuid('contexts-provider'), name: 'team-model', ready: true };
let usedTarget: (typeof targets)[number] | null = null;
function usedPlace() {
  if (!usedTarget) return null;
  const { target } = usedTarget;
  return { key: `${target.context_name}/${target.service_namespace}/${target.service_name}`, context: target.context_name, service: `${target.service_namespace}/${target.service_name}`, namespace: target.watched_namespace, stale: false, observedAt: Date.now(), checkedAt: Date.now(), agents: [] };
}
async function contextReply(command: string, args?: Record<string, unknown>): Promise<unknown> {
  const place = usedPlace();
  switch (command) {
    case 'cluster_contexts': case 'cluster_reload': await wait(0.3); return contexts;
    case 'cluster_discover': await wait(0.8); return targets.filter(item => item.target.context_id === args?.contextId).map(item => item.target);
    case 'cluster_use': {
      await wait(0.8);
      const item = targets.find(candidate => candidate.target.id === args?.targetId);
      if (!item) throw { check: 'configuration', result: 'unknown' };
      if (item.serviceAccount && !args?.saName) return { status: 'needServiceAccount', providers: [], failure: null };
      usedTarget = item;
      return { status: 'connected', providers: [contextProvider], failure: null };
    }
    case 'cluster_service_accounts': await wait(0.3); return { names: ['bridge-user', 'default', 'orka-reader'], truncated: false };
    case 'cluster_service_account_preview': await wait(0.3); return { status: 'allowed', checks: orkaPermissions.map(([resource, verb]) => ({ resource, verb, status: 'allowed' })) };
    case 'cluster_places': return { places: place ? [place] : [], selectedKey: place?.key ?? null, persistenceState: 'complete' };
    case 'overview_places': return { places: place ? [place] : [], historyState: 'complete', unobserved: [] };
    case 'cluster_select':
      if (!place || !usedTarget || args?.key !== place.key) throw { check: 'configuration', result: 'unknown' };
      return { target: usedTarget.target, providers: [contextProvider] };
  }
  return unscripted;
}

async function scenarioReply(command: string, args?: Record<string, unknown>): Promise<unknown> {
  if (scenario === 'clusterContexts') {
    const reply = await contextReply(command, args);
    if (reply !== unscripted) return reply;
  }
  switch (command) {
    case 'setup_preflight':
      if (scenario !== 'memoryLow') break;
      return { platform: 'windows', virtualization: 'ready', memory: 'insufficient', installedMemoryBytes: 4 * GiB, requiredMemoryBytes: 6 * GiB };
    case 'setup_status':
      if (failed) return failedStatus(failed);
      if (replaced) return { ...status(), workspaceReplaced: true };
      break;
    case 'setup_select_local':
      // Restart after a stop or a failed start reopens the installed workspace.
      if (scenario !== 'wsStopped' && scenario !== 'wsCouldntStart') break;
      await wait(0.3);
      failed = null; stopped = null; startedAt = now(); reopened = true;
      return status();
    case 'overview_agents':
      if (!failed) break;
      return { state: 'unavailable', installation: null, agents: [], observedAt: null, observation: null, runActive: false, reason: 'stopped' };
    case 'workspace_catalog':
      if (!failed) break;
      throw 'catalogUnavailable';
    case 'workspace_upgrade_status':
      if (!upgrade) break;
      return upgrade;
    case 'workspace_upgrade_replace': case 'workspace_upgrade_retry':
      if (!upgrade) break;
      await wait(2);
      upgrade = upgradeStatus('consumed', uuid('upgrade'), credential);
      failed = null; replaced = true; startedAt = null;
      return { outcome: 'completed', status: upgrade };
    case 'workspace_upgrade_restore':
      if (!upgrade) break;
      await wait(1);
      upgrade = upgradeStatus('restored', uuid('upgrade'), credential);
      failed = 'wsRestore'; replaced = true;
      return upgrade;
    case 'workspace_upgrade_transfer': case 'workspace_upgrade_remove':
      if (!upgrade) break;
      credential = command === 'workspace_upgrade_transfer' ? 'transferred' : 'removed';
      upgrade = { ...upgrade, credential };
      return upgrade;
    case 'workspace_upgrade_cancel':
      if (!upgrade) break;
      return 'notCancellable';
    case 'setup_connect_key': {
      if (scenario !== 'keyRejected') break;
      await wait(2);
      if (!ready()) throw modelGuest;
      const vendor = args?.vendor === 'anthropic' ? 'anthropic' : 'openai';
      const key = String(args?.key ?? '');
      const prefix = vendor === 'anthropic' ? 'sk-ant-' : 'sk-';
      if (!key.startsWith(prefix) || key.length <= prefix.length || (vendor === 'openai' && key.startsWith('sk-ant-'))) throw keyInvalid;
      throw keyRejected(vendor);
    }
    case 'setup_connect_model':
      if (!mismatch) break;
      await wait(2);
      if (!ready()) throw modelGuest;
      throw installationMismatch(installation, uuid('earlier-installation'), uuid('credential-slot'));
    case 'setup_reset_model':
      if (scenario !== 'aiMismatch') break;
      await wait(1);
      mismatch = false; modelConnected = false;
      return { ready: false };
    case 'cluster_places':
      if (!remoteScenario) break;
      return { places: clusterKnown ? [clusterPlace()] : [], selectedKey: clusterKnown && clusterChecked ? clusterPlace().key : null, persistenceState: 'complete' };
    case 'overview_places':
      if (!remoteScenario) break;
      return { places: clusterKnown ? [clusterPlace()] : [], historyState: 'complete', unobserved: [] };
    case 'cluster_reconnect':
      if (!remoteScenario) break;
      await wait(1);
      clusterChecked = false;
      if (scenario === 'clusterUnreachable') throw unreachableCluster;
      return clusterTarget;
    case 'cluster_use':
      if (!remoteScenario) break;
      await wait(1);
      if (scenario === 'clusterDenied') return deniedAccess;
      if (scenario === 'clusterUnreachable') return { status: 'unavailable', providers: [], failure: unreachableCluster };
      clusterChecked = true;
      return { status: 'connected', providers: [clusterProvider], failure: null };
    case 'cluster_select':
      if (!remoteScenario) break;
      if (!clusterChecked) throw { check: 'configuration', result: 'unknown' };
      return { target: clusterTarget, providers: [clusterProvider] };
    case 'cluster_catalog':
      if (!sendScenario) break;
      return clusterCatalog(scenario === 'sendBlocked' ? [] : ['web_fetch']);
    case 'cluster_forget':
      if (!remoteScenario) break;
      clusterKnown = false; clusterChecked = false;
      return null;
    case 'send_preflight': {
      if (!sendScenario) break;
      await wait(0.5);
      const id = documentId(args);
      const doc = docs.get(id);
      if (!doc) throw 'missing';
      const listed = scenario !== 'sendBlocked';
      return {
        documentId: id, source: source(doc), targetId: String(args?.targetId), providerId: String(args?.providerId), model: modelOf(doc),
        rows: sendRows(toolsOf(doc), listed), canSend: listed, action: !listed ? 'blocked' : sent ? 'unchanged' : 'create', canSeparate: false,
      };
    }
    case 'send_start': {
      if (!sendScenario) break;
      const id = documentId(args);
      if (!docs.has(id) || scenario === 'sendBlocked' || sendStatus(id).busy) throw 'busy';
      remoteSend = { id: uuid(`send${++sendCount}`), documentId: id, startedAt: now(), count: sendCount, created: !sent, cancelled: false };
      sent = true;
      return sendStatus(id);
    }
    case 'send_status':
      if (!added) break;
      return sendStatus(documentId(args));
    case 'send_cancel':
      if (!sendScenario || !remoteSend) break;
      remoteSend.cancelled = true;
      return sendStatus(documentId(args));
    case 'send_agent_focus':
      if (!sendScenario || !sent) break;
      return { key: clusterPlace().key, name: sendAgent.name, namespace: clusterTarget.watched_namespace, uid: sendAgent.uid };
    case 'send_agent_check':
      if (!sendScenario || !sent) break;
      return 'present';
    case 'overview_files': {
      if (scenario !== 'testIncomplete') break;
      // Native marks an untouched created pair as a draft and skips its declarations.
      const saved = savedFiles();
      return { ...saved, files: saved.files.map(file => file.documentId === draftDocument
        ? { ...file, draft: true, summary: null, reason: 'Add instructions and a test to get started.' } : { ...file, draft: false }) };
    }
  }
  return unscripted;
}

async function handle(command: string, args?: Record<string, unknown>): Promise<unknown> {
  if (scenario === 'clusterReady') {
    const reply = clusterReply(command, args);
    if (reply !== undefined) return reply;
  }
  const scripted = await scenarioReply(command, args);
  if (scripted !== unscripted) return scripted;
  switch (command) {
    case 'setup_requirements': return { memoryBytes: 6 * GiB, imageBytes, stateDiskBytes: 32 * GiB, downloadBytes: null };
    case 'setup_preflight': return { platform: 'windows', virtualization: scenario === 'vmFailed' ? 'disabled' : 'ready', memory: 'ready', installedMemoryBytes: 32 * GiB, requiredMemoryBytes: 6 * GiB };
    case 'setup_status': return status();
    case 'setup_select_local':
      await wait(0.3);
      if (!selected || stopped) { selected = 'localVm'; stopped = null; startedAt = now(); }
      return status();
    case 'setup_cancel_local': stopped = 'cancelled'; startedAt = null; return status();
    case 'setup_change_local': await wait(1); selected = null; stopped = null; startedAt = null; modelConnected = false; return status();
    case 'setup_example': return { agent: agentSource, tests: testSource };
    case 'setup_open_help': return null;
    case 'setup_connect_model':
      await wait(2);
      if (!ready()) throw { kind: 'modelGuest' };
      if (gh === 'missing') throw { kind: 'modelCaptureUnavailable' };
      if (gh === 'signedOut') throw { kind: 'modelCaptureFailed' };
      modelConnected = true;
      return { ready: true, connection: { kind: 'copilot', model: 'gpt-5.6-terra', account: 'octocat' } };
    case 'setup_remove_model': case 'setup_reset_model': await wait(1); modelConnected = false; return { ready: false };
    case 'workspace_upgrade_status': return upgradeNone;
    case 'workspace_catalog':
      if (!ready() || !modelConnected) throw 'catalogUnavailable';
      return {
        defaultModel: 'gpt-5.6-terra',
        models: { state: 'ready', items: [{ id: 'gpt-5.6-terra', pickerEnabled: true, chatSupported: true }, { id: 'claude-sonnet-5-5', pickerEnabled: true, chatSupported: true }] },
        tools: { state: 'ready', items: [{ kind: 'builtin', name: 'web_fetch', description: 'Fetch a web page and return its text.' }, { kind: 'builtin', name: 'web_search', description: 'Search the web.' }] },
      };
    case 'agents_list': return [...docs].map(([id, doc]) => ({ id, name: doc.name, path: pair(id).path, available: true }));
    case 'agent_selected': return selectedDocument;
    case 'agent_select': selectedDocument = documentId(args); return null;
    case 'agent_creation_root': return 'C:\\Users\\you\\Orka\\agents';
    case 'agent_pick_repo': return null;
    case 'agent_create': {
      const name = String(args?.name ?? '');
      if (!/^[a-z0-9][a-z0-9-]{0,62}$/.test(name) || [...docs.values()].some(doc => doc.name === name)) throw 'invalid';
      const id = uuid(`doc-${name}`);
      docs.set(id, { name, agent: blankAgent(name, typeof args?.model === 'string' ? args.model : undefined), tests: blankTests(name) });
      return id;
    }
    case 'agent_open': return pair(documentId(args));
    case 'agent_save': {
      const id = documentId(args);
      await wait(0.2);
      const doc = docs.get(id);
      if (!doc) throw 'missing';
      doc.agent = String(args?.content ?? doc.agent);
      return pair(id);
    }
    case 'agent_reveal': return null;
    case 'agent_run_status': return runSnapshot(documentId(args));
    case 'agent_run': {
      const id = documentId(args);
      if (!runSnapshot(id).canRun) return runSnapshot(id);
      const doc = docs.get(id);
      run = { documentId: id, startedAt: now(), attemptId: typeof args?.attemptId === 'string' ? args.attemptId : null, count: ++runCount, source: doc ? source(doc) : null, cancelledAt: null };
      return runSnapshot(id);
    }
    case 'agent_run_cancel': {
      // Stop ends a running test; a finished one is left as it was.
      const id = documentId(args);
      if (run?.documentId === id && runSnapshot(id).state === 'running') run.cancelledAt = now();
      return runSnapshot(id);
    }
    case 'agent_run_check_cancellation': return runSnapshot(documentId(args));
    case 'agent_chat_status': case 'agent_chat_reconnect': return chatSnapshot(documentId(args));
    case 'agent_chat_new': {
      const id = documentId(args);
      if (chatSnapshot(id).busy) throw 'busy';
      chats.set(id, { versions: chats.get(id)?.versions ?? [], fresh: true });
      return chatSnapshot(id);
    }
    case 'agent_chat_stop': {
      const id = documentId(args);
      for (const turn of chats.get(id)?.versions.flat() ?? []) if (turn.answer === null) { turn.stopped = true; turn.terminationUnconfirmed = true; }
      return chatSnapshot(id);
    }
    case 'agent_chat_send': {
      const id = documentId(args);
      if (!chatSnapshot(id).canSend) throw 'busy';
      const chat = chats.get(id) ?? { versions: [], fresh: true };
      if (chat.fresh || !chat.versions.length) chat.versions.push([]);
      chat.fresh = false;
      chat.versions.at(-1)!.push({ message: String(args?.message ?? ''), answer: null, phase: '', tools: [], contextRetained: false, terminationUnconfirmed: false, stopUnconfirmed: false, sentAt: now(), startedAt: Date.now(), finishedAt: null, stopped: false });
      chats.set(id, chat);
      return chatSnapshot(id);
    }
    case 'overview_files': return savedFiles();
    case 'overview_agents': return ready()
      ? { state: 'complete', installation, agents: [], observedAt: Math.floor(Date.now() / 1000), observation: null, runActive: false, reason: null }
      : { state: 'unavailable', installation: null, agents: [], observedAt: null, observation: null, runActive: false, reason: 'stopped' };
    case 'overview_places': return { places: [], historyState: 'complete', unobserved: [] };
    case 'cluster_places': return { places: [], selectedKey: null, persistenceState: 'complete' };
    case 'cluster_contexts': case 'cluster_reload': return [];
    case 'send_history': return [];
    case 'send_status': return { documentId: documentId(args), view: null, busy: null };
    case 'quit_bridge': document.body.textContent = 'Bridge quit.'; return null;
    default:
      if (command.startsWith('plugin:window|')) return command === 'plugin:window|is_maximized' ? false : null;
      throw `preview: ${command} is not scripted`;
  }
}

function toolbar() {
  const bar = document.createElement('form');
  bar.className = 'preview-toolbar';
  bar.setAttribute('aria-label', 'Preview controls');
  const select = (name: string, value: string, options: Record<string, string>) =>
    `<label>${name}<select name="${name}">${Object.entries(options).map(([key, label]) => `<option value="${key}"${key === value ? ' selected' : ''}>${label}</option>`).join('')}</select></label>`;
  bar.innerHTML = `<b>Preview</b>${select('scenario', scenario, scenarios)}${select('speed', String(speed), { 1: '1×', 5: '5×', 20: '20×', 100: '100×' })}${select('gh', gh, { signedIn: 'signed in', signedOut: 'signed out', missing: 'not installed' })}<button type="submit">Restart</button>`;
  bar.querySelector<HTMLSelectElement>('select[name=gh]')!.addEventListener('change', event => { gh = (event.target as HTMLSelectElement).value as Gh; });
  bar.addEventListener('submit', event => {
    event.preventDefault();
    const next = new URLSearchParams(location.search);
    for (const key of ['scenario', 'speed']) next.set(key, (bar.elements.namedItem(key) as HTMLSelectElement).value);
    location.search = next.toString();
  });
  const style = document.createElement('style');
  style.textContent = '.preview-toolbar{position:fixed;right:8px;bottom:8px;z-index:9999;display:flex;gap:8px;align-items:center;padding:6px 10px;border-radius:8px;background:#2b2140;color:#e9e3ff;font:12px system-ui;opacity:.35;transition:opacity .15s}.preview-toolbar:hover,.preview-toolbar:focus-within{opacity:1}.preview-toolbar select,.preview-toolbar button{margin-left:4px;font:inherit}';
  document.head.append(style);
  document.body.append(bar);
}

// `page=` opens a sidebar page the way a click would, so App keeps owning its page state.
const pages = ['Home', 'Overview', 'Designer', 'Clusters'];
function openPage(name: string) {
  let tries = 0;
  const timer = setInterval(() => {
    const tab = document.querySelector<HTMLButtonElement>(`.page-tab[aria-label="${name}"]`);
    if (tab && !tab.disabled) { clearInterval(timer); tab.click(); } else if (++tries > 100) clearInterval(timer);
  }, 50);
}

export function installPreview() {
  try {
    // Each load starts from the scenario, not from the previous walk's browser storage. Theme and sidebar choices are kept: they are presentation only.
    for (const key of Object.keys(localStorage)) if (key.startsWith('bridge.') && key !== themePreference && key !== sidebarPreference) localStorage.removeItem(key);
    if (!fresh) localStorage.setItem('bridge.welcomeSeen.v1', '1');
    // wsStarting is a first setup still under way, so its first test is still pending.
    if (!fresh) localStorage.setItem('bridge.firstTest.v1', scenario === 'wsStarting' ? 'pending' : 'attempted');
  } catch { /* Storage is optional in the preview. */ }
  (globalThis as { isTauri?: boolean }).isTauri = true;
  mockWindows('main');
  mockIPC((command, args) => handle(command, args as Record<string, unknown> | undefined));
  if (!params.has('clean')) toolbar();
  const page = params.get('page');
  if (page && pages.includes(page)) openPage(page);
}
