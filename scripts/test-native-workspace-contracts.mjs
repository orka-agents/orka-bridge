// Executable contracts for synthetic renderer drivers; not native rendering evidence.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const read = async name => (await readFile(new URL(name, import.meta.url), 'utf8')).replaceAll('\r\n', '\n');
const source = await read('./native-recovery-windows.mjs');
const refusalExpression = source.match(/const refusal = await evaluate\(`([^`]+)`\);/)[1];
const retry = { textContent: 'Try again', disabled: true, title: 'Open the matching Bridge version.' };
const refusalCard = { innerText: 'Open the matching Bridge version.', getClientRects: () => [1], querySelectorAll: () => [retry] };
const refusal = Function('document', `return ${refusalExpression}`)({ querySelectorAll: selector => selector === '.workspace-failure' ? [refusalCard] : [retry] });
assert.equal(refusal.disabled, true, 'first-setup image refusal must find its actual disabled Try again control');
assert.equal(refusal.title, 'Open the matching Bridge version.');
const recoverySource = await read('../app/src/setup/recovery-info.ts');
const causes = Function(`return ${recoverySource.match(/const causes = (\[[^\n]+\]) as const/)[1]}`)();
const captureSetup = source.slice(source.indexOf('  async function captureSetup('), source.indexOf('  const actionFixtures'));
let setupInjection;
const setupStop = new Error('setup injection captured');
const setupCapture = vm.runInNewContext(`(${captureSetup})`, {
  frameTree: { frame: { id: 'fixture' } }, samplePair: { agent: '', tests: '' },
  call: async () => ({ executionContextId: 1 }), evaluate: async script => { setupInjection = script; throw setupStop; },
});
await assert.rejects(setupCapture('dark', 'setup-fit', ['setup-transfer', 'installing', null, false, 'Downloading']), error => error === setupStop);
const setupValues = new Map([['bridge.firstTest.v1', 'attempted']]);
vm.runInNewContext(setupInjection, { window: {}, localStorage: {
  setItem: (key, value) => setupValues.set(key, value), removeItem: key => setupValues.delete(key),
}, document: { getElementById: () => ({ replaceWith() {} }), createElement: () => ({}) } });
assert.equal(setupValues.has('bridge.firstTest.v1'), false, 'fresh setup must not inherit returning intent from earlier captures');
const captureActions = source.slice(source.indexOf('  async function captureActions('), source.indexOf('  // Overview renderer fixtures:'));
async function fixture(name) {
  let injection;
  const stop = new Error('injection captured');
  const capture = vm.runInNewContext(`(${captureActions})`, {
    pair: { agent: '', tests: '' }, replySavedAgent: '', GiB: 1024 ** 3, fixtureAccount: 'synthetic-user',
    call: async () => ({ frameTree: { frame: { id: 'fixture' } }, executionContextId: 1 }),
    until: async () => {},
    evaluate: async script => { injection = script; throw stop; },
  });
  await assert.rejects(capture('light', 'wide', name), error => error === stop);
  const values = new Map();
  const window = { dispatchEvent() {} };
  vm.runInNewContext(injection, { window, Event: class {}, localStorage: {
    setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key),
  }, document: { getElementById: () => ({ replaceWith() {} }), createElement: () => ({}) } });
  return { window, values, invoke: window.__TAURI_INTERNALS__.invoke };
}
for (const name of ['local-stopped', 'local-stop-failed', 'local-stop-lost-reply', 'designer-waiting']) {
  const { window, values, invoke } = await fixture(name);
  assert.equal(values.get('bridge.firstTest.v1'), 'attempted', `${name}: saved returning intent`);
  window.__bridgeSettleWorkspace('starting');
  let status = await invoke('setup_status');
  assert.equal(status.selected, 'localVm');
  assert.equal(status.localVm, 'starting');
  assert.equal(status.workspaceProgress.elapsedMs, 18000);
  for (const phase of ['ready', 'stopped-unexpectedly', 'failed', 'stop-failed', 'closed']) {
    window.__bridgeSettleWorkspace(phase);
    status = await invoke('setup_status');
    assert.equal(status.localVm, ['stop-failed', 'stopped-unexpectedly'].includes(phase) ? 'failed' : phase);
    if (status.workspaceFailure) assert.ok(causes.includes(status.workspaceFailure.cause), 'fixture must use the actual native failure schema');
    if (phase === 'stop-failed') assert.equal(status.workspaceFailure.operation, 'stop');
    if (phase === 'stopped-unexpectedly') {
      assert.equal(status.workspaceFailure.cause, 'unknown');
      assert.match(status.localVmError, /stopped unexpectedly/);
    }
  }
  assert.equal(window.__bridgeStops, 0, 'state capture must not click removed workspace Stop');
  assert.equal(window.__bridgeLocalSelections, 0);
  await invoke('quit_bridge');
  assert.equal(window.__bridgeQuits, 1, 'Quit escape must use terminal native IPC');
}
for (const name of ['first-test-running', 'ai-connected']) {
  const { values } = await fixture(name);
  assert.equal(values.has('bridge.firstTest.v1'), false, 'first setup intent must not become returning');
}
const start = source.indexOf('  function assertWorkspacePresentation(');
assert.ok(start >= 0, 'workspace presentation acceptance gate missing');
const gate = Function(`${source.slice(start, source.indexOf('  async function captureWorkspace(', start))}; return assertWorkspacePresentation;`)();
const probe = { heading: 'This computer', pill: 'Ready', text: 'Agents Files', buttons: [], primary: [], firstTest: 'attempted', bar: false, choices: false, runs: 0, grid: true, detailsOpen: false, width: 540, height: 640 };
gate('ready', probe, 'compact');
gate('starting-after-quit-synthetic', { ...probe, pill: 'Starting', text: 'Agents Files 0:18 so far' }, 'compact');
gate('stop-failed', { ...probe, pill: "Couldn't stop", text: "Agents Files The workspace didn't stop.", buttons: ['Quit Bridge'], primary: ['Quit Bridge'] }, 'compact');
for (const mutation of [{ buttons: ['Stop'] }, { buttons: ['Start'] }, { grid: false }, { firstTest: undefined }, { text: 'Files only' }, { detailsOpen: true }]) {
  assert.throws(() => gate('ready', { ...probe, ...mutation }, 'compact'));
}
assert.throws(() => gate('starting-after-quit-synthetic', { ...probe, pill: 'Starting' }, 'compact'));
assert.throws(() => gate('stop-failed', { ...probe, text: "The workspace didn't stop.", buttons: ['Restart workspace'], primary: ['Restart workspace'] }, 'compact'));
for (const name of ['native-lifecycle-macos.swift', 'native-lifecycle-windows.ps1']) {
  const driver = await read(`./${name}`);
  assert.ok(driver.includes('idle-close-exited'), `${name}: idle Close evidence missing`);
  assert.ok(!driver.includes('Stop the local cluster and quit'), `${name}: obsolete native menu label`);
  assert.ok(!driver.includes('Close quit instead of hiding'), `${name}: idle must exit, not hide`);
  assert.ok(!driver.includes('first-close-notice'), `${name}: no native Ready producer, no notice claim`);
}
const captures = JSON.parse(await read('./native-model-captures.json'));
// Upgrade frames share the destination journey but have their own exact matrix gate.
const destinationCaptures = new Set(captures['destinations-workspace'].filter(name => !name.startsWith('local-upgrade-')));
for (const size of ['wide', 'minimum']) {
  for (const state of ['ready', 'starting-after-quit-synthetic', 'failed', 'stopped-unexpectedly', 'stop-failed', 'forced-stop', 'closed', 'image-update', 'image-unsupported']) {
    assert.ok(destinationCaptures.has(`local-workspace-${state}-${size}`), `allocated capture missing: ${state}/${size}`);
    if (['failed', 'stopped-unexpectedly', 'stop-failed', 'forced-stop'].includes(state)) assert.ok(destinationCaptures.has(`local-workspace-${state}-details-${size}`), `expanded diagnostic capture missing: ${state}/${size}`);
  }
  for (const view of ['tests', 'chat']) assert.ok(destinationCaptures.has(`local-designer-waiting-${view}-${size}`), `allocated Designer waiting capture missing: ${view}/${size}`);
}
assert.ok(destinationCaptures.has('local-workspace-inside'));
assert.equal(captures.model.length, 208, '202 incoming model frames plus six existing Overview list/evidence captures must remain intact');
assert.equal(destinationCaptures.size, 51, '18 cluster/catalog frames plus33 workspace/setup/Designer frames');
for (const size of ['minimum', 'wide']) assert.ok(destinationCaptures.has(`local-workspace-setup-forced-stop-${size}`), 'first-run forced-stop capture missing');
assert.ok(captures.model.every(name => !destinationCaptures.has(name)), 'native journeys must be disjoint');
const mac = await read('./native-model-macos.swift');
assert.ok(!mac.includes('click("Stop the workspace"'), 'Mac must not navigate via removed workspace Stop');
assert.ok(mac.includes('workspaceCapture("starting-after-quit-synthetic"'), 'Mac synthetic startup capture must be labeled');
assert.ok(mac.includes('workspacePhase("setup-forced-stop")') && mac.includes('First-run forced stop must offer Quit without retry'), 'Mac first-run forced-stop controls must be checked');
const fixtureSource = await read('../app/src-tauri/examples/model_walk.rs');
for (const state of ['ready', 'starting', 'failed', 'stopped-unexpectedly', 'stop-failed', 'closed']) {
  assert.ok(fixtureSource.includes(`"workspace-scene-${state}"`), `Mac explicit scene missing: ${state}`);
}
assert.ok(fixtureSource.includes("localStorage.setItem('bridge.firstTest.v1', 'attempted'); location.reload();"), 'Mac returning reload must supply retired first-test intent');
assert.ok(mac.includes('workspaceCapture("stop-failed"') && mac.includes('workspaceCapture("stopped-unexpectedly"'), 'Mac failure captures missing');
// Execute the actual isolated-world Upgrade IPC producers, not a copied DTO.
const captureUpgrade = source.slice(source.indexOf('  async function captureUpgrade('), source.indexOf('  const notes = [];'));
async function upgradeFixture(name) {
  let injection;
  const stop = new Error('upgrade injection captured');
  const capture = vm.runInNewContext(`(${captureUpgrade.replace('of variants)', 'of variants.filter(v => v[1] === selectedFixture))')})`, {
    runUi: true, selectedFixture: name, frameTree: { frame: { id: 'fixture' } },
    call: async () => ({ executionContextId: 1 }),
    evaluate: async script => { injection = script; throw stop; },
  });
  await assert.rejects(capture('light', 'wide'), error => error === stop, name);
  const window = {};
  vm.runInNewContext(injection, { window, localStorage: { setItem() {}, removeItem() {} },
    document: { getElementById: () => ({ replaceWith() {} }), createElement: () => ({}) } });
  return { window, invoke: window.__TAURI_INTERNALS__.invoke };
}
const dtoKeys = 'additionalBytes,busy,canCancel,canReplace,canRestore,createdAt,credential,error,repeated,retainedAllocatedBytes,sourceReleases,state,targetRelease,transaction';
for (const name of ['upgrade-eligible-absent', 'upgrade-busy-cancel', 'upgrade-busy-locked', 'upgrade-error-forced']) {
  const { invoke } = await upgradeFixture(name);
  const status = await invoke('workspace_upgrade_status');
  assert.equal(Object.keys(status).sort().join(','), dtoKeys, name);
  assert.equal(status.canCancel, name === 'upgrade-busy-cancel', name);
  if (name === 'upgrade-busy-locked') assert.equal(status.busy, true, 'busy is not cancellation authority');
  if (name === 'upgrade-error-forced') assert.equal(status.error, 'forcedStop');
}
for (const [name, outcome] of [['upgrade-cancel-requested', 'requested'], ['upgrade-cancel-completed', 'completed'], ['upgrade-cancel-not-cancellable', 'notCancellable']]) {
  const { invoke, window } = await upgradeFixture(name);
  assert.equal(await invoke('workspace_upgrade_cancel'), outcome, 'bare cancellation outcome');
  const status = await invoke('workspace_upgrade_status');
  assert.equal(status.busy, outcome !== 'completed', 'acknowledgement is not completion');
  assert.equal(status.canCancel, false, 'no repeat cancellation after acknowledgement');
  assert.equal(window.__bridgeUpgradeCalls.some(call => call.args?.refresh), false, 'post-cancel observation is cached only');
}
for (const name of ['upgrade-refresh-busy', 'upgrade-refresh-io']) {
  const { invoke } = await upgradeFixture(name);
  const before = await invoke('workspace_upgrade_status');
  await assert.rejects(invoke('workspace_upgrade_status', { refresh: true }), error => name.endsWith('busy') ? error === 'refreshBusy' : error === 'Synthetic owner I/O failure');
  assert.deepEqual(await invoke('workspace_upgrade_status'), before, 'failed owner refresh must retain cached transaction/history');
}
{
  const { invoke, window } = await upgradeFixture('upgrade-refresh-pending');
  const before = await invoke('workspace_upgrade_status');
  let settled = false;
  const pending = invoke('workspace_upgrade_status', { refresh: true }).then(value => { settled = true; return value; });
  await Promise.resolve();
  assert.equal(settled, false, 'manual refresh must actually remain pending');
  assert.deepEqual(await invoke('workspace_upgrade_status'), before, 'cached observation stays independent of slow owner refresh');
  window.__bridgeFinishUpgradeRefresh();
  assert.deepEqual(await pending, before);
}
{
  const { invoke } = await upgradeFixture('upgrade-forward-cancelled');
  const reply = await invoke('workspace_upgrade_retry', { transaction: '40000000-0000-4000-8000-000000000004' });
  assert.equal(Object.keys(reply).sort().join(','), 'outcome,status');
  assert.equal(reply.outcome, 'cancelled');
  assert.equal(reply.status.state, 'incomplete');
  assert.equal(reply.status.busy, false);
  await assert.rejects(invoke('workspace_upgrade_restore', { transaction: reply.status.transaction }), /Unexpected native mutation/);
}
console.log('PASS native workspace driver contracts: returning intent, explicit states, Quit IPC, presentation gates, idle Close and exact Upgrade wire/refresh/cancel transitions');
