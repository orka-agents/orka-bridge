import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import Overview from './Overview';
import { sendAgentCheck, sendAgentFocus } from '../clusters/send-api';

const source = { agent: 'a'.repeat(64), tests: 'b'.repeat(64) };
const id = '12345678-1234-1234-1234-123456789abc';
const targetUids = { serviceNamespace: 'control', serviceUid: 'svc', deploymentNamespace: 'control', deploymentUid: 'dep', watchedNamespace: 'agents' };
const record = (uid: string) => ({ id, source, targetDisplay: 'team · agents', targetUids, provider: 'model', plannedAgentName: 'same-name', agentUid: uid, verdict: 'passed', time: 1234567890 });
const agent = (uid: string) => ({ name: 'same-name', namespace: 'agents', uid, agentType: 'native', model: 'm', tools: [], record: record(uid) });
const key = '["team","control","svc","control","dep","agents"]';
const place = (agents: unknown[]) => ({ key, context: 'team', service: 'control/orka', namespace: 'agents', stale: false, observedAt: 1234567890000, agents });
const places = (list: unknown[]) => ({ places: [place(list)], historyState: 'complete', unobserved: [] });
const files = { state: 'complete', files: [], runActive: false, historyState: 'complete', runAgents: [] };
const live = { state: 'complete', installation: id, agents: [], observedAt: 123, observation: null, runActive: false, reason: null };
const props = { active: true, runBusy: false, workspaceKey: 'ready', onOpenDocument: vi.fn(), onOpenExample: vi.fn(), onChooseWorkspace: vi.fn() };
const target = (uid: string) => ({ key, name: 'same-name', namespace: 'agents', uid });
beforeEach(() => { vi.stubGlobal('isTauri', true); });
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); vi.clearAllMocks(); });
function ipc(inventory: unknown, check: unknown = 'present') {
  const calls = vi.fn<(command: string, args?: unknown) => unknown>(command => command === 'overview_places' ? inventory : command === 'overview_files' ? files : command === 'send_agent_check' ? check : live);
  mockIPC(calls); return calls;
}
const focused = () => (document.activeElement as HTMLElement | null)?.dataset.cardKey;
test('two same-name agents with different UIDs: the exact UID card takes focus, refreshing only that place', async () => {
  const calls = ipc(places([agent('uid-one'), agent('uid-two')]));
  const view = render(<Overview {...props} focus={{ id: 1, sendId: 's1', target: target('uid-two') }} />);
  await waitFor(() => expect(focused()).toBe(`${key}/uid-two`));
  expect(calls.mock.calls.some(([command, args]) => command === 'overview_places' && (args as { key?: string } | undefined)?.key === key)).toBe(true);
  expect(calls.mock.calls.some(([command]) => command === 'send_agent_check')).toBe(false);
  // A concurrent full inventory can remount a balanced card after focus; navigation refreshes only its destination.
  expect(calls.mock.calls.filter(([command]) => command === 'overview_places').every(([, args]) => (args as { key?: string } | undefined)?.key === key)).toBe(true);
  // Repeated navigation focuses the new exact target again.
  view.rerender(<Overview {...props} focus={{ id: 2, sendId: 's2', target: target('uid-one') }} />);
  await waitFor(() => expect(focused()).toBe(`${key}/uid-one`));
});
test('an older focus reply cannot supersede a newer navigation', async () => {
  let release!: (value: unknown) => void;
  let slow = true;
  vi.stubGlobal('isTauri', true);
  mockIPC((command, args) => command === 'overview_places' ? (slow && (args as { key?: string } | undefined)?.key ? (slow = false, new Promise(resolve => { release = resolve; })) : places([agent('uid-one'), agent('uid-two')])) : command === 'overview_files' ? files : live);
  const view = render(<Overview {...props} focus={{ id: 1, sendId: 's1', target: target('uid-one') }} />);
  await waitFor(() => expect(release).toBeDefined());
  view.rerender(<Overview {...props} focus={{ id: 2, sendId: 's2', target: target('uid-two') }} />);
  await waitFor(() => expect(focused()).toBe(`${key}/uid-two`));
  release(places([agent('uid-one'), agent('uid-two')]));
  await new Promise(resolve => setTimeout(resolve, 50));
  expect(focused()).toBe(`${key}/uid-two`);
});
test.each([
  ['present', 'Not shown in Overview yet.'],
  ['gone', 'The original agent is no longer available. Your saved result is unchanged.'],
  ['cannotCheck', 'Cannot check this agent. Your saved result is unchanged.'],
])('a missing card is checked read-only and reported as %s, never assumed deleted', async (check, text) => {
  const calls = ipc(places([agent('uid-other')]), check);
  render(<Overview {...props} focus={{ id: 1, sendId: 's1', target: target('uid-one') }} />);
  expect(await screen.findByText(text)).toBeTruthy();
  expect(within(screen.getByText(text).closest('.overview-note') as HTMLElement).getByRole('button', { name: 'Refresh' })).toBeTruthy();
  expect(calls.mock.calls.find(([command]) => command === 'send_agent_check')?.[1]).toEqual({ sendId: 's1' });
});
test('Refresh retries a present-but-unlisted target and clears the stale absence note once its card appears', async () => {
  ipc(places([]));
  render(<Overview {...props} focus={{ id: 1, sendId: 's1', target: target('uid-one') }} />);
  const note = await screen.findByText('Not shown in Overview yet.');
  ipc(places([agent('uid-one')]));
  fireEvent.click(within(note.closest('.overview-note') as HTMLElement).getByRole('button', { name: 'Refresh' }));
  await waitFor(() => expect(focused()).toBe(`${key}/uid-one`));
  expect(screen.queryByText('Not shown in Overview yet.')).toBeNull();
});
test('leaving an agent-specific request clears its warning before ordinary Overview navigation', async () => {
  const calls = ipc(places([]), 'gone');
  const view = render(<Overview {...props} focus={{ id: 1, sendId: 's1', target: target('uid-one') }} />);
  await screen.findByText(/The original agent is no longer available/);
  view.rerender(<Overview {...props} active={false} />);
  view.rerender(<Overview {...props} />);
  await waitFor(() => expect(screen.queryByText(/The original agent is no longer available/)).toBeNull());
  fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
  expect(screen.queryByText(/The original agent is no longer available/)).toBeNull();
  expect(calls.mock.calls.filter(([command]) => command === 'send_agent_check')).toHaveLength(1);
});
test('a Send with no recorded agent says so without any check', async () => {
  const calls = ipc(places([agent('uid-one')]));
  render(<Overview {...props} focus={{ id: 1, sendId: 's1' }} />);
  expect(await screen.findByText(/Cannot check this agent/)).toBeTruthy();
  expect(calls.mock.calls.some(([command]) => command === 'send_agent_check')).toBe(false);
});
test('the native agent commands take only a send id and treat anything malformed as not found or cannot check', async () => {
  const calls = vi.fn<(command: string, args?: unknown) => unknown>(command => command === 'send_agent_focus' ? { ...target('uid-one'), extra: 'field' } : 'deleted');
  mockIPC(calls);
  expect(await sendAgentFocus('s1')).toBeUndefined();
  expect(await sendAgentCheck('s1')).toBe('cannotCheck');
  expect(calls.mock.calls.map(([, args]) => args)).toEqual([{ sendId: 's1' }, { sendId: 's1' }]);
  mockIPC(() => target('uid-one'));
  expect(await sendAgentFocus('s1')).toEqual(target('uid-one'));
});
