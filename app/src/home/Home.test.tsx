import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { SAMPLE_DOCUMENT_ID } from '../designer/agent-api';
import { conversationState, resetConversation, updateConversation } from './conversation-store';
import Home, { type NavigatorGate } from './Home';
import { NAVIGATOR_ID } from './navigator-api';
import tests from '../../src-tauri/example-agent/web-reader.agent.test.md?raw';

type Turn = { message: string; answer: string | null; phase: string; tools: { name: string; callId: string; phase: string }[]; contextRetained: boolean; terminationUnconfirmed: boolean; stopUnconfirmed: boolean; startedAt: number; finishedAt: number | null };
const turn = (message: string, answer: string | null, extra: Partial<Turn> = {}): Turn => ({ message, answer, phase: answer === null ? 'Running' : 'Succeeded', tools: [], contextRetained: false,
  terminationUnconfirmed: false, stopUnconfirmed: false, startedAt: 1_700_000_000_000, finishedAt: null, ...extra });
const card = (value: unknown) => '```navigator-card\n' + JSON.stringify(value) + '\n```';
const webReader = '---\nname: web-reader\ndescription: Answers from the web page named in the request\nmodel: gpt-5.6-terra\nkmx:\n  schemaVersion: 1\n  runtime: orka\n  tools: []\n---\nRead pages.';

// `refuse`: navigator_send throws this native error. `settle`: it is accepted, then ends with this error and no turn.
let nav: { turns: Turn[]; busy: boolean; state: string; error: string | null; cluster?: boolean; tooOld?: boolean; refuse?: string; settle?: string; denied?: string | null };
let task: { turns: Turn[]; busy: boolean };
const calls: { command: string; args: Record<string, unknown> }[] = [];
const navSnapshot = () => ({ chat: { documentId: NAVIGATOR_ID, state: nav.state, busy: nav.busy, canSend: !nav.busy, versions: [{ turns: nav.turns }], error: nav.error, denied: nav.denied ?? null, keptOnCluster: false, readOnly: false }, thinksWith: nav.cluster ? { kind: 'cluster', model: 'fixture-model' } : { kind: 'copilot', model: 'gpt-5.6-terra' }, workspaceTooOld: !!nav.tooOld });
beforeEach(() => {
  resetConversation(); calls.length = 0;
  nav = { turns: [], busy: false, state: 'ready', error: null }; task = { turns: [], busy: false };
  vi.stubGlobal('isTauri', true);
  mockIPC((command, raw) => {
    const args = (raw ?? {}) as Record<string, unknown>;
    calls.push({ command, args });
    switch (command) {
      case 'agents_list': return [{ id: SAMPLE_DOCUMENT_ID, name: 'web-reader', path: null, available: true }];
      case 'agent_open': return { documentId: SAMPLE_DOCUMENT_ID, name: 'web-reader', agent: webReader, tests, source: { agent: 'a'.repeat(64), tests: 'b'.repeat(64) } };
      case 'navigator_status': case 'navigator_reconnect': return navSnapshot();
      case 'navigator_send': {
        if (nav.refuse) throw nav.refuse;
        nav.cluster = 'targetId' in args;
        if (nav.settle) {
          const error = nav.settle; nav.busy = true; nav.state = 'preparing';
          const reply = navSnapshot();
          setTimeout(() => { nav.busy = false; nav.state = 'ready'; nav.error = error; nav.denied = error === 'permission' ? 'createAgents' : null; }, 0);
          return reply;
        }
        nav.turns.push(turn(String(args.message), null)); nav.busy = true; nav.state = 'working'; return navSnapshot();
      }
      case 'navigator_stop': nav.turns.at(-1)!.terminationUnconfirmed = true; nav.busy = false; nav.state = 'ready'; return navSnapshot();
      case 'navigator_new': return navSnapshot();
      case 'agent_chat_status': case 'agent_chat_new': return { documentId: args.documentId, state: task.busy ? 'working' : 'ready', busy: task.busy, canSend: !task.busy, versions: [{ turns: task.turns }], error: null, denied: null, keptOnCluster: false, readOnly: false };
      case 'agent_chat_send':
        task.turns.push(turn(String(args.message), null, { phase: 'Pending', startedAt: Date.now() })); task.busy = true;
        return { documentId: args.documentId, state: 'working', busy: true, canSend: false, versions: [{ turns: task.turns }], error: null, denied: null, keptOnCluster: false, readOnly: false };
      default: throw Error(`unexpected ${command}`);
    }
  });
});
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); });

const ready: NavigatorGate = { kind: 'ready' };
function home(gate: NavigatorGate = ready, extra: { blockedFor?: (id: string) => string | undefined } = {}) {
  const handlers = { onOpenPage: vi.fn(), onOpenAgent: vi.fn(), onAgentCreated: vi.fn(), onThisComputer: vi.fn(), onManageClusters: vi.fn() };
  render(<main aria-labelledby="page-title"><Home active revision={0} gate={gate} {...handlers} {...extra} /></main>);
  return handlers;
}

test('an empty conversation offers the message box and starters, and sends to the Navigator', async () => {
  const user = userEvent.setup(); home();
  expect(screen.getByRole('main', { name: 'What do you want done?' })).toBeTruthy();
  expect(screen.getByText('The Navigator explains Orka and sets things up with you. Nothing is created until you confirm a card.')).toBeTruthy();
  expect(await screen.findByText('gpt-5.6-terra', { exact: false })).toBeTruthy();
  expect(screen.getByRole('group', { name: 'Or start with' }).textContent).toContain('Explain Orka to me');
  await user.type(screen.getByRole('textbox', { name: 'Message' }), 'What is a Session?');
  await user.click(screen.getByRole('button', { name: 'Send' }));
  await waitFor(() => expect(calls.find(call => call.command === 'navigator_send')?.args).toEqual({ message: 'What is a Session?' }));
  expect(await screen.findByText('Thinking…')).toBeTruthy();
  // The page keeps its name once the headline gives way to the conversation.
  expect(screen.getByRole('main', { name: 'What do you want done?' })).toBeTruthy();
  // While it thinks, Send becomes Stop.
  await user.click(screen.getByRole('button', { name: 'Stop' }));
  expect(calls.some(call => call.command === 'navigator_stop')).toBe(true);
  expect(await screen.findByText('Stopped. Nothing was created or changed.')).toBeTruthy();
});

test('a starter sends its question', async () => {
  const user = userEvent.setup(); home();
  const starter = await screen.findByRole('button', { name: 'Explain Orka to me' });
  await waitFor(() => expect(starter.matches(':disabled')).toBe(false));
  await user.click(starter);
  await waitFor(() => expect(calls.find(call => call.command === 'navigator_send')?.args.message).toBe('I’m new to Orka. What is a Task, and how is it different from an agent?'));
});

test('a reply that failed says nothing changed and can be asked again', async () => {
  nav.turns = [turn('Hello', null, { phase: 'Failed' })];
  const user = userEvent.setup(); const { onOpenPage } = home();
  const failed = await screen.findByRole('article', { name: 'The Navigator couldn’t answer' });
  expect(within(failed).getByText('The model did not respond. Nothing was created or changed.')).toBeTruthy();
  await user.click(within(failed).getByRole('button', { name: 'Check the AI connection' }));
  expect(onOpenPage).toHaveBeenCalledWith('Clusters');
  await user.click(within(failed).getByRole('button', { name: 'Try again' }));
  await waitFor(() => expect(calls.find(call => call.command === 'navigator_send')?.args.message).toBe('Hello'));
});

test('a proposed task runs on the chosen saved agent only after Create task', async () => {
  const question = 'What does https://example.com say?';
  nav.turns = [turn(question, `That needs the live page.\n\n${card({ kind: 'task', agent: 'web-reader', task: question })}`)];
  const user = userEvent.setup(); home();
  const proposed = await screen.findByRole('article', { name: 'Proposed task' });
  expect(within(proposed).getByText('Not created')).toBeTruthy();
  expect(await within(proposed).findByText('the agent’s test: Copies the checksum from the page')).toBeTruthy();
  expect(within(proposed).getByText('Answers from the web page named in the request', { exact: false })).toBeTruthy();
  expect(calls.some(call => call.command === 'agent_chat_send')).toBe(false);
  await waitFor(() => expect(within(proposed).getByRole('button', { name: 'Create task' }).matches(':disabled')).toBe(false));
  await user.click(within(proposed).getByRole('button', { name: 'Create task' }));
  await waitFor(() => expect(calls.filter(call => ['agent_chat_new', 'agent_chat_send'].includes(call.command)).map(call => [call.command, call.args])).toEqual([
    ['agent_chat_new', { documentId: SAMPLE_DOCUMENT_ID }], ['agent_chat_send', { documentId: SAMPLE_DOCUMENT_ID, message: question }],
  ]));
  const running = await screen.findByRole('article', { name: 'Task web-reader: Running' });
  expect(within(running).getByRole('list', { name: 'Phases' }).textContent).toContain('Finalizing');
  expect(screen.getByText('Created. It is running on this computer.')).toBeTruthy();
});

test('Designer\'s reason an agent can\'t take a task holds the card', async () => {
  nav.turns = [turn('Q', card({ kind: 'task', agent: 'web-reader', task: 'Read it' }))];
  home(ready, { blockedFor: id => id === SAMPLE_DOCUMENT_ID ? 'Wait for your changes in Designer to save.' : undefined });
  const proposed = await screen.findByRole('article', { name: 'Proposed task' });
  expect(within(proposed).getByText('Wait for your changes in Designer to save.')).toBeTruthy();
  expect(within(proposed).getByRole('button', { name: 'Create task' }).matches(':disabled')).toBe(true);
});

test('Edit changes the proposal before anything is created', async () => {
  nav.turns = [turn('Q', card({ kind: 'task', agent: 'web-reader', task: 'Old task' }))];
  const user = userEvent.setup(); home();
  await user.click(await screen.findByRole('button', { name: 'Edit' }));
  const editing = screen.getByRole('article', { name: 'Proposed task' });
  expect(within(editing).getByText('Editing')).toBeTruthy();
  const field = within(editing).getByRole('textbox', { name: 'Task' });
  await user.clear(field); await user.type(field, 'New task');
  await user.click(within(editing).getByRole('button', { name: 'Save changes' }));
  expect(await screen.findByText('New task')).toBeTruthy();
  expect(calls.some(call => call.command === 'agent_chat_send')).toBe(false);
});

test('only the newest proposal can act; an older one stays Not created', async () => {
  nav.turns = [turn('A', card({ kind: 'task', agent: 'web-reader', task: 'First' })), turn('B', card({ kind: 'task', agent: 'web-reader', task: 'Second' }), { startedAt: 1_700_000_001_000 })];
  home();
  const cards = await screen.findAllByRole('article', { name: 'Proposed task' });
  expect(cards).toHaveLength(2);
  expect(within(cards[0]).queryByRole('button', { name: 'Create task' })).toBeNull();
  expect(within(cards[1]).getByRole('button', { name: 'Create task' })).toBeTruthy();
});

test('a block that fails validation is shown as text and offers nothing', async () => {
  nav.turns = [turn('Q', 'Here.\n\n```navigator-card\n{"kind":"monitor","repo":"x"}\n```')];
  home();
  expect(await screen.findByText('{"kind":"monitor","repo":"x"}', { exact: false })).toBeTruthy();
  expect(screen.queryByRole('article')).toBeNull();
});

test('an agent card writes an ordinary agent file only after Create agent', async () => {
  const draft = { kind: 'agent', name: 'issue-summary', description: 'Summarises new issues', instructions: 'Read each issue.', tools: ['web_fetch'] };
  nav.turns = [turn('Q', `A first draft.\n\n${card(draft)}`)];
  let created = '';
  mockIPC((command, raw) => {
    const args = (raw ?? {}) as Record<string, unknown>;
    calls.push({ command, args });
    if (command === 'navigator_status') return navSnapshot();
    if (command === 'agents_list') return [{ id: SAMPLE_DOCUMENT_ID, name: 'web-reader', path: null, available: true }];
    if (command === 'agent_create') return '1b2b93c4-375c-4815-9717-39852a96e412';
    if (command === 'agent_open') return { documentId: '1b2b93c4-375c-4815-9717-39852a96e412', name: 'issue-summary', agent: '---\nname: issue-summary\ndescription: ""\nmodel: gpt-5.6-terra\nkmx:\n  schemaVersion: 1\n  runtime: orka\n  tools: []\n---\n', tests: '---\nschemaVersion: 1\n---\n', source: { agent: 'a'.repeat(64), tests: 'b'.repeat(64) } };
    if (command === 'agent_save') { created = String(args.content); return { documentId: args.documentId, name: 'issue-summary', agent: created, tests: '---\nschemaVersion: 1\n---\n', source: { agent: 'c'.repeat(64), tests: 'b'.repeat(64) } }; }
    throw Error(`unexpected ${command}`);
  });
  const user = userEvent.setup(); const { onAgentCreated, onOpenAgent } = home();
  const proposed = await screen.findByRole('article', { name: 'Proposed agent' });
  expect(calls.some(call => call.command === 'agent_create')).toBe(false);
  await user.click(within(proposed).getByRole('button', { name: 'Create agent' }));
  const done = await screen.findByRole('article', { name: 'Agent issue-summary: Created' });
  expect(calls.find(call => call.command === 'agent_create')?.args).toEqual({ name: 'issue-summary' });
  expect(created).toContain('description: "Summarises new issues"');
  expect(created).toContain('Read each issue.');
  expect(created).toContain('web_fetch');
  expect(onAgentCreated).toHaveBeenCalled();
  await user.click(within(done).getByRole('button', { name: 'Open in Designer' }));
  expect(onOpenAgent).toHaveBeenCalledWith('1b2b93c4-375c-4815-9717-39852a96e412');
});

test('an explanation card can ask a follow-up or open a page', async () => {
  nav.turns = [turn('Q', `Terms.\n\n${card({ kind: 'explain', title: 'How it fits together', rows: [{ term: 'Agent', text: 'Does the work.' }, { term: 'Task', text: 'One piece of work.' }], actions: [{ label: 'Create my first task', ask: 'Help me create my first task.' }, { label: 'Show me my agents', open: 'Overview' }] })}`)];
  const user = userEvent.setup(); const { onOpenPage } = home();
  const explained = await screen.findByRole('article', { name: 'How it fits together' });
  await user.click(within(explained).getByRole('button', { name: 'Show me my agents' }));
  expect(onOpenPage).toHaveBeenCalledWith('Overview');
  await user.click(within(explained).getByRole('button', { name: 'Create my first task' }));
  await waitFor(() => expect(calls.find(call => call.command === 'navigator_send')?.args.message).toBe('Help me create my first task.'));
});

test.each<[NavigatorGate, string, string]>([
  [{ kind: 'noModel' }, 'Connect the AI first', 'Navigator · no model connected'],
  [{ kind: 'unreachable' }, 'Can’t reach the selected cluster. Check your network or VPN.', 'Navigator · can’t reach the cluster'],
  [{ kind: 'noConnection', name: 'team-dev' }, 'Choose an AI connection for team-dev.', 'Navigator · no AI connection chosen'],
  [{ kind: 'waiting' }, 'Waiting for the workspace to start.', 'Navigator · waiting for the workspace'],
  [{ kind: 'reconnect', name: 'team-dev', onReconnect: () => {} }, 'team-dev needs reconnecting.', 'Navigator · team-dev needs reconnecting'],
  [{ kind: 'reconnect', name: 'team-dev', onReconnect: () => {}, first: true }, 'team-dev isn’t connected yet.', 'Navigator · team-dev not connected'],
])('when it cannot answer (%o) the message box is disabled and a card says why', (gate, title, line) => {
  home(gate);
  expect(screen.getByRole('main', { name: 'What do you want done?' })).toBeTruthy();
  expect(screen.getByRole('heading', { level: 2, name: title })).toBeTruthy();
  expect(screen.getByText(line)).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Send' }).matches(':disabled')).toBe(true);
  expect(screen.queryByRole('textbox')).toBeNull();
  expect(calls.some(call => call.command.startsWith('navigator_'))).toBe(false);
});

const cluster = { name: 'team-dev', targetId: '00000000-0000-4000-8000-000000000001', providerId: '00000000-0000-4000-8000-000000000002' };
test('on a connected cluster the Navigator sends there and says where it runs', async () => {
  const user = userEvent.setup(); home({ kind: 'ready', place: cluster });
  expect(await screen.findByText('runs on team-dev', { exact: false })).toBeTruthy();
  await user.type(screen.getByRole('textbox', { name: 'Message' }), 'What is a Session?');
  await user.click(screen.getByRole('button', { name: 'Send' }));
  await waitFor(() => expect(calls.find(call => call.command === 'navigator_send')?.args).toEqual({ message: 'What is a Session?', targetId: cluster.targetId, providerId: cluster.providerId }));
  // Its answers come from the cluster's AI connection, named by the model it used.
  await waitFor(() => expect(document.querySelector('.nav-line')?.textContent).toBe('Navigator · runs on team-dev · thinks with fixture-model'));
});

test('on a cluster a proposed task runs there, while the agent card still writes the file here', async () => {
  nav.turns = [turn('Check a page', 'Here is a task.\n\n' + card({ kind: 'task', agent: 'web-reader', task: 'Read https://example.com' }))];
  const user = userEvent.setup(); home({ kind: 'ready', place: cluster });
  const proposed = await screen.findByRole('article', { name: 'Proposed task' });
  expect(within(proposed).getByText('Runs on').nextElementSibling?.textContent).toBe('team-dev');
  const create = within(proposed).getByRole('button', { name: 'Create task' });
  await waitFor(() => expect(create.matches(':disabled')).toBe(false));
  await user.click(create);
  await waitFor(() => expect(calls.find(call => call.command === 'agent_chat_send')?.args).toEqual({ documentId: SAMPLE_DOCUMENT_ID, message: 'Read https://example.com', targetId: cluster.targetId, providerId: cluster.providerId }));
  expect(await screen.findByText('Created. It is running on team-dev.')).toBeTruthy();
});

test('a cluster that needs reconnecting offers Reconnect and Open Clusters, not a network warning', async () => {
  const onReconnect = vi.fn(); const user = userEvent.setup(); const { onManageClusters } = home({ kind: 'reconnect', name: 'team-dev', onReconnect });
  expect(screen.queryByText(/network or VPN/)).toBeNull();
  expect(screen.queryByRole('button', { name: 'Switch to This computer' })).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Reconnect' }));
  await user.click(screen.getByRole('button', { name: 'Open Clusters' }));
  expect(onReconnect).toHaveBeenCalledOnce();
  expect(onManageClusters).toHaveBeenCalledOnce();
});
test('an unreachable cluster offers This computer', async () => {
  const user = userEvent.setup(); const { onThisComputer, onOpenPage } = home({ kind: 'unreachable' });
  await user.click(screen.getByRole('button', { name: 'Switch to This computer' }));
  await user.click(screen.getByRole('button', { name: 'Check again' }));
  expect(onThisComputer).toHaveBeenCalledOnce();
  expect(onOpenPage).toHaveBeenCalledWith('Clusters');
});

test('a conversation forgotten by a replaced workspace starts over instead of hiding new turns', async () => {
  updateConversation(() => ({ startVersion: 3, cards: { old: { status: 'dismissed' } } }));
  nav.turns = [turn('After the replacement', 'A new answer.')];
  home();
  expect(await screen.findByText('A new answer.')).toBeTruthy();
  expect(conversationState()).toEqual({ startVersion: 0, cards: {} });
});

// A message that fails, for any reason, stays on screen with why; nothing silently resets to the start.
const failures: { kind: string; refuse?: string; settle?: string; says: string }[] = [
  { kind: 'connection', refuse: 'connection', says: 'The Navigator couldn’t be reached. Nothing was created or changed. Try again.' },
  { kind: 'lost workspace', settle: 'workspaceUnavailable', says: 'The workspace on this computer stopped answering. Quit Bridge and open it again to restart the workspace. Your agents and files are kept.' },
  { kind: 'permission', settle: 'permission', says: 'Your access to this cluster doesn’t allow “create agents” in its namespace. Ask the cluster’s administrator to add it.' },
  { kind: 'model', refuse: 'model', says: 'Connect the AI in Clusters, then try again.' },
  { kind: 'unsupported', settle: 'workspaceOutdated', says: 'The workspace on this computer is older than this Bridge and can’t run the Navigator yet. Nothing was sent to it.' },
  { kind: 'storage', refuse: 'source', says: 'Bridge could not prepare the Navigator. Try again.' },
];
for (const failure of failures) test(`a ${failure.kind} failure keeps the message and says why`, async () => {
  nav.refuse = failure.refuse; nav.settle = failure.settle;
  const user = userEvent.setup(); home();
  const field = await screen.findByRole('textbox', { name: 'Message' });
  await screen.findByText('gpt-5.6-terra', { exact: false });
  await user.type(field, 'whats going on');
  await user.click(screen.getByRole('button', { name: 'Send' }));
  const turn = await screen.findByRole('article', { name: 'Not sent' });
  expect(within(turn).getByRole('alert').textContent).toBe(`Not sent.${failure.says}`);
  expect(screen.getByText('whats going on', { selector: '.nav-user' })).toBeTruthy();
  expect((screen.getByRole('textbox', { name: 'Message' }) as HTMLTextAreaElement).value).toBe('whats going on');
  // Home did not go back to its start.
  expect(document.querySelector('.home-start')).toBeNull();
});

test('an uncertain send keeps the message waiting and says to check again', async () => {
  nav.refuse = 'not a native error';
  const user = userEvent.setup(); home();
  await user.type(await screen.findByRole('textbox', { name: 'Message' }), 'whats going on');
  await user.click(screen.getByRole('button', { name: 'Send' }));
  expect((await screen.findByRole('alert')).textContent).toBe('The Navigator’s reply could not be confirmed. Reconnect to check again without sending it again.');
  expect(screen.getByText('whats going on', { selector: '.nav-user' })).toBeTruthy();
});

test('a starter that fails behaves exactly as typing its text and sending', async () => {
  nav.refuse = 'connection';
  const user = userEvent.setup(); home();
  const starter = await screen.findByRole('button', { name: 'Create a task' });
  await waitFor(() => expect(starter.matches(':disabled')).toBe(false));
  await user.click(starter);
  expect(await screen.findByRole('article', { name: 'Not sent' })).toBeTruthy();
  expect(screen.getByText('I want to give one of my agents a task.', { selector: '.nav-user' })).toBeTruthy();
  expect((screen.getByRole('textbox', { name: 'Message' }) as HTMLTextAreaElement).value).toBe('I want to give one of my agents a task.');
  nav.refuse = undefined;
  await user.click(screen.getByRole('button', { name: 'Try again' }));
  await waitFor(() => expect(calls.filter(call => call.command === 'navigator_send').map(call => call.args.message)).toEqual(['I want to give one of my agents a task.', 'I want to give one of my agents a task.']));
  expect(screen.queryByRole('article', { name: 'Not sent' })).toBeNull();
});

test('a workspace too old for the Navigator says so before anything is sent, and lists the routes that work', async () => {
  nav.tooOld = true;
  const user = userEvent.setup(); const choose = vi.fn();
  const handlers = { onOpenPage: vi.fn(), onOpenAgent: vi.fn(), onAgentCreated: vi.fn(), onThisComputer: vi.fn() };
  render(<main aria-labelledby="page-title"><Home active revision={0} gate={ready} {...handlers} switchTo={{ name: 'team-dev', choose }} /></main>);
  expect(await screen.findByRole('heading', { name: 'The workspace on this computer is older than this Bridge.' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Send' }).matches(':disabled')).toBe(true);
  for (const starter of within(screen.getByRole('group', { name: 'Or start with' })).getAllByRole('button')) expect(starter.matches(':disabled')).toBe(true);
  expect(screen.queryByRole('textbox', { name: 'Message' })).toBeNull();
  expect(screen.queryByText(/Start a fresh workspace/)).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Switch to team-dev' }));
  await user.click(screen.getByRole('button', { name: 'Open Designer' }));
  expect(choose).toHaveBeenCalledOnce();
  expect(handlers.onOpenPage).toHaveBeenCalledWith('Designer');
  expect(calls.some(call => call.command === 'navigator_send')).toBe(false);
});

test('each place has its own conversation: switching shows that place\'s turns and sends there', async () => {
  const places: Record<string, Turn[]> = { '': [turn('Here first', 'An answer here.')], [cluster.targetId]: [] };
  mockIPC((command, raw) => {
    const args = (raw ?? {}) as Record<string, unknown>;
    calls.push({ command, args });
    const key = typeof args.targetId === 'string' ? args.targetId : '';
    const reply = () => ({ chat: { documentId: NAVIGATOR_ID, state: 'ready', busy: false, canSend: true, versions: [{ turns: places[key] }], error: null, denied: null, keptOnCluster: false, readOnly: false },
      thinksWith: !key ? { kind: 'copilot', model: 'gpt-5.6-terra' } : places[key].length ? { kind: 'cluster', model: 'fixture-model' } : null, workspaceTooOld: false });
    if (command === 'navigator_send') places[key].push(turn(String(args.message), 'Answered on ' + (key ? 'team-dev.' : 'this computer.')));
    if (command.startsWith('navigator_')) return reply();
    if (command === 'agents_list') return [];
    throw Error(`unexpected ${command}`);
  });
  const user = userEvent.setup();
  const handlers = { onOpenPage: vi.fn(), onOpenAgent: vi.fn(), onAgentCreated: vi.fn(), onThisComputer: vi.fn() };
  const view = (gate: NavigatorGate) => <main aria-labelledby="page-title"><Home active revision={0} gate={gate} {...handlers} /></main>;
  const { rerender } = render(view(ready));
  expect(await screen.findByText('An answer here.')).toBeTruthy();
  await waitFor(() => expect(document.querySelector('.nav-line')?.textContent).toBe('Navigator · runs on this computer · thinks with GitHub Copilot · gpt-5.6-terra'));
  rerender(view({ kind: 'ready', place: cluster }));
  expect(await screen.findByRole('heading', { name: 'What do you want done?' })).toBeTruthy();
  expect(screen.queryByText('An answer here.')).toBeNull();
  expect(document.querySelector('.nav-line')?.textContent).toBe('Navigator · runs on team-dev');
  await user.type(screen.getByRole('textbox', { name: 'Message' }), 'On the cluster');
  await waitFor(() => expect(screen.getByRole('button', { name: 'Send' }).matches(':disabled')).toBe(false));
  await user.click(screen.getByRole('button', { name: 'Send' }));
  expect(await screen.findByText('Answered on team-dev.')).toBeTruthy();
  expect(calls.find(call => call.command === 'navigator_send')?.args).toEqual({ message: 'On the cluster', targetId: cluster.targetId, providerId: cluster.providerId });
  rerender(view(ready));
  expect(await screen.findByText('An answer here.')).toBeTruthy();
  expect(screen.queryByText('Answered on team-dev.')).toBeNull();
  await user.type(screen.getByRole('textbox', { name: 'Message' }), 'Back here');
  await waitFor(() => expect(screen.getByRole('button', { name: 'Send' }).matches(':disabled')).toBe(false));
  await user.click(screen.getByRole('button', { name: 'Send' }));
  expect(await screen.findByText('Answered on this computer.')).toBeTruthy();
  expect(calls.filter(call => call.command === 'navigator_send').at(-1)?.args).toEqual({ message: 'Back here' });
  expect(places[cluster.targetId].map(t => t.message)).toEqual(['On the cluster']);
  expect(calls.some(call => call.command === 'navigator_new')).toBe(false);
});

test('New is final for the place: reconnecting it in this run does not bring cleared turns back', async () => {
  nav.turns = [turn('Hello', 'An earlier answer.')];
  const handlers = { onOpenPage: vi.fn(), onOpenAgent: vi.fn(), onAgentCreated: vi.fn(), onThisComputer: vi.fn() };
  const gate = (targetId: string, providerId: string): NavigatorGate => ({ kind: 'ready', place: { name: 'team-dev', targetId, providerId, conversation: '["team-dev","control","orka","agents","fixture"]' } });
  const user = userEvent.setup();
  const view = render(<main aria-labelledby="page-title"><Home active revision={0} gate={gate(cluster.targetId, cluster.providerId)} {...handlers} /></main>);
  expect(await screen.findByText('An earlier answer.')).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'New conversation' }));
  await waitFor(() => expect(screen.queryByText('An earlier answer.')).toBeNull());
  // Reconnecting mints new opaque ids for the same place and AI connection; native keeps its turns.
  view.rerender(<main aria-labelledby="page-title"><Home active revision={0} gate={gate('00000000-0000-4000-8000-0000000000a1', '00000000-0000-4000-8000-0000000000a2')} {...handlers} /></main>);
  await waitFor(() => expect(calls.filter(call => call.command === 'navigator_status').length).toBeGreaterThan(2));
  expect(screen.queryByText('An earlier answer.')).toBeNull();
  expect(screen.getByRole('heading', { level: 1, name: 'What do you want done?' })).toBeTruthy();
});

test('Home says conversations are kept only while Bridge is open', () => {
  home();
  expect(screen.getByText('Conversations are kept only while Bridge is open.')).toBeTruthy();
});
