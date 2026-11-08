import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import App from '../App';

const status = (selected: 'localVm' | null = null) => ({ selected, localVm: 'notIntegrated', model: 'locked' });
function native() {
  vi.stubGlobal('isTauri', true);
  let selected: 'localVm' | null = null;
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_status') return status(selected);
    if (command === 'setup_select_local') { selected = 'localVm'; return status(selected); }
    if (command === 'engine_call') throw { kind: 'unavailable' };
    throw new Error('Forbidden command: ' + command);
  });
  mockIPC(ipc);
  return ipc;
}
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); });

test('first launch is a native-backed chooser with no runtime or credential effects', async () => {
  const ipc = native();
  const fetch = vi.fn(); vi.stubGlobal('fetch', fetch);
  render(<App />);
  expect(screen.getByRole('main', { name: 'Clusters' })).toBeTruthy();
  expect(await screen.findByText('Native setup available')).toBeTruthy();
  expect(screen.getByText('VM integration is not included in this build.')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Connect model' }).hasAttribute('disabled')).toBe(true);
  const existing = screen.getByRole('radio', { name: /Existing cluster/ });
  expect(existing.hasAttribute('disabled')).toBe(true);
  const before = ipc.mock.calls.length;
  fireEvent.click(existing);
  expect(ipc).toHaveBeenCalledTimes(before);
  expect(ipc.mock.calls.map(call => call[0])).not.toContain('setup_select_local');
  expect(ipc.mock.calls.every(call => ['setup_status', 'engine_call'].includes(call[0]))).toBe(true);
  expect(fetch).not.toHaveBeenCalled();
  expect(screen.queryByRole('textbox')).toBeNull();
});

test('only the choice is retained across app remount, never runtime or model readiness', async () => {
  const ipc = native(); const user = userEvent.setup();
  const first = render(<App />);
  await screen.findByText('Native setup available');
  await user.click(screen.getByRole('radio', { name: /Local VM/ }));
  expect(await screen.findByText('Local VM selected.')).toBeTruthy();
  first.unmount(); render(<App />);
  await screen.findByText('Local VM selected.');
  expect((screen.getByRole('radio', { name: /Local VM/ }) as HTMLInputElement).checked).toBe(true);
  expect(screen.getByText('VM integration is not included in this build.')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Connect model' }).hasAttribute('disabled')).toBe(true);
  expect(ipc.mock.calls.filter(call => call[0] === 'setup_select_local')).toEqual([['setup_select_local', {}]]);
});

test('browser preview never pretends that native persistence or integration works', async () => {
  vi.stubGlobal('isTauri', false);
  const ipc = vi.fn(); mockIPC(ipc);
  render(<App />);
  expect(await screen.findByText('Native setup is not available in this preview.')).toBeTruthy();
  expect(screen.queryByText('Native setup available')).toBeNull();
  expect(screen.getByRole('radio', { name: /Local VM/ }).matches(':disabled')).toBe(true);
  expect(ipc).not.toHaveBeenCalled();
});

test('unsupported native replies cannot claim availability or leak extra data', async () => {
  native(); mockIPC(command => command === 'setup_status'
    ? { ...status(), model: 'ready', credential: 'SECRET_MARKER' } : Promise.reject({ kind: 'unavailable' }));
  render(<App />);
  expect(await screen.findByRole('alert')).toBeTruthy();
  expect(screen.queryByText('Native setup available')).toBeNull();
  expect(document.body.textContent).not.toContain('SECRET_MARKER');
  expect(screen.getByRole('button', { name: 'Connect model' }).hasAttribute('disabled')).toBe(true);
});

test('save failures do not mark the choice saved and never render rejected paths', async () => {
  native(); mockIPC(command => command === 'setup_status' ? status()
    : Promise.reject({ kind: 'storage', message: '/private/SECRET_MARKER' }));
  const user = userEvent.setup(); render(<App />);
  await screen.findByText('Native setup available');
  await user.click(screen.getByRole('radio', { name: /Local VM/ }));
  expect(await screen.findByRole('alert')).toBeTruthy();
  expect(screen.getByText('Setup choice unavailable.')).toBeTruthy();
  expect(screen.queryByText('No runtime selected.')).toBeNull();
  expect(screen.queryByText('Local VM selected.')).toBeNull();
  expect(document.body.textContent).not.toContain('SECRET_MARKER');
});

test('a failed initial read can be checked again without changing the saved choice', async () => {
  native(); let reads = 0;
  mockIPC(command => command === 'setup_status'
    ? ++reads === 1 ? Promise.reject('SECRET_MARKER') : status('localVm')
    : Promise.reject({ kind: 'unavailable' }));
  const user = userEvent.setup(); render(<App />);
  expect(await screen.findByRole('alert')).toBeTruthy();
  expect(screen.getByText('Setup choice unavailable.')).toBeTruthy();
  expect(screen.queryByText('No runtime selected.')).toBeNull();
  expect(screen.queryByText('Local VM selected.')).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Check again' }));
  expect(await screen.findByText('Local VM selected.')).toBeTruthy();
  expect(screen.queryByRole('alert')).toBeNull();
});

test('selection shows real startup states and only a ready session can open the example', async () => {
  native(); let selected: 'localVm' | null = null; let reads = 0;
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_select_local') { selected = 'localVm'; return { ...status(selected), localVm: 'preparing' }; }
    if (command === 'setup_status') return { ...status(selected), localVm: selected === null ? 'idle' : ++reads === 1 ? 'starting' : 'ready' };
    if (command === 'setup_example') return { agent: 'exact agent source', tests: 'exact test source' };
    throw { kind: 'unavailable' };
  }); mockIPC(ipc);
  const user = userEvent.setup(); render(<App />);
  await screen.findByText('Native setup available');
  expect(screen.queryByRole('button', { name: 'Open example' })).toBeNull();
  await user.click(screen.getByRole('radio', { name: /Local VM/ }));
  expect(screen.queryByText('Orka is running on this computer.')).toBeNull();
  expect(await screen.findByText('Orka is running on this computer.')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Connect model' }).matches(':disabled')).toBe(true);
  expect(ipc.mock.calls.filter(([name]) => name === 'setup_select_local')).toHaveLength(1);
  await user.click(screen.getByRole('button', { name: 'Open example' }));
  expect(await screen.findByText('exact agent source')).toBeTruthy();
  expect(screen.getByText('exact test source')).toBeTruthy();
  expect(ipc.mock.calls.every(([name]) => ['setup_status', 'setup_select_local', 'setup_example', 'engine_call'].includes(name))).toBe(true);
});

test.each([
  'Starting the local VM failed: Runtime operation failed: Io(<b data-vm-error>launch detail</b>)',
  'Stopping the local VM failed: Runtime operation failed: Io(<b data-vm-error>stop detail</b>)',
])('shows retained failure reason as plain text without new actions: %s', async localVmError => {
  native();
  mockIPC(command => command === 'setup_status'
    ? { ...status('localVm'), localVm: 'failed', localVmError }
    : Promise.reject({ kind: 'unavailable' }));
  render(<App />);
  expect(await screen.findByText(localVmError)).toBeTruthy();
  expect(document.querySelector('[data-vm-error]')).toBeNull();
  expect(screen.getByText('Local VM selected.')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Open example' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Check again' })).toBeNull();
  expect(screen.getByRole('button', { name: 'Connect model' }).matches(':disabled')).toBe(true);
});

test('keyboard selection does not enable the existing route or duplicate the save', async () => {
  const ipc = native(); const user = userEvent.setup(); render(<App />);
  await screen.findByText('Native setup available');
  screen.getByRole('radio', { name: /Local VM/ }).focus();
  await user.keyboard(' ');
  await screen.findByText('Local VM selected.');
  await user.keyboard('{ArrowRight} ');
  expect(ipc.mock.calls.filter(call => call[0] === 'setup_select_local')).toHaveLength(1);
  expect(screen.getByRole('radio', { name: /Existing cluster/ }).matches(':disabled')).toBe(true);
});

test('late responses cannot update another page or mark an unacknowledged choice saved', async () => {
  native(); let complete!: (value: unknown) => void;
  mockIPC(command => command === 'setup_status' ? status() : command === 'setup_select_local'
    ? new Promise(resolve => { complete = resolve; }) : Promise.reject({ kind: 'unavailable' }));
  const user = userEvent.setup(); render(<App />);
  await screen.findByText('Native setup available');
  await user.click(screen.getByRole('radio', { name: /Local VM/ }));
  expect(screen.queryByText('Local VM selected.')).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  complete(status('localVm'));
  await waitFor(() => expect(screen.getByRole('main', { name: 'Designer' })).toBeTruthy());
  expect(screen.queryByText('Local VM selected.')).toBeNull();
});
