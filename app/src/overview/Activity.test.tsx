import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { StrictMode } from 'react';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import { isActivity, isToolCalls, merge, type PlaceActivity } from './activity-api';
import { RecentActivity } from './Activity';
import type { Places, RemoteAgent } from './api';

const agent: RemoteAgent = { name: 'reader', namespace: 'agents', uid: 'agent-one', agentType: null, model: null, tools: null, record: null };
const place = (key: string, context: string, agents: RemoteAgent[] = []) => ({ key, context, service: 'control/orka', namespace: 'agents', stale: false, observedAt: 1000, agents });
const places: Places = { places: [place('one', 'team-one', [agent]), place('two', 'team-two')], historyState: 'complete', unobserved: [] };
const entry = (task: string, over: object = {}) => ({ task, uid: `uid-${task}`, agent: 'reader', agentUid: null, phase: 'succeeded', createdAt: 1000, startedAt: null, completedAt: null, message: null, test: false, ...over });
const read = (key: string, entries: unknown[], over: object = {}) => ({ key, state: 'complete', observedAt: 5000, entries, connection: 1, ...over });
function ipc(activity: unknown, tools: (args: Record<string, unknown>) => unknown = () => ({ state: 'complete', calls: [] })) {
  vi.stubGlobal('isTauri', true);
  const calls: [string, unknown][] = [];
  mockIPC((command, args) => {
    calls.push([command, args]);
    return command === 'overview_activity' ? activity : tools(args as Record<string, unknown>);
  });
  return calls;
}
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); });
const items = () => screen.findAllByRole('listitem');
const noShow = () => undefined;

test('runs from every connected place merge newest first, by latest recorded time', () => {
  const rows = merge([
    read('one', [entry('a', { createdAt: 1, completedAt: 50 }), entry('b', { createdAt: 40 })]),
    read('two', [entry('c', { createdAt: 10, startedAt: 45, phase: 'running' }), entry('d', { createdAt: 5, phase: 'failed' })]),
  ] as PlaceActivity[], 'all');
  expect(rows.map(r => `${r.placeKey}/${r.task}`)).toEqual(['one/a', 'two/c', 'one/b', 'two/d']);
  expect(merge([read('two', [entry('c', { phase: 'running' }), entry('d', { phase: 'failed' }), entry('e', { phase: 'scheduled' })])] as PlaceActivity[], 'active').map(r => r.task)).toEqual(['c', 'e']);
});

test('native payloads are accepted only when each state carries what it promises', () => {
  expect(isActivity({ places: [read('one', [entry('a')]), read('two', [], { state: 'disconnected', observedAt: null })] })).toBe(true);
  for (const bad of [
    { places: [read('one', [entry('a')]), read('one', [])] },
    { places: [read('one', [entry('a')], { state: 'unavailable', observedAt: null })] },
    { places: [read('one', [], { observedAt: null })] },
    { places: [read('one', [entry('a'), entry('a')])] },
    { places: [read('one', [entry('a', { phase: 'paused' })])] },
    { places: [read('one', [entry('a', { agent: null, agentUid: 'x' })])] },
    { places: [read('one', [entry('a', { createdAt: -1 })])] },
    { places: [read('one', Array.from({ length: 51 }, (_, i) => entry(`t${i}`)))] },
  ]) expect(isActivity(bad)).toBe(false);
  expect(isToolCalls({ state: 'partial', calls: [{ tool: 'web_fetch', failed: false, at: 1 }] })).toBe(true);
  expect(isToolCalls({ state: 'denied', calls: [{ tool: 'web_fetch', failed: false, at: 1 }] })).toBe(false);
});

test('each run names its agent and place, and only a bound agent on the board gets Show agent', async () => {
  ipc({ places: [
    read('one', [entry('bound', { agentUid: 'agent-one', createdAt: 3000 }), entry('gone', { agentUid: 'removed', createdAt: 2000 })]),
    read('two', [entry('bridge-task-x', { phase: 'failed', message: 'model unavailable', test: true, createdAt: 2500 })]),
  ] });
  const show = vi.fn();
  render(<RecentActivity onShowAgent={show} onCheck={noShow} active places={places} />);
  const [bound, failed, gone] = await items();
  expect(bound.getAttribute('aria-label')).toBe('reader on team-one, Succeeded');
  expect(within(failed).getByText('Failed')).toBeTruthy();
  expect(within(failed).getByText('model unavailable')).toBeTruthy();
  expect(failed.textContent).toContain('Bridge test');
  expect(within(gone).queryByRole('button', { name: /Show/ })).toBeNull();
  await userEvent.click(within(bound).getByRole('button', { name: 'Show reader on team-one' }));
  expect(show).toHaveBeenCalledWith('one/agent-one');
  expect(screen.getByText("Runs on this computer aren't listed here yet.")).toBeTruthy();
});

test('filters narrow the list and say when nothing matches', async () => {
  ipc({ places: [read('one', [entry('ok'), entry('broken', { phase: 'failed' })])] });
  render(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} />);
  expect(await items()).toHaveLength(2);
  await userEvent.click(screen.getByRole('button', { name: 'Failed' }));
  expect(screen.getAllByRole('listitem').map(i => i.getAttribute('aria-label'))).toEqual(['reader on team-one, Failed']);
  await userEvent.click(screen.getByRole('button', { name: 'In progress' }));
  expect(screen.getByText('Nothing is running or waiting.')).toBeTruthy();
});

test('tool calls load only when a run is expanded, from that place and run', async () => {
  const calls = ipc({ places: [read('one', [entry('run-1')])] }, args => args.task === 'run-1'
    ? { state: 'partial', calls: [{ tool: 'web_fetch', failed: false, at: 1000 }, { tool: 'shell', failed: true, at: 2000 }] }
    : { state: 'unavailable', calls: [] });
  render(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} />);
  const [run] = await items();
  expect(calls.filter(c => c[0] === 'overview_tool_calls')).toHaveLength(0);
  await userEvent.click(within(run).getByRole('button', { name: 'Tool calls' }));
  expect(await within(run).findByText('web_fetch')).toBeTruthy();
  expect(within(run).getByText('shell').closest('li')!.textContent).toContain('failed');
  expect(within(run).getByText("Some of this run's tool calls aren't shown.")).toBeTruthy();
  expect(calls.filter(c => c[0] === 'overview_tool_calls').map(c => c[1])).toEqual([{ key: 'one', task: 'run-1' }]);
});

test('place failures are explained, and a failed refresh keeps what Bridge last saw', async () => {
  let activity: unknown = { places: [read('one', [entry('kept')]), read('two', [], { state: 'partial' })] };
  ipc(undefined);
  mockIPC(command => command === 'overview_activity' ? activity : null);
  const view = render(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} />);
  expect(await items()).toHaveLength(1);
  expect(screen.getByText('team-two has more runs than Bridge reads at once. Orka lists runs by name, so the newest may be missing.')).toBeTruthy();
  activity = { places: [read('one', [], { state: 'unavailable', observedAt: null }), read('two', [], { state: 'denied', observedAt: null })] };
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={{ ...places }} />);
  expect(await screen.findByText("Couldn't read runs on team-one just now. Showing what Bridge last saw.")).toBeTruthy();
  expect(screen.getByText("Can't list runs on team-two: this account isn't allowed to read Tasks.")).toBeTruthy();
  expect(screen.getAllByRole('listitem')).toHaveLength(1);
  activity = 'not activity';
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={{ ...places }} />);
  expect(await screen.findByText("Couldn't read recent activity just now. Showing what Bridge last saw.")).toBeTruthy();
  expect(screen.getAllByRole('listitem')).toHaveLength(1);
});

test('without a connected cluster the section says how to see runs, and an inactive tab never reads', async () => {
  const calls = ipc({ places: [read('one', [], { state: 'disconnected', observedAt: null })] });
  const view = render(<RecentActivity onShowAgent={noShow} onCheck={noShow} active={false} places={places} />);
  await waitFor(() => expect(calls).toHaveLength(0));
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} />);
  expect(await screen.findByText('Connect a cluster to see its runs here.')).toBeTruthy();
});

test('until the first read answers the section says it is reading, and a failed first read never asks to connect', async () => {
  let answer: (value: unknown) => void = () => undefined;
  vi.stubGlobal('isTauri', true);
  mockIPC(() => new Promise(resolve => { answer = resolve; }));
  render(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} />);
  expect(await screen.findByText('Reading recent runs…')).toBeTruthy();
  expect(screen.queryByText('Connect a cluster to see its runs here.')).toBeNull();
  answer('not activity');
  expect(await screen.findByText("Couldn't read recent activity just now.")).toBeTruthy();
  expect(screen.queryByText('Reading recent runs…')).toBeNull();
  expect(screen.queryByText('Connect a cluster to see its runs here.')).toBeNull();
});

test('a refusal stands through unreadable answers on the same connection until a Task read succeeds', async () => {
  let activity: unknown = { places: [read('one', [], { state: 'denied', observedAt: null })] };
  ipc(undefined);
  mockIPC(command => command === 'overview_activity' ? activity : null);
  const denied = "Can't list runs on team-one: this account isn't allowed to read Tasks.";
  const view = render(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} />);
  expect(await screen.findByText(denied)).toBeTruthy();
  activity = { places: [read('one', [], { state: 'unavailable', observedAt: null })] };
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={{ ...places }} />);
  await waitFor(() => expect(screen.queryByText("Couldn't read runs on team-one just now.")).toBeNull());
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(screen.getByText(denied)).toBeTruthy();
  expect(screen.queryByText("Couldn't read runs on team-one just now.")).toBeNull();
  activity = { places: [read('one', [entry('allowed')])] };
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={{ ...places }} />);
  expect(await items()).toHaveLength(1);
  expect(screen.queryByText(denied)).toBeNull();
});

test('each places read starts one activity read, so a revisit waits for its own places read', async () => {
  const calls = ipc({ places: [read('one', [entry('run')])] });
  const view = render(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} />);
  await waitFor(() => expect(calls).toHaveLength(1));
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active={false} places={places} />);
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} />);
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(calls).toHaveLength(1);
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={{ ...places }} />);
  await waitFor(() => expect(calls).toHaveLength(2));
});

test('a failed or unreadable activity read offers Check runs again, and a complete one does not', async () => {
  let activity: unknown = 'not activity';
  ipc(undefined);
  mockIPC(command => command === 'overview_activity' ? activity : null);
  const check = vi.fn();
  const view = render(<RecentActivity onShowAgent={noShow} onCheck={check} active places={places} />);
  await userEvent.click(await screen.findByRole('button', { name: 'Check runs again' }));
  expect(check).toHaveBeenCalledTimes(1);
  activity = { places: [read('one', [entry('run')]), read('two', [], { state: 'unavailable', observedAt: null })] };
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={check} active places={{ ...places }} />);
  expect(await screen.findByText("Couldn't read runs on team-two just now.")).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Check runs again' })).toBeTruthy();
  activity = { places: [read('one', [entry('run')]), read('two', [])] };
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={check} active places={{ ...places }} />);
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Check runs again' })).toBeNull());
});

test('leaving the tab drops a queued read', async () => {
  let release: () => void = () => undefined;
  const calls: string[] = [];
  vi.stubGlobal('isTauri', true);
  mockIPC(async command => {
    calls.push(command);
    if (calls.length === 1) await new Promise<void>(resolve => { release = resolve; });
    return { places: [read('one', [entry('run')])] };
  });
  const view = render(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} />);
  await waitFor(() => expect(calls).toHaveLength(1));
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={{ ...places }} />);
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active={false} places={{ ...places }} />);
  release();
  expect(await items()).toHaveLength(1);
  await new Promise(resolve => setTimeout(resolve, 50));
  expect(calls).toHaveLength(1);
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={{ ...places }} />);
  await waitFor(() => expect(calls).toHaveLength(2));
});

test('a denied place drops its runs, and places that change during a read queue one more read', async () => {
  let release: () => void = () => undefined;
  let activity: unknown = { places: [read('one', [entry('kept')])] };
  const calls: string[] = [];
  vi.stubGlobal('isTauri', true);
  mockIPC(async command => {
    calls.push(command);
    if (calls.length === 2) await new Promise<void>(resolve => { release = resolve; });
    return activity;
  });
  const view = render(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} />);
  expect(await items()).toHaveLength(1);
  activity = { places: [read('one', [], { state: 'denied', observedAt: null })] };
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={{ ...places }} />);
  await waitFor(() => expect(calls).toHaveLength(2));
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={{ ...places }} />);
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={{ ...places }} />);
  expect(calls).toHaveLength(2);
  release();
  await waitFor(() => expect(calls).toHaveLength(3));
  expect(await screen.findByText("Can't list runs on team-one: this account isn't allowed to read Tasks.")).toBeTruthy();
  expect(screen.queryAllByRole('listitem')).toHaveLength(0);
});

test('a place whose access was reloaded or replaced drops its runs and asks to connect again', async () => {
  let activity: unknown = { places: [read('one', [entry('gone')])] };
  ipc(undefined);
  mockIPC(command => command === 'overview_activity' ? activity : null);
  const view = render(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} />);
  expect(await items()).toHaveLength(1);
  activity = { places: [read('one', [], { state: 'revoked', observedAt: null })] };
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={{ ...places }} />);
  expect(await screen.findByText('Connect to team-one again to see its runs.')).toBeTruthy();
  expect(screen.queryAllByRole('listitem')).toHaveLength(0);
});

test('a new connection drops the old connection\'s runs even when its first read fails', async () => {
  let activity: unknown = { places: [read('one', [entry('old')])] };
  ipc(undefined);
  mockIPC(command => command === 'overview_activity' ? activity : null);
  const view = render(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} />);
  expect(await items()).toHaveLength(1);
  activity = { places: [read('one', [], { state: 'unavailable', observedAt: null, connection: 2 })] };
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={{ ...places }} />);
  expect(await screen.findByText("Couldn't read runs on team-one just now.")).toBeTruthy();
  expect(screen.queryAllByRole('listitem')).toHaveLength(0);
});

test('an expanded run rereads its tool calls when its place is read again, and a partial empty read says so', async () => {
  let activity: unknown = { places: [read('one', [entry('run-1', { phase: 'running' })])] };
  let tools: unknown = { state: 'complete', calls: [] };
  const calls = ipc(undefined);
  mockIPC((command, args) => { calls.push([command, args]); return command === 'overview_activity' ? activity : tools; });
  const view = render(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} />);
  const [run] = await items();
  await userEvent.click(within(run).getByRole('button', { name: 'Tool calls' }));
  expect(await within(run).findByText('No tool calls recorded.')).toBeTruthy();
  tools = { state: 'partial', calls: [] };
  activity = { places: [read('one', [entry('run-1', { phase: 'succeeded' })], { observedAt: 6000 })] };
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={{ ...places }} />);
  expect(await screen.findByText("Some of this run's tool calls aren't shown.")).toBeTruthy();
  expect(screen.queryByText('No tool calls recorded.')).toBeNull();
  expect(calls.filter(c => c[0] === 'overview_tool_calls')).toHaveLength(2);
});

test('a run read on a new connection starts collapsed, without the old connection\'s tool calls', async () => {
  let activity: unknown = { places: [read('one', [entry('run-1')])] };
  let tools: Promise<unknown> = Promise.resolve({ state: 'complete', calls: [{ tool: 'old_tool', failed: false, at: 1000 }] });
  ipc(undefined);
  mockIPC(command => command === 'overview_activity' ? activity : tools);
  const view = render(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} />);
  const [run] = await items();
  await userEvent.click(within(run).getByRole('button', { name: 'Tool calls' }));
  expect(await within(run).findByText('old_tool')).toBeTruthy();
  tools = new Promise(() => undefined);
  activity = { places: [read('one', [entry('run-1')], { connection: 2, observedAt: 6000 })] };
  view.rerender(<RecentActivity onShowAgent={noShow} onCheck={noShow} active places={{ ...places }} />);
  await waitFor(() => expect(screen.queryByText('old_tool')).toBeNull());
  expect(screen.getByRole('button', { name: 'Tool calls' }).getAttribute('aria-expanded')).toBe('false');
});

test('under StrictMode the first read still lands', async () => {
  ipc({ places: [read('one', [entry('a')])] });
  render(<StrictMode><RecentActivity onShowAgent={noShow} onCheck={noShow} active places={places} /></StrictMode>);
  expect(await items()).toHaveLength(1);
});
