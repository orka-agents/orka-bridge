import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import Setup from '../setup/Setup';
import type { SelectedPlace } from './api';

const context = { id: '00000000-0000-4000-8000-000000000001', name: 'team-dev' };
const target = { id: '00000000-0000-4000-8000-000000000002', context_id: context.id, context_name: context.name,
  service_name: 'orka', service_namespace: 'control', watched_namespace: 'agents', image: 'ghcr.io/orka:0.9.4' };
const provider = { id: '00000000-0000-4000-8000-000000000003', name: 'team-model', ready: true };
const primary = { key: 'binding-primary', context: 'team-dev', namespace: 'agents', service: 'control / orka', stale: true, observedAt: 1700000000000, agents: [] };
const secondary = { ...primary, key: 'binding-secondary', context: 'team-other', observedAt: null };
const connected: SelectedPlace = { kind: 'cluster', target, providers: [provider], providerId: provider.id };
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
function native() {
  vi.stubGlobal('isTauri', true);
  const sim = { status: { selected: null as 'localVm' | null, localVm: 'idle', model: 'locked', modelSupported: true },
    snapshot: { places: [primary, secondary], selectedKey: primary.key as string | null }, unavailable: false };
  const ipc = vi.fn((command: string): unknown => {
    if (command === 'setup_status') return sim.status;
    if (command === 'setup_requirements') return null;
    if (command === 'cluster_places') { if (sim.unavailable) throw Error('unreadable'); return sim.snapshot; }
    if (command === 'cluster_contexts') return [context];
    if (command === 'cluster_discover') return [target];
    if (command === 'cluster_use') return { status: 'connected', providers: [provider], failure: null };
    if (command === 'setup_connect_model') return { ready: true };
    if (command === 'setup_select_local') { sim.status = { ...sim.status, selected: 'localVm', localVm: 'starting' }; return sim.status; }
    throw Error('Unexpected: ' + command);
  });
  mockIPC(ipc); return { ipc, sim };
}
function Host({ initial = { kind: 'thisComputer' }, observe = () => {} }: { initial?: SelectedPlace; observe?: (place: SelectedPlace) => void }) {
  const [place, setPlace] = useState(initial);
  return <Setup place={place} onPlace={value => { setPlace(value); observe(value); }} onOpenExample={() => {}} />;
}
const row = (name: string) => screen.getByRole('region', { name });
beforeEach(() => localStorage.setItem('bridge.welcomeSeen.v1', '1'));
afterEach(() => { cleanup(); clearMocks(); vi.restoreAllMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

test('Places passively lists this computer and retained identities with truthful historical status', async () => {
  const { ipc } = native(); render(<Host initial={connected} />);
  await screen.findByRole('heading', { name: 'Places for your agents' });
  await screen.findByRole('region', { name: 'team-other' });
  expect(within(row('This computer')).getAllByRole('button')).toHaveLength(1);
  expect(within(row('This computer')).getByRole('button', { name: 'Set up' })).toBeTruthy();
  expect(within(row('team-dev')).getByText('Selected for Designer')).toBeTruthy();
  expect(within(row('team-dev')).getByRole('button', { name: 'Manage' })).toBeTruthy();
  expect(within(row('team-dev')).queryByRole('button', { name: 'Connect' })).toBeNull();
  expect(within(row('team-dev')).getByText(/^Last checked /)).toBeTruthy();
  expect(within(row('team-other')).getByText('Not checked yet')).toBeTruthy();
  expect(within(row('team-other')).queryByText(/0 Agents/)).toBeNull();
  expect(document.body.textContent).not.toMatch(/Observation stale|Not observed yet|kept while Bridge/);
  expect(ipc.mock.calls.map(([c]) => c).every(c => ['setup_status', 'setup_requirements', 'cluster_places'].includes(c))).toBe(true);
});

test('an unreadable workspace status stays unknown while retained places remain readable', async () => {
  const { ipc } = native();
  mockIPC(command => command === 'setup_status' ? Promise.reject(Error('lost status reply')) : ipc(command));
  render(<Host />);
  await screen.findByRole('alert');
  const local = within(row('This computer'));
  expect(local.queryByText('Not set up')).toBeNull();
  expect(local.queryByText('AI not connected')).toBeNull();
  expect(local.getByText('Status unavailable')).toBeTruthy();
  expect(local.getByRole('button', { name: 'View details' })).toBeTruthy();
  expect(await screen.findByRole('region', { name: 'team-other' })).toBeTruthy();
  expect(screen.queryByText('Reading your places…')).toBeNull();
  expect(ipc.mock.calls.map(([command]) => command)).not.toContain('setup_select_local');
});

test.each<SelectedPlace>([{ kind: 'thisComputer' }, { kind: 'cluster' }])('native access alone is not a Designer selection: %j', async initial => {
  native(); render(<Host initial={initial} />);
  await screen.findByRole('region', { name: 'team-dev' });
  expect(screen.queryByText('Selected for Designer')).toBeNull();
  expect(within(row('team-dev')).getByRole('button', { name: 'Connect' })).toBeTruthy();
});

test('equal display names do not substitute for the native selected key', async () => {
  const { sim } = native(); sim.snapshot.places = [primary, { ...secondary, context: primary.context }];
  sim.snapshot.selectedKey = secondary.key;
  render(<Host initial={connected} />);
  const rows = await screen.findAllByRole('region', { name: 'team-dev' });
  expect(within(rows[0]).queryByText('Selected for Designer')).toBeNull();
  expect(within(rows[1]).getByText('Selected for Designer')).toBeTruthy();
});

test('Add and Connect only navigate, reset the context, and never choose a place implicitly', async () => {
  const { ipc } = native(); const observe = vi.fn(); const user = userEvent.setup(); render(<Host initial={connected} observe={observe} />);
  await user.click(await screen.findByRole('button', { name: 'Add a place' }));
  expect(await screen.findByRole('combobox', { name: 'kubectl context' })).toBeTruthy();
  expect(screen.queryByRole('radio')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Set up' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Existing cluster' })).toBeNull();
  expect(observe).not.toHaveBeenCalled();
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('cluster_discover');
  await user.selectOptions(screen.getByRole('combobox', { name: 'kubectl context' }), context.id);
  await screen.findByText('Orka found');
  expect(observe).toHaveBeenLastCalledWith({ kind: 'cluster' });
  await user.click(screen.getByRole('button', { name: 'Back to places' }));
  await user.click(within(await screen.findByRole('region', { name: 'team-dev' })).getByRole('button', { name: 'Connect' }));
  expect((screen.getByRole('combobox', { name: 'kubectl context' }) as HTMLSelectElement).value).toBe('');
  expect(screen.getByText(/team-dev/, { selector: '.cluster-intent' })).toBeTruthy();
  expect(ipc.mock.calls.filter(([c]) => c === 'cluster_discover')).toHaveLength(1);
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('setup_select_local');
});

test('Manage rechecks the same installation with its already-entered ServiceAccount', async () => {
  const { ipc } = native(); const user = userEvent.setup(); const accounts: unknown[] = [];
  mockIPC((command, args) => {
    if (command === 'cluster_use') {
      const name = args && 'saName' in args ? args.saName : undefined;
      accounts.push(name);
      if (name !== 'team-runner') return { status: 'needServiceAccount', providers: [], failure: null };
    }
    return ipc(command);
  });
  render(<Host />);
  await user.click(await screen.findByRole('button', { name: 'Add a place' }));
  await user.selectOptions(await screen.findByRole('combobox', { name: 'kubectl context' }), context.id);
  await user.click(await screen.findByRole('button', { name: 'Use this cluster' }));
  await user.type(await screen.findByRole('textbox', { name: 'ServiceAccount name' }), 'team-runner');
  await user.click(screen.getByRole('button', { name: 'Use this cluster' }));
  await screen.findByText('Access confirmed');
  await user.click(screen.getByRole('button', { name: 'Back to places' }));
  await user.click(within(await screen.findByRole('region', { name: 'team-dev' })).getByRole('button', { name: 'Manage' }));
  await user.click(await screen.findByRole('button', { name: 'Check access again' }));
  expect(await screen.findByText('Access confirmed')).toBeTruthy();
  expect(screen.queryByRole('textbox', { name: 'ServiceAccount name' })).toBeNull();
  expect(accounts).toEqual([null, 'team-runner', 'team-runner']);
});

test('selected Manage uses current App capabilities without discovery and changes context only explicitly', async () => {
  const { ipc } = native(); const observe = vi.fn(); const user = userEvent.setup(); render(<Host initial={connected} observe={observe} />);
  await user.click(within(await screen.findByRole('region', { name: 'team-dev' })).getByRole('button', { name: 'Manage' }));
  expect(await screen.findByRole('combobox', { name: 'Provider' })).toBeTruthy();
  expect(screen.getByText('team-dev', { selector: 'h2 code' })).toBeTruthy();
  expect(screen.getByText('agents', { selector: '.cluster-result code' })).toBeTruthy();
  expect((screen.getByRole('combobox', { name: 'Provider' }) as HTMLSelectElement).value).toBe(provider.id);
  expect(screen.getByRole('button', { name: 'Change context' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Use this cluster' })).toBeNull();
  expect(observe).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Back to places' }));
  expect(within(await screen.findByRole('region', { name: 'team-dev' })).getByText('Selected for Designer')).toBeTruthy();
  expect(observe).not.toHaveBeenCalled();
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('cluster_use');
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('cluster_discover');
});

test('local Manage is navigation; explicit Use this computer never starts a stopped VM or trusts its native cluster key', async () => {
  const { ipc } = native(); const observe = vi.fn(); const user = userEvent.setup(); render(<Host initial={connected} observe={observe} />);
  await user.click(within(await screen.findByRole('region', { name: 'This computer' })).getByRole('button', { name: 'Set up' }));
  expect(observe).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: 'Use this computer' }));
  expect(observe).toHaveBeenLastCalledWith({ kind: 'thisComputer' });
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('setup_select_local');
  await user.click(screen.getByRole('button', { name: 'Back to places' }));
  expect(screen.queryByText('Selected for Designer')).toBeNull();
});

test.each([['closed', 'Workspace stopped'], ['cancelled', 'Setup paused'], ['idle', 'Not set up']] as const)('a selected %s local workspace never claims setup is still running', async (localVm, label) => {
  const { sim } = native(); sim.status = { ...sim.status, selected: 'localVm', localVm };
  const user = userEvent.setup(); render(<Host />);
  await user.click(await screen.findByRole('button', { name: 'Back to places' }));
  const local = row('This computer');
  const pill = within(local).getByText(label, { selector: '.pill' });
  expect(pill.className).toContain('status-not-started');
  expect(within(local).queryByText('Setup in progress')).toBeNull();
  expect(within(local).getByRole('button', { name: 'Continue setup' })).toBeTruthy();
});

test('snapshot failure retains known rows but does not invent a selected row or empty inventory', async () => {
  const { sim } = native(); const user = userEvent.setup(); render(<Host initial={connected} />);
  await user.click(await screen.findByRole('button', { name: 'Add a place' }));
  sim.unavailable = true;
  await user.click(screen.getByRole('button', { name: 'Back to places' }));
  await screen.findByText("Couldn't read your places. Try opening Clusters again.");
  expect(row('team-dev')).toBeTruthy();
  expect(screen.queryByText('Selected for Designer')).toBeNull();
  expect(screen.queryByText('No clusters added yet.')).toBeNull();
});

test('a Use recheck clears the frontend target before native replies and Back cannot restore it', async () => {
  const { ipc } = native(); const pending = deferred<unknown>(); const old = ipc.getMockImplementation()!;
  ipc.mockImplementation(c => c === 'cluster_use' ? pending.promise : old(c));
  const observe = vi.fn(); const user = userEvent.setup(); render(<Host initial={connected} observe={observe} />);
  await user.click(within(await screen.findByRole('region', { name: 'team-dev' })).getByRole('button', { name: 'Manage' }));
  await user.click(await screen.findByRole('button', { name: 'Check access again' }));
  expect(observe).toHaveBeenLastCalledWith({ kind: 'cluster' });
  await user.click(screen.getByRole('button', { name: 'Back to places' }));
  expect(screen.queryByText('Selected for Designer')).toBeNull();
  const calls = observe.mock.calls.length;
  await act(async () => pending.resolve({ status: 'connected', providers: [provider], failure: null }));
  expect(observe).toHaveBeenCalledTimes(calls);
  expect(screen.queryByText('Selected for Designer')).toBeNull();
});

test('Back preserves a queued local Model and its one eventual connection while Places is visible', async () => {
  const { ipc, sim } = native(); sim.status = { ...sim.status, selected: 'localVm', localVm: 'installing' };
  const observe = vi.fn(); const user = userEvent.setup(); render(<Host observe={observe} />);
  await user.click(await screen.findByRole('button', { name: 'Connect the AI' }));
  expect(screen.getByText('Queued', { selector: '.pill' })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Back to places' }));
  expect(within(row('This computer')).getByText('AI connection queued')).toBeTruthy();
  expect(observe).not.toHaveBeenCalled();
  sim.status = { ...sim.status, localVm: 'ready', model: 'available' };
  await act(async () => window.dispatchEvent(new Event('focus')));
  await within(row('This computer')).findByText('AI connected');
  await user.click(within(row('This computer')).getByRole('button', { name: 'Manage' }));
  expect(screen.getByText('Connected with GitHub Copilot.')).toBeTruthy();
  expect(ipc.mock.calls.filter(([c]) => c === 'setup_connect_model')).toHaveLength(1);
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('setup_select_local');
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('setup_change_local');
});

test('hidden snapshot replies are fenced and returning to Clusters reads a new passive snapshot', async () => {
  const { ipc } = native(); const pending = deferred<unknown>(); const old = ipc.getMockImplementation()!;
  let reads = 0;
  ipc.mockImplementation(c => c === 'cluster_places' && ++reads === 1 ? pending.promise : old(c));
  const view = render(<Setup onOpenExample={() => {}} />);
  await waitFor(() => expect(reads).toBe(1));
  view.rerender(<Setup active={false} onOpenExample={() => {}} />);
  await act(async () => pending.resolve({ places: [primary], selectedKey: primary.key }));
  expect(screen.queryByRole('region', { name: 'team-dev' })).toBeNull();
  expect(reads).toBe(1);
  view.rerender(<Setup active onOpenExample={() => {}} />);
  expect(await screen.findByRole('region', { name: 'team-other' })).toBeTruthy();
  expect(reads).toBe(2);
  expect(ipc.mock.calls.map(([c]) => c)).not.toContain('overview_places');
});

test('loading local size facts keeps all three rows visible and Continue gated', async () => {
  const { ipc } = native(); const pending = deferred<unknown>(); const old = ipc.getMockImplementation()!;
  ipc.mockImplementation(c => c === 'setup_requirements' ? pending.promise : old(c));
  const user = userEvent.setup(); render(<Host />);
  await user.click(within(await screen.findByRole('region', { name: 'This computer' })).getByRole('button', { name: 'Set up' }));
  expect(screen.getAllByText('Reading…', { selector: 'dd' })).toHaveLength(3);
  expect(screen.getByRole('button', { name: 'Continue' }).matches(':disabled')).toBe(true);
  await act(async () => pending.resolve(null));
  expect(screen.getByText('Workspace size will be shown before download')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Continue' }).matches(':disabled')).toBe(false);
  expect(ipc.mock.calls.map(([command]) => command)).not.toContain('setup_select_local');
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  expect(await screen.findByRole('region', { name: 'Workspace setup' })).toBeTruthy();
  expect(ipc.mock.calls.filter(([command]) => command === 'setup_select_local')).toHaveLength(1);
});
