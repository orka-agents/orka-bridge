import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import Send from './Send';
import Example from '../designer/Example';
import { SAMPLE_DOCUMENT_ID } from '../designer/agent-api';
const idle = { documentId: SAMPLE_DOCUMENT_ID, view: null, busy: null };

const source = { agent: 'a'.repeat(64), tests: 'b'.repeat(64) };
const summary = {
  id: '00000000-0000-4000-8000-000000000001', source,
  targetDisplay: 'team-dev · agents',
  targetUids: { serviceNamespace: 'control', serviceUid: 'service-1', deploymentNamespace: 'control', deploymentUid: 'deployment-1', watchedNamespace: 'agents' },
  provider: 'team-model', plannedAgentName: 'bridge-run-before-restart',
  agentUid: null, verdict: 'unknown', time: 1794412800,
};
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); });
function native(history: unknown) {
  vi.stubGlobal('isTauri', true);
  const ipc = vi.fn((command: string) => {
    if (command === 'send_status') return idle;
    if (command === 'send_history') return history;
    throw Error('Unexpected operation');
  });
  mockIPC(ipc); return ipc;
}
const panel = () => <Send place={{ kind: 'thisComputer' }} source={source} stale={false} visible remote={false} />;

test('restart shows saved pass and unknown without observing or adopting either Agent', async () => {
  const ipc = native([summary, { ...summary, id: '00000000-0000-4000-8000-000000000002', plannedAgentName: 'bridge-run-passed', agentUid: 'agent-uid', verdict: 'passed' }]);
  render(panel());
  const history = await screen.findByRole('region', { name: 'Past Sends' });
  expect(within(history).getByText('bridge-run-before-restart')).toBeTruthy();
  expect(within(history).getByText('bridge-run-passed')).toBeTruthy();
  expect(within(history).getByText('Tests: unknown')).toBeTruthy();
  expect(within(history).getByText('Tests: pass')).toBeTruthy();
  expect(within(history).getByText(/interrupted Send may still be running/)).toBeTruthy();
  expect(within(history).getAllByText('team-dev · agents')).toHaveLength(2);
  expect(ipc.mock.calls.every(([command]) => ['send_status', 'send_history'].includes(command))).toBe(true);
  expect(within(history).queryByRole('button', { name: /resume|adopt/i })).toBeNull();
  const user = userEvent.setup();
  await user.click(within(history).getAllByText('Show saved details')[0]);
  expect(within(history).getByText('Not confirmed')).toBeTruthy();
  expect(within(within(history).getAllByRole('listitem')[0]).getByText(source.agent)).toBeTruthy();
});

test.each([
  [{ ...summary, verdict: 'invented-pass' }],
  [{ ...summary, token: 'PRIVATE_DIAGNOSTIC' }],
  [{ ...summary, source: { agent: 'not-a-hash', tests: source.tests } }],
  [{ ...summary, time: Number.MAX_SAFE_INTEGER }],
])('malformed stored summary is unavailable, never a pass or native diagnostic', async history => {
  native(history); render(panel());
  expect(await screen.findByText(/Saved Send summaries could not be read/)).toBeTruthy();
  expect(screen.queryByText('Checks: pass')).toBeNull();
  expect(document.body.textContent).not.toContain('PRIVATE_DIAGNOSTIC');
});

test('history read failure is explicit and retry only rereads local summaries', async () => {
  vi.stubGlobal('isTauri', true); let broken = true;
  const ipc = vi.fn((command: string) => {
    if (command === 'send_status') return idle;
    if (command === 'send_history') { if (broken) throw Error('PRIVATE_DIAGNOSTIC'); return [summary]; }
    throw Error('Unexpected operation');
  });
  mockIPC(ipc); render(panel());
  await screen.findByText(/Saved Send summaries could not be read/);
  broken = false;
  await userEvent.setup().click(screen.getByRole('button', { name: 'Retry saved Sends' }));
  expect(await screen.findByText('bridge-run-before-restart')).toBeTruthy();
  expect(screen.queryByText(/Saved Send summaries could not be read/)).toBeNull();
  expect(ipc.mock.calls.every(([command]) => ['send_status', 'send_history'].includes(command))).toBe(true);
});

test('saved history remains readable without a selected cluster or permission to send during setup', async () => {
  const ipc = native([summary]);
  render(<Send place={{ kind: 'thisComputer' }} source={source} stale={false} visible={false} remote={false} blocked="Read-only during setup" />);
  expect(await screen.findByRole('region', { name: 'Past Sends' })).toBeTruthy();
  expect(screen.getByText('bridge-run-before-restart')).toBeTruthy();
  expect(ipc.mock.calls.every(([command]) => ['send_status', 'send_history'].includes(command))).toBe(true);
});

test('global history refreshes when another document’s Send releases admission', async () => {
  vi.stubGlobal('isTauri', true);
  const other = '00000000-0000-4000-8000-000000000003';
  let busy: { documentId: string; name: string } | null = { documentId: other, name: 'other-agent' };
  let history: unknown[] = [];
  mockIPC(command => {
    if (command === 'send_status') return { ...idle, busy };
    if (command === 'send_history') return history;
    throw Error('Unexpected operation');
  });
  render(panel());
  await screen.findByText('No saved Sends yet.');
  history = [summary]; busy = null;
  expect(await screen.findByText('bridge-run-before-restart')).toBeTruthy();
});

test.each(['pending', 'failed'])('saved Sends are readable while the document open is %s', async state => {
  vi.stubGlobal('isTauri', true);
  const ipc = vi.fn((command: string) => {
    if (command === 'agent_open') {
      if (state === 'failed') throw Error('PRIVATE_SOURCE_FAILURE');
      return new Promise(() => {});
    }
    if (command === 'send_history') return [summary];
    throw Error('Unexpected operation');
  });
  mockIPC(ipc); render(<Example />);
  expect(await screen.findByText('bridge-run-before-restart')).toBeTruthy();
  if (state === 'failed') expect(screen.getByText('Your agent folder could not be opened. Open it again from Designer.')).toBeTruthy();
  expect(document.body.textContent).not.toContain('PRIVATE_SOURCE_FAILURE');
  expect(ipc.mock.calls.every(([command]) => ['agent_open', 'agents_list', 'send_history'].includes(command))).toBe(true);
});

test('empty history says no saved Sends, not that no remote Agents exist', async () => {
  native([]); render(panel());
  expect(await screen.findByText('No saved Sends yet.')).toBeTruthy();
  expect(screen.queryByText(/no agents/i)).toBeNull();
});

test('a failed initial save explains that nothing was sent, rather than suggesting remote work exists', async () => {
  vi.stubGlobal('isTauri', true);
  mockIPC(command => {
    if (command === 'send_history') return [];
    if (command === 'send_status') return { ...idle, view: {
      documentId: SAMPLE_DOCUMENT_ID, id: summary.id, targetId: summary.id, providerId: summary.id, targetName: 'team-dev', source,
      model: 'model', agentName: summary.plannedAgentName, state: 'unknown', stage: 'submit',
      acknowledged: [], pending: null, report: null, error: 'storage', storageError: true,
    } };
    throw Error('Unexpected operation');
  });
  render(panel());
  expect(await screen.findByText(/Nothing was sent because Bridge could not save a summary/)).toBeTruthy();
  expect(screen.queryByText(/Named resources may still exist/)).toBeNull();
});

test('a storage write failure stays visible with an unknown live result', async () => {
  vi.stubGlobal('isTauri', true);
  mockIPC(command => {
    if (command === 'send_history') return [summary];
    if (command === 'send_status') return { ...idle, view: {
      documentId: SAMPLE_DOCUMENT_ID, id: summary.id, targetId: summary.id, providerId: summary.id, targetName: 'team-dev', source,
      model: 'model', agentName: summary.plannedAgentName, state: 'unknown', stage: 'done',
      acknowledged: [], pending: null, report: null, error: null, storageError: true,
    } };
    throw Error('Unexpected operation');
  });
  render(panel());
  await waitFor(() => expect(screen.getByText(/could not save this Send/)).toBeTruthy());
  expect(screen.getByText('Earlier result unknown')).toBeTruthy();
  expect(document.querySelector('.send-panel .pill.status-stale')).toBeTruthy();
  expect(screen.queryByText('Checks: pass')).toBeNull();
});
