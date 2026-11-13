import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from '../App';
import Setup from './Setup';

const status = (selected: 'localVm' | null = null) => ({ selected, localVm: 'notIntegrated', model: 'locked' });
const unavailable = 'Workspace setup is not included in this build. Nothing will be downloaded or started.';
function native() {
  vi.stubGlobal('isTauri', true);
  let selected: 'localVm' | null = null;
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_status') return status(selected);
    if (command === 'setup_requirements') return null;
    if (command === 'setup_select_local') { selected = 'localVm'; return status(selected); }
    throw new Error('Forbidden command: ' + command);
  });
  mockIPC(ipc);
  return ipc;
}
beforeEach(() => { localStorage.setItem('bridge.welcomeSeen.v1', '1'); });
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

test('reopen observes a lost workspace immediately without replaying setup, Connect or Test', async () => {
  vi.stubGlobal('isTauri', true);
  let lost = false;
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: lost ? 'failed' : 'ready', model: lost ? 'locked' : 'available' };
    if (command === 'setup_requirements') return null;
    if (command === 'setup_connect_model') return { ready: true };
    throw { kind: 'unavailable' };
  });
  mockIPC(ipc); const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Connect the AI' }));
  expect(await screen.findByText('Connected with GitHub Copilot.')).toBeTruthy();
  lost = true;
  await act(async () => { window.dispatchEvent(new Event('focus')); });
  expect(screen.getByText("Your workspace couldn't start")).toBeTruthy();
  expect(screen.queryByText('Your workspace is running on this computer.')).toBeNull();
  expect(ipc.mock.calls.filter(([name]) => name === 'setup_connect_model')).toHaveLength(1);
  expect(ipc.mock.calls.some(([name]) => ['setup_select_local', 'agent_run'].includes(name))).toBe(false);
});

test('running native session unlocks only its explicit model command', async () => {
  vi.stubGlobal('isTauri', true);
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'setup_connect_model') return { ready: true };
    throw new Error('Forbidden command');
  });
  const user = userEvent.setup(); mockIPC(ipc); render(<App />);
  const connect = await screen.findByRole('button', { name: 'Connect the AI' });
  expect(connect.hasAttribute('disabled')).toBe(false);
  expect(ipc.mock.calls.some(([command]) => command === 'setup_connect_model')).toBe(false);
  await user.click(connect);
  expect(await screen.findByText('Connected with GitHub Copilot.')).toBeTruthy();
  expect(ipc.mock.calls.filter(([command]) => command === 'setup_connect_model')).toEqual([['setup_connect_model', {}]]);
});
test('failed Connect refreshes retained guest details without locking the running VM', async () => {
  vi.stubGlobal('isTauri', true);
  let failed = false;
  const detail = 'Guest Connect: gateway, timeout, 60000 ms. Stderr withheld.';
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available', localVmError: failed ? detail : null };
    if (command === 'engine_call') throw { kind: 'unavailable' };
    if (command === 'setup_connect_model') { failed = true; throw { kind: 'modelConnect', message: 'SECRET_MARKER' }; }
    throw new Error('Forbidden command');
  });
  const user = userEvent.setup(); mockIPC(ipc); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Connect the AI' }));
  expect(screen.queryByText(detail)).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Show details' }));
  expect(await screen.findByText(detail)).toBeTruthy();
  expect(screen.getByText('Your workspace is running on this computer.')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Check again' }).matches(':disabled')).toBe(false);
  expect(document.body.textContent).not.toContain('SECRET_MARKER');
  expect(ipc.mock.calls.filter(([command]) => command === 'setup_connect_model')).toHaveLength(1);
});

test('the chooser only reads setup and build information, without runtime or credential effects', async () => {
  const ipc = native(); const fetch = vi.fn(); vi.stubGlobal('fetch', fetch); render(<App />);
  expect(await screen.findByRole('heading', { name: 'Where should your agents run?' })).toBeTruthy();
  expect(screen.getByText(unavailable)).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Connect the AI' })).toBeNull();
  const existing = screen.getByRole('radio', { name: 'On a cluster you already have' });
  expect(existing.matches(':disabled')).toBe(false);
  expect(ipc.mock.calls.every(call => ['setup_status', 'setup_requirements', 'engine_call'].includes(call[0]))).toBe(true);
  expect(fetch).not.toHaveBeenCalled(); expect(screen.queryByRole('textbox')).toBeNull();
});

test('remount observes the native choice without inventing workspace or model readiness', async () => {
  const ipc = native(); const user = userEvent.setup(); const first = render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Continue' }));
  expect(await screen.findByRole('region', { name: 'Workspace setup' })).toBeTruthy();
  first.unmount(); render(<App />);
  await screen.findByRole('region', { name: 'Workspace setup' });
  expect(screen.getByText(unavailable)).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Connect the AI' })).toBeNull();
  expect(ipc.mock.calls.filter(call => call[0] === 'setup_select_local')).toEqual([['setup_select_local', {}]]);
});

test.each([null, 'attempted'])('a lost selection reply preserves durable intent (%s) and resumes only observation across remount', async previous => {
  vi.stubGlobal('isTauri', true);
  if (previous) localStorage.setItem('bridge.firstTest.v1', previous);
  let selected = false; let reads = 0; let loseReply!: () => void;
  let intentAtSelection: string | null = null;
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_status') return { selected: selected ? 'localVm' : null, localVm: !selected ? 'idle' : ++reads === 1 ? 'starting' : 'ready', model: selected && reads > 1 ? 'available' : 'locked' };
    if (command === 'setup_requirements') return null;
    if (command === 'setup_select_local') {
      intentAtSelection = localStorage.getItem('bridge.firstTest.v1'); selected = true;
      return new Promise((_, reject) => { loseReply = () => reject(Error('Lost reply')); });
    }
    throw Error('Forbidden command: ' + command);
  });
  mockIPC(ipc); const user = userEvent.setup(); const view = render(<Setup onOpenExample={vi.fn()} />);
  await user.click(await screen.findByRole('button', { name: 'Continue' }));
  expect(intentAtSelection).toBe(previous ?? 'pending');
  await act(async () => { loseReply(); });
  await screen.findByText('Your workspace is running on this computer.', {}, { timeout: 2000 });
  view.unmount(); render(<Setup onOpenExample={vi.fn()} />);
  await screen.findByText('Your workspace is running on this computer.');
  expect(localStorage.getItem('bridge.firstTest.v1')).toBe(previous ?? 'pending');
  expect(ipc.mock.calls.filter(([command]) => command === 'setup_select_local')).toHaveLength(1);
  expect(ipc.mock.calls.every(([command]) => ['setup_status', 'setup_requirements', 'setup_example', 'setup_select_local'].includes(command))).toBe(true);
});

test('browser preview never pretends that native persistence or integration works', async () => {
  vi.stubGlobal('isTauri', false); const ipc = vi.fn(); mockIPC(ipc); render(<App />);
  expect(await screen.findByText('Workspace setup is not available in this preview.')).toBeTruthy();
  expect(screen.getByRole('radio', { name: 'On this computer' }).matches(':disabled')).toBe(true);
  expect(screen.getByRole('button', { name: 'Continue' }).matches(':disabled')).toBe(true);
  expect(ipc).not.toHaveBeenCalled();
});

test('unsupported native replies cannot claim availability or leak extra data', async () => {
  native(); mockIPC(command => command === 'setup_status'
    ? { ...status(), model: 'ready', credential: 'SECRET_MARKER' } : Promise.reject({ kind: 'unavailable' }));
  render(<App />); expect(await screen.findByRole('alert')).toBeTruthy();
  expect(document.body.textContent).not.toContain('SECRET_MARKER');
  expect(screen.getByRole('button', { name: 'Continue' }).matches(':disabled')).toBe(true);
});

test('save failures do not mark the choice saved and never render rejected paths', async () => {
  native(); mockIPC(command => command === 'setup_status' ? status() : command === 'setup_requirements' ? null
    : Promise.reject({ kind: 'storage', message: '/private/SECRET_MARKER' }));
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Continue' }));
  expect(await screen.findByRole('alert')).toBeTruthy();
  expect(screen.queryByRole('region', { name: 'Workspace setup' })).toBeNull();
  expect(document.body.textContent).not.toContain('SECRET_MARKER');
});

test('a failed initial read can be checked again without changing the saved choice', async () => {
  native(); let reads = 0;
  mockIPC(command => command === 'setup_status'
    ? ++reads === 1 ? Promise.reject('SECRET_MARKER') : status('localVm') : Promise.reject({ kind: 'unavailable' }));
  const user = userEvent.setup(); render(<App />);
  expect(await screen.findByRole('alert')).toBeTruthy();
  expect(screen.queryByRole('region', { name: 'Workspace setup' })).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Check again' }));
  expect(await screen.findByRole('region', { name: 'Workspace setup' })).toBeTruthy();
  expect(screen.queryByRole('alert')).toBeNull();
});

test('Continue shows real startup states and a ready session can open the owned example', async () => {
  native(); let selected: 'localVm' | null = null; let reads = 0;
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_select_local') { selected = 'localVm'; return { ...status(selected), localVm: 'preparing' }; }
    if (command === 'setup_status') return { ...status(selected), localVm: selected === null ? 'idle' : ++reads === 1 ? 'starting' : 'ready' };
    if (command === 'setup_requirements') return null;
    if (command === 'agent_open') return { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', name: 'web-reader', agent: 'exact agent source', tests: 'exact test source' };
    if (command === 'agent_run_status') return { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', busy: null, state: 'idle', canRun: true, report: null, acknowledged: [], pending: null, error: null };
    if (command === 'send_history') return [];
    throw { kind: 'unavailable' };
  }); mockIPC(ipc);
  const user = userEvent.setup(); render(<App />);
  await screen.findByRole('button', { name: 'Continue' });
  expect(screen.queryByRole('button', { name: 'Read its instructions' })).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  expect(screen.queryByText('Your workspace is running on this computer.')).toBeNull();
  expect(await screen.findByText('Your workspace is running on this computer.')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Connect the AI' }).matches(':disabled')).toBe(true);
  expect(ipc.mock.calls.filter(([name]) => name === 'setup_select_local')).toHaveLength(1);
  await user.click(screen.getByRole('button', { name: 'Read its instructions' }));
  expect(await screen.findByText('exact agent source')).toBeTruthy();
  expect(screen.getByText('exact test source')).toBeTruthy();
  expect(ipc.mock.calls.every(([name]) => ['setup_status', 'setup_requirements', 'setup_select_local', 'setup_example', 'agent_open', 'agent_run_status', 'send_status', 'send_history', 'engine_call'].includes(name))).toBe(true);
});

test.each([
  'Starting the local VM failed: Runtime operation failed: Io(<b data-vm-error>launch detail</b>)',
  'Stopping the local VM failed: Runtime operation failed: Io(<b data-vm-error>stop detail</b>)',
])('keeps retained failure reason plain and behind disclosure: %s', async localVmError => {
  native(); mockIPC(command => command === 'setup_status'
    ? { ...status('localVm'), localVm: 'failed', localVmError } : Promise.reject({ kind: 'unavailable' }));
  const user = userEvent.setup(); render(<App />);
  expect((await screen.findByRole('alert')).textContent).toContain('Your workspace and agent files are kept.');
  expect(screen.getByRole('button', { name: 'Try again' })).toBeTruthy();
  expect(screen.queryByText(localVmError)).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Show details' }));
  expect(screen.getByText(localVmError)).toBeTruthy();
  expect(document.querySelector('[data-vm-error]')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Read its instructions' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Check again' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Connect the AI' })).toBeNull();
});

test('a delayed Continue reply stays pending across repeat activation and navigation', async () => {
  native(); let complete!: (value: unknown) => void; let selected: 'localVm' | null = null;
  const ipc = vi.fn(command => command === 'setup_status' ? status(selected) : command === 'setup_select_local'
    ? new Promise(resolve => { complete = resolve; }) : Promise.reject({ kind: 'unavailable' }));
  mockIPC(ipc);
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Continue' }));
  expect(screen.getByRole('button', { name: 'Continue' }).matches(':disabled')).toBe(true);
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  const designer = screen.getByRole('button', { name: 'Designer' });
  await user.click(designer); selected = 'localVm'; complete(status(selected));
  await waitFor(() => expect(document.querySelector('[aria-label="Workspace setup"]')?.closest('[hidden]')).toBeTruthy());
  expect(screen.getByRole('main', { name: 'Designer' })).toBeTruthy();
  expect(document.activeElement).toBe(designer);
  expect(screen.queryByRole('region', { name: 'Workspace setup' })).toBeNull();
  expect(ipc.mock.calls.filter(([command]) => command === 'setup_select_local')).toEqual([['setup_select_local', {}]]);
  expect(ipc.mock.calls.some(([command]) => command === 'setup_connect_model')).toBe(false);
});

function nativeDirect() {
  vi.stubGlobal('isTauri', true);
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_status') return { selected: null, localVm: 'idle', model: 'locked' };
    if (command === 'setup_requirements') return null;
    if (command === 'cluster_contexts') return [{ id: '00000000-0000-4000-8000-000000000001', name: 'team-dev' }];
    throw Error('Unsupported');
  });
  mockIPC(ipc);
  return ipc;
}

test('without a place prop Setup keeps its own local/cluster toggle (compatibility default)', async () => {
  const ipc = nativeDirect(); const user = userEvent.setup();
  render(<Setup onOpenExample={vi.fn()} />);
  await screen.findByRole('radio', { name: 'On this computer' });
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('cluster_contexts');
  await user.click(screen.getByRole('radio', { name: 'On a cluster you already have' }));
  expect(await screen.findByRole('combobox', { name: 'kubectl context' })).toBeTruthy();
  await user.click(screen.getByRole('radio', { name: 'On this computer' }));
  expect(screen.queryByRole('combobox', { name: 'kubectl context' })).toBeNull();
});

test('a place prop derives the radio selection and keeps ClusterCard mounted hidden, not refetched, after first use', async () => {
  const ipc = nativeDirect();
  const onPlace = vi.fn();
  const view = render(<Setup onOpenExample={vi.fn()} place={{ kind: 'thisComputer' }} onPlace={onPlace} />);
  await screen.findByRole('radio', { name: 'On this computer' });
  expect(screen.getByRole('radio', { name: 'On this computer' }).matches(':checked')).toBe(true);
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('cluster_contexts');

  view.rerender(<Setup onOpenExample={vi.fn()} place={{ kind: 'cluster' }} onPlace={onPlace} />);
  expect(screen.getByRole('radio', { name: 'On a cluster you already have' }).matches(':checked')).toBe(true);
  expect(await screen.findByRole('combobox', { name: 'kubectl context' })).toBeTruthy();
  expect(ipc.mock.calls.filter(([c]) => c === 'cluster_contexts')).toHaveLength(1);

  view.rerender(<Setup onOpenExample={vi.fn()} place={{ kind: 'thisComputer' }} onPlace={onPlace} />);
  expect(screen.getByRole('radio', { name: 'On this computer' }).matches(':checked')).toBe(true);
  expect(screen.queryByRole('combobox', { name: 'kubectl context' })).toBeNull();

  view.rerender(<Setup onOpenExample={vi.fn()} place={{ kind: 'cluster' }} onPlace={onPlace} />);
  expect(screen.getByRole('combobox', { name: 'kubectl context' })).toBeTruthy();
  expect(ipc.mock.calls.filter(([c]) => c === 'cluster_contexts')).toHaveLength(1);
});

test('a shared place change alone never admits setup or starts the workspace', async () => {
  const ipc = nativeDirect();
  const view = render(<Setup onOpenExample={vi.fn()} place={{ kind: 'thisComputer' }} onPlace={vi.fn()} />);
  await screen.findByRole('radio', { name: 'On this computer' });
  view.rerender(<Setup onOpenExample={vi.fn()} place={{ kind: 'cluster' }} onPlace={vi.fn()} />);
  await screen.findByRole('combobox', { name: 'kubectl context' });
  view.rerender(<Setup onOpenExample={vi.fn()} place={{ kind: 'thisComputer' }} onPlace={vi.fn()} />);
  await screen.findByRole('radio', { name: 'On this computer' });
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('setup_select_local');
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('setup_change_local');
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('setup_cancel_local');
  expect(ipc.mock.calls.every(([c]) => ['setup_status', 'setup_requirements', 'cluster_contexts'].includes(c))).toBe(true);
});

test('cluster configuration survives returning to an already-selected local workspace and back, without auto-using the cluster again', async () => {
  vi.stubGlobal('isTauri', true);
  const context = { id: '00000000-0000-4000-8000-000000000001', name: 'team-dev' };
  const target = { id: '00000000-0000-4000-8000-000000000002', context_id: context.id, context_name: context.name,
    service_name: 'orka', service_namespace: 'control', watched_namespace: 'agents', image: 'ghcr.io/orka:0.9.4' };
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'setup_requirements') return null;
    if (command === 'cluster_contexts') return [context];
    if (command === 'cluster_discover') return [target];
    if (command === 'cluster_use') return { status: 'needServiceAccount', providers: [], failure: null };
    throw Error('Unsupported');
  });
  mockIPC(ipc);
  const user = userEvent.setup();
  const view = render(<Setup onOpenExample={vi.fn()} place={{ kind: 'thisComputer' }} onPlace={vi.fn()} />);
  // Already selected and running locally: Setup shows the workspace-setup-grid, not the chooser.
  await screen.findByText('Your workspace is running on this computer.');

  view.rerender(<Setup onOpenExample={vi.fn()} place={{ kind: 'cluster' }} onPlace={vi.fn()} />);
  await waitFor(() => expect(screen.getByRole('combobox', { name: 'kubectl context' }).matches(':disabled')).toBe(false));
  await user.selectOptions(screen.getByRole('combobox', { name: 'kubectl context' }), context.id);
  await screen.findByText('Orka found');
  await user.click(screen.getByRole('button', { name: 'Use this cluster' }));
  const account = await screen.findByRole('textbox', { name: 'ServiceAccount name' });
  await user.type(account, 'remembered-account');
  expect(ipc.mock.calls.filter(([c]) => c === 'cluster_use')).toHaveLength(1);

  // Back to the already-selected local workspace: Setup returns to the setup-grid view. ClusterCard
  // must stay mounted (hidden), not be torn down and lose its context/target/account.
  view.rerender(<Setup onOpenExample={vi.fn()} place={{ kind: 'thisComputer' }} onPlace={vi.fn()} />);
  await screen.findByText('Your workspace is running on this computer.');
  expect(screen.queryByRole('combobox', { name: 'kubectl context' })).toBeNull();

  view.rerender(<Setup onOpenExample={vi.fn()} place={{ kind: 'cluster' }} onPlace={vi.fn()} />);
  expect((screen.getByRole('combobox', { name: 'kubectl context' }) as HTMLSelectElement).value).toBe(context.id);
  expect(screen.getByText('Orka found')).toBeTruthy();
  const reenteredAccount = screen.getByRole('textbox', { name: 'ServiceAccount name' }) as HTMLInputElement;
  expect(reenteredAccount.value).toBe('remembered-account');
  // No auto re-Use: the remembered access confirmation is gone and no extra cluster_use fired.
  expect(screen.queryByText('Access confirmed')).toBeNull();
  expect(ipc.mock.calls.filter(([c]) => c === 'cluster_use')).toHaveLength(1);
  expect(screen.getByRole('button', { name: 'Use this cluster' })).toBeTruthy();
});
