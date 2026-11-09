import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from '../App';

const context = { id: '00000000-0000-4000-8000-000000000001', name: 'team-dev' };
const target = { id: '00000000-0000-4000-8000-000000000002', context_id: context.id, context_name: context.name,
  service_name: 'orka', service_namespace: 'control', watched_namespace: 'agents', image: 'ghcr.io/orka:0.9.4' };
const provider = { id: '00000000-0000-4000-8000-000000000003', name: 'team-model', ready: true };
function native(status: object = { selected: null, localVm: 'idle', model: 'locked' }, use: unknown = { status: 'connected', providers: [provider] }) {
  vi.stubGlobal('isTauri', true);
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_status') return status;
    if (command === 'setup_requirements') return null;
    if (command === 'cluster_contexts') return [context];
    if (command === 'cluster_discover') return [target];
    if (command === 'cluster_use') return use;
    throw Error('Unsupported');
  });
  mockIPC(ipc); return ipc;
}
async function open() {
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'I already run Orka on a cluster' }));
  await screen.findByRole('combobox', { name: 'kubectl context' });
  return user;
}
async function discover(user: ReturnType<typeof userEvent.setup>) {
  await user.selectOptions(screen.getByRole('combobox', { name: 'kubectl context' }), context.id);
  await screen.findByText('Orka found');
}
beforeEach(() => localStorage.clear());
afterEach(() => { cleanup(); clearMocks(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

test('local readiness never enables remote checks, and changing places keeps the VM', async () => {
  const ipc = native({ selected: 'localVm', localVm: 'ready', model: 'available' });
  ipc.mockImplementation(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'setup_requirements') return null;
    if (command === 'cluster_contexts') return [context];
    if (command === 'cluster_discover') return [target];
    if (command === 'agent_open') return { agent: '---\\nname: reader\\n---\\nRead.', tests: 'Checks' };
    throw Error('Unsupported');
  });
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Change where agents run' }));
  await user.click(screen.getByRole('button', { name: 'Use a cluster you already have instead' }));
  await screen.findByRole('combobox', { name: 'kubectl context' });
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  const run = await screen.findByRole('button', { name: 'Run checks' });
  expect(run.matches(':disabled')).toBe(true);
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  expect(screen.getByRole('combobox', { name: 'kubectl context' })).toBeTruthy();
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('setup_change_local');
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('setup_run_example');
});

test('a selected unavailable workspace can choose a cluster without resetting it', async () => {
  const ipc = native({ selected: 'localVm', localVm: 'notIntegrated', model: 'locked' });
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Use a cluster you already have instead' }));
  expect(await screen.findByRole('combobox', { name: 'kubectl context' })).toBeTruthy();
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('setup_change_local');
});

test('a failed account attempt keeps the native-requested name field for correction', async () => {
  const ipc = native(undefined, { status: 'needServiceAccount', providers: [] });
  const user = await open(); await discover(user);
  await user.click(screen.getByRole('button', { name: 'Use this cluster' }));
  const account = await screen.findByRole('textbox', { name: 'ServiceAccount name' });
  await user.type(account, 'misspelled');
  ipc.mockImplementation(command => command === 'cluster_use' ? { status: 'unavailable', providers: [] } : Promise.reject('Unsupported'));
  await user.click(screen.getByRole('button', { name: 'Use this cluster' }));
  await screen.findByRole('alert');
  expect(screen.getByRole('textbox', { name: 'ServiceAccount name' })).toBe(account);
});

test('local results remain explicitly local after confirming cluster access', async () => {
  native();
  mockIPC(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'setup_requirements') return null;
    if (command === 'agent_open') return { agent: '---\\nname: reader\\n---\\nRead.', tests: 'Checks' };
    if (command === 'setup_run_status') return { state: 'complete', canRun: true, report: { verdict: 'passed', cases: [{ id: 'check', phase: 'Succeeded', verdict: 'passed', answer: 'yes', completedTools: [], assertions: [{ id: 'yes', kind: 'contains', verdict: 'passed' }] }] }, acknowledged: [], pending: null, error: null };
    if (command === 'cluster_contexts') return [context];
    if (command === 'cluster_discover') return [target];
    if (command === 'cluster_use') return { status: 'connected', providers: [provider] };
    throw Error('Unsupported');
  });
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Designer' }));
  await screen.findByText('1 of 1 checks passing', { selector: '.checks-status' });
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  await user.click(screen.getByRole('button', { name: 'Change where agents run' }));
  await user.click(screen.getByRole('button', { name: 'Use a cluster you already have instead' }));
  await screen.findByRole('combobox', { name: 'kubectl context' }); await discover(user);
  await user.click(screen.getByRole('button', { name: 'Use this cluster' }));
  await screen.findAllByText('Access confirmed');
  expect(screen.getByText('This computer: 1 of 1 checks passing', { selector: '.checks-status' })).toBeTruthy();
});

test('expert_skip_selects_cluster_without_vm', async () => {
  const ipc = native(); await open();
  expect(screen.getByRole('radio', { name: 'On a cluster you already have' }).matches(':checked')).toBe(true);
  expect(localStorage.getItem('bridge.welcomeSeen.v1')).toBe('1');
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('setup_select_local');
});
test('v6_cluster_card_keyboard and Use confirms access only after discovery', async () => {
  const ipc = native(); const user = await open();
  const select = screen.getByRole('combobox', { name: 'kubectl context' }); select.focus();
  await user.selectOptions(select, context.id); await screen.findByText('Orka found');
  expect(screen.getByText('ghcr.io/orka:0.9.4')).toBeTruthy();
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('cluster_use');
  const use = screen.getByRole('button', { name: 'Use this cluster' }); use.focus(); await user.keyboard('{Enter}');
  expect((await screen.findAllByText('Access confirmed')).length).toBeGreaterThan(0);
  expect(ipc.mock.calls.filter(([c]) => c === 'cluster_use')).toEqual([['cluster_use', { targetId: target.id, saName: null }]]);
  await user.click(screen.getByRole('button', { name: 'Show details' }));
  expect(screen.getByText('control / orka')).toBeTruthy(); expect(screen.getByText('agents')).toBeTruthy();
  expect(screen.getByText('team-model')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Send to this cluster' }).matches(':disabled')).toBe(true);
});
test('native_need_account_only_then_name_field', async () => {
  const ipc = native(undefined, { status: 'needServiceAccount', providers: [] }); const user = await open(); await discover(user);
  expect(screen.queryByRole('textbox', { name: 'ServiceAccount name' })).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Use this cluster' }));
  const account = await screen.findByRole('textbox', { name: 'ServiceAccount name' });
  expect(document.activeElement).toBe(account); await user.type(account, 'bridge-user');
  await user.click(screen.getByRole('button', { name: 'Use this cluster' }));
  expect(ipc.mock.calls.filter(([c]) => c === 'cluster_use').at(-1)).toEqual(['cluster_use', { targetId: target.id, saName: 'bridge-user' }]);
});
test.each(['unsupportedCredential', 'denied', 'unavailable'])('unsupported_access_stays_visible: %s', async status => {
  native(undefined, { status, providers: [] }); const user = await open(); await discover(user);
  await user.click(screen.getByRole('button', { name: 'Use this cluster' }));
  expect(await screen.findByRole('alert')).toBeTruthy(); expect(screen.getByText('Orka found')).toBeTruthy();
  expect(screen.queryByRole('textbox', { name: 'ServiceAccount name' })).toBeNull();
});
test('failure alternative does not stop or reset a local VM', async () => {
  const ipc = native({ selected: 'localVm', localVm: 'failed', localVmError: 'vmStart: unavailable', model: 'locked' });
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Use a cluster you already have instead' }));
  await screen.findByRole('combobox', { name: 'kubectl context' });
  await waitFor(() => expect(ipc.mock.calls.some(([c]) => c === 'setup_status')).toBe(true));
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('setup_change_local');
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('setup_cancel_local');
  await user.click(screen.getByRole('radio', { name: 'On this computer' }));
  expect(await screen.findByRole('button', { name: 'Try again' })).toBeTruthy();
});
test('unsupported discovery names the access limitation and keeps the context', async () => {
  native(); mockIPC(command => command === 'setup_status' ? { selected: null, localVm: 'idle', model: 'locked' }
    : command === 'cluster_contexts' ? [context] : Promise.reject('unsupportedCredential'));
  const user = await open(); await user.selectOptions(screen.getByRole('combobox', { name: 'kubectl context' }), context.id);
  expect(await screen.findByText(/sign-in method Bridge cannot use yet/)).toBeTruthy();
  expect((screen.getByRole('combobox', { name: 'kubectl context' }) as HTMLSelectElement).value).toBe(context.id);
});
test('empty discovery is not an access error or a made-up version', async () => {
  native(); mockIPC(command => command === 'setup_status' ? { selected: null, localVm: 'idle', model: 'locked' }
    : command === 'cluster_contexts' ? [context] : command === 'cluster_discover' ? [] : Promise.reject('Unsupported'));
  const user = await open(); await user.selectOptions(screen.getByRole('combobox', { name: 'kubectl context' }), context.id);
  expect(await screen.findByText('No Orka was found on this context.')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Use this cluster' })).toBeNull();
});
