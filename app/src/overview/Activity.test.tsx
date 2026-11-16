import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import { isActivity, isToolCalls, merge, type PlaceActivity } from './activity';
import { RecentActivity } from './Activity';
import type { Places, RemoteAgent } from './api';

const agent: RemoteAgent = { name: 'reader', namespace: 'agents', uid: 'agent-one', agentType: null, model: null, tools: null, record: null };
const place = (key: string, context: string, agents: RemoteAgent[] = []) => ({ key, context, service: 'control/orka', namespace: 'agents', stale: false, observedAt: 1000, agents });
const places: Places = { places: [place('one', 'team-one', [agent]), place('two', 'team-two')], historyState: 'complete', unobserved: [] };
const entry = (task: string, over: object = {}) => ({ task, uid: `uid-${task}`, agent: 'reader', agentUid: null, phase: 'succeeded', createdAt: 1000, startedAt: null, completedAt: null, message: null, test: false, ...over });
const read = (key: string, entries: unknown[], over: object = {}) => ({ key, state: 'complete', observedAt: 5000, entries, ...over });
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
  render(<><article data-card-key="one/agent-one" tabIndex={-1}>card</article><RecentActivity active places={places} /></>);
  const [bound, failed, gone] = await items();
  expect(bound.getAttribute('aria-label')).toBe('reader on team-one, Succeeded');
  expect(within(failed).getByText('Failed')).toBeTruthy();
  expect(within(failed).getByText('model unavailable')).toBeTruthy();
  expect(failed.textContent).toContain('Bridge test');
  expect(within(gone).queryByRole('button', { name: /Show/ })).toBeNull();
  await userEvent.click(within(bound).getByRole('button', { name: 'Show reader on team-one' }));
  expect(document.activeElement?.getAttribute('data-card-key')).toBe('one/agent-one');
  expect(screen.getByText("Runs on this computer aren't listed here yet.")).toBeTruthy();
});

test('filters narrow the list and say when nothing matches', async () => {
  ipc({ places: [read('one', [entry('ok'), entry('broken', { phase: 'failed' })])] });
  render(<RecentActivity active places={places} />);
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
  render(<RecentActivity active places={places} />);
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
  const view = render(<RecentActivity active places={places} />);
  expect(await items()).toHaveLength(1);
  expect(screen.getByText('team-two has more runs than Bridge reads at once. Orka lists runs by name, so the newest may be missing.')).toBeTruthy();
  activity = { places: [read('one', [], { state: 'unavailable', observedAt: null }), read('two', [], { state: 'denied', observedAt: null })] };
  view.rerender(<RecentActivity active places={{ ...places }} />);
  expect(await screen.findByText("Couldn't read runs on team-one just now. Showing what Bridge last saw.")).toBeTruthy();
  expect(screen.getByText("Can't list runs on team-two: this account isn't allowed to read Tasks.")).toBeTruthy();
  expect(screen.getAllByRole('listitem')).toHaveLength(1);
  activity = 'not activity';
  view.rerender(<RecentActivity active places={{ ...places }} />);
  expect(await screen.findByText("Couldn't read recent activity just now. Showing what Bridge last saw.")).toBeTruthy();
  expect(screen.getAllByRole('listitem')).toHaveLength(1);
});

test('without a connected cluster the section says how to see runs, and an inactive tab never reads', async () => {
  const calls = ipc({ places: [read('one', [], { state: 'disconnected', observedAt: null })] });
  const view = render(<RecentActivity active={false} places={places} />);
  await waitFor(() => expect(calls).toHaveLength(0));
  view.rerender(<RecentActivity active places={places} />);
  expect(await screen.findByText('Connect a cluster to see its runs here.')).toBeTruthy();
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
  const view = render(<RecentActivity active places={places} />);
  expect(await items()).toHaveLength(1);
  activity = { places: [read('one', [], { state: 'denied', observedAt: null })] };
  view.rerender(<RecentActivity active places={{ ...places }} />);
  await waitFor(() => expect(calls).toHaveLength(2));
  view.rerender(<RecentActivity active places={{ ...places }} />);
  view.rerender(<RecentActivity active places={{ ...places }} />);
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
  const view = render(<RecentActivity active places={places} />);
  expect(await items()).toHaveLength(1);
  activity = { places: [read('one', [], { state: 'revoked', observedAt: null })] };
  view.rerender(<RecentActivity active places={{ ...places }} />);
  expect(await screen.findByText('Connect to team-one again to see its runs.')).toBeTruthy();
  expect(screen.queryAllByRole('listitem')).toHaveLength(0);
});
