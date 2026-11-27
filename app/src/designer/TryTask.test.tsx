import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import TryTask, { duration } from './TryTask';
import { readFileSync } from 'node:fs';
import type { ChatSnapshot, ChatTurn } from './chat-api';
const id = '9b2b93c4-375c-4815-9717-39852a96e411';
const idle: ChatSnapshot = { documentId: id, state: 'ready', busy: false, canSend: true, versions: [], error: null, denied: null, keptOnCluster: false };
const turn: ChatTurn = { message: 'Hello', answer: 'Hello back', phase: 'Succeeded', tools: [], contextRetained: true, terminationUnconfirmed: false, stopUnconfirmed: false, startedAt: 1_700_000_000_000, finishedAt: 1_700_000_012_000 };
let snapshot: ChatSnapshot; let calls: { command: string; args: Record<string, unknown> }[];
let send: (() => unknown) | undefined; let renew: (() => unknown) | undefined;
beforeEach(() => {
  vi.useFakeTimers(); vi.stubGlobal('isTauri', true); snapshot = structuredClone(idle); calls = []; send = undefined; renew = undefined;
  mockIPC((command, args) => {
    calls.push({ command, args: args as Record<string, unknown> });
    if (command === 'agent_chat_send') return send ? send() : snapshot;
    if (command === 'agent_chat_new') { if (renew) return renew(); snapshot = { ...snapshot, state: 'ready', canSend: true, error: null }; }
    return structuredClone(snapshot);
  });
});
afterEach(() => { cleanup(); clearMocks(); vi.useRealTimers(); vi.unstubAllGlobals(); });
const element = (props: { blocked?: string; active?: boolean; model?: string; documentId?: string } = {}) => <TryTask documentId={props.documentId ?? id} agentName="web-reader" {...props} />;
async function open(blocked?: string, model?: string) { const result = render(element({ blocked, model })); await settle(); return result; }
async function settle() { for (let i = 0; i < 4; i++) await act(async () => {}); }
async function poll() { await act(async () => { await vi.advanceTimersByTimeAsync(500); }); }
function field() { return screen.getByRole('textbox', { name: 'Give web-reader one task' }) as HTMLTextAreaElement; }
const runButton = () => screen.getByRole('button', { name: 'Run task' });
async function enter(value = 'Hello') { fireEvent.change(field(), { target: { value } }); field().focus(); fireEvent.keyDown(field(), { key: 'Enter' }); await settle(); }
const sends = () => calls.filter(c => c.command === 'agent_chat_send');
const commands = () => calls.map(c => c.command).filter(c => c !== 'agent_chat_status');
const result = () => screen.getByRole('article', { name: /^Task result/ });

test('names this computer and keeps earlier tasks folded, newest first, without numbering conversations', async () => {
  snapshot = { ...idle, versions: [{ turns: [{ ...turn, message: 'First task' }] }, { turns: [{ ...turn, message: 'Second task' }] }, { turns: [{ ...turn, message: 'Latest task' }] }] };
  await open();
  expect(screen.getByRole('region', { name: 'Try a task on this computer' })).toBeTruthy();
  expect(within(result()).getByText('Latest task')).toBeTruthy();
  expect(screen.queryByText(/Conversation \d/)).toBeNull();
  const earlier = screen.getByText('Earlier tasks: 2').closest('details')!;
  expect(earlier.open).toBe(false);
  const tasks = within(earlier).getAllByRole('article', { name: /^Earlier task/, hidden: true }).map(card => card.querySelector('.try-result-task')?.textContent);
  expect(tasks).toEqual(['Second task', 'First task']);
});
test('About tasks here says, folded, what is kept, that each task starts fresh, what Stop can promise and the way out', async () => {
  await open();
  const about = screen.getByText('About tasks here').closest('details')!;
  expect(about.open).toBe(false);
  expect(about.closest('.try-task')).toBe(screen.getByRole('region', { name: 'Try a task on this computer' }));
  expect([...about.querySelectorAll('p')].map(p => p.textContent)).toEqual([
    'Earlier tasks are kept only while Bridge is open.',
    'Each task starts fresh and doesn’t remember earlier ones.',
    'Stop asks the task to stop. A tool that already started may finish, and what it did isn’t undone.',
    'If a task can’t be stopped, restart the workspace.',
  ]);
  // One task is one new conversation, so the conversation's message limit never applies here.
  expect(about.textContent).not.toMatch(/\d/);
  fireEvent.click(within(about).getByRole('button', { name: 'About tasks here' }));
  expect(about.open).toBe(true);
});
test('an empty Try a task shows its labelled composer and where the result will appear, without routine status clutter', async () => {
  await open();
  expect(field().placeholder).toBe('Describe the task');
  expect(screen.getByText('No tasks run yet. The result shows here: what it used, what it answered, and how the task ended.')).toBeTruthy();
  expect(screen.queryByText('Ready for a task.')).toBeNull();
  expect(screen.queryByRole('button', { name: /Stop|Reconnect|Run again|Save as a test/ })).toBeNull();
  expect(screen.getByText('Runs once on this computer. It does not change test results.')).toBeTruthy();
  expect(field().getAttribute('aria-describedby')).toBeTruthy();
  expect(document.getElementById(field().getAttribute('aria-describedby')!)?.textContent).toContain('Enter runs the task · Shift+Enter for a new line');
  expect(runButton().closest('form')).toBe(field().closest('form'));
  expect(runButton().hasAttribute('disabled')).toBe(true);
  expect(screen.queryByText(/^Earlier tasks:/)).toBeNull();
});
test('one task is one fresh conversation: Enter starts clean, then runs exactly once, and the appended turn consumes the composer', async () => {
  send = () => (snapshot = { ...idle, state: 'preparing', busy: true, canSend: false });
  await open(); await enter(); fireEvent.keyDown(field(), { key: 'Enter' }); await settle();
  expect(commands()).toEqual(['agent_chat_new', 'agent_chat_send']);
  expect(sends()).toEqual([{ command: 'agent_chat_send', args: { documentId: id, message: 'Hello' } }]);
  expect(field().value).toBe('Hello'); expect(document.activeElement).toBe(field());
  snapshot = { ...idle, versions: [{ turns: [turn] }] }; await poll();
  expect(field().value).toBe(''); expect(screen.getByText('Hello back')).toBeTruthy();
});
test('every task, including Run again, starts a new conversation with no context from the last one', async () => {
  snapshot = { ...idle, versions: [{ turns: [turn] }] };
  send = () => (snapshot = { ...snapshot, state: 'preparing', busy: true, canSend: false });
  await open();
  fireEvent.click(screen.getByRole('button', { name: 'Run again' })); await settle();
  expect(commands()).toEqual(['agent_chat_new', 'agent_chat_send']);
  expect(sends()[0].args).toEqual({ documentId: id, message: 'Hello' });
  snapshot = { ...idle, versions: [{ turns: [turn] }, { turns: [turn] }] }; await poll();
  await enter('Another task');
  expect(commands()).toEqual(['agent_chat_new', 'agent_chat_send', 'agent_chat_new', 'agent_chat_send']);
});
test('a refused new conversation, such as a held earlier task, sends nothing and keeps the task', async () => {
  renew = () => Promise.reject('busy');
  await open(); await enter('Keep me');
  expect(sends()).toEqual([]); expect(field().value).toBe('Keep me');
  expect(screen.getByRole('alert').textContent).toBe('The workspace is busy. Wait for the current test or task, then try again.');
});
test('a held, unconfirmed task offers Reconnect and Stop instead of a new task', async () => {
  snapshot = { ...idle, state: 'observationLost', canSend: false, versions: [{ turns: [{ ...turn, answer: null, phase: 'Running', finishedAt: null }] }] };
  await open(); fireEvent.change(field(), { target: { value: 'Next' } });
  expect(runButton().hasAttribute('disabled')).toBe(true);
  expect(screen.getByText('Check the last task first: reconnect or stop it.')).toBeTruthy();
  expect(within(result()).getByText('Result unknown')).toBeTruthy();
  fireEvent.click(within(result()).getByRole('button', { name: 'Reconnect' })); await settle();
  expect(commands()).toEqual(['agent_chat_reconnect']);
  expect(within(result()).getByRole('button', { name: 'Stop' })).toBeTruthy();
});
test('a newer composer draft survives acceptance of the earlier task', async () => {
  send = () => (snapshot = { ...idle, state: 'preparing', busy: true, canSend: false });
  await open(); await enter();
  fireEvent.change(field(), { target: { value: 'Next task' } });
  snapshot = { ...idle, versions: [{ turns: [turn] }] }; await poll(); expect(field().value).toBe('Next task');
});
test.each(['busy', 'source', 'model'])('native %s rejection keeps the task and permits retry', async error => {
  send = () => Promise.reject(error); await open(); await enter();
  expect(field().value).toBe('Hello'); expect(screen.getByRole('alert')).toBeTruthy();
  expect(runButton().hasAttribute('disabled')).toBe(false);
});
test('an uncertain run keeps acknowledgement and cannot run twice before status shows its turn', async () => {
  send = () => Promise.reject('unknown transport error'); await open(); await enter();
  expect(screen.getByRole('alert').closest('form')).toBe(field().closest('form'));
  expect(screen.getByRole('status').textContent).toBe('Checking whether your task was accepted. Do not run it again.');
  await poll(); expect(field().value).toBe('Hello');
  expect(runButton().hasAttribute('disabled')).toBe(true);
  fireEvent.keyDown(field(), { key: 'Enter' }); await settle(); expect(sends()).toHaveLength(1);
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' })); await settle();
  expect(calls.filter(c => c.command === 'agent_chat_reconnect')).toHaveLength(0);
  snapshot = { ...idle, versions: [{ turns: [turn] }] }; await poll();
  expect(field().value).toBe(''); expect(screen.getByText('Hello back')).toBeTruthy(); expect(sends()).toHaveLength(1);
  expect(screen.queryByRole('alert')).toBeNull();
});
test('an unconfirmed result releases an uncertain run so a new task can start, without losing the draft', async () => {
  send = () => Promise.reject('unknown transport error'); await open(); await enter();
  snapshot = { ...idle, state: 'contextUnavailable', canSend: false, error: 'response' }; await poll();
  expect(field().value).toBe('Hello');
  expect(runButton().hasAttribute('disabled')).toBe(false);
  expect(screen.getByRole('alert').textContent).toBe('The task could not be read. Reconnect to check what happened.');
  send = () => (snapshot = { ...idle, state: 'preparing', busy: true, canSend: false });
  fireEvent.click(runButton()); await settle();
  expect(commands().slice(-2)).toEqual(['agent_chat_new', 'agent_chat_send']);
});
test('automatic acknowledgement clears only the uncertain run warning', async () => {
  send = () => Promise.reject('unknown transport error'); await open(); await enter();
  expect(screen.getByRole('alert')).toBeTruthy();
  snapshot = { ...idle, versions: [{ turns: [turn] }] }; await poll();
  expect(field().value).toBe(''); expect(screen.queryByRole('alert')).toBeNull();
});
test('an uncertain run can reconcile an observed preflight without a turn while keeping the task', async () => {
  send = () => Promise.reject('unknown transport error'); await open(); await enter();
  snapshot = { ...idle, state: 'preparing', busy: true, canSend: false }; await poll();
  snapshot = { ...idle, error: 'model' }; await poll();
  expect(field().value).toBe('Hello'); expect(runButton().hasAttribute('disabled')).toBe(false);
});
test('a pre-turn observation loss keeps acknowledgement through native reconnect', async () => {
  send = () => Promise.reject('unknown transport error'); await open(); await enter();
  snapshot = { ...idle, state: 'preparing', busy: true, canSend: false }; await poll();
  snapshot = { ...idle, state: 'observationLost', busy: false, canSend: false }; await poll();
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' })); await settle();
  expect(calls.filter(c => c.command === 'agent_chat_reconnect')).toHaveLength(1);
  snapshot = { ...idle, versions: [{ turns: [turn] }] }; await poll(); expect(field().value).toBe('');
});
test('ready-state connection preflight failure offers a safe retry, not an impossible reconnect', async () => {
  snapshot = { ...idle, error: 'connection' }; await open();
  expect(screen.queryByText(/do not run the task again/i)).toBeNull();
  expect(screen.getByRole('alert').textContent).toContain('try running the task again');
  expect(screen.queryByRole('button', { name: 'Reconnect' })).toBeNull();
});
test('background status polling does not immediately erase an actionable admission error', async () => {
  send = () => Promise.reject('busy'); await open(); await enter();
  await poll(); expect(screen.getByRole('alert').textContent).toContain('busy'); expect(field().value).toBe('Hello');
});
test('asynchronous preflight failure before a turn keeps the task', async () => {
  send = () => (snapshot = { ...idle, state: 'preparing', busy: true, canSend: false });
  await open(); await enter();
  snapshot = { ...idle, error: 'model' }; await poll();
  expect(field().value).toBe('Hello'); expect(screen.getByText(/Connect the AI/)).toBeTruthy();
  expect(sends()).toHaveLength(1);
});
test('identical earlier task text is not mistaken for the newly admitted turn', async () => {
  snapshot = { ...idle, versions: [{ turns: [turn] }] };
  send = () => (snapshot = { ...snapshot, state: 'preparing', busy: true, canSend: false });
  await open(); await enter(); await poll(); expect(field().value).toBe('Hello');
  snapshot = { ...idle, versions: [{ turns: [turn] }, { turns: [turn] }] }; await poll(); expect(field().value).toBe('');
});
test('Shift Enter and IME composition do not run or prevent normal editing', async () => {
  await open(); fireEvent.change(field(), { target: { value: 'Lines' } });
  expect(fireEvent.keyDown(field(), { key: 'Enter', shiftKey: true })).toBe(true);
  fireEvent.compositionStart(field()); fireEvent.keyDown(field(), { key: 'Enter' }); fireEvent.compositionEnd(field());
  fireEvent.keyDown(field(), { key: 'Enter', isComposing: true });
  fireEvent.keyDown(field(), { key: 'Enter', keyCode: 229 }); await settle(); expect(commands()).toEqual([]);
});
test('a blocked reason prevents every command while keeping the composer', async () => {
  const view = await open('Wait for your changes to save.'); await enter();
  expect(commands()).toEqual([]); expect(field().value).toBe('Hello');
  expect(screen.getByText('Wait for your changes to save.')).toBeTruthy();
  expect(runButton().getAttribute('title')).toBe('Wait for your changes to save.');
  view.rerender(element()); await settle();
  fireEvent.keyDown(field(), { key: 'Enter' }); await settle(); expect(sends()).toHaveLength(1);
});
test('remount restores the result by status without running again', async () => {
  snapshot = { ...idle, versions: [{ turns: [turn] }] };
  const view = await open(); view.unmount(); await open();
  expect(screen.getByText('Hello back')).toBeTruthy(); expect(commands()).toEqual([]);
});
test('renders untrusted task, answer and tool names as literal text, with tool rows but no call ids', async () => {
  snapshot = { ...idle, versions: [{ turns: [{ ...turn, message: '<img src=x onerror=alert(1)>', answer: '<script>secret()</script>', tools: [{ name: '<iframe>', callId: 'private-id', phase: 'ToolCallCompleted' }] }] }] };
  const view = await open();
  expect(screen.getByText('<script>secret()</script>')).toBeTruthy(); expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeTruthy();
  expect(view.container.querySelector('img,script,iframe')).toBeNull();
  expect(screen.queryByText('private-id')).toBeNull();
  const tools = screen.getByRole('list', { name: 'Tools used' });
  expect(tools.closest('details')).toBeNull();
  expect(within(tools).getByText('<iframe>')).toBeTruthy();
});
test('tool rows name each call once with its latest state, the tool id in mono', async () => {
  snapshot = { ...idle, versions: [{ turns: [{ ...turn, tools: [
    { name: 'web_fetch', callId: 'a', phase: 'ToolCallStarted' }, { name: 'web_search', callId: 'b', phase: 'ToolCallStarted' },
    { name: 'web_fetch', callId: 'a', phase: 'ToolCallCompleted' }, { name: 'web_search', callId: 'b', phase: 'ToolCallFailed' },
  ] }] }] };
  await open();
  const section = screen.getByRole('region', { name: 'Used tools' });
  const rows = within(section).getAllByRole('listitem');
  expect(rows.map(row => row.textContent)).toEqual(['Read a web page web_fetchDone', 'Search the web web_searchFailed']);
  expect(rows[0].querySelector('code')?.textContent).toBe('web_fetch');
  expect(rows[0].querySelector('.status-passed')).toBeTruthy(); expect(rows[1].querySelector('.status-failed')).toBeTruthy();
});
test('one tool call reads as Used a tool, and a call still open when the task settled shows no result', async () => {
  snapshot = { ...idle, versions: [{ turns: [{ ...turn, tools: [{ name: 'custom_tool', callId: 'a', phase: 'ToolCallStarted' }] }] }] };
  await open();
  const section = screen.getByRole('region', { name: 'Used a tool' });
  expect(within(section).getByRole('listitem').textContent).toBe('custom_toolNo result');
});
test('a finished task shows how long it took and the model it ran with, and a running one shows neither', async () => {
  send = () => (snapshot = { ...idle, state: 'working', busy: true, canSend: false, versions: [{ turns: [{ ...turn, answer: null, phase: 'Running', finishedAt: null }] }] });
  await open(undefined, 'gpt-5.6-terra'); await enter();
  expect(within(result()).queryByText(/^Took/)).toBeNull();
  snapshot = { ...idle, versions: [{ turns: [turn] }] }; await poll();
  const meta = result().querySelector('.try-result-meta')!;
  expect(meta.textContent).toBe('Took 12 s · Thinks with GPT-5.6 Terra');
  expect(meta.querySelector('code')?.textContent).toBe('GPT-5.6 Terra');
});
test.each([[400, 'under a second'], [12_400, '12 s'], [60_000, '1 min'], [125_000, '2 min 5 s']])('duration %i ms reads %s', (ms, text) => {
  expect(duration(ms)).toBe(text);
});
test('a succeeded task shows its phases done, the answer in reading type and Run again, but no Save as a test', async () => {
  snapshot = { ...idle, versions: [{ turns: [turn] }] }; await open();
  expect(within(result()).getByText('Succeeded', { selector: '.pill' })).toBeTruthy();
  const phases = within(result()).getByRole('list', { name: 'Phases' });
  expect(within(phases).getAllByRole('listitem').map(item => item.textContent)).toEqual(['Pending', 'Scheduled', 'Running', 'Finalizing', 'Succeeded']);
  expect(phases.querySelectorAll('.status-passed')).toHaveLength(5);
  expect(screen.getByText('Hello back').classList.contains('reading')).toBe(true);
  expect(within(result()).getByRole('button', { name: 'Run again' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: /Save as a test/ })).toBeNull();
});
test('read-only reconnect never runs an observation-lost task again', async () => {
  snapshot = { ...idle, state: 'observationLost', canSend: false, versions: [{ turns: [{ ...turn, answer: null, finishedAt: null }] }] };
  await open(); fireEvent.click(screen.getByRole('button', { name: 'Reconnect' })); await settle();
  expect(calls.some(c => c.command === 'agent_chat_reconnect')).toBe(true); expect(sends()).toEqual([]);
  expect(screen.getAllByText(/connection was lost/i).length).toBeGreaterThan(0);
  expect(screen.queryByRole('button', { name: 'Run again' })).toBeNull();
});
test.each([
  ['requestingStop', 'Requesting stop…'],
  ['stopRequested', 'Stop requested. Execution termination is unconfirmed.'],
  ['stopUnconfirmed', 'Stop request unconfirmed. Execution may still be running.'],
] as const)('shows %s distinctly, never a Stopped guarantee', async (state, text) => {
  snapshot = { ...idle, state, canSend: false }; await open();
  expect(screen.getByText(text)).toBeTruthy(); expect(screen.queryByText(/^Stopped$/)).toBeNull();
});
test('Stop dispatches only stop, and its notice stays on that task after another task runs', async () => {
  snapshot = { ...idle, state: 'working', busy: true, canSend: false, versions: [{ turns: [{ ...turn, answer: null, phase: 'Running', finishedAt: null }] }] };
  await open();
  expect(within(result()).getByText('Running', { selector: '.pill' })).toBeTruthy();
  fireEvent.click(within(result()).getByRole('button', { name: 'Stop' })); await settle();
  expect(commands()).toEqual(['agent_chat_stop']);
  snapshot = { ...idle, versions: [{ turns: [{ ...turn, answer: null, phase: 'Running', finishedAt: null, terminationUnconfirmed: true, contextRetained: false }] }] }; await poll();
  expect(within(result()).getByText('Stop requested', { selector: '.pill' })).toBeTruthy();
  expect(within(result()).getByText('Stop requested. Execution termination is unconfirmed.')).toBeTruthy();
  expect(within(result()).queryByText(/^Took/)).toBeNull();
  snapshot = { ...idle, versions: [snapshot.versions[0], { turns: [{ ...turn, message: 'After Stop' }] }] }; await poll();
  const earlier = screen.getByText('Earlier tasks: 1').closest('details')!;
  expect(within(earlier).getByText('Stop requested. Execution termination is unconfirmed.')).toBeTruthy();
});
test('after Run task settles, focus returns to the task field', async () => {
  send = () => (snapshot = { ...idle, state: 'preparing', busy: true, canSend: false });
  await open(); fireEvent.change(field(), { target: { value: 'Hello' } });
  runButton().focus(); fireEvent.click(runButton()); await settle();
  expect(commands()).toEqual(['agent_chat_new', 'agent_chat_send']);
  expect(runButton().hasAttribute('disabled')).toBe(true);
  expect(document.activeElement).toBe(field());
});
test('after Run again settles, focus returns to the task field', async () => {
  snapshot = { ...idle, versions: [{ turns: [turn] }] };
  send = () => (snapshot = { ...snapshot, state: 'preparing', busy: true, canSend: false });
  await open();
  const again = screen.getByRole('button', { name: 'Run again' }); again.focus(); fireEvent.click(again); await settle();
  expect(document.activeElement).toBe(field());
});
test.each(['Stop', 'Reconnect'])('after %s settles, focus returns to the task field', async name => {
  snapshot = { ...idle, state: name === 'Stop' ? 'working' : 'observationLost', busy: name === 'Stop', canSend: false, versions: [{ turns: [{ ...turn, answer: null, phase: 'Running', finishedAt: null }] }] };
  await open();
  const button = within(result()).getByRole('button', { name }); button.focus(); fireEvent.click(button); await settle();
  expect(commands()).toEqual([name === 'Stop' ? 'agent_chat_stop' : 'agent_chat_reconnect']);
  expect(document.activeElement).toBe(field());
});
test('a settling command does not take focus back from a control the person moved to', async () => {
  snapshot = { ...idle, state: 'working', busy: true, canSend: false, versions: [{ turns: [{ ...turn, answer: null, phase: 'Running', finishedAt: null }] }] };
  let reply!: (value: ChatSnapshot) => void;
  mockIPC(command => {
    calls.push({ command, args: {} });
    if (command === 'agent_chat_stop') return new Promise(resolve => { reply = resolve; });
    return structuredClone(snapshot);
  });
  render(<>{element()}<button type="button">Elsewhere</button></>); await settle();
  const stop = within(result()).getByRole('button', { name: 'Stop' }); stop.focus(); fireEvent.click(stop); await settle();
  const elsewhere = screen.getByRole('button', { name: 'Elsewhere' }); elsewhere.focus();
  await act(async () => { reply({ ...snapshot, state: 'stopRequested' }); }); await settle();
  expect(document.activeElement).toBe(elsewhere);
});
test.each(['Pending', 'Queued', 'Scheduled', ''])('a working snapshot with phase %s shows waiting to start', async phase => {
  snapshot = { ...idle, state: 'working', busy: true, canSend: false, versions: [{ turns: [{ ...turn, answer: null, phase, finishedAt: null }] }] };
  await open(); expect(screen.getByRole('status').textContent).toBe('Waiting to start…');
  expect(result().querySelector('.try-phases .status-running')).toBeTruthy();
});
test.each(['Pending', 'Queued', 'Scheduled', 'Running', 'Finalizing'])('a pending %s answer reads as waiting, not missing', async phase => {
  snapshot = { ...idle, state: 'working', busy: true, canSend: false, versions: [{ turns: [{ ...turn, answer: null, contextRetained: false, phase, finishedAt: null }] }] };
  await open();
  expect(within(result()).queryAllByText(/No answer (observed|was received)/)).toHaveLength(0);
  expect(within(result()).getByText(['Running', 'Finalizing'].includes(phase) ? 'Waiting for your agent’s reply…' : 'Waiting for your agent to start…')).toBeTruthy();
});
test('a newly admitted task stays waiting while the saved agent is preparing', async () => {
  snapshot = { ...idle, state: 'preparing', busy: true, canSend: false, versions: [{ turns: [{ ...turn, answer: null, contextRetained: false, phase: '', finishedAt: null }] }] };
  await open();
  expect(within(result()).queryAllByText(/No answer was received/)).toHaveLength(0);
  expect(within(result()).getByText('Waiting for your agent to start…')).toBeTruthy();
});
test.each([
  ['waitingForSession', null], ['waitingForSession', 'An already observed answer'],
  ['working', null], ['working', 'An already observed answer'],
] as const)('%s confirmation keeps answer %s until the task settles', async (state, answer) => {
  snapshot = { ...idle, state, busy: true, canSend: false, versions: [{ turns: [{ ...turn, answer, contextRetained: false }] }] };
  await open();
  expect(within(result()).queryByText('No answer was received.')).toBeNull();
  if (answer) expect(within(result()).getByText(answer)).toBeTruthy();
  else expect(within(result()).getByText(state === 'waitingForSession' ? 'Waiting for the task to finish saving…' : 'Confirming your agent’s reply…')).toBeTruthy();
  snapshot = { ...snapshot, state: 'ready', busy: false, canSend: true };
  await poll();
  if (!answer) expect(within(result()).getByText('No answer was received.')).toBeTruthy();
});
test.each(['Failed', 'Cancelled'])('session confirmation keeps %s and stop warnings', async phase => {
  snapshot = { ...idle, state: 'waitingForSession', busy: true, canSend: false, versions: [{ turns: [
    { ...turn, answer: null, phase, contextRetained: false, terminationUnconfirmed: true },
  ] }] };
  await open();
  expect(within(result()).getByText(phase, { selector: '.pill' })).toBeTruthy();
  if (phase === 'Failed') expect(screen.getByText('This task failed.')).toBeTruthy();
  expect(screen.getByText('Stop requested. Execution termination is unconfirmed.')).toBeTruthy();
  expect(screen.queryByText(/failed without an answer|No answer was received/)).toBeNull();
});
test('a failed task says what happened first, then what is unchanged, in a failed alert', async () => {
  snapshot = { ...idle, versions: [{ turns: [{ ...turn, phase: 'Failed', answer: null }] }] }; await open();
  const alert = screen.getByText('This task failed.').closest('.alert')!;
  expect(alert.classList.contains('alert-failed')).toBe(true);
  expect(alert.textContent).toBe('This task failed.Your agent file and test results are unchanged.');
  expect(within(result()).getByText('Failed', { selector: '.pill' }).classList.contains('status-failed')).toBe(true);
  expect(screen.getByText('The task failed without an answer.')).toBeTruthy();
});
test('composer grows with text and shrinks again after acceptance', async () => {
  await open();
  Object.defineProperty(field(), 'scrollHeight', { configurable: true, value: 144 });
  fireEvent.change(field(), { target: { value: 'A long task' } });
  expect(field().style.height).toBe('144px');
  send = () => (snapshot = { ...idle, versions: [{ turns: [{ ...turn, message: 'A long task' }] }] });
  await enter('A long task');
  expect(field().value).toBe('');
  expect(field().style.height).toBe('auto');
});
test('composer remeasures an unchanged draft when window wrapping changes', async () => {
  await open();
  let height = 80;
  Object.defineProperty(field(), 'scrollHeight', { configurable: true, get: () => height });
  fireEvent.change(field(), { target: { value: 'A draft that wraps differently after resizing' } });
  expect(field().style.height).toBe('80px');
  height = 180; fireEvent(window, new Event('resize'));
  expect(field().style.height).toBe('180px');
  height = 80; fireEvent(window, new Event('resize'));
  expect(field().style.height).toBe('80px');
});
test('composer waits for activation to remeasure and removes its resize listener on unmount', async () => {
  const view = await open();
  let height = 80; const measure = vi.fn(() => height);
  Object.defineProperty(field(), 'scrollHeight', { configurable: true, get: measure });
  fireEvent.change(field(), { target: { value: 'Keep this draft' } });
  view.rerender(element({ active: false })); measure.mockClear();
  height = 180; fireEvent(window, new Event('resize'));
  expect(measure).not.toHaveBeenCalled();
  view.rerender(element({ active: true }));
  expect(field().style.height).toBe('180px');
  view.unmount(); measure.mockClear(); fireEvent(window, new Event('resize'));
  expect(measure).not.toHaveBeenCalled();
});
test('a settled Finalizing task does not look active and says its result is unknown', async () => {
  snapshot = { ...idle, versions: [{ turns: [{ ...turn, phase: 'Finalizing', answer: null, contextRetained: false, finishedAt: null }] }] };
  await open();
  expect(screen.getByText('No answer was received.')).toBeTruthy();
  expect(within(result()).getByText('Result unknown', { selector: '.pill' })).toBeTruthy();
  expect(result().querySelector('.try-phases .status-running')).toBeNull();
});
test('a failed task with an observed answer does not look like a successful one', async () => {
  snapshot = { ...idle, versions: [{ turns: [{ ...turn, phase: 'Failed' }] }] };
  await open(); expect(screen.getByText('This task failed.')).toBeTruthy(); expect(screen.getByText('Hello back')).toBeTruthy();
});
test('an uncertain Stop reply does not forget an admitted task awaiting its turn', async () => {
  send = () => (snapshot = { ...idle, state: 'preparing', busy: true, canSend: false });
  mockIPC(command => {
    calls.push({ command, args: {} });
    if (command === 'agent_chat_stop') return Promise.reject('unknown transport error');
    if (command === 'agent_chat_send') return send!();
    return snapshot;
  });
  await open(); await enter(); expect(field().value).toBe('Hello');
  fireEvent.click(screen.getByRole('button', { name: 'Stop' })); await settle();
  snapshot = { ...idle, versions: [{ turns: [turn] }] }; await poll();
  expect(field().value).toBe(''); expect(screen.getByText('Hello back')).toBeTruthy();
});
test('unsupported input requests keep Stop without inventing approvals', async () => {
  snapshot = { ...idle, state: 'waitingForInput', busy: true, canSend: false, versions: [{ turns: [{ ...turn, answer: null, phase: 'WaitingForInput', finishedAt: null }] }] }; await open();
  expect(screen.getAllByText(/input that Try a task does not support/).length).toBeGreaterThan(0);
  expect(within(result()).getByText('Needs you', { selector: '.pill' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Stop' })).toBeTruthy(); expect(screen.queryByRole('button', { name: /approve/i })).toBeNull();
});
test('a long tool trace stays bounded while every call remains available on demand', async () => {
  const tools = Array.from({ length: 50 }, (_, index) => ({ name: `tool-${index}`, callId: `call-${index}`, phase: 'ToolCallCompleted' }));
  snapshot = { ...idle, versions: [{ turns: [{ ...turn, tools }] }] }; await open();
  const activity = screen.getByRole('list', { name: 'Tools used' });
  expect(within(activity).getAllByRole('listitem')).toHaveLength(3);
  expect(within(activity).getByText('tool-49')).toBeTruthy();
  const more = screen.getByText('47 more tool calls').closest('details')!;
  expect(more.open).toBe(false);
  expect(within(more).getAllByRole('listitem', { hidden: true })).toHaveLength(47);
  expect(within(more).getByText('tool-0')).toBeTruthy();
  expect(screen.getByText('Hello back').closest('details')).toBeNull();
  fireEvent.click(more.querySelector('summary')!); expect(more.open).toBe(true);
});
test('state announcement stays the same when only polled tools change', async () => {
  snapshot = { ...idle, state: 'working', busy: true, canSend: false, versions: [{ turns: [{ ...turn, phase: 'Running' }] }] };
  await open(); const announcement = screen.getByRole('status'); const before = announcement.textContent;
  snapshot.versions[0].turns[0].tools.push({ name: 'tool', phase: 'ToolCallStarted', callId: '1' }); await poll();
  expect(announcement.textContent).toBe(before);
});
test('an older status reply cannot roll back a newer mutation', async () => {
  let finish!: (value: unknown) => void; let polls = 0;
  mockIPC(command => {
    if (command === 'agent_chat_status' && ++polls === 2) return new Promise(resolve => { finish = resolve; });
    if (command === 'agent_chat_send') return { ...idle, state: 'working', busy: true, canSend: false, versions: [{ turns: [turn] }] };
    return idle;
  });
  await open(); await poll(); await enter();
  await act(async () => finish(idle)); expect(screen.getByText('Hello back')).toBeTruthy();
});
test('pending runs suppress polling until the command settles', async () => {
  let rejectSend!: (error: string) => void; let reads = 0;
  mockIPC(command => {
    if (command === 'agent_chat_status') reads++;
    if (command === 'agent_chat_send') return new Promise((_resolve, reject) => { rejectSend = reject; });
    return structuredClone(snapshot);
  });
  await open(); expect(reads).toBe(1); await enter();
  await poll(); await poll(); expect(reads).toBe(1);
  await act(async () => rejectSend('unknown transport error')); await poll();
  expect(reads).toBe(2);
});
test('switching agent ignores a late command from the previous agent', async () => {
  let finish!: (value: unknown) => void; send = () => new Promise(resolve => { finish = resolve; });
  const view = await open(); await enter();
  const other = 'a7f34ba2-b9bd-4b55-b69d-f0a84a1bf528'; snapshot = { ...idle, documentId: other };
  view.rerender(<TryTask key={other} documentId={other} agentName="web-reader" />); await settle();
  await act(async () => finish({ ...idle, versions: [{ turns: [turn] }] }));
  expect(screen.queryByText('Hello back')).toBeNull(); expect(field().value).toBe('');
});
test('polling is single-flight even when the previous read has not replied', async () => {
  let finish!: (value: unknown) => void; let reads = 0;
  mockIPC(() => { reads++; return new Promise(resolve => { finish = resolve; }); });
  render(element()); await poll(); await poll(); expect(reads).toBe(1);
  await act(async () => finish(idle)); await poll(); expect(reads).toBe(2);
});
test('the empty live region stays rendered before its first status update', async () => {
  snapshot = { ...idle, versions: [{ turns: [turn] }] };
  await open();
  const style = document.createElement('style');
  style.textContent = readFileSync('src/app.css', 'utf8'); document.head.append(style);
  try { expect(getComputedStyle(screen.getByRole('status', { hidden: true })).display).not.toBe('none'); }
  finally { style.remove(); }
});
test('completion stays announced to screen readers without routine visible Ready copy', async () => {
  snapshot = { ...idle, state: 'working', busy: true, canSend: false, versions: [{ turns: [{ ...turn, answer: null, phase: 'Running', finishedAt: null }] }] };
  await open(); expect(screen.getByRole('status').textContent).toContain('working');
  snapshot = { ...idle, versions: [{ turns: [turn] }] }; await poll();
  const status = screen.getByRole('status');
  expect(status.textContent).toBe('Ready for a task.');
  expect(status.classList.contains('sr-only')).toBe(true);
});
const blockedReason = 'Wait for your changes to save.';
test('a blocked idle Try a task does not claim Ready and disables Run again with the exact reason', async () => {
  snapshot = { ...idle, versions: [{ turns: [turn] }] };
  await open(blockedReason);
  // The hint gives the reason; the status line does not add a second message.
  expect(screen.getByRole('status').textContent).toBe('');
  expect(screen.getByText(blockedReason)).toBeTruthy();
  const again = screen.getByRole('button', { name: 'Run again' });
  expect(again.hasAttribute('disabled')).toBe(true); expect(again.getAttribute('title')).toBe(blockedReason);
  fireEvent.click(again); await settle();
  expect(commands()).toEqual([]);
});
test('blocked unconfirmed history shows the blocker and starts nothing', async () => {
  snapshot = { ...idle, state: 'contextUnavailable', error: 'identity' };
  await open(blockedReason); await enter();
  expect(screen.getByText(blockedReason)).toBeTruthy();
  expect(screen.getByRole('alert').textContent).toBe('The task could not be verified. You can run a new task.');
  expect(commands()).toEqual([]);
});
test('unblocked unconfirmed history lets the next task start clean', async () => {
  snapshot = { ...idle, state: 'contextUnavailable', canSend: false, error: 'identity', versions: [{ turns: [turn] }] };
  send = () => (snapshot = { ...snapshot, state: 'preparing', busy: true, canSend: false });
  await open(); await enter('Next');
  expect(commands()).toEqual(['agent_chat_new', 'agent_chat_send']);
  expect(screen.getByText('Hello back')).toBeTruthy();
});
test('a blocked running task keeps its progress and Stop', async () => {
  snapshot = { ...idle, state: 'working', busy: true, canSend: false, versions: [{ turns: [{ ...turn, answer: null, phase: 'Running', finishedAt: null }] }] };
  await open(blockedReason);
  expect(screen.getByRole('status').textContent).toBe('Your agent is working…');
  fireEvent.click(screen.getByRole('button', { name: 'Stop' })); await settle();
  expect(commands()).toEqual(['agent_chat_stop']);
});
test('a blocked observation loss keeps its warning and Reconnect', async () => {
  snapshot = { ...idle, state: 'observationLost', canSend: false, versions: [{ turns: [{ ...turn, answer: null, finishedAt: null }] }] };
  await open(blockedReason);
  expect(screen.getByRole('status').textContent).toMatch(/connection was lost/i);
  fireEvent.click(screen.getByRole('button', { name: 'Reconnect' })); await settle();
  expect(commands()).toEqual(['agent_chat_reconnect']);
});
test('a blocked parent keeps an uncertain run status instead of replacing it', async () => {
  send = () => Promise.reject('unknown transport error'); const view = await open(); await enter();
  view.rerender(element({ blocked: blockedReason })); await settle();
  expect(screen.getByRole('status').textContent).toBe('Checking whether your task was accepted. Do not run it again.');
  expect(screen.getByRole('button', { name: 'Reconnect' })).toBeTruthy();
});
test('two Try a task views of the same agent keep their own field and hint ids', async () => {
  render(<>{element()}{element()}</>); await settle();
  const fields = screen.getAllByRole('textbox', { name: 'Give web-reader one task' });
  expect(fields).toHaveLength(2);
  expect(new Set(fields.map(field => field.id)).size).toBe(2);
  expect(new Set(fields.map(field => field.getAttribute('aria-describedby'))).size).toBe(2);
  for (const field of fields) expect(document.getElementById(field.getAttribute('aria-describedby')!)?.textContent).toBeTruthy();
});
const place = { name: 'team-dev', targetId: '00000000-0000-4000-8000-000000000001', providerId: '00000000-0000-4000-8000-000000000002' };
test('on a cluster a task is sent there by its ids and the composer says where it runs', async () => {
  render(<TryTask documentId={id} agentName="web-reader" place={place} />); await settle();
  expect(screen.getByRole('region', { name: 'Try a task on team-dev' })).toBeTruthy();
  expect(screen.getByText('Runs once on team-dev. It does not change test results.')).toBeTruthy();
  send = () => (snapshot = { ...idle, state: 'preparing', busy: true, canSend: false });
  await enter('Read it');
  expect(sends().map(c => c.args)).toEqual([{ documentId: id, message: 'Read it', targetId: place.targetId, providerId: place.providerId }]);
  // Stop and New act on the conversation where it started: they name no place.
  expect(calls.find(c => c.command === 'agent_chat_new')?.args).toEqual({ documentId: id });
});
test.each([
  [{ error: 'permission', denied: 'deleteTasks' }, 'Your access to this cluster doesn’t allow “delete tasks” in its namespace. Ask the cluster’s administrator to add it.'],
  [{ error: 'sessionsUnsupported', denied: null }, 'This cluster’s Orka is too old for Try a task. It needs Orka v0.4.0 or later.'],
  [{ error: 'clusterUnavailable', denied: null }, 'Bridge couldn’t reach the selected cluster. Reconnect it in Clusters, then try again.'],
] as const)('a cluster refusal %j is one precise message', async (extra, message) => {
  snapshot = { ...idle, state: 'contextUnavailable', canSend: false, ...extra };
  render(<TryTask documentId={id} agentName="web-reader" place={place} />); await settle();
  expect(screen.getByRole('alert').textContent).toBe(message);
  expect(screen.getByRole('status').classList.contains('sr-only')).toBe(true);
});
test('a cluster that kept earlier tasks says so once, and About names what Quit leaves', async () => {
  snapshot = { ...idle, keptOnCluster: true, versions: [{ turns: [turn] }] };
  render(<TryTask documentId={id} agentName="web-reader" place={place} />); await settle();
  expect(screen.getAllByText('Earlier tasks stay on team-dev: your access there doesn’t allow Bridge to delete them.')).toHaveLength(1);
  const about = screen.getByText('About tasks here').closest('details')!;
  expect(about.textContent).toContain('Quitting Bridge leaves the last task there.');
});
