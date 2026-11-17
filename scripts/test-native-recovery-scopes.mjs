// Exercise the real driver's argument parsing and fixture rosters without launching Windows.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import vm from 'node:vm';

const source = (await readFile(new URL('./native-recovery-windows.mjs', import.meta.url), 'utf8')).replaceAll('\r\n', '\n');
const prefix = source.slice(0, source.indexOf('for (const root of')).replace(/^import .*;\n/gm, '');
function selection(scope) {
  const argv = ['node', 'driver', 'app.exe', 'captures', ...(scope === undefined ? [] : [scope])];
  return vm.runInNewContext(`${prefix}\n({ executable, output, scope: typeof captureScope === 'undefined' ? undefined : captureScope, runCore: typeof runCore === 'undefined' ? true : runCore, runUi: typeof runUi === 'undefined' ? true : runUi })`, {
    process: { platform: 'win32', env: { GITHUB_ACTIONS: 'true' }, argv }, resolve,
  });
}
assert.throws(() => selection('typo'), /capture scope/i, 'unknown scope must not silently run the full suite');
assert.equal(selection().scope, 'all');
assert.equal(selection('core').runUi, false);
assert.equal(selection('chat-agent-clusters').runCore, false);
assert.equal(selection('chat-agent-clusters').output, resolve('captures'), 'scope must not become a path');

function roster(name, chosen) {
  const expression = source.match(new RegExp(`const ${name} = ([\\s\\S]*?);\\n`))?.[1];
  assert.ok(expression, `missing ${name}`);
  return Function('runCore', 'transfer', 'startup', `return (${expression});`)(chosen.runCore, {}, () => ({}));
}
function actions(chosen) {
  const names = roster('actionFixtures', chosen);
  const append = source.match(/^  .*actionFixtures\.push\([^\n]+$/m)?.[0];
  assert.ok(append);
  Function('runUi', 'actionFixtures', append)(chosen.runUi, names);
  return names;
}
const all = actions(selection()), core = actions(selection('core')), ui = actions(selection('chat-agent-clusters'));
assert.equal(all.length, 62);
assert.equal(core.length, 46);
assert.equal(ui.length, 16);
assert.ok(core.includes('designer-reply-saved'), 'saved reply captures belong to the core scope');
assert.ok(!ui.includes('designer-reply-saved'), 'UI scope must not repeat saved reply captures');
assert.equal(new Set([...core, ...ui]).size, all.length, 'scopes must partition, not repeat or omit cases');
assert.deepEqual([...core, ...ui], all);
assert.deepEqual(ui, ['chat-empty', 'chat-pending', 'chat-running', 'chat-answer', 'chat-session-null', 'chat-session-answer', 'new-agent-form', 'new-agent-created', 'cluster-picker-open', 'cluster-checking', 'cluster-failure', 'cluster-connected', 'cluster-account-picker-open', 'cluster-account-allowed', 'cluster-account-forbidden', 'designer-no-destination']);
const chatStatus = source.match(/if \(command === 'agent_chat_status'\) \{([\s\S]*?return \{ documentId,[^\n]+;)\n        \}/)?.[1];
assert.ok(chatStatus, 'bounded Chat snapshot producer is missing');
for (const name of ['chat-pending', 'chat-running', 'chat-answer', 'chat-session-null', 'chat-session-answer']) {
  const snapshot = Function('name', 'documentId', chatStatus)(name, 'fixture-document');
  const turn = snapshot.versions[0].turns[0];
  const confirming = name.startsWith('chat-session-');
  assert.equal(snapshot.state, confirming ? 'waitingForSession' : name === 'chat-answer' ? 'ready' : 'working', name);
  assert.equal(turn.phase, confirming || name === 'chat-answer' ? 'Succeeded' : name === 'chat-pending' ? 'Pending' : 'Running', name);
  assert.equal(turn.answer, ['chat-answer', 'chat-session-answer'].includes(name) ? 'The page says fixture-checksum.' : null, name);
  assert.equal(turn.contextRetained, name === 'chat-answer', name);
  assert.equal(snapshot.busy, name !== 'chat-answer', name);
  assert.equal(snapshot.canSend, name === 'chat-answer', name);
}
for (const name of ['fixtures', 'setupFixtures', 'overviewVariants']) {
  assert.equal(roster(name, selection('chat-agent-clusters')).length, 0, `UI scope must not repeat ${name}`);
  assert.ok(roster(name, selection('core')).length > 0);
}
assert.match(source, /if \(runCore\) for \(const name of \['designer-connected'/, 'UI scope must not repeat minimum Designer cases');
assert.ok(source.includes("if (runUi) await resizeTo({ width: 1280, height: 800 }, 'Cluster wide');"), 'UI wide captures must verify the requested native 1280x800 client size');
// Exercise the real layout gate with measurements, without pretending to render a native window.
const layoutGate = source.slice(source.indexOf('  async function checkLayout('), source.indexOf('  // Setup-progress states:'));
assert.ok(layoutGate.includes('wide tabs must stay 180px'));
const wide = { width: 1280, scrollWidth: 1000, clientWidth: 1000, forms: [], rows: [], checks: [],
  tabs: [0, 1, 2].map(i => ({ width: 180, grow: '0', right: 180 * (i + 1) })),
  host: { x: 556 }, hostLabel: { width: 100 }, subtitles: 3, quiet: [{ start: '0px', end: '0px' }],
  scrollbar: { width: 'thin', color: 'rgb(90, 90, 90) rgb(240, 240, 240)' }, fallback: {} };
async function gate(layout) {
  return Function('evaluate', 'writeFile', 'join', 'output', `${layoutGate}; return checkLayout('light', 'wide', 'probe');`)(
    async () => layout, async () => {}, () => '', '');
}
await gate(wide);
await gate({ ...wide, width: 540, subtitles: 0 });
for (const mutation of [
  { tabs: wide.tabs.map(tab => ({ ...tab, width: 181.5 })) },
  { tabs: wide.tabs.map(tab => ({ ...tab, grow: '1' })) }, { host: { x: 555 } },
  { width: 540 }, { width: 540, subtitles: 0, hostLabel: { width: 161 } },
  { quiet: [{ start: '4px', end: '0px' }] }, { scrollbar: { width: 'auto', color: 'auto' } },
]) await assert.rejects(() => gate({ ...wide, ...mutation }));
const workflow = (await readFile(new URL('../.github/workflows/checks.yml', import.meta.url), 'utf8')).replaceAll('\r\n', '\n');
for (const [scope, directory] of [['core', 'native-recovery'], ['chat-agent-clusters', 'native-chat-agent-clusters']]) {
  const command = `node ../scripts/native-recovery-windows.mjs src-tauri/target/debug/orka-bridge.exe windows-artifact/${directory} ${scope}`;
  assert.ok(workflow.includes(command), `CI must run ${scope} into its own artifact directory`);
  const before = workflow.slice(0, workflow.indexOf(command));
  const step = before.slice(before.lastIndexOf('      - name:'));
  assert.match(step, /timeout-minutes: 3\n/, 'each scope keeps the existing three-minute ceiling');
}
assert.ok(workflow.includes('node ../scripts/test-native-recovery-scopes.mjs'));
for (const path of ['build.rs', 'permissions/setup.toml']) {
  const registration = await readFile(new URL(`../app/src-tauri/${path}`, import.meta.url), 'utf8');
  for (const command of ['cluster_service_accounts', 'cluster_service_account_preview']) {
    assert.ok(registration.includes(`"${command}"`), `${path} must permit the real desktop ${command} command`);
  }
}
// Saved-only header counts apply before Overview is visited, not after its deliberate visit.
const hiddenHeader = source.split('\n').find(line => line.includes("'hidden saved-file count'"));
assert.ok(hiddenHeader);
const checkHeader = Function('noDestination', 'until', 'evaluate', `return (async () => { ${hiddenHeader} })()`);
let headerReads = 0;
await checkHeader(true, async () => { headerReads++; }, () => true);
assert.equal(headerReads, 0, 'visited Overview must not be required to keep its pre-visit saved-only label');
await checkHeader(false, async () => { headerReads++; }, () => true);
assert.equal(headerReads, 1, 'other Designer cases retain the hidden saved-only count gate');
// Execute the driver's actual isolated-world IPC setup, not a second copy of its payloads.
const captureActions = source.slice(source.indexOf('  async function captureActions('), source.indexOf('  // Overview renderer fixtures:'));
for (const name of ['cluster-account-picker-open', 'cluster-account-allowed', 'cluster-account-forbidden', 'designer-no-destination']) {
  let injection;
  const stop = new Error('injection captured');
  const capture = vm.runInNewContext(`(${captureActions})`, {
    pair: { agent: '', tests: '' }, replySavedAgent: '', GiB: 1024 ** 3, fixtureAccount: 'synthetic-user',
    call: async () => ({ frameTree: { frame: { id: 'fixture' } }, executionContextId: 1 }),
    evaluate: async script => { injection = script; throw stop; },
  });
  await assert.rejects(capture('light', 'wide', name), error => error === stop);
  const window = {};
  vm.runInNewContext(injection, { window, localStorage: { setItem() {}, removeItem() {} }, document: { getElementById: () => ({ replaceWith() {} }), createElement: () => ({}) } });
  const invoke = window.__TAURI_INTERNALS__.invoke;
  if (name === 'designer-no-destination') {
    assert.equal((await invoke('setup_status')).selected, null);
    await assert.rejects(invoke('workspace_catalog'), /must not request a catalog/);
    continue;
  }
  const [target] = await invoke('cluster_discover');
  const args = { targetId: target.id, saName: null };
  assert.equal((await invoke('cluster_use', args)).status, 'needServiceAccount');
  if (name === 'cluster-account-forbidden') {
    await assert.rejects(invoke('cluster_service_accounts', args), error => error.check === 'serviceAccount' && error.result === 'permissionDenied');
  } else {
    assert.equal(JSON.stringify(await invoke('cluster_service_accounts', args)), JSON.stringify({ names: ['team-runner', 'task-reader'], truncated: false }));
    const preview = await invoke('cluster_service_account_preview', { ...args, saName: 'team-runner' });
    assert.equal(preview.status, 'allowed');
    assert.equal(preview.checks.map(check => `${check.resource}/${check.verb}/${check.status}`).join(), 'agents/create/allowed,agents/get/allowed,agents/list/allowed,tasks/create/allowed,tasks/get/allowed,tasks/list/allowed,providers/get/allowed,providers/list/allowed,tools/get/allowed,tools/list/allowed');
    await assert.rejects(invoke('cluster_service_account_preview', { ...args, saName: 'task-reader' }), /Only the selected account/);
  }
}
const reloadRecovery = source.slice(source.indexOf("if (name.startsWith('cluster-reload'))"), source.indexOf("if (name === 'model-copy') {"));
assert.ok(reloadRecovery.includes("document.querySelector('.cluster-card summary').click()"), 'connected recovery fixtures must reveal maintenance before reloading kubeconfig');

const clusterProbe = source.slice(source.indexOf("const probe = await evaluate(`({ heading: document.querySelector('h1')"), source.indexOf("throw Error('Connected cluster actions/wording:"));
assert.ok(clusterProbe.includes('summary[role="button"]'), 'connected actions include semantic summary disclosures, not only button elements');
assert.ok(!source.includes("await clickAction('Show details')") && !source.includes("await clickAction('Hide details')"), 'cluster disclosure uses its real summary, not the button-only helper');

// The connected scene must use the same real injection as discovery, then retain
// that selection for its success/details/list captures without another mount.
{
  let injection;
  const stop = new Error('injection captured');
  const capture = vm.runInNewContext(`(${captureActions})`, {
    pair: { agent: '', tests: '' }, replySavedAgent: '', GiB: 1024 ** 3, fixtureAccount: 'synthetic-user',
    call: async () => ({ frameTree: { frame: { id: 'fixture' } }, executionContextId: 1 }),
    evaluate: async script => { injection = script; throw stop; },
  });
  await assert.rejects(capture('light', 'wide', 'cluster-connected'), error => error === stop);
  const window = {};
  vm.runInNewContext(injection, { window, localStorage: { setItem() {}, removeItem() {} }, document: { getElementById: () => ({ replaceWith() {} }), createElement: () => ({}) } });
  const invoke = window.__TAURI_INTERNALS__.invoke;
  assert.equal((await invoke('cluster_places')).selectedKey, null);
  const [context] = await invoke('cluster_contexts');
  const [target] = await invoke('cluster_discover', { contextId: context.id });
  assert.equal((await invoke('cluster_use', { targetId: target.id, saName: null })).status, 'connected');
  const selected = await invoke('cluster_places');
  assert.equal(selected.places.find(place => place.key === selected.selectedKey).context, 'team-dev');
  assert.equal(selected.places.length, 3, 'selected, previously checked and unchecked rows stay distinct');
  assert.equal(selected.places.filter(place => place.checkedAt == null && place.observedAt === null).length, 1);
  const inventory = await invoke('overview_places', { key: selected.selectedKey });
  assert.equal(inventory.places[0].agents[0].name, 'runtime-reader');
  assert.equal(window.__bridgeClusterUses, 1, 'capture fixture must not replay Use');
}
for (const scene of ['cluster-success', 'cluster-details', 'cluster-selected-list']) {
  assert.ok(source.includes(`'${scene}'`), `Windows connected mount must capture ${scene}`);
}
const smoke = (await readFile(new URL('./native-smoke.swift', import.meta.url), 'utf8')).replaceAll('\r\n', '\n');
const matrix = smoke.slice(smoke.indexOf('    // Connected cluster capture matrix.'), smoke.indexOf('    // Done leaves the confirmed destination'));
assert.ok(matrix.includes('CGFloat(540), CGFloat(640)'), 'Mac connected scenes need true minimum content size');
assert.ok(matrix.includes('CGFloat(1280), CGFloat(800)'), 'Mac connected scenes need matching wide content size');
for (const scene of ['cluster-success-', 'cluster-details-', 'cluster-selected-list-']) assert.ok(matrix.includes(scene), `Mac matrix must capture ${scene}`);
assert.ok(matrix.includes('"Show details"') && matrix.includes('"Hide details"'), 'capture both closed and open disclosure, then restore it');
assert.ok(smoke.includes('if name.hasPrefix("cluster-success")'), 'scrolled matrix frames need their own visible-content gate, not the off-screen page heading');
assert.ok(smoke.includes('nativeElements(element).dropFirst()'), 'named page h1 must also read its inline code accessibility child, not depend on a cluster-result h2');
assert.ok(smoke.includes('clickButton("Show details")\n    clickButton("Change context")'), 'context maintenance must explicitly open details');
assert.ok(smoke.includes('clickButton("Show details")\n    clickButton("Remove")'), 'restart removal must explicitly open details');
console.log('PASS real scope parsing; 46+16 unique action cases; connected cluster IPC/matrix; Chat snapshot matrix; layout acceptance/rejection; IPC registration/injection; disjoint CI scopes with unchanged three-minute ceilings');
