// Hosted-CI renderer fixtures in the real app's WebView2, not native IPC proofs.
// CDP is exposed only on loopback for this child process; no product test hooks.
import { spawn } from 'node:child_process';
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { resolve, join } from 'node:path';
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
  const bundle = await readFile(join('dist', 'assets', new URL(entry).pathname.split('/').pop()), 'utf8');
  const { frameTree } = await call('Page.getFrameTree');
  const fixtures = [
    ['permission', { operation: 'start', cause: 'permission', codes: [5] }, 'operating system denied'],
    ['prepare', { operation: 'prepare', cause: 'preparation', codes: [] }, 'local copies'],
    ['download-reserved', { operation: 'download', cause: 'network', codes: [] }, 'Rejected content is not kept'],
    ['unknown', { operation: 'start', cause: 'unknown', codes: [] }, "couldn't establish"],
    ['cli', 'modelCaptureUnavailable', 'gh auth login --hostname github.com --web'],
    ['sign-in', 'modelCaptureFailed', 'gh auth status --hostname github.com'],
    ['model-store', 'modelStore', 'native credential store'],
  ];
  for (const theme of ['light', 'dark']) {
    await call('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: theme }] });
    for (const [name, failure, expected] of fixtures) {
      const model = typeof failure === 'string';
      ({ executionContextId: contextId } = await call('Page.createIsolatedWorld', { frameId: frameTree.frame.id, worldName: `recovery-${theme}-${name}` }));
      await evaluate(`(() => {
        const failure = ${JSON.stringify(failure)};
        localStorage.setItem('bridge.welcomeSeen.v1', '1');
        window.isTauri = true;
        window.__TAURI_INTERNALS__ = { invoke: async command => {
          if (command === 'setup_status') return { selected: 'localVm', localVm: ${JSON.stringify(model ? 'ready' : 'failed')}, model: ${JSON.stringify(model ? 'available' : 'locked')}, ${model ? '' : 'workspaceFailure: failure,'} localVmError: null };
          if (command === 'setup_connect_model') throw { kind: failure };
          if (command === 'setup_requirements') return null;
          if (command === 'engine_call') throw { kind: 'unavailable' };
          throw Error('Unmocked fixture command: ' + command);
        } };
        document.getElementById('root').replaceWith(Object.assign(document.createElement('div'), { id: 'root' }));
      })()`);
      await evaluate(bundle);
      if (model) {
        await until(() => evaluate(`!![...document.querySelectorAll('button')].find(button => button.textContent === 'Use GitHub sign-in')`), 'model action');
        await evaluate(`[...document.querySelectorAll('button')].find(button => button.textContent === 'Use GitHub sign-in').click()`);
      }
      await until(() => evaluate(`document.body.textContent.includes(${JSON.stringify(expected)}) && !!document.querySelector('[role=alert]')`), `${name} recovery`);
      await evaluate('document.fonts.ready.then(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))');
      const image = await call('Page.captureScreenshot', { format: 'png' });
      await writeFile(join(output, `${theme}-${name}.png`), Buffer.from(image.data, 'base64'));
      console.log(`Captured native WebView2 renderer fixture: ${theme}/${name}`);
    }
  }
  await writeFile(join(output, 'README.txt'), 'Injected renderer states in the final-head embedded Windows app WebView2. Not native IPC, VM, credential or download-path verification. Download is reserved and unreachable from current native startup. Themes are WebView2 media emulation; system preferences are unchanged.\n');
  // Keep the source identity next to images without inferring it from branch names.
  await writeFile(join(output, 'head.txt'), await readFile(join(output, '..', 'native-lifecycle', 'head.txt')));
} finally {
  socket?.close();
  child.kill();
}
