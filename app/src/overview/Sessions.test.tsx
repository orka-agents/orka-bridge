import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { StrictMode } from 'react';
import { afterEach, expect, test, vi } from 'vitest';
import type { Places } from './api';
import { isSessions, isTranscript, merge, type PlaceSessions, roleLabel } from './sessions-api';
import { SessionList } from './Sessions';

const place = (key: string, context: string) => ({ key, context, service: 'control/orka', namespace: 'agents', stale: false, observedAt: 1000, agents: [] });
const places: Places = { places: [place('one', 'team-one'), place('two', 'team-two')], historyState: 'complete', unobserved: [] };
const entry = (name: string, over: object = {}) => ({ name, sessionType: 'chat', messageCount: 2, createdAt: 1000, updatedAt: 2000, activeTask: null, agent: null, ...over });
const read = (key: string, entries: unknown[], over: object = {}) => ({ key, state: 'complete', observedAt: 5000, entries, connection: 1, unsupported: 0, ...over });
function ipc(sessions: unknown, transcript: (args: Record<string, unknown>) => unknown = () => ({ state: 'complete', messages: [] })) {
  vi.stubGlobal('isTauri', true);
  const calls: [string, unknown][] = [];
  mockIPC((command, args) => {
    calls.push([command, args]);
    return command === 'overview_sessions' ? sessions : transcript(args as Record<string, unknown>);
  });
  return calls;
}
const noop = () => {};
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); });

test('sessions from every connected place merge most recently updated first', () => {
  const rows = merge([
    read('one', [entry('a', { updatedAt: 50 }), entry('b', { updatedAt: 10 })]),
    read('two', [entry('c', { updatedAt: 30 }), entry('a', { updatedAt: 50 })]),
  ] as PlaceSessions[]);
  expect(rows.map(r => `${r.placeKey}/${r.name}`)).toEqual(['one/a', 'two/a', 'two/c', 'one/b']);
});

test('native payloads are accepted only when each state carries what it promises', () => {
  expect(isSessions({ places: [read('one', [entry('a', { activeTask: 'run-1', agent: 'reader' })]), read('two', [], { state: 'denied', observedAt: null })] })).toBe(true);
  for (const bad of [
    { places: [read('one', []), read('one', [])] },
    { places: [read('one', [entry('a')], { state: 'unavailable', observedAt: null })] },
    { places: [read('one', [], { observedAt: null })] },
    { places: [read('one', [entry('a'), entry('a')])] },
    { places: [read('one', [entry('a', { agent: 'reader' })])] },
    { places: [read('one', [entry('a', { messageCount: -1 })])] },
    { places: [read('one', Array.from({ length: 51 }, (_, i) => entry(`s${i}`)))] },
    { places: [read('one', [], { state: 'denied', observedAt: null, unsupported: 1 })] },
    { places: [read('one', [], { unsupported: -1 })] },
  ]) expect(isSessions(bad)).toBe(false);
  expect(isTranscript({ state: 'partial', messages: [{ role: 'user', text: 'hi', cut: false, tools: [], at: 1 }] })).toBe(true);
  expect(isTranscript({ state: 'denied', messages: [{ role: 'user', text: 'hi', cut: false, tools: [], at: 1 }] })).toBe(false);
  expect(isTranscript({ state: 'complete', messages: [{ role: '', text: 'hi', cut: false, tools: [], at: 1 }] })).toBe(false);
});

test('each conversation names its place and counts, and a running Task names its agent', async () => {
  ipc({ places: [
    read('one', [entry('bridge-chat-1', { updatedAt: 3000, activeTask: 'run-1', agent: 'reader', messageCount: 1 })]),
    read('two', [entry('older', { updatedAt: 2500 }), entry('unread', { updatedAt: 2000, activeTask: 'run-2' })]),
  ] });
  render(<SessionList onCheck={noop} active places={places} />);
  const [running, older, unread] = await screen.findAllByRole('listitem');
  expect(running.getAttribute('aria-label')).toBe('bridge-chat-1 on team-one, 1 message');
  expect(within(running).getByText(/Running/)).toBeTruthy();
  expect(within(running).getByText('run-1')).toBeTruthy();
  expect(within(running).getByText('reader')).toBeTruthy();
  expect(older.getAttribute('aria-label')).toBe('older on team-two, 2 messages');
  expect(within(older).queryByText(/Running|Active Task/)).toBeNull();
  // A Task Bridge could not confirm is named without claiming it runs.
  expect(within(unread).getByText(/Active Task/)).toBeTruthy();
  expect(within(unread).queryByText(/Running/)).toBeNull();
  expect(screen.getByText(/Telegram, Teams or other channels/)).toBeTruthy();
});

test('opening a conversation reads only that Session and shows its messages', async () => {
  const calls = ipc({ places: [read('one', [entry('bridge-chat-1')])] }, () => ({ state: 'partial', messages: [
    { role: 'user', text: 'What changed?', cut: false, tools: [], at: 1000 },
    { role: 'assistant', text: 'Two files.', cut: true, tools: ['web_fetch'], at: null },
  ] }));
  render(<SessionList onCheck={noop} active places={places} />);
  await screen.findByRole('listitem');
  expect(calls.some(c => c[0] === 'overview_transcript')).toBe(false);
  await userEvent.click(screen.getByRole('button', { name: /Conversation/ }));
  expect(await screen.findByText('What changed?')).toBeTruthy();
  expect(screen.getByText('Two files.…')).toBeTruthy();
  expect(screen.getByText('web_fetch')).toBeTruthy();
  expect(screen.getByText(/Showing the latest 2 messages/)).toBeTruthy();
  expect(calls.filter(c => c[0] === 'overview_transcript').map(c => c[1])).toEqual([{ key: 'one', name: 'bridge-chat-1' }]);
});

test('a refused or unreadable place says so and keeps the other places listed', async () => {
  ipc({ places: [read('one', [], { state: 'denied', observedAt: null }), read('two', [entry('kept')], { state: 'partial' })] }, () => ({ state: 'denied', messages: [] }));
  render(<SessionList onCheck={noop} active places={places} />);
  expect(await screen.findByText(/Can't list conversations on team-one/)).toBeTruthy();
  expect(screen.getByText(/team-two has more conversations/)).toBeTruthy();
  await userEvent.click(screen.getByRole('button', { name: /Conversation/ }));
  expect(await screen.findByText("This account can't read this conversation.")).toBeTruthy();
});

test('a revoked place asks to connect again, and its conversations read the same way', async () => {
  ipc({ places: [read('one', [], { state: 'revoked', observedAt: null }), read('two', [entry('kept')])] }, () => ({ state: 'revoked', messages: [] }));
  render(<SessionList onCheck={noop} active places={places} />);
  expect(await screen.findByText('Connect to team-one again to see its conversations.')).toBeTruthy();
  await userEvent.click(screen.getByRole('button', { name: /Conversation/ }));
  expect(await screen.findByText('Connect to this cluster again to read the conversation.')).toBeTruthy();
});

test('without a ready workspace or a connected cluster the section says where conversations come from', async () => {
  ipc({ places: [read('one', [], { state: 'disconnected', observedAt: null })] });
  render(<SessionList onCheck={noop} active places={places} />);
  await waitFor(() => expect(screen.getByText("Conversations appear here once the workspace is ready on this computer or a cluster is connected.")).toBeTruthy());
});

test("this computer's conversations list beside the clusters' and open the same way", async () => {
  const calls = ipc({ places: [read('local', [entry('bridge-chat-local', { updatedAt: 4000 })], { connection: 7 }), read('one', [entry('remote')])] },
    () => ({ state: 'complete', messages: [{ role: 'user', text: 'hello here', cut: false, tools: [], at: 1 }] }));
  render(<SessionList onCheck={noop} active places={places} />);
  const [local, remote] = await screen.findAllByRole('listitem');
  expect(local.getAttribute('aria-label')).toBe('bridge-chat-local on this computer, 2 messages');
  expect(remote.getAttribute('aria-label')).toBe('remote on team-one, 2 messages');
  await userEvent.click(within(local).getByRole('button', { name: /Conversation/ }));
  expect(await within(local).findByText('hello here')).toBeTruthy();
  expect(calls.find(([c]) => c === 'overview_transcript')?.[1]).toEqual({ key: 'local', name: 'bridge-chat-local' });
});

test("an older workspace image says it can't list or read this computer's conversations", async () => {
  ipc({ places: [read('local', [], { state: 'outdated', observedAt: null }), read('one', [entry('remote')])] }, () => ({ state: 'outdated', messages: [] }));
  render(<SessionList onCheck={noop} active places={places} />);
  expect(await screen.findByText("Listing conversations on this computer needs a newer version of Bridge.")).toBeTruthy();
  await userEvent.click(within(await screen.findByRole('listitem')).getByRole('button', { name: /Conversation/ }));
  expect(await screen.findByText("Reading conversations on this computer needs a newer version of Bridge.")).toBeTruthy();
  expect(isSessions({ places: [read('local', [entry('a')], { state: 'outdated', observedAt: null })] })).toBe(false);
});

test('the connect prompt waits for a read; until then the section says it is reading', async () => {
  let answer: (v: unknown) => void = () => {};
  ipc(undefined);
  mockIPC(command => command === 'overview_sessions' ? new Promise(resolve => { answer = resolve; }) : null);
  render(<SessionList onCheck={noop} active places={places} />);
  expect(await screen.findByText('Reading conversations…')).toBeTruthy();
  expect(screen.queryByText('Conversations appear here once the workspace is ready on this computer or a cluster is connected.')).toBeNull();
  answer({ places: [read('one', [], { state: 'disconnected', observedAt: null })] });
  expect(await screen.findByText('Conversations appear here once the workspace is ready on this computer or a cluster is connected.')).toBeTruthy();
  expect(screen.queryByText('Reading conversations…')).toBeNull();
});

test('a new connection drops the old connection\'s conversations even when its first read fails', async () => {
  let sessions: unknown = { places: [read('one', [entry('old')])] };
  ipc(undefined);
  mockIPC(command => command === 'overview_sessions' ? sessions : null);
  const view = render(<SessionList onCheck={noop} active places={places} />);
  expect(await screen.findAllByRole('listitem')).toHaveLength(1);
  sessions = { places: [read('one', [], { state: 'unavailable', observedAt: null, connection: 2 })] };
  view.rerender(<SessionList onCheck={noop} active places={{ ...places }} />);
  expect(await screen.findByText("Couldn't read conversations on team-one just now.")).toBeTruthy();
  expect(screen.queryAllByRole('listitem')).toHaveLength(0);
});

test('an unreadable place on the same connection keeps what Bridge last saw', async () => {
  let sessions: unknown = { places: [read('one', [entry('kept')])] };
  ipc(undefined);
  mockIPC(command => command === 'overview_sessions' ? sessions : null);
  const view = render(<SessionList onCheck={noop} active places={places} />);
  expect(await screen.findAllByRole('listitem')).toHaveLength(1);
  sessions = { places: [read('one', [], { state: 'unavailable', observedAt: null })] };
  view.rerender(<SessionList onCheck={noop} active places={{ ...places }} />);
  expect(await screen.findByText(/Showing what Bridge last saw/)).toBeTruthy();
  expect(screen.getAllByRole('listitem')).toHaveLength(1);
});

test("this computer keeps what Bridge last saw while its workspace is busy or stopped on the same boot", async () => {
  const boot = 2 ** 53 - 1;
  let sessions: unknown = { places: [read('local', [entry('kept')], { connection: boot })] };
  ipc(undefined);
  mockIPC(command => command === 'overview_sessions' ? sessions : null);
  const view = render(<SessionList onCheck={noop} active places={places} />);
  expect(await screen.findAllByRole('listitem')).toHaveLength(1);
  sessions = { places: [read('local', [], { state: 'unavailable', observedAt: null, connection: boot })] };
  view.rerender(<SessionList onCheck={noop} active places={{ ...places }} />);
  expect(await screen.findByText("Couldn't read conversations on this computer just now. Showing what Bridge last saw.")).toBeTruthy();
  sessions = { places: [read('local', [], { state: 'disconnected', observedAt: null, connection: boot })] };
  view.rerender(<SessionList onCheck={noop} active places={{ ...places }} />);
  expect(await screen.findByText("The workspace on this computer isn't running now. Showing what Bridge last saw.")).toBeTruthy();
  expect(screen.getAllByRole('listitem')).toHaveLength(1);
});

test('under StrictMode the first read still lands', async () => {
  ipc({ places: [read('one', [entry('a')])] });
  render(<StrictMode><SessionList onCheck={noop} active places={places} /></StrictMode>);
  expect(await screen.findAllByRole('listitem')).toHaveLength(1);
});

test('an open conversation is read again on refresh, and a new connection closes it', async () => {
  let sessions: unknown = { places: [read('one', [entry('bridge-chat-1')])] };
  let messages = [{ role: 'user', text: 'First', cut: false, tools: [], at: 1000 }];
  const calls = ipc(undefined);
  mockIPC((command, args) => { calls.push([command, args]); return command === 'overview_sessions' ? sessions : { state: 'complete', messages }; });
  const view = render(<SessionList onCheck={noop} active places={places} />);
  await screen.findByRole('listitem');
  await userEvent.click(screen.getByRole('button', { name: /Conversation/ }));
  expect(await screen.findByText('First')).toBeTruthy();
  messages = [...messages, { role: 'assistant', text: 'Second', cut: false, tools: [], at: 2000 }];
  sessions = { places: [read('one', [entry('bridge-chat-1', { updatedAt: 3000 })], { observedAt: 6000 })] };
  view.rerender(<SessionList onCheck={noop} active places={{ ...places }} />);
  expect(await screen.findByText('Second')).toBeTruthy();
  expect(calls.filter(c => c[0] === 'overview_transcript')).toHaveLength(2);
  sessions = { places: [read('one', [entry('bridge-chat-1', { updatedAt: 3000 })], { observedAt: 7000, connection: 2 })] };
  view.rerender(<SessionList onCheck={noop} active places={{ ...places }} />);
  await waitFor(() => expect(screen.queryByText('Second')).toBeNull());
  expect(screen.getByRole('button', { name: /Conversation/ }).getAttribute('aria-expanded')).toBe('false');
});

test('a refusal stands through unreadable answers on the same connection until a Sessions read succeeds', async () => {
  let sessions: unknown = { places: [read('one', [], { state: 'denied', observedAt: null })] };
  ipc(undefined);
  mockIPC(command => command === 'overview_sessions' ? sessions : null);
  const denied = "Can't list conversations on team-one: this account isn't allowed to read Sessions.";
  const view = render(<SessionList onCheck={noop} active places={places} />);
  expect(await screen.findByText(denied)).toBeTruthy();
  sessions = { places: [read('one', [], { state: 'unavailable', observedAt: null })] };
  view.rerender(<SessionList onCheck={noop} active places={{ ...places }} />);
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(screen.getByText(denied)).toBeTruthy();
  expect(screen.queryByText("Couldn't read conversations on team-one just now.")).toBeNull();
  sessions = { places: [read('one', [entry('allowed')])] };
  view.rerender(<SessionList onCheck={noop} active places={{ ...places }} />);
  expect(await screen.findAllByRole('listitem')).toHaveLength(1);
  expect(screen.queryByText(denied)).toBeNull();
});

test('each places read starts one Sessions read, so a revisit waits for its own places read', async () => {
  const calls = ipc({ places: [read('one', [entry('a')])] });
  const view = render(<SessionList onCheck={noop} active places={places} />);
  await waitFor(() => expect(calls).toHaveLength(1));
  view.rerender(<SessionList onCheck={noop} active={false} places={places} />);
  view.rerender(<SessionList onCheck={noop} active places={places} />);
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(calls).toHaveLength(1);
  view.rerender(<SessionList onCheck={noop} active places={{ ...places }} />);
  await waitFor(() => expect(calls).toHaveLength(2));
});

test("a workspace starting or stopping reads this computer's conversations again without a new places read", async () => {
  const calls = ipc({ places: [read('one', [entry('a')])] });
  const view = render(<SessionList onCheck={noop} active places={places} workspace="stopped" />);
  await waitFor(() => expect(calls).toHaveLength(1));
  view.rerender(<SessionList onCheck={noop} active places={places} workspace="stopped" />);
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(calls).toHaveLength(1);
  view.rerender(<SessionList onCheck={noop} active places={places} workspace="ready" />);
  await waitFor(() => expect(calls).toHaveLength(2));
});

test('a failed or unreadable Sessions read offers Check conversations again, and a complete one does not', async () => {
  let sessions: unknown = 'not sessions';
  ipc(undefined);
  mockIPC(command => command === 'overview_sessions' ? sessions : null);
  const check = vi.fn();
  const view = render(<SessionList onCheck={check} active places={places} />);
  await userEvent.click(await screen.findByRole('button', { name: 'Check conversations again' }));
  expect(check).toHaveBeenCalledTimes(1);
  sessions = { places: [read('one', [entry('a')]), read('two', [], { state: 'denied', observedAt: null })] };
  view.rerender(<SessionList onCheck={check} active places={{ ...places }} />);
  expect(await screen.findByText(/Can't list conversations on team-two/)).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Check conversations again' })).toBeTruthy();
  sessions = { places: [read('one', [entry('a')]), read('two', [])] };
  view.rerender(<SessionList onCheck={check} active places={{ ...places }} />);
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Check conversations again' })).toBeNull());
});

test('only known roles are relabelled; an inherited property name shows as it is', async () => {
  expect(roleLabel('assistant')).toBe('Agent');
  for (const role of ['__proto__', 'constructor', 'toString']) expect(roleLabel(role)).toBe(role);
  ipc({ places: [read('one', [entry('a')])] }, () => ({ state: 'complete', messages: [{ role: '__proto__', text: 'odd', cut: false, tools: [], at: null }] }));
  render(<SessionList onCheck={noop} active places={places} />);
  await userEvent.click(await screen.findByRole('button', { name: /Conversation/ }));
  expect(await screen.findByText('odd')).toBeTruthy();
  expect(screen.getByText('__proto__')).toBeTruthy();
});

test('a partial read with no readable messages says so, and content left out is marked', async () => {
  let transcript: unknown = { state: 'partial', messages: [] };
  ipc({ places: [read('one', [entry('a'), entry('b', { updatedAt: 1500 })])] }, () => transcript);
  render(<SessionList onCheck={noop} active places={places} />);
  const [a, b] = await screen.findAllByRole('listitem');
  await userEvent.click(within(a).getByRole('button', { name: /Conversation/ }));
  expect(await within(a).findByText("Bridge couldn't read this conversation's messages.")).toBeTruthy();
  expect(within(a).queryByText('No messages yet.')).toBeNull();
  transcript = { state: 'complete', messages: [{ role: 'user', text: '', cut: true, tools: [], at: null }, { role: 'assistant', text: '', cut: false, tools: ['shell'], at: null }] };
  await userEvent.click(within(b).getByRole('button', { name: /Conversation/ }));
  expect(await within(b).findByText("Content Bridge doesn't show here.")).toBeTruthy();
  expect(within(b).getAllByText("Content Bridge doesn't show here.")).toHaveLength(1);
});

test('a connected place whose last read failed offers Check conversations again; a never-connected one does not', async () => {
  let sessions: unknown = { places: [read('one', [], { state: 'disconnected', observedAt: null, connection: null })] };
  ipc(undefined);
  mockIPC(command => command === 'overview_sessions' ? sessions : null);
  const view = render(<SessionList onCheck={noop} active places={places} />);
  expect(await screen.findByText('Conversations appear here once the workspace is ready on this computer or a cluster is connected.')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Check conversations again' })).toBeNull();
  sessions = { places: [read('one', [], { state: 'disconnected', observedAt: null, connection: 3 })] };
  view.rerender(<SessionList onCheck={noop} active places={{ ...places }} />);
  expect(await screen.findByRole('button', { name: 'Check conversations again' })).toBeTruthy();
});

test('conversations whose IDs Bridge cannot open are counted for their place', async () => {
  ipc({ places: [read('one', [entry('a')], { unsupported: 1 }), read('two', [entry('b')], { unsupported: 3 })] });
  render(<SessionList onCheck={noop} active places={places} />);
  expect(await screen.findByText("1 conversation on team-one has an ID Bridge can't open yet, so it isn't listed.")).toBeTruthy();
  expect(screen.getByText("3 conversations on team-two have IDs Bridge can't open yet, so they aren't listed.")).toBeTruthy();
});
