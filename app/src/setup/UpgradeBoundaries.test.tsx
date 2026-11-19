import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import App from '../App';
import Setup from './Setup';
import { openLocalSetup } from './test-navigation';
import type { UpgradeStatus } from './upgrade-api';

const eligible: UpgradeStatus = { state: 'eligible', transaction: null, createdAt: null, sourceReleases: ['0.1.2'], targetRelease: '0.1.3', retainedAllocatedBytes: 1200000000, additionalBytes: 2400000000, credential: 'absent', busy: false, error: null, repeated: false, canReplace: true, canRestore: false };
const current: UpgradeStatus = { ...eligible, state: 'consumed', transaction: '10000000-0000-4000-8000-000000000001', createdAt: 1700000000, canReplace: false, canRestore: true };
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });
function fixture(update: () => unknown, phase = 'ready', returning = true) {
  vi.stubGlobal('isTauri', true); localStorage.setItem('bridge.welcomeSeen.v1', '1');
  if (returning) localStorage.setItem('bridge.firstTest.v1', 'attempted');
  const ipc = vi.fn((command: string): unknown => {
    if (command === 'workspace_upgrade_status') return update();
    if (command === 'setup_status') return { selected: 'localVm', localVm: phase, model: phase === 'ready' ? 'available' : 'locked', workspaceReplaced: true, ...(phase === 'failed' ? { workspaceFailure: { cause: 'services', operation: 'start', codes: [] }, localVmError: 'Services did not start' } : {}) };
    if (command === 'setup_select_local') return { selected: 'localVm', localVm: 'starting', model: 'locked', workspaceReplaced: true };
    if (command === 'setup_requirements') return null;
    throw Error('Unexpected ' + command);
  });
  mockIPC(ipc); return ipc;
}
test.each(['sourceSelected', 'fresh'] as const)('Places is not green during %s attention', async state => {
  fixture(() => ({ ...current, state, error: state === 'fresh' ? 'network' : null })); render(<App />);
  const row = await screen.findByRole('region', { name: 'This computer' });
  await waitFor(() => expect(screen.getByRole('contentinfo').textContent).toContain(state === 'fresh' ? 'Workspace update needs attention' : 'Workspace recovery needs attention'));
  expect(within(row).getByText('Needs you', { selector: '.pill' })).toBeTruthy();
  expect(row.textContent).not.toContain('Workspace ready');
});
test('Overview shows recovery even with a ready VM', async () => {
  fixture(() => ({ ...current, state: 'sourceSelected' })); render(<App />);
  await waitFor(() => expect(screen.getByRole('contentinfo').textContent).toContain('Workspace recovery needs attention'));
  expect(document.querySelector('.overview')?.textContent).toContain('Workspace recovery needs attention');
});
test.each(['pending', 'failed'] as const)('independent setup observation blocks replaced admission with %s upgrade status', async observation => {
  const pending = deferred<unknown>(); const notice = vi.fn();
  fixture(() => { if (observation === 'failed') throw Error('Lost upgrade status'); return pending.promise; });
  render(<Setup onOpenExample={() => {}} onWorkspaceAction={notice} />);
  await screen.findByRole('region', { name: 'This computer' });
  await waitFor(() => expect(notice.mock.lastCall?.[3]).toBeTruthy());
  expect(document.querySelector('.setup-model')).toBeNull();
  fireEvent(window, new Event('focus'));
  await screen.findByRole('region', { name: 'This computer' });
});
test.each(['hung', 'failed'] as const)('unknown upgrade status never admits the automatic first test (%s)', async observation => {
  const ipc = fixture(() => { if (observation === 'failed') throw Error('Lost upgrade status'); return new Promise(() => {}); }, 'ready', false);
  localStorage.setItem('bridge.firstTest.v1', 'pending');
  const original = ipc.getMockImplementation()!;
  ipc.mockImplementation(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'setup_connect_model') return { ready: true };
    return original(command);
  });
  render(<App />); await openLocalSetup();
  fireEvent.click(await screen.findByRole('button', { name: 'Connect the AI' }));
  await screen.findByText('Connected', { selector: '.pill' });
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 550)); });
  expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toHaveLength(0);
  expect(localStorage.getItem('bridge.firstTest.v1')).toBe('pending');
});

test('slow upgrade status does not block or get superseded by recurring setup polls', async () => {
  const update = deferred<unknown>();
  const onStatus = vi.fn();
  fixture(() => update.promise, 'starting');
  render(<Setup onOpenExample={() => {}} onStatus={onStatus} />);
  await waitFor(() => expect(onStatus.mock.calls.length).toBeGreaterThan(1));
  await act(async () => { update.resolve({ ...current, state: 'sourceSelected' }); });
  await screen.findByText(/Workspace setup was interrupted/);
});

test.each(['fresh', 'consumed'] as const)('Manage retains ordinary %s failure restart and details', async state => {
  const ipc = fixture(() => ({ ...current, state }), 'failed'); render(<App />); await openLocalSetup();
  const restart = await screen.findByRole('button', { name: 'Restart workspace' });
  fireEvent.click(screen.getByRole('button', { name: 'Show details' }));
  expect(screen.getByText('Services did not start', { selector: 'pre' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Restore previous workspace' })).toBeTruthy();
  fireEvent.click(restart); await waitFor(() => expect(ipc.mock.calls.map(([c]) => c)).toContain('setup_select_local'));
});
test('Manage resumes a cancelled fresh workspace without losing Restore', async () => {
  const ipc = fixture(() => ({ ...current, state: 'fresh' }), 'cancelled'); render(<App />); await openLocalSetup();
  fireEvent.click(await screen.findByRole('button', { name: 'Restart workspace' }));
  await waitFor(() => expect(ipc.mock.calls.map(([c]) => c)).toContain('setup_select_local'));
  expect(screen.getByRole('button', { name: 'Restore previous workspace' })).toBeTruthy();
});
test('first setup retains fresh start-failure recovery', async () => {
  fixture(() => ({ ...current, state: 'fresh' }), 'failed', false); render(<App />); await openLocalSetup();
  expect(await screen.findByRole('button', { name: 'Try again' })).toBeTruthy();
  expect(screen.getByText(/Press Try again to restart the workspace/)).toBeTruthy();
});
test.each(['failed', 'cancelled'])('unknown replacement does not expose ordinary first-setup recovery (%s)', async phase => {
  const update = deferred<unknown>();
  fixture(() => update.promise, phase, false);
  render(<App />);
  await screen.findByRole('list', { name: 'Setup steps' });
  expect(screen.queryByRole('button', { name: 'Try again' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Start again' })).toBeNull();
  await act(async () => { update.resolve({ ...current, state: 'fresh' }); });
  await screen.findByRole('button', { name: phase === 'failed' ? 'Try again' : 'Start again' });
});

test('transferred connection uses only shared removal and reconnect remains explicit', async () => {
  let removed = false;
  const ipc = fixture(() => ({ ...current, credential: removed ? 'removed' : 'transferred' }));
  const original = ipc.getMockImplementation()!;
  ipc.mockImplementation(command => {
    if (command === 'setup_connect_model') return { ready: true };
    if (command === 'workspace_upgrade_remove') { removed = true; return { ...current, credential: 'removed' }; }
    return original(command);
  });
  render(<App />); await openLocalSetup();
  fireEvent.click(await screen.findByRole('button', { name: 'Connect the AI' }));
  fireEvent.click(await screen.findByRole('button', { name: 'Change' }));
  expect(screen.queryByRole('button', { name: 'Remove saved sign-in' })).toBeNull();
  expect(screen.getByText(/Remove saved connection in the Workspace card/)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Remove saved connection' }));
  expect(screen.getByText(/every retained workspace that references it/)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Remove for all retained workspaces' }));
  await screen.findByRole('button', { name: 'Connect the AI' });
  expect(screen.queryByText('Connected', { selector: '.pill' })).toBeNull();
  expect(ipc.mock.calls.filter(([c]) => c === 'setup_connect_model')).toHaveLength(1);
  expect(ipc.mock.calls.filter(([c]) => c === 'setup_remove_model' || c === 'agent_run')).toHaveLength(0);
});

test('screen reader ready announcement does not contradict recovery', async () => {
  fixture(() => ({ ...current, state: 'sourceSelected' }), 'ready', false); render(<App />); await openLocalSetup();
  await screen.findByText(/Workspace setup was interrupted/);
  expect(screen.queryByText('Your workspace is ready on this computer.')).toBeNull();
});
test('mutation invalidates prior status reads and refreshes after a lost reply', async () => {
  const stale = deferred<unknown>(); const reply = deferred<unknown>(); let polls = 0;
  const ipc = fixture(() => ++polls === 1 ? eligible : polls === 2 ? stale.promise : current, 'failed');
  const original = ipc.getMockImplementation()!;
  ipc.mockImplementation(command => command === 'workspace_upgrade_replace' ? reply.promise : original(command));
  render(<App />); await openLocalSetup();
  fireEvent(window, new Event('focus')); await waitFor(() => expect(polls).toBe(2));
  fireEvent.click(screen.getByRole('button', { name: 'Set up new workspace' }));
  fireEvent.click(screen.getByRole('button', { name: 'Keep old workspace and set up new' }));
  await act(async () => { reply.resolve(null); });
  expect(polls).toBeGreaterThan(2);
  await act(async () => { stale.resolve(eligible); });
  await screen.findByText(/New workspace selected/);
  expect(screen.queryByRole('button', { name: 'Set up new workspace' })).toBeNull();
});
