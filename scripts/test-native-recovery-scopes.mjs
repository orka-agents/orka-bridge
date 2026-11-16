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
assert.equal(all.length, 59);
assert.equal(core.length, 46);
assert.equal(ui.length, 13);
assert.ok(core.includes('designer-reply-saved'), 'saved reply captures belong to the core scope');
assert.ok(!ui.includes('designer-reply-saved'), 'UI scope must not repeat saved reply captures');
assert.equal(new Set([...core, ...ui]).size, all.length, 'scopes must partition, not repeat or omit cases');
assert.deepEqual([...core, ...ui], all);
assert.deepEqual(ui, ['chat-empty', 'chat-pending', 'chat-running', 'chat-answer', 'new-agent-form', 'new-agent-created', 'cluster-picker-open', 'cluster-checking', 'cluster-failure', 'cluster-account-picker-open', 'cluster-account-allowed', 'cluster-account-forbidden', 'designer-no-destination']);
for (const name of ['fixtures', 'setupFixtures', 'overviewVariants']) {
  assert.equal(roster(name, selection('chat-agent-clusters')).length, 0, `UI scope must not repeat ${name}`);
  assert.ok(roster(name, selection('core')).length > 0);
}
assert.match(source, /if \(runCore\) for \(const name of \['designer-connected'/, 'UI scope must not repeat minimum Designer cases');
const workflow = (await readFile(new URL('../.github/workflows/checks.yml', import.meta.url), 'utf8')).replaceAll('\r\n', '\n');
for (const [scope, directory] of [['core', 'native-recovery'], ['chat-agent-clusters', 'native-chat-agent-clusters']]) {
  const command = `node ../scripts/native-recovery-windows.mjs src-tauri/target/debug/orka-bridge.exe windows-artifact/${directory} ${scope}`;
  assert.ok(workflow.includes(command), `CI must run ${scope} into its own artifact directory`);
  const before = workflow.slice(0, workflow.indexOf(command));
  const step = before.slice(before.lastIndexOf('      - name:'));
  assert.match(step, /timeout-minutes: 3\n/, 'each scope keeps the existing three-minute ceiling');
}
assert.ok(workflow.includes('node ../scripts/test-native-recovery-scopes.mjs'));
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
console.log('PASS real scope parsing; 46+13 unique action cases; disjoint CI scopes with unchanged three-minute ceilings');
