// Disposable hosted Windows check using native-recovery-windows.mjs's CDP seam.
// Real embedded frontend and native context discovery; synthetic local Run/Chat.
import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
if (process.platform !== 'win32' || process.env.GITHUB_ACTIONS !== 'true') throw Error('Disposable hosted Windows only');
for (const root of [process.env.APPDATA, process.env.LOCALAPPDATA]) {
  if (!root || existsSync(join(root, 'ai.orka.bridge'))) throw Error('Expected fresh Bridge profile');
}
const [executable, output] = process.argv.slice(2).map(p => resolve(p));
await mkdir(output, { recursive: true });
const nativeCalls = [];
let socket, evaluate, contextId;
const child = spawn(executable, [], { stdio: 'ignore', env: { ...process.env,
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-address=127.0.0.1 --remote-debugging-port=9222' } });
async function until(work, label) {
  const end = Date.now() + 15000;
  while (Date.now() < end) { if (await work()) return; await delay(100); }
  throw Error(`Missing ${label}`);
}
let capture;
try {
  let page;
  await until(async () => {
    try { page = (await (await fetch('http://127.0.0.1:9222/json/list')).json()).find(p => p.type === 'page' && p.url.includes('tauri.localhost')); return page; }
    catch { return false; }
  }, 'WebView2 target');
  socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((ok, no) => { socket.onopen = ok; socket.onerror = no; });
  let sequence = 0;
  const pending = new Map();
  function call(method, params = {}) {
    const id = ++sequence;
    return new Promise((ok, no) => {
      const timer = setTimeout(() => { pending.delete(id); no(Error(`CDP timeout ${method}`)); }, 10000);
      pending.set(id, { ok, no, timer }); socket.send(JSON.stringify({ id, method, params }));
    });
  }
  evaluate = async (expression, world = contextId) => {
    const r = await call('Runtime.evaluate', { expression, ...(world == null ? {} : { contextId: world }), awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw Error(JSON.stringify(r.exceptionDetails));
    return r.result.value;
  };
  socket.onmessage = async event => {
    const reply = JSON.parse(event.data);
    if (pending.has(reply.id)) {
      const p = pending.get(reply.id); pending.delete(reply.id); clearTimeout(p.timer);
      if (reply.error) p.no(Error(JSON.stringify(reply.error))); else p.ok(reply.result);
    }
    if (reply.method === 'Runtime.bindingCalled' && reply.params.name === 'nativeCluster') {
      const { id, command, args } = JSON.parse(reply.params.payload);
      try {
        if (!['cluster_contexts', 'cluster_discover'].includes(command)) throw Error('Unapproved native command');
        const result = await evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${JSON.stringify(args)}).then(value => ({ok:true,value}), error => ({ok:false,error}))`, null);
        nativeCalls.push({ command, args, result });
        await evaluate(`window.nativeReplies.get(${id})(${JSON.stringify(result)}); window.nativeReplies.delete(${id})`);
      } catch (error) { console.error(error); }
    }
  };
  await until(() => evaluate('!!window.__TAURI_INTERNALS__ && !!document.querySelector(".app-shell")', null), 'embedded app');
  const entry = await evaluate('document.querySelector("script[type=module][src]").src', null);
  const bundle = await readFile(join('app/dist/assets', new URL(entry).pathname.split('/').pop()), 'utf8');
  const pair = { agent: await readFile('app/src-tauri/example-agent/web-reader.agent.md', 'utf8'), tests: await readFile('app/src-tauri/example-agent/web-reader.agent.test.md', 'utf8') };
  const { frameTree } = await call('Page.getFrameTree');
  ({ executionContextId: contextId } = await call('Page.createIsolatedWorld', { frameId: frameTree.frame.id, worldName: 'failed-context-check' }));
  await call('Runtime.enable');
  await call('Runtime.addBinding', { name: 'nativeCluster', executionContextId: contextId });
  await evaluate(`(() => {
    const pair = ${JSON.stringify(pair)};
    const documentId = '9b2b93c4-375c-4815-9717-39852a96e411';
    const source = {agent:'a'.repeat(64),tests:'b'.repeat(64)};
    let connected = false, ran = false, serial = 0;
    window.nativeReplies = new Map(); window.fixtureCalls = [];
    const report = {verdict:'passed',cases:[{id:'page-checksum',question:'What checksum does the page show?',phase:'Succeeded',answer:'Synthetic checksum.',completedTools:['web_fetch'],assertions:[{id:'checksum',kind:'contains',expected:'checksum',verdict:'passed'}],verdict:'passed'}]};
    const run = () => ({documentId,source,busy:null,state:ran?'complete':'idle',canRun:true,modelConnected:connected,model:'fixture-model',report:ran?report:null,acknowledged:[],pending:null,error:null});
    let chat = {documentId,state:'ready',busy:false,canSend:true,versions:[],error:null};
    localStorage.setItem('bridge.welcomeSeen.v1','1'); localStorage.removeItem('bridge.firstTest.v1');
    window.isTauri = true;
    window.__TAURI_INTERNALS__ = { invoke: async (command,args = {}) => {
      window.fixtureCalls.push({command,args});
      if (['cluster_contexts','cluster_discover'].includes(command)) {
        const id = ++serial;
        const result = await new Promise(resolve => { window.nativeReplies.set(id,resolve); nativeCluster(JSON.stringify({id,command,args})); });
        if (!result.ok) throw result.error; return result.value;
      }
      if (command === 'setup_status') return {selected:'localVm',localVm:'ready',model:'available'};
      if (command === 'setup_connect_model') { connected = true; return {ready:true}; }
      if (command === 'setup_requirements') return null;
      if (command === 'cluster_places') return {places:[],selectedKey:null,persistenceState:'complete'};
      if (command === 'setup_example') return pair;
      if (command === 'agent_open') return {...pair,documentId,source,name:'web-reader'};
      if (command === 'agents_list') return [{id:documentId,name:'web-reader',path:null,available:true}];
      if (command === 'agent_selected') return documentId;
      if (command === 'agent_select') return null;
      if (command === 'agent_run_status') return run();
      if (command === 'agent_run') { ran = true; return run(); }
      if (command === 'agent_chat_status') return chat;
      if (command === 'agent_chat_send') { chat = {...chat,versions:[{turns:[...(chat.versions[0]?.turns ?? []),{message:args.message,phase:'Succeeded',answer:'Synthetic local answer.',tools:[],contextRetained:true,terminationUnconfirmed:false,stopUnconfirmed:false}]}]}; return chat; }
      if (command === 'send_history') return [];
      if (command === 'send_status') return {documentId,view:null,busy:null};
      if (command === 'workspace_catalog') return {defaultModel:'fixture-model',models:{state:'ready',items:[{id:'fixture-model'}]},tools:{state:'ready',items:[{kind:'builtin',name:'web_fetch'}]}};
      if (command === 'overview_files') return {state:'complete',runActive:false,historyState:'complete',runAgents:[],files:[]};
      if (command === 'overview_agents') return {state:'unavailable',installation:null,agents:[],observedAt:null,observation:null,runActive:false,reason:'stopped'};
      if (command === 'overview_places') return {historyState:'complete',unobserved:[],places:[]};
      if (command === 'engine_call') return {kind:'version',version:'synthetic',hasDiagnostics:false};
      throw Error('Unmocked fixture command '+command);
    }};
    document.getElementById('root').replaceWith(Object.assign(document.createElement('div'),{id:'root'}));
  })()`);
  await evaluate(bundle);
  async function click(label) {
    const expression = `([...document.querySelectorAll('button')].find(b => b.getClientRects().length && (b.getAttribute('aria-label') === ${JSON.stringify(label)} || b.textContent.trim() === ${JSON.stringify(label)})))`;
    await until(() => evaluate(`!!${expression} && !${expression}.disabled`), `enabled ${label}`);
    await evaluate(`${expression}.click()`); await delay(150);
  }
  capture = async name => {
    await evaluate('document.fonts.ready');
    const state = await evaluate(`({host:document.querySelector('.host-chip')?.textContent,text:document.body.innerText,buttons:[...document.querySelectorAll('button')].filter(b=>b.getClientRects().length).map(b=>({label:b.getAttribute('aria-label')||b.textContent.trim(),disabled:b.disabled})),calls:window.fixtureCalls})`);
    await writeFile(join(output, name+'.json'), JSON.stringify({ ...state, nativeCalls }, null, 2));
    const image = await call('Page.captureScreenshot', { format: 'png' });
    await writeFile(join(output, name+'.png'), Buffer.from(image.data,'base64'));
    console.log(name, JSON.stringify({host:state.host,buttons:state.buttons})); return state;
  };
  await click('Connect the AI'); await click('Connect the AI');
  await click('Designer');
  await click('Run tests');
  await until(() => evaluate('document.body.innerText.includes("1 of 1")'), 'baseline passing synthetic test');
  await capture('01-local-run');
  await click('Chat');
  async function message(text) {
    await evaluate(`(() => {const t = document.querySelector('.chat-composer textarea'); Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(t,${JSON.stringify(text)}); t.dispatchEvent(new Event('input',{bubbles:true}));})()`);
    await delay(100);
  }
  await message('Before choosing the failed context'); await click('Send message');
  await until(() => evaluate('document.body.innerText.includes("Synthetic local answer.")'), 'baseline synthetic chat');
  await capture('02-local-chat');
  await click('Agent'); await click('Clusters'); await click('Back to clusters'); await click('Add a cluster');
  await until(() => evaluate('document.querySelector("#cluster-context")?.options.length > 1'), 'real kubeconfig context');
  await capture('03-context-choice');
  await evaluate(`(() => {const s=document.querySelector('#cluster-context');s.value=s.options[1].value;s.dispatchEvent(new Event('change',{bubbles:true}));})()`);
  await until(() => evaluate('!!document.querySelector(".cluster-card [role=alert]")'), 'real failed context discovery');
  const failure = await capture('04-failed-context');
  if (!failure.host.includes('This computer') || !nativeCalls.some(c=>c.command==='cluster_discover' && !c.result.ok && c.result.error.check === 'clusterApi' && c.result.error.result === 'connectionFailed')) throw Error('Expected native connection failure with local host preserved');
  const count = command => evaluate(`window.fixtureCalls.filter(c=>c.command === ${JSON.stringify(command)}).length`);
  if (await count('agent_run') !== 1 || await count('agent_chat_send') !== 1) throw Error('Baseline must submit one Run and one Chat');
  await click('Designer');
  await click('Run tests');
  await until(async () => await count('agent_run') === 2, 'second local Run submitted');
  await until(() => evaluate('document.body.innerText.includes("1 of 1")'), 'passing synthetic test after failure');
  const localRun = await capture('05-local-run-after-failure');
  if (!localRun.host.includes('This computer') || !localRun.buttons.some(b=>b.label==='Run tests' && !b.disabled)) throw Error('Local Run unavailable after discovery failure');
  await click('Chat'); await message('After choosing the failed context');
  await click('Send message');
  await until(async () => await count('agent_chat_send') === 2, 'second local Chat submitted');
  await until(() => evaluate('document.body.innerText.includes("After choosing the failed context") && document.querySelectorAll(".chat-turn").length === 2'), 'second synthetic Chat turn');
  const localChat = await capture('06-local-chat-after-failure');
  if (!localChat.host.includes('This computer')) throw Error('Host switched away from This computer');
  if (await evaluate('window.fixtureCalls.some(c=>["send_start","setup_select_local","setup_change_local"].includes(c.command))')) throw Error('Unexpected remote Run or local reselection/reset');
  if (await count('agent_run') !== 2 || await count('agent_chat_send') !== 2) throw Error('Unexpected Run/Chat submission count');
  await writeFile(join(output,'result.json'),JSON.stringify({result:'local-preserved-after-failed-context',runs:2,chats:2,nativeCalls,limits:'Real Windows WebView2 and native context discovery. Local setup, Run and Chat IPC are synthetic; no VM or inference.'},null,2));
} catch (error) {
  if (capture) await capture('failure').catch(()=>{});
  await writeFile(join(output,'error.txt'),String(error)); throw error;
} finally { if (socket) socket.close(); child.kill(); }
