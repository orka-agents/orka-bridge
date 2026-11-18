import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import Upgrade from './Upgrade';
import type { UpgradeStatus } from './upgrade-api';

const transaction = '10000000-0000-4000-8000-000000000001';
const eligible: UpgradeStatus = { state: 'eligible', transaction: null, createdAt: null, sourceReleases: ['0.1.2'], targetRelease: '0.1.3', retainedAllocatedBytes: 1200000000, additionalBytes: 2400000000, credential: 'eligible', busy: false, error: null, repeated: false, canReplace: true, canRestore: false };
const fresh: UpgradeStatus = { ...eligible, state: 'consumed', transaction, createdAt: 1700000000, canReplace: false, canRestore: true };
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); });
function mount(status = eligible) {
  vi.stubGlobal('isTauri', true); const ipc = vi.fn<(command: string) => unknown>(() => fresh); mockIPC(ipc);
  const onChange = vi.fn(); const onBusy = vi.fn();
  const view = render(<Upgrade status={status} onChange={onChange} onBusy={onBusy} onCheck={() => {}} />);
  return { ...view, ipc, onChange, onBusy };
}
test('new workspace is explicit, measured, and Not now cannot switch or reuse a credential', () => {
  const { ipc } = mount();
  expect(screen.getByText(/1.2 GB/)).toBeTruthy(); expect(screen.getByText(/2.4 GB/)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Set up new workspace' }));
  const confirm = screen.getByRole('group', { name: 'Set up new workspace?' });
  expect(within(confirm).getByText(/Do not open the older version until Restore has completed/)).toBeTruthy();
  expect(within(confirm).getByText(/Old cluster objects, chats and volumes/)).toBeTruthy();
  fireEvent.click(within(confirm).getByRole('button', { name: 'Not now' }));
  expect(ipc).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Set up new workspace' }));
});
test('confirmed replacement does not connect or replay any test and drops old model queues', async () => {
  const { ipc, onBusy, onChange } = mount();
  fireEvent.click(screen.getByRole('button', { name: 'Set up new workspace' }));
  fireEvent.click(screen.getByRole('button', { name: 'Keep old workspace and set up new' }));
  await waitFor(() => expect(onChange).toHaveBeenCalledWith(fresh));
  expect(ipc).toHaveBeenCalledExactlyOnceWith('workspace_upgrade_replace', {});
  expect(onBusy.mock.calls).toEqual([[true], [false]]);
});
test.each(['eligible', 'absent', 'attention', 'transferred', 'removed'] as const)('saved connection handling is explicit for %s', async credential => {
  const { ipc } = mount({ ...fresh, credential });
  if (credential === 'eligible') {
    fireEvent.click(screen.getByRole('button', { name: 'Use saved connection' }));
    await waitFor(() => expect(ipc).toHaveBeenCalledExactlyOnceWith('workspace_upgrade_transfer', { transaction }));
    expect(screen.getByText(/Connect the AI explicitly/)).toBeTruthy();
  } else {
    expect(screen.queryByRole('button', { name: 'Use saved connection' })).toBeNull();
    if (credential === 'attention') expect(screen.getByText(/needs attention/)).toBeTruthy();
    if (credential === 'absent' || credential === 'removed') expect(screen.getByText(/Enter a new sign-in or key in the AI card/)).toBeTruthy();
    if (credential === 'transferred') expect(screen.getByText(/Use Connect in the AI card/)).toBeTruthy();
  }
});
test('restore confirmation names the exact pair and warns that the newer workspace cannot reopen', async () => {
  const { ipc } = mount(fresh);
  expect(screen.queryByText(transaction)).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Restore previous workspace' }));
  const confirm = screen.getByRole('group', { name: 'Restore previous workspace?' });
  expect(screen.queryByRole('button', { name: 'Use saved connection' })).toBeNull();
  expect(within(confirm).getByText(/Restore the workspace.*Bridge 0.1.2/)).toBeTruthy();
  expect(within(confirm).getByText(/Bridge 0.1.3/)).toBeTruthy();
  expect(within(confirm).getByText(/cannot be reopened/)).toBeTruthy();
  expect(within(confirm).getByText(/Removing sign-in in the older app/)).toBeTruthy();
  fireEvent.click(within(confirm).getByRole('button', { name: 'Restore this workspace' }));
  await waitFor(() => expect(ipc).toHaveBeenCalledExactlyOnceWith('workspace_upgrade_restore', { transaction }));
});
test('a fresh workspace cannot change credentials before its first boot completes', () => {
  const { rerender, ipc, onChange, onBusy } = mount({ ...fresh, state: 'fresh' });
  rerender(<Upgrade status={{ ...fresh, state: 'fresh' }} canUseCredential={false} onChange={onChange} onBusy={onBusy} onCheck={() => {}} />);
  const reuse = screen.getByRole('button', { name: 'Use saved connection' });
  expect(reuse.matches(':disabled')).toBe(true);
  fireEvent.click(reuse); expect(ipc).not.toHaveBeenCalled();
  expect(screen.getByRole('button', { name: 'Restore previous workspace' }).matches(':disabled')).toBe(false);
});

test('shared credential removal cannot happen before warning and confirmation', async () => {
  const { ipc } = mount({ ...fresh, credential: 'attention' });
  expect(screen.getByRole('button', { name: 'Remove saved connection' }).classList.contains('button-danger')).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Remove saved connection' }));
  const confirm = screen.getByRole('group', { name: 'Remove saved connection?' });
  expect(within(confirm).getByText(/every retained workspace/)).toBeTruthy();
  expect(ipc).not.toHaveBeenCalled();
  expect(within(confirm).getByRole('button', { name: 'Remove for all retained workspaces' }).classList.contains('button-danger')).toBe(true);
  fireEvent.click(within(confirm).getByRole('button', { name: 'Remove for all retained workspaces' }));
  await waitFor(() => expect(ipc).toHaveBeenCalledExactlyOnceWith('workspace_upgrade_remove', { transaction }));
});
test('interrupted setup retries only its transaction and busy cancellation preserves recovery', async () => {
  const { ipc, rerender, onChange, onBusy } = mount({ ...fresh, state: 'incomplete', canReplace: true });
  fireEvent.click(screen.getByRole('button', { name: 'Try setup again' }));
  fireEvent.click(screen.getByRole('button', { name: 'Keep old workspace and set up new' }));
  await waitFor(() => expect(ipc).toHaveBeenCalledExactlyOnceWith('workspace_upgrade_retry', { transaction }));
  rerender(<Upgrade status={{ ...fresh, state: 'incomplete', canReplace: false, busy: true }} onChange={onChange} onBusy={onBusy} onCheck={() => {}} />);
  fireEvent.click(screen.getByRole('button', { name: 'Cancel update' }));
  await waitFor(() => expect(ipc).toHaveBeenLastCalledWith('workspace_upgrade_cancel', {}));
});
test('Cancel remains usable while this renderer awaits replacement, without releasing its busy gate early', async () => {
  const { ipc, rerender, onChange, onBusy } = mount();
  let finish!: (value: unknown) => void;
  ipc.mockImplementation(command => command === 'workspace_upgrade_replace' ? new Promise(resolve => { finish = resolve; }) : fresh);
  fireEvent.click(screen.getByRole('button', { name: 'Set up new workspace' }));
  fireEvent.click(screen.getByRole('button', { name: 'Keep old workspace and set up new' }));
  rerender(<Upgrade status={{ ...eligible, busy: true }} onChange={onChange} onBusy={onBusy} onCheck={() => {}} />);
  const cancel = screen.getByRole('button', { name: 'Cancel update' });
  expect(cancel.matches(':disabled')).toBe(false); fireEvent.click(cancel);
  await waitFor(() => expect(ipc).toHaveBeenLastCalledWith('workspace_upgrade_cancel', {}));
  expect(onBusy.mock.calls).toEqual([[true]]);
  finish(fresh);
  await waitFor(() => expect(onBusy.mock.calls).toEqual([[true], [false]]));
});

test('quarantined native stop guidance supersedes an earlier generic command failure', async () => {
  const { ipc, rerender, onChange, onBusy } = mount(fresh);
  ipc.mockRejectedValue('private stop error');
  fireEvent.click(screen.getByRole('button', { name: 'Use saved connection' }));
  await screen.findByText('Workspace update information is unavailable. Check again before changing your workspace.');
  rerender(<Upgrade status={{ ...fresh, state: 'conflict', error: 'stopUnconfirmed', canRestore: false }} onChange={onChange} onBusy={onBusy} onCheck={() => {}} />);
  expect(screen.getByRole('alert').textContent).toContain('Quit and reopen');
});

test('interrupted restore offers only exact restore continuation, never another replacement', async () => {
  const { ipc } = mount({ ...fresh, state: 'restoring', credential: 'absent' });
  expect(screen.getByRole('status').textContent).toContain('Restore was interrupted');
  expect(screen.queryByRole('button', { name: /Set up.*workspace/ })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Restore previous workspace' }));
  fireEvent.click(screen.getByRole('button', { name: 'Restore this workspace' }));
  await waitFor(() => expect(ipc).toHaveBeenCalledExactlyOnceWith('workspace_upgrade_restore', { transaction }));
});
test('restored transaction is recovery-only and fresh C is another confirmed setup', () => {
  const { ipc } = mount({ ...fresh, state: 'restored', repeated: true, canReplace: true, canRestore: false });
  expect(screen.queryByRole('button', { name: 'Restore previous workspace' })).toBeNull();
  expect(screen.getByText(/Quit and open Bridge 0.1.2/)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Set up another new workspace' }));
  expect(screen.getByText(/workspace kept from your last update will not be reopened/)).toBeTruthy();
  expect(ipc).not.toHaveBeenCalled();
});
test.each(['network', 'noSpace', 'stopUnconfirmed', 'integrity'] as const)('finite %s failure stays announced with no automatic retry', error => {
  const { ipc } = mount({ ...eligible, error });
  expect(screen.getByRole('alert').textContent).toBeTruthy();
  expect(ipc).not.toHaveBeenCalled();
});
test('a changed transaction dismisses a stale confirmation rather than restoring another pair', () => {
  const { rerender, onChange, onBusy, ipc } = mount(fresh);
  fireEvent.click(screen.getByRole('button', { name: 'Restore previous workspace' }));
  rerender(<Upgrade status={{ ...fresh, transaction: '20000000-0000-4000-8000-000000000002' }} onChange={onChange} onBusy={onBusy} onCheck={() => {}} />);
  expect(screen.queryByRole('button', { name: 'Restore this workspace' })).toBeNull();
  expect(ipc).not.toHaveBeenCalled();
});
