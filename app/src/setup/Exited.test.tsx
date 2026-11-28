import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from '../App';
import { SAMPLE_DOCUMENT_ID } from '../designer/agent-api';
import { NAVIGATOR_ID } from '../home/navigator-api';
import { setupView, workspaceView } from './progress';
import type { SetupStatus } from './state';
import { noUpgrade } from './test-upgrade';
import { openLocalSetup } from './test-navigation';

// A running workspace whose VM stopped by itself: stopped, not a failed start, on every surface.
const exited: SetupStatus = { selected: 'localVm', localVm: 'failed', model: 'locked', modelSupported: true, localVmError: 'The workspace stopped unexpectedly: its VM is no longer running.', workspaceFailure: { operation: 'serve', cause: 'exited', codes: [] } };
let status: SetupStatus;
beforeEach(() => {
  vi.stubGlobal('isTauri', true); localStorage.setItem('bridge.welcomeSeen.v1', '1'); localStorage.setItem('bridge.firstTest.v1', 'attempted');
  status = exited;
  mockIPC(command => {
    if (command === 'workspace_upgrade_status') return noUpgrade;
    if (command === 'setup_status') return status;
    if (command === 'setup_requirements') return null;
    if (command === 'setup_select_local') return status = { selected: 'localVm', localVm: 'starting', model: 'locked', modelSupported: true, workspaceProgress: { phase: 'starting', bytes: null, totalBytes: null, elapsedMs: 0, remainingMs: null } };
    if (command === 'agents_list') return [{ id: SAMPLE_DOCUMENT_ID, name: 'web-reader', path: null, available: true }];
    if (command === 'agent_creation_root') return '/agents';
    if (command === 'agent_selected') return SAMPLE_DOCUMENT_ID;
    if (command === 'agent_open') return { documentId: SAMPLE_DOCUMENT_ID, name: 'web-reader', agent: '---\nname: web-reader\nmodel: gpt-5.6-terra\nkmx:\n  runtime: orka\n  schemaVersion: 1\n  tools: []\n---\nRead pages.', tests: 'Checks' };
    if (command === 'agent_run_status') return { documentId: SAMPLE_DOCUMENT_ID, busy: null, state: 'idle', canRun: false, modelConnected: false, report: null, acknowledged: [], pending: null, error: null };
    if (command === 'agent_chat_status') return { documentId: SAMPLE_DOCUMENT_ID, state: 'ready', busy: false, canSend: true, versions: [{ turns: [] }], error: null, denied: null, keptOnCluster: false, readOnly: false };
    if (command === 'navigator_status') return { chat: { documentId: NAVIGATOR_ID, state: 'ready', busy: false, canSend: true, error: null, denied: null, keptOnCluster: false, readOnly: false, versions: [{ turns: [] }] }, thinksWith: null, workspaceTooOld: false };
    if (command === 'cluster_places') return { places: [], selectedKey: null, persistenceState: 'complete', accessLost: false };
    if (command === 'cluster_contexts') return [];
    return null;
  });
});
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

test('the shared views say Stopped and wait on the person, not Couldn\'t start', () => {
  expect(workspaceView(exited)).toEqual({ word: 'Stopped', detail: 'The workspace on this computer stopped. Your agents and files are kept.', status: 'needs-you', tone: 'needs-you' });
  expect(setupView(exited)).toMatchObject({ line: 'Workspace stopped', detail: 'The workspace on this computer stopped.', status: 'needs-you' });
  expect(setupView(exited, { workspace: true })).toMatchObject({ line: 'Workspace stopped', status: 'needs-you' });
});

test('every surface agrees the workspace stopped, and Restart workspace starts it in place', async () => {
  const user = userEvent.setup();
  render(<App />);
  await openLocalSetup(user);
  const card = await screen.findByRole('region', { name: 'Workspace' });
  expect(within(card).getByText('Stopped', { selector: '.pill' }).className).toContain('status-needs-you');
  expect(within(card).getByText('The workspace on this computer stopped.')).toBeTruthy();
  expect(document.body.textContent).not.toMatch(/Couldn't start|Setup failed/);
  expect(screen.queryByText('Ready', { selector: '.pill' })).toBeNull();
  expect(within(screen.getByRole('navigation')).getByText(/stopped/i)).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Home' }));
  expect(await screen.findByRole('heading', { name: 'The workspace on this computer stopped.' })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  await waitFor(() => expect(document.body.textContent).toContain('The workspace on this computer stopped. Start it again from This computer in Clusters.'));
  expect(document.body.textContent).not.toContain('Waiting for the workspace to start.');
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  await user.click(await screen.findByRole('button', { name: 'Back to clusters' }));
  const row = await screen.findByRole('region', { name: 'This computer' });
  expect(row.textContent).toContain('stopped');
  await user.click(within(row).getByRole('button', { name: /Manage|Restart|Start/ }));
  await user.click(await screen.findByRole('button', { name: 'Restart workspace' }));
  expect(await within(await screen.findByRole('region', { name: 'Workspace' })).findByText('Starting', { selector: '.pill' })).toBeTruthy();
});
