import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { SAMPLE_DOCUMENT_ID } from '../designer/agent-api';
import { resetConversation } from './conversation-store';
import Home, { type NavigatorGate } from './Home';
import { NAVIGATOR_ID } from './navigator-api';
import tests from '../../src-tauri/example-agent/web-reader.agent.test.md?raw';

type Turn = { message: string; answer: string | null; phase: string; tools: { name: string; callId: string; phase: string }[]; contextRetained: boolean; terminationUnconfirmed: boolean; stopUnconfirmed: boolean; startedAt: number; finishedAt: number | null };
const turn = (message: string, answer: string | null, extra: Partial<Turn> = {}): Turn => ({ message, answer, phase: answer === null ? 'Running' : 'Succeeded', tools: [], contextRetained: false,
  terminationUnconfirmed: false, stopUnconfirmed: false, startedAt: 1_700_000_000_000, finishedAt: null, ...extra });
const card = (value: unknown) => '```navigator-card\n' + JSON.stringify(value) + '\n```';
const webReader = '---\nname: web-reader\ndescription: Answers from the web page named in the request\nmodel: gpt-5.6-terra\nkmx:\n  schemaVersion: 1\n  runtime: orka\n  tools: []\n---\nRead pages.';

let nav: { turns: Turn[]; busy: boolean; state: string; error: string | null };
let task: { turns: Turn[]; busy: boolean };
const calls: { command: string; args: Record<string, unknown> }[] = [];
const navSnapshot = () => ({ chat: { documentId: NAVIGATOR_ID, state: nav.state, busy: nav.busy, canSend: !nav.busy, versions: [{ turns: nav.turns }], error: nav.error }, thinksWith: { kind: 'copilot', model: 'gpt-5.6-terra' } });
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
      case 'navigator_send': nav.turns.push(turn(String(args.message), null)); nav.busy = true; nav.state = 'working'; return navSnapshot();
      case 'navigator_stop': nav.turns.at(-1)!.terminationUnconfirmed = true; nav.busy = false; nav.state = 'ready'; return navSnapshot();
      case 'navigator_new': return navSnapshot();
      case 'agent_chat_status': case 'agent_chat_new': return { documentId: args.documentId, state: task.busy ? 'working' : 'ready', busy: task.busy, canSend: !task.busy, versions: [{ turns: task.turns }], error: null };
      case 'agent_chat_send':
        task.turns.push(turn(String(args.message), null, { phase: 'Pending', startedAt: Date.now() })); task.busy = true;
        return { documentId: args.documentId, state: 'working', busy: true, canSend: false, versions: [{ turns: task.turns }], error: null };
      default: throw Error(`unexpected ${command}`);
    }
  });
});
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); });

const ready: NavigatorGate = { kind: 'ready' };
function home(gate: NavigatorGate = ready, extra: { blockedFor?: (id: string) => string | undefined } = {}) {
  const handlers = { onOpenPage: vi.fn(), onOpenAgent: vi.fn(), onAgentCreated: vi.fn(), onThisComputer: vi.fn() };
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
  [{ kind: 'elsewhere', name: 'team-dev' }, 'The Navigator works with This computer for now.', 'Navigator · works with This computer for now'],
  [{ kind: 'waiting' }, 'Waiting for the workspace to start.', 'Navigator · waiting for the workspace'],
])('when it cannot answer (%o) the message box is disabled and a card says why', (gate, title, line) => {
  home(gate);
  expect(screen.getByRole('main', { name: 'What do you want done?' })).toBeTruthy();
  expect(screen.getByRole('heading', { level: 2, name: title })).toBeTruthy();
  expect(screen.getByText(line)).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Send' }).matches(':disabled')).toBe(true);
  expect(screen.queryByRole('textbox')).toBeNull();
  expect(calls.some(call => call.command.startsWith('navigator_'))).toBe(false);
});

test('an unreachable cluster offers This computer', async () => {
  const user = userEvent.setup(); const { onThisComputer, onOpenPage } = home({ kind: 'unreachable' });
  await user.click(screen.getByRole('button', { name: 'Switch to This computer' }));
  await user.click(screen.getByRole('button', { name: 'Check again' }));
  expect(onThisComputer).toHaveBeenCalledOnce();
  expect(onOpenPage).toHaveBeenCalledWith('Clusters');
});
