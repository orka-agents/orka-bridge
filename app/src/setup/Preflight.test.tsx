import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import Setup from './Setup';

const requirements = { memoryBytes: 6442450944, imageBytes: 3651994962, stateDiskBytes: 34359738368 };
const ready = { platform: 'windows', virtualization: 'ready', memory: 'ready', installedMemoryBytes: 8589934592, requiredMemoryBytes: 6442450944 };
function native(preflight: unknown = ready, integrated = true) {
  vi.stubGlobal('isTauri', true);
  localStorage.setItem('bridge.welcomeSeen.v1', '1');
  let value = preflight;
  const ipc = vi.fn(command => {
    if (command === 'setup_status') return { selected: null, localVm: integrated ? 'idle' : 'notIntegrated', model: 'locked' };
    if (command === 'setup_requirements') return integrated ? requirements : null;
    if (command === 'setup_preflight') return value;
    if (command === 'setup_select_local') return { selected: 'localVm', localVm: 'starting', model: 'locked' };
    if (command === 'setup_open_help') return Promise.reject('SECRET_MARKER');
    if (command === 'cluster_contexts') return [];
    throw Error('unexpected command');
  });
  mockIPC(ipc);
  render(<Setup onOpenExample={() => {}} onReady={() => {}} />);
  return { ipc, update: (next: unknown) => { value = next; } };
}
afterEach(() => { cleanup(); clearMocks(); vi.restoreAllMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

test('insufficient installed memory blocks preparation, explains RAM, and can be rechecked without starting', async () => {
  const { ipc, update } = native({ ...ready, memory: 'insufficient', installedMemoryBytes: 4294967296 });
  const user = userEvent.setup();
  const panel = await screen.findByRole('region', { name: 'Computer requirements' });
  await waitFor(() => expect(screen.getByRole('button', { name: 'Continue' }).matches(':disabled')).toBe(true));
  const memory = within(panel).getByRole('region', { name: 'Installed memory' });
  expect(memory.textContent).toContain('4 GiB');
  await user.click(within(memory).getByRole('button', { name: 'Show me how' }));
  expect(memory.textContent).toMatch(/Closing apps does not increase installed memory/);
  expect(memory.textContent).toMatch(/cluster/);
  expect(ipc.mock.calls.some(([c]) => c === 'setup_select_local')).toBe(false);
  update(ready);
  await user.click(screen.getByRole('button', { name: 'Check again' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Continue' }).matches(':disabled')).toBe(false));
  expect(ipc.mock.calls.filter(([c]) => c === 'setup_preflight')).toHaveLength(2);
  expect(ipc.mock.calls.some(([c]) => c === 'setup_select_local')).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  expect(await screen.findByText('Starting your workspace…')).toBeTruthy();
  expect(ipc.mock.calls.filter(([c]) => c === 'setup_select_local')).toHaveLength(1);
});

test('virtualization failure offers native fixed-topic help, sanitized copy, and a usable cluster alternative', async () => {
  const { ipc } = native({ ...ready, virtualization: 'disabled' });
  const user = userEvent.setup();
  const virtualization = await screen.findByRole('region', { name: 'Virtualization' });
  await user.click(within(virtualization).getByRole('button', { name: 'Show me how' }));
  expect(ipc.mock.calls.find(([c]) => c === 'setup_open_help')).toEqual(['setup_open_help', { topic: 'virtualization' }]);
  expect(await screen.findByText(/Couldn't open help/)).toBeTruthy();
  const copy = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue();
  await user.click(screen.getByRole('button', { name: 'Copy details for help' }));
  expect(copy.mock.calls[0][0]).toContain('disabled');
  expect(copy.mock.calls[0][0]).not.toMatch(/SECRET_MARKER|C:\\/);
  await user.click(screen.getByRole('radio', { name: 'On a cluster you already have' }));
  expect(await screen.findByLabelText('kubectl context')).toBeTruthy();
  expect(ipc.mock.calls.some(([c]) => c === 'setup_select_local')).toBe(false);
});

test.each([null, { ...ready, extra: 'SECRET_MARKER' }, { ...ready, requiredMemoryBytes: 1 }, { ...ready, installedMemoryBytes: -1 }, { ...ready, memory: 'SECRET_MARKER' }])('unreadable observations settle to unknown and defer to native launch: %j', async value => {
  native(value);
  expect(await screen.findAllByText(/Couldn't check/)).toHaveLength(2);
  await waitFor(() => expect(screen.getByRole('button', { name: 'Continue' }).matches(':disabled')).toBe(false));
  expect(document.body.textContent).not.toContain('SECRET_MARKER');
});

test('Mac unsupported virtualization explains platform support without Windows help', async () => {
  const { ipc } = native({ ...ready, platform: 'macos', virtualization: 'unsupported' });
  const user = userEvent.setup();
  const virtualization = await screen.findByRole('region', { name: 'Virtualization' });
  await user.click(within(virtualization).getByRole('button', { name: 'Show me how' }));
  expect(virtualization.textContent).toMatch(/Mac|cluster/);
  expect(ipc.mock.calls.some(([c]) => c === 'setup_open_help')).toBe(false);
  expect(screen.getByRole('button', { name: 'Continue' }).matches(':disabled')).toBe(true);
});

test('unavailable builds never query or claim host readiness', async () => {
  const { ipc } = native(ready, false);
  await screen.findByText(/Workspace setup is not included/);
  expect(screen.queryByRole('region', { name: 'Computer requirements' })).toBeNull();
  expect(ipc.mock.calls.some(([c]) => c === 'setup_preflight')).toBe(false);
});

test('a pending refresh disables Continue and an old request cannot survive leaving the local choice', async () => {
  let complete!: (value: unknown) => void;
  const { ipc } = native(new Promise(resolve => { complete = resolve; }));
  const user = userEvent.setup();
  await screen.findByRole('region', { name: 'Computer requirements' });
  expect(screen.getByRole('button', { name: 'Continue' }).matches(':disabled')).toBe(true);
  await user.click(screen.getByRole('radio', { name: 'On a cluster you already have' }));
  complete({ ...ready, virtualization: 'disabled' });
  await screen.findByLabelText('kubectl context');
  expect(screen.queryByRole('region', { name: 'Computer requirements' })).toBeNull();
  expect(ipc.mock.calls.some(([c]) => c === 'setup_select_local')).toBe(false);
});

test('a direct native preflight failure is decoded and explained without raw diagnostic text', async () => {
  vi.stubGlobal('isTauri', true);
  mockIPC(command => command === 'setup_status' ? { selected: 'localVm', localVm: 'failed', model: 'locked', preflightFailure: 'insufficientMemory', localVmError: 'SECRET_MARKER RuntimeError::Preflight' } : command === 'setup_requirements' ? requirements : null);
  render(<Setup onOpenExample={() => {}} onReady={() => {}} />);
  expect(await screen.findByText(/less installed memory/)).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Try again' })).toBeTruthy();
  expect(document.body.textContent).not.toContain('SECRET_MARKER');
});
