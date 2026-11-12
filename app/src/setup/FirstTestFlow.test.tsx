import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from '../App';

const sample = '9b2b93c4-375c-4815-9717-39852a96e411';
const pair = { agent: '---\nname: web-reader\n---\nRead a page.', tests: 'A checksum test.' };
const idle = { documentId: sample, busy: null, state: 'idle', canRun: true, modelConnected: true, report: null, acknowledged: [], pending: null, error: null };
function native() {
  vi.stubGlobal('isTauri', true);
  let selected = false; let running = false; let finished = false;
  const status = () => ({ selected: selected ? 'localVm' : null, localVm: selected ? 'ready' : 'idle', model: selected ? 'available' : 'locked' });
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_status') return status();
    if (command === 'setup_requirements') return null;
    if (command === 'setup_select_local') { selected = true; return status(); }
    if (command === 'setup_change_local') { selected = false; return status(); }
    if (command === 'setup_example') return pair;
    if (command === 'setup_connect_model') return { ready: true };
    if (command === 'agent_open') return { ...pair, documentId: sample, name: 'web-reader' };
    if (command === 'agent_run') { running = true; return { ...idle, state: 'running', canRun: false, busy: { documentId: sample, name: 'web-reader' } }; }
    if (command === 'agent_run_status') return finished ? { ...idle, state: 'complete', report: { verdict: 'passed', cases: [{ id: 'checksum', phase: 'Succeeded', answer: 'Correct checksum', completedTools: ['web_fetch'], assertions: [{ id: 'checksum', kind: 'contains', verdict: 'passed' }], verdict: 'passed' }] } } : running ? { ...idle, state: 'running', canRun: false, busy: { documentId: sample, name: 'web-reader' } } : idle;
    if (command === 'send_history' || command === 'cluster_contexts') return [];
    if (command === 'overview_files') return { files: [], unavailable: 0 };
    if (command === 'overview_agents') return { state: 'stopped', agents: [] };
    throw Error('Unexpected command: ' + command);
  });
  mockIPC(ipc); return { ipc, finish: () => { finished = true; }, loseWorkspace: () => { selected = false; } };
}
beforeEach(() => localStorage.setItem('bridge.welcomeSeen.v1', '1'));
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

async function completeSetup() {
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Continue' }));
  await user.click(await screen.findByRole('button', { name: 'Connect the AI' }));
  return user;
}

test('first setup shows a running test, opens Designer on its result and never auto-runs on another visit', async () => {
  const { ipc, finish } = native(); const user = await completeSetup();
  await waitFor(() => expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toHaveLength(1));
  expect(screen.getByRole('button', { name: 'Clusters' }).getAttribute('aria-current')).toBe('page');
  expect(screen.getByRole('list', { name: 'Setup steps' }).children[4].textContent).toBe('Run your first testRunning');
  finish();
  await waitFor(() => expect(screen.getByRole('button', { name: 'Designer' }).getAttribute('aria-current')).toBe('page'));
  expect(await screen.findByText('Correct checksum')).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  await user.click(screen.getByRole('button', { name: 'Open your test result' }));
  expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toEqual([['agent_run', { documentId: sample }]]);
});

test.each(['Overview', 'Designer', 'cluster', 'Stop', 'workspace lost'])('first-test completion does not interrupt %s', async destination => {
  const { ipc, finish, loseWorkspace } = native(); const user = await completeSetup();
  await waitFor(() => expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toHaveLength(1));
  if (destination === 'Overview' || destination === 'Designer') await user.click(screen.getByRole('button', { name: destination }));
  else if (destination === 'workspace lost') { loseWorkspace(); window.dispatchEvent(new Event('focus')); await screen.findByRole('button', { name: 'Continue' }); }
  else {
    await user.click(screen.getByRole('button', { name: 'Change where agents run' }));
    await user.click(screen.getByRole('button', { name: destination === 'cluster' ? 'Choose a cluster, keep this workspace running' : 'Stop workspace and choose a place' }));
  }
  finish();
  await new Promise(resolve => setTimeout(resolve, 750));
  const expected = destination === 'Overview' || destination === 'Designer' ? destination : 'Clusters';
  expect(within(screen.getByRole('navigation')).getByRole('button', { name: expected }).getAttribute('aria-current')).toBe('page');
  expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toHaveLength(1);
});
