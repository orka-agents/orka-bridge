// Native replies for the preview scenarios that reach states the ordinary walk can't. Each value
// mirrors what the Rust side serialises for that state; backend.ts decides when to send it.

const day = 86_400;
const seconds = () => Math.floor(Date.now() / 1000);

// Setup snapshots. `localVmError` is the native diagnostic string Rust records with the failure.
export const workspaceFailures = {
  wsCouldntStart: {
    workspaceFailure: { operation: 'start', cause: 'guestConnection', codes: [] },
    localVmError: 'Starting the local VM failed: Runtime operation failed: Protocol(ConnectionReset)',
  },
  wsForced: {
    workspaceFailure: { operation: 'stop', cause: 'forcedStop', codes: [] },
    localVmError: 'Starting the local VM failed: Runtime operation failed: ShutdownForced',
  },
  // A newer Bridge refuses the older retained image before booting it.
  wsUpdate: {
    workspaceFailure: { operation: 'verify', cause: 'workspaceUpdateRequired', codes: [] },
    localVmError: 'Starting the local VM failed: Runtime operation failed: Image(WorkspaceUpdateRequired)',
  },
  // An interrupted update blocks ordinary start until recovery (LocalVm::block_for_recovery).
  wsRestore: {
    workspaceFailure: { operation: 'verify', cause: 'preparation', codes: [] },
    localVmError: 'Workspace recovery is required before starting the local cluster.',
  },
  wsRecovery: {
    workspaceFailure: { operation: 'verify', cause: 'preparation', codes: [] },
    localVmError: 'Workspace recovery is required before starting the local cluster.',
  },
} as const;
export type FailureScenario = keyof typeof workspaceFailures;

// Workspace update owner. Field rules follow UpgradeStatusDto::status_dto and eligible_status.
type UpgradeState = 'eligible' | 'sourceRetained' | 'incomplete' | 'consumed' | 'restored';
export type Credential = 'absent' | 'eligible' | 'transferred' | 'removed';
export function upgradeStatus(state: UpgradeState, transaction: string, credential: Credential) {
  const recorded = state !== 'eligible';
  return {
    state, transaction: recorded ? transaction : null, createdAt: recorded ? seconds() - 3 * day : null,
    sourceReleases: ['0.1.2'], targetRelease: '0.1.3', retainedAllocatedBytes: 8_000_000_000, additionalBytes: 34_400_000_000,
    credential, busy: false, error: null,
    repeated: state === 'incomplete' || state === 'restored',
    canReplace: ['eligible', 'sourceRetained', 'incomplete', 'restored'].includes(state),
    canRestore: ['sourceRetained', 'incomplete', 'consumed'].includes(state),
    canCancel: false,
  };
}

// AI connection errors, as SetupError serialises them.
export const keyRejected = (vendor: 'openai' | 'anthropic') => ({ kind: 'modelCredentialRejected', message: 'The model company did not accept this key.', vendor });
export const keyInvalid = { kind: 'modelKeyInvalid', message: "Paste the whole key with the selected company's key prefix." };
export const modelGuest = { kind: 'modelGuest', message: 'The guest model handler is unavailable or is not idle.' };
export const installationMismatch = (installation: string, savedInstallation: string, credentialSlot: string) => ({
  kind: 'modelInstallationMismatch', message: "Bridge's saved AI connection is from an earlier setup.",
  recovery: { installation, savedInstallation, credentialSlot },
});

// One remembered cluster, restored from disk after a restart: not yet checked in this run.
// Native observation times are Unix milliseconds.
export function failingClusterPlace(uid: (seed: string) => string, checked: boolean) {
  const [service, deployment] = [uid('cluster-service'), uid('cluster-deployment')];
  return {
    key: JSON.stringify(['team-dev', null, 'control', service, 'control', deployment, 'agents']),
    context: 'team-dev', service: 'control/orka', namespace: 'agents',
    stale: !checked, observedAt: checked ? Date.now() : null, checkedAt: checked ? Date.now() : Date.now() - 2 * day * 1000, agents: [],
  };
}
export const failingClusterTarget = (uid: (seed: string) => string) => ({
  id: uid('cluster-target'), context_id: uid('cluster-context'), context_name: 'team-dev',
  service_name: 'orka', service_namespace: 'control', watched_namespace: 'agents', image: 'ghcr.io/orka-agents/orka:v0.1.3',
});
export const failingClusterProvider = (uid: (seed: string) => string) => ({ id: uid('cluster-provider'), name: 'team-model', ready: true });
export const unreachableCluster = { check: 'clusterApi', result: 'connectionFailed' };
export const deniedAccess = { status: 'denied', providers: [], failure: { check: 'permissions', result: 'permissionDenied' } };
export function clusterCatalog(tools: string[]) {
  return {
    defaultModel: 'gpt-5.6-terra',
    models: { state: 'ready', items: [{ id: 'gpt-5.6-terra' }, { id: 'claude-sonnet-5-5' }] },
    tools: { state: 'ready', items: tools.map(name => ({ kind: 'builtin', name, description: name === 'web_fetch' ? 'Fetch a web page and return its text.' : 'Search the web.' })) },
  };
}

// Send destination checks, with the row wording from clusters/preflight.rs.
export function sendRows(tools: string[], listed: boolean) {
  return [
    { label: 'Access', status: 'allowed', message: 'Read access confirmed.' },
    listed ? { label: 'AI connection', status: 'allowed', message: 'The selected AI connection is ready.' }
      : { label: 'AI connection', status: 'cannotCheck', message: 'Bridge cannot check the selected AI connection.' },
    { label: 'Model', status: 'unverified', message: 'Model will be checked when tests run.' },
    ...tools.map(name => listed ? { label: name, status: 'allowed', message: 'Built in to this cluster.' }
      : { label: name, status: 'blocked', message: 'This built-in tool is not listed on this cluster.' }),
  ];
}

// The exact pair agent_folder::created_seed writes for a new agent; Designer treats these tests as a draft.
export function createdSeed(name: string, model = 'gpt-5.6-terra') {
  return {
    agent: `---\nname: ${JSON.stringify(name)}\nmodel: ${JSON.stringify(model)}\nkmx:\n  schemaVersion: 1\n  runtime: orka\n---\n\n`,
    tests: `---\nschemaVersion: 1\n---\n\n# ${name} tests\n\n\`\`\`yaml agent-test\nid: first-test\ninput: ""\nassertions: []\n\`\`\`\n`,
  };
}

// kubeconfig contexts for Add a cluster, with what discovery finds behind each: one Orka installation, two, none,
// and one whose installation is used through a ServiceAccount. Shapes follow ContextView and TargetView.
export function kubeContexts(uid: (seed: string) => string) {
  return [
    { id: uid('context-team-dev'), name: 'team-dev', current: true },
    { id: uid('context-team-shared'), name: 'team-shared', current: false },
    { id: uid('context-kind-scratch'), name: 'kind-scratch', current: false },
    { id: uid('context-team-restricted'), name: 'team-restricted', current: false },
  ];
}
export function contextTargets(uid: (seed: string) => string, contexts: { id: string; name: string }[]) {
  const target = (context: { id: string; name: string }, namespace: string, watched: string) => ({
    id: uid(`target-${context.name}-${namespace}`), context_id: context.id, context_name: context.name,
    service_name: 'orka', service_namespace: namespace, watched_namespace: watched, image: 'ghcr.io/orka-agents/orka:v0.1.3',
  });
  const [dev, shared, , restricted] = contexts;
  return [
    { target: target(dev, 'control', 'agents'), serviceAccount: false },
    { target: target(shared, 'control', 'agents'), serviceAccount: false },
    { target: target(shared, 'staging', 'agents-staging'), serviceAccount: false },
    { target: target(restricted, 'control', 'agents'), serviceAccount: true },
  ];
}
