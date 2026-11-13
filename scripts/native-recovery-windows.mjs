// Hosted-CI renderer fixtures in the real app's WebView2, not native IPC proofs.
// CDP is exposed only on loopback for this child process; no product test hooks.
import { spawn, spawnSync } from 'node:child_process';
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

if (process.platform !== 'win32' || process.env.GITHUB_ACTIONS !== 'true') throw Error('Hosted Windows CI only');
const [executable, output] = process.argv.slice(2).map(value => resolve(value));
if (!executable || !output) throw Error('Expected executable and output directory');
for (const root of [process.env.APPDATA, process.env.LOCALAPPDATA]) {
  if (!root) throw Error('Missing native profile root');
  for (const leaf of ['setup-v1/local-vm', 'local-vm-v1']) {
    if (existsSync(join(root, 'ai.orka.bridge', leaf))) throw Error('Refusing to resume an existing workspace');
  }
}
const source = process.env.BRIDGE_DEV_IMAGE_DIR;
if (!source || !(await stat(source)).isDirectory() || (await readdir(source)).length) throw Error('Expected empty development image source');
await mkdir(output, { recursive: true });
let socket;
const child = spawn(executable, [], { stdio: 'ignore', env: { ...process.env,
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-address=127.0.0.1 --remote-debugging-port=9222' } });
child.on('error', error => console.error(error));
async function until(work, label) {
  const deadline = Date.now() + 15000;
  while (Date.now() < deadline) {
    const value = await work();
    if (value) return value;
    await delay(100);
  }
  throw Error(`Missing ${label}`);
}
try {
  const page = await until(async () => {
    if (child.exitCode !== null) throw Error('Native app exited');
    try {
      const pages = await (await fetch('http://127.0.0.1:9222/json/list')).json();
      return pages.find(item => item.type === 'page' && item.url.includes('tauri.localhost'));
    } catch { return undefined; }
  }, 'native WebView2 debug target');
  socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
  let sequence = 0;
  const pending = new Map();
  socket.onmessage = event => {
    const reply = JSON.parse(event.data);
    if (pending.has(reply.id)) {
      const { resolve, reject, timer } = pending.get(reply.id); pending.delete(reply.id); clearTimeout(timer);
      if (reply.error) reject(Error(JSON.stringify(reply.error))); else resolve(reply.result);
    }
  };
  function call(method, params = {}) {
    const id = ++sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { pending.delete(id); reject(Error(`CDP timeout: ${method}`)); }, 10000);
      pending.set(id, { resolve, reject, timer }); socket.send(JSON.stringify({ id, method, params }));
    });
  }
  let contextId;
  async function evaluate(expression) {
    const result = await call('Runtime.evaluate', { expression, contextId, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  }
  await until(() => evaluate('!!window.__TAURI_INTERNALS__ && !!document.querySelector(".app-shell")'), 'embedded app');
  // Tauri's IPC properties are immutable in the main world. Use a debugger
  // isolated world with its own fixture IPC, sharing only the native WebView DOM.
  // Re-run the exact compiled entry; no product hooks or native setup mutations.
  const entry = await evaluate('document.querySelector("script[type=module][src]").src');
  const bundle = await readFile(join(fileURLToPath(new URL('../app/dist/assets/', import.meta.url)), new URL(entry).pathname.split('/').pop()), 'utf8');
  const { frameTree } = await call('Page.getFrameTree');
  const fixtures = [
    ['permission', { operation: 'start', cause: 'permission', codes: [5] }, 'operating system denied'],
    ['prepare', { operation: 'prepare', cause: 'preparation', codes: [] }, 'local copies'],
    ['download-reserved', { operation: 'download', cause: 'network', codes: [] }, 'Rejected content is not kept'],
    ['unknown', { operation: 'start', cause: 'unknown', codes: [] }, "couldn't establish"],
    ['cli', 'modelCaptureUnavailable', 'gh auth login --hostname github.com --web'],
    ['sign-in', 'modelCaptureFailed', 'gh auth status --hostname github.com'],
    ['model-store', 'modelStore', 'could not save or load your sign-in'],
    ['model-copy', 'modelCaptureFailed', 'Share the visible error message'],
    ['cluster-empty', null, 'No kubectl contexts were found'],
    ['cluster-configuration', { check: 'configuration', result: 'unknown' }, 'kubeconfig could not be read'],
    ['cluster-permission', { check: 'providers', result: 'permissionDenied' }, 'Provider reading failed. Access was denied.'],
    ['cluster-unknown', { check: 'orkaIdentity', result: 'unknown' }, 'The cause could not be established'],
    ['cluster-reload-pending', null, 'Reload pending'],
    ['cluster-reloaded', null, 'corrected-context'],
  ];
  for (const theme of ['light', 'dark']) {
    await call('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: theme }] });
    for (const [name, failure, expected] of fixtures) {
      const cluster = name.startsWith('cluster-');
      const model = typeof failure === 'string';
      ({ executionContextId: contextId } = await call('Page.createIsolatedWorld', { frameId: frameTree.frame.id, worldName: `recovery-${theme}-${name}` }));
      await evaluate(`(() => {
        const failure = ${JSON.stringify(failure)};
        const context = { id: '00000000-0000-4000-8000-000000000001', name: 'fixture-context' };
        const target = { id: '00000000-0000-4000-8000-000000000002', context_id: context.id, context_name: context.name, service_name: 'orka', service_namespace: 'control', watched_namespace: 'agents', image: 'orka:fixture' };
        const provider = { id: '00000000-0000-4000-8000-000000000003', name: 'fixture-model', ready: true };
        if (${JSON.stringify(name)} === 'model-copy') Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async () => { throw Error('Fixture clipboard unavailable'); } } });
        localStorage.setItem('bridge.welcomeSeen.v1', '1');
        window.isTauri = true;
        window.__TAURI_INTERNALS__ = { invoke: async command => {
          if (command === 'setup_status') return { selected: ${JSON.stringify(cluster ? null : 'localVm')}, localVm: ${JSON.stringify(cluster ? 'idle' : model ? 'ready' : 'failed')}, model: ${JSON.stringify(model ? 'available' : 'locked')}, ${model || cluster ? '' : 'workspaceFailure: failure,'} localVmError: null };
          if (command === 'setup_connect_model') throw { kind: failure };
          if (command === 'cluster_contexts') {
            if (${JSON.stringify(name)} === 'cluster-configuration') throw failure;
            return ${JSON.stringify(name)} === 'cluster-empty' ? [] : [context];
          }
          if (command === 'cluster_discover') return [target];
          if (command === 'cluster_use') return failure ? { status: 'unavailable', providers: [], failure } : { status: 'connected', providers: [provider], failure: null };
          if (command === 'cluster_reload') return ${JSON.stringify(name)} === 'cluster-reload-pending' ? new Promise(() => {}) : [{ ...context, id: '00000000-0000-4000-8000-000000000004', name: 'corrected-context' }];
          if (command === 'setup_requirements') return null;
          if (command === 'engine_call') throw { kind: 'unavailable' };
          throw Error('Unmocked fixture command: ' + command);
        } };
        document.getElementById('root').replaceWith(Object.assign(document.createElement('div'), { id: 'root' }));
      })()`);
      await evaluate(bundle);
      if (model) {
        await until(() => evaluate(`!![...document.querySelectorAll('button')].find(button => button.textContent === 'Connect the AI')`), 'model action');
        await evaluate(`[...document.querySelectorAll('button')].find(button => button.textContent === 'Connect the AI').click()`);
      }
      if (cluster) {
        await until(() => evaluate(`!!document.querySelector('#workspace-cluster')`), 'cluster choice');
        await evaluate(`document.querySelector('#workspace-cluster').click()`);
        if (!['cluster-empty', 'cluster-configuration'].includes(name)) {
          await until(() => evaluate(`document.querySelector('#cluster-context')?.options.length > 1`), 'fixture contexts');
          await evaluate(`(() => { const select = document.querySelector('#cluster-context'); select.value = select.options[1].value; select.dispatchEvent(new Event('change', { bubbles: true })); })()`);
          await until(() => evaluate(`!![...document.querySelectorAll('button')].find(button => button.textContent === 'Use this cluster')`), 'cluster Use');
          await evaluate(`[...document.querySelectorAll('button')].find(button => button.textContent === 'Use this cluster').click()`);
          if (name.startsWith('cluster-reload')) {
            await until(() => evaluate(`document.body.textContent.includes('Access confirmed')`), 'confirmed fixture access');
            await evaluate(`[...document.querySelectorAll('button')].find(button => button.textContent === 'Reload kubeconfig').click()`);
          }
        }
      }
      if (name === 'model-copy') {
        await until(() => evaluate(`!![...document.querySelectorAll('button')].find(button => button.textContent === 'Copy details for help')`), 'model copy');
        await evaluate(`[...document.querySelectorAll('button')].find(button => button.textContent === 'Copy details for help').click()`);
      }
      await until(() => evaluate(`document.body.textContent.includes(${JSON.stringify(expected)}) && !!document.querySelector(${JSON.stringify(cluster ? '.cluster-card' : '[role=alert]')})`), `${name} recovery`);
      if (cluster) await evaluate(`document.querySelector('.cluster-card').scrollIntoView({ block: 'end' })`);
      await evaluate('document.fonts.ready.then(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))');
      const image = await call('Page.captureScreenshot', { format: 'png' });
      await writeFile(join(output, `${theme}-${name}.png`), Buffer.from(image.data, 'base64'));
      console.log(`Captured native WebView2 renderer fixture: ${theme}/${name}`);
    }
  }
  // Setup-progress states: renderer fixtures for transfer, startup, ready and a queued model choice.
  // Fixture IPC only: nothing here exercises real downloads, the VM or credentials.
  const GiB = 1024 ** 3;
  const transfer = { phase: 'materializing', bytes: Math.round(1.1 * GiB), totalBytes: Math.round(2.8 * GiB), elapsedMs: 20000, remainingMs: 30000 };
  const startup = elapsedMs => ({ phase: 'starting', bytes: null, totalBytes: null, elapsedMs, remainingMs: null });
  const setupFixtures = [
    ['setup-transfer', 'installing', transfer, false, 'Installing, 1.1 GiB of 2.8 GiB'],
    ['setup-transfer-queued', 'installing', transfer, true, 'Connects automatically when Orka is ready.'],
    // A valid first observation, then every later read fails: the card must say unavailable, not keep the old bar.
    ['setup-unavailable', 'installing', transfer, false, 'Workspace status is unavailable.', true],
    // A platform without native model support (reported by the snapshot) offers no early choice: a Waiting row, no action.
    ['setup-transfer-unsupported', 'installing', transfer, false, 'Installing, 1.1 GiB of 2.8 GiB', false, false],
    ['setup-startup', 'starting', startup(12000), false, 'Rough estimate, not a measurement'],
    ['setup-startup-slow', 'starting', startup(45000), false, 'Taking longer than the rough 30-second estimate'],
    ['setup-ready', 'ready', null, false, 'Your workspace is running on this computer.'],
  ];
  const pair = { agent: '---\nname: web-reader\ndescription: Answers from the web page named in the request\n---\nFetch before answering.', tests: 'The checksum check.' };
  async function captureSetup(theme, size, [name, localVm, progress, queue, expected, failLater, modelSupported = true]) {
    ({ executionContextId: contextId } = await call('Page.createIsolatedWorld', { frameId: frameTree.frame.id, worldName: `setup-${theme}-${size}-${name}` }));
    await evaluate(`(() => {
      const pair = ${JSON.stringify(pair)};
      let reads = 0;
      localStorage.setItem('bridge.welcomeSeen.v1', '1');
      window.isTauri = true;
      window.__TAURI_INTERNALS__ = { invoke: async command => {
        if (command === 'setup_status' && ${JSON.stringify(!!failLater)} && reads++ > 0) throw Error('Fixture status unreadable');
        if (command === 'setup_status') return { selected: 'localVm', localVm: ${JSON.stringify(localVm)}, model: ${JSON.stringify(localVm === 'ready' ? 'available' : 'locked')}, modelSupported: ${JSON.stringify(modelSupported)}, workspaceProgress: ${JSON.stringify(progress)}, localVmError: null };
        if (command === 'setup_example') return pair;
        if (command === 'setup_requirements') return null;
        if (command === 'engine_call') throw { kind: 'unavailable' };
        throw Error('Unmocked fixture command: ' + command);
      } };
      document.getElementById('root').replaceWith(Object.assign(document.createElement('div'), { id: 'root' }));
    })()`);
    await evaluate(bundle);
    if (queue) {
      await until(() => evaluate(`!![...document.querySelectorAll('button')].find(button => button.textContent === 'Connect the AI')`), 'queueable model action');
      await evaluate(`[...document.querySelectorAll('button')].find(button => button.textContent === 'Connect the AI').click()`);
    }
    await until(() => evaluate(`document.body.textContent.includes(${JSON.stringify(expected)}) && !!document.querySelector('.setup-progress')`), `${name} setup progress`);
    await evaluate('document.fonts.ready.then(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))');
    if (!modelSupported && !await evaluate(`document.body.textContent.includes('Waiting') && !document.body.textContent.includes('Queued') && ![...document.querySelectorAll('button')].some(button => button.textContent === 'Connect the AI')`)) {
      throw new Error(`${name}: an unsupported platform must show Waiting with no model action`);
    }
    const shoot = async suffix => {
      const image = await call('Page.captureScreenshot', { format: 'png' });
      await writeFile(join(output, `${theme}-${size}-${name}${suffix}.png`), Buffer.from(image.data, 'base64'));
      console.log(`Captured native WebView2 renderer fixture: ${theme}/${size}/${name}${suffix}`);
    };
    await shoot('');
    if (size === 'compact') { // The long step history sits below the model action; show it too.
      await evaluate(`document.querySelector('main').scrollTop = document.querySelector('main').scrollHeight`);
      await shoot('-end');
    }
  }
  // Setup actions and the first test use the compiled app and synthetic IPC only.
  const actionFixtures = ['chooser', 'ai-idle', 'ai-connecting', 'ai-connected', 'first-test-running', 'first-test-result', 'change', 'designer'];
  async function clickAction(label) {
    await until(() => evaluate(`!![...document.querySelectorAll('button')].find(button => button.textContent === ${JSON.stringify(label)} && !button.disabled)`), label);
    await evaluate(`[...document.querySelectorAll('button')].find(button => button.textContent === ${JSON.stringify(label)} && !button.disabled).click()`);
  }
  async function captureActions(theme, size, name) {
    ({ executionContextId: contextId } = await call('Page.createIsolatedWorld', { frameId: frameTree.frame.id, worldName: `actions-${theme}-${size}-${name}` }));
    const first = name.startsWith('first-test-');
    await evaluate(`(() => {
      const pair = ${JSON.stringify(pair)};
      const name = ${JSON.stringify(name)};
      const documentId = '9b2b93c4-375c-4815-9717-39852a96e411';
      let selected = ${JSON.stringify(!first && name !== 'chooser')};
      let connected = false;
      let submitted = false;
      const status = () => ({ selected: selected ? 'localVm' : null, localVm: selected ? 'ready' : 'idle', model: selected ? 'available' : 'locked' });
      const report = { verdict: 'failed', cases: [{ id: 'page-checksum', question: 'What checksum does the page show?', phase: 'Succeeded', answer: 'No checksum was given.', completedTools: ['web_fetch'], assertions: [{ id: 'checksum', kind: 'contains', expected: 'fixture-checksum', verdict: 'failed' }], verdict: 'failed' }] };
      const snapshot = () => {
        const running = submitted && name === 'first-test-running';
        const complete = submitted && name === 'first-test-result';
        return { documentId, busy: running ? { documentId, name: 'web-reader' } : null, state: running ? 'running' : complete ? 'complete' : 'idle', canRun: !running, modelConnected: connected, stage: running ? 'observe' : complete ? 'done' : null, model: connected ? 'fixture-model' : null, report: complete ? report : null, acknowledged: [], pending: null, error: null };
      };
      localStorage.setItem('bridge.welcomeSeen.v1', '1');
      localStorage.removeItem('bridge.firstTest.v1');
      window.isTauri = true;
      window.__TAURI_INTERNALS__ = { invoke: async (command, args) => {
        if (command === 'setup_status') return status();
        if (command === 'setup_select_local') { selected = true; return status(); }
        if (command === 'setup_requirements') return null;
        if (command === 'setup_example') return pair;
        if (command === 'setup_connect_model') {
          if (name === 'ai-connecting') return new Promise(() => {});
          connected = true; return { ready: true };
        }
        if (command === 'agent_open') return { ...pair, documentId, name: 'web-reader' };
        if (command === 'agent_run') { if (args?.bundledExample !== true) throw Error('Fixture automatic test requires native source guard'); if (submitted) throw Error('Fixture automatic test replayed'); submitted = true; return snapshot(); }
        if (command === 'agent_run_status') return snapshot();
        if (command === 'send_history' || command === 'cluster_contexts') return [];
        throw Error('Unmocked action fixture command: ' + command);
      } };
      document.getElementById('root').replaceWith(Object.assign(document.createElement('div'), { id: 'root' }));
    })()`);
    await evaluate(bundle);
    if (first) await clickAction('Continue');
    if (first || name === 'ai-connecting' || name === 'ai-connected') await clickAction('Connect the AI');
    if (name === 'change') await clickAction('Change where agents run');
    if (name === 'designer') await clickAction('Read its instructions');
    const expected = { chooser: 'Where should your agents run?', 'ai-idle': 'GitHub CLI installed and signed in', 'ai-connecting': 'Connecting', 'ai-connected': 'Connected with GitHub Copilot', 'first-test-running': 'Run your first testRunning', 'first-test-result': 'No checksum was given.', change: 'Keep this workspace running?', designer: 'Instructions and settings' };
    await until(() => evaluate(`document.body.textContent.includes(${JSON.stringify(expected[name])})`), `${name} action state`);
    if (name === 'first-test-result' || name === 'designer') {
      await until(() => evaluate(`!!document.querySelector('.designer-work')`), 'Designer columns');
      const layout = await evaluate(`(() => {
        const tests = document.querySelector('.designer-tests').getBoundingClientRect();
        const editor = document.querySelector('.designer-file').getBoundingClientRect();
        const send = document.querySelector('.designer-checks').getBoundingClientRect();
        return { tests: { x: tests.x, y: tests.y, bottom: tests.bottom }, editor: { x: editor.x, y: editor.y, bottom: editor.bottom }, sendY: send.y, width: innerWidth, height: innerHeight, scrollWidth: document.querySelector('main').scrollWidth };
      })()`);
      if (size === 'wide' ? Math.abs(layout.tests.y - layout.editor.y) > 2 || layout.tests.x <= layout.editor.x : layout.tests.bottom > layout.editor.y + 2) throw Error(`Test visibility failed: ${JSON.stringify(layout)}`);
      if (layout.sendY < layout.editor.bottom || layout.scrollWidth > layout.width + 2) throw Error(`Editor/Send layout failed: ${JSON.stringify(layout)}`);
      await writeFile(join(output, `${theme}-${size}-${name}-layout.json`), JSON.stringify(layout, null, 2));
    }
    await evaluate('document.fonts.ready.then(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))');
    const image = await call('Page.captureScreenshot', { format: 'png' });
    await writeFile(join(output, `${theme}-${size}-${name}.png`), Buffer.from(image.data, 'base64'));
    if (size === 'compact' && name === 'change') {
      await evaluate(`document.querySelector('.workspace-change').scrollIntoView({ block: 'center' })`);
      const detail = await call('Page.captureScreenshot', { format: 'png' });
      await writeFile(join(output, `${theme}-${size}-${name}-confirmation.png`), Buffer.from(detail.data, 'base64'));
    }
    console.log(`Captured native WebView2 action fixture: ${theme}/${size}/${name}`);
  }
  const notes = [];
  for (const theme of ['light', 'dark']) {
    await call('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: theme }] });
    for (const fixture of setupFixtures) await captureSetup(theme, 'wide', fixture);
    for (const name of actionFixtures) await captureActions(theme, 'wide', name);
  }
  // Resize the real native window so its client area is 540x900, as a user would. Win32 SetWindowPos through
  // PowerShell, then verify what the embedded WebView actually reports; never assume the resize worked.
  const target = { width: 540, height: 900 };
  const resizeScript = `
$ErrorActionPreference = 'Stop'
Add-Type @'
using System; using System.Runtime.InteropServices;
public struct WinRect { public int L, T, R, B; }
public static class WinSize {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out WinRect r);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out WinRect r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int h2, uint flags);
}
'@
$h = (Get-Process -Id ${child.pid}).MainWindowHandle
if ($h -eq [IntPtr]::Zero) { throw 'No native main window handle' }
for ($i = 0; $i -lt 4; $i++) {
  $w = New-Object WinRect; $c = New-Object WinRect
  [void][WinSize]::GetWindowRect($h, [ref]$w); [void][WinSize]::GetClientRect($h, [ref]$c)
  $dw = ${target.width} - ($c.R - $c.L); $dh = ${target.height} - ($c.B - $c.T)
  if ($dw -eq 0 -and $dh -eq 0) { break }
  [void][WinSize]::SetWindowPos($h, [IntPtr]::Zero, $w.L, $w.T, ($w.R - $w.L) + $dw, ($w.B - $w.T) + $dh, 0x0014)
  Start-Sleep -Milliseconds 300
}
$c = New-Object WinRect; [void][WinSize]::GetClientRect($h, [ref]$c)
"client=$($c.R - $c.L)x$($c.B - $c.T)"`;
  // EncodedCommand avoids any argument-quoting ambiguity for the embedded C#.
  const resized = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(resizeScript, 'utf16le').toString('base64')], { encoding: 'utf8', timeout: 20000 });
  let mode = 'native-window-resize';
  const viewport = async () => evaluate('({ width: innerWidth, height: innerHeight, dpr: devicePixelRatio })');
  ({ executionContextId: contextId } = await call('Page.createIsolatedWorld', { frameId: frameTree.frame.id, worldName: 'viewport-probe' }));
  await delay(500);
  let reported = await viewport();
  const close = (a, b) => Math.abs(a - b) <= 2;
  if (resized.status !== 0 || !close(reported.width * reported.dpr, target.width) || !close(reported.height * reported.dpr, target.height)) {
    // Limitation, reported honestly: the window could not be resized, so use renderer viewport emulation.
    console.warn(`Native window resize unverified (exit ${resized.status}); using renderer viewport emulation. ${resized.stderr ?? ''}`);
    mode = 'renderer-viewport-emulation';
    await call('Emulation.setDeviceMetricsOverride', { width: target.width, height: target.height, deviceScaleFactor: 1, mobile: false });
    reported = await viewport();
  }
  notes.push(`Compact captures: ${mode}. Requested content area ${target.width}x${target.height}; WebView reported ${reported.width}x${reported.height} CSS px at devicePixelRatio ${reported.dpr}. ${resized.stdout.trim()}`);
  for (const theme of ['light', 'dark']) {
    await call('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: theme }] });
    for (const fixture of setupFixtures) await captureSetup(theme, 'compact', fixture);
    for (const name of actionFixtures) await captureActions(theme, 'compact', name);
  }
  await writeFile(join(output, 'compact-window.txt'), notes.join('\n') + '\n');
  await writeFile(join(output, 'README.txt'), 'Injected renderer states in the final-head embedded Windows app WebView2. Setup-progress states (setup-*) and setup actions/first-test/Designer states use synthetic IPC. Designer layout JSON records test/editor/Send geometry. compact-window.txt says whether 540x900 was a real native window resize or renderer viewport emulation. Not native IPC, VM, credential or download-path verification. Download is reserved and unreachable from current native startup. Themes are WebView2 media emulation; system preferences are unchanged.\n');
  // Keep the source identity next to images without inferring it from branch names.
  await writeFile(join(output, 'head.txt'), await readFile(join(output, '..', 'native-lifecycle', 'head.txt')));
} finally {
  socket?.close();
  await new Promise(resolve => {
    if (child.exitCode !== null || child.signalCode !== null || !child.pid) { resolve(); return; }
    child.once('exit', resolve);
    child.kill();
  });
}
