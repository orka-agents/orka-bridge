import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from '../App';
import type { SelectedPlace } from '../clusters/api';
import type { Place } from '../overview/api';
import OrkaSwitcher from './OrkaSwitcher';

const target = { id: '00000000-0000-4000-8000-000000000002', context_id: '00000000-0000-4000-8000-000000000001', context_name: 'team-dev', service_name: 'orka', service_namespace: 'control', watched_namespace: 'agents', image: 'fixture' };
const provider = { id: '00000000-0000-4000-8000-000000000003', name: 'team-model', ready: true };
const remembered = (key: string, context = key): Place => ({ key, context, service: 'control/orka', namespace: 'agents', stale: false, observedAt: 123, agents: [] });
const calls: { command: string; args: unknown }[] = [];
function ipc(places: Place[], selectedKey: string | null, extra: (command: string, args: unknown) => unknown = () => undefined) {
  mockIPC((command, args) => {
    calls.push({ command, args });
    const other = extra(command, args); if (other !== undefined) return other;
    if (command === 'cluster_places') return { places, selectedKey, persistenceState: 'complete' };
    if (command === 'cluster_select') return { target, providers: [provider] };
    return null;
  });
}
beforeEach(() => { vi.stubGlobal('isTauri', true); calls.length = 0; });
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

function switcher(place: SelectedPlace = { kind: 'thisComputer' }) {
  const onPlace = vi.fn(); const onClusters = vi.fn(); const onManage = vi.fn();
  render(<><OrkaSwitcher place={place} computerLine="Workspace ready" computerTone="ready" onPlace={onPlace} onClusters={onClusters} onManage={onManage}>
    <span className="host-label">This computer</span><span className="sr-only">: The workspace is ready on this computer.</span>
  </OrkaSwitcher><button type="button">After</button></>);
  return { onPlace, onClusters, onManage, button: screen.getByRole('button', { name: /^This computer/ }) };
}

test('opens a menu of this computer, remembered clusters and the cluster actions, and Escape returns focus', async () => {
  ipc([remembered('k-open-a', 'team-dev'), remembered('k-open-b', 'team-old')], 'k-open-a');
  const user = userEvent.setup(); const { button } = switcher();
  expect(button.getAttribute('aria-haspopup')).toBe('menu'); expect(button.getAttribute('aria-expanded')).toBe('false');
  await user.click(button);
  const menu = await screen.findByRole('menu', { name: 'Orka in use' });
  expect(button.getAttribute('aria-expanded')).toBe('true');
  const radios = within(menu).getAllByRole('menuitemradio');
  expect(radios.map(item => item.textContent)).toEqual(['This computerWorkspace ready', 'team-devOrka in control/orka', 'team-oldConnect again']);
  expect(radios.map(item => item.getAttribute('aria-checked'))).toEqual(['true', 'false', 'false']);
  expect(within(menu).getAllByRole('menuitem').map(item => item.textContent)).toEqual(['Add a cluster', 'Manage clusters']);
  expect(within(menu).getByRole('separator')).toBeTruthy();
  expect(document.activeElement).toBe(radios[0]);
  await user.keyboard('{Escape}');
  expect(screen.queryByRole('menu')).toBeNull();
  expect(document.activeElement).toBe(button);
  expect(calls.map(call => call.command)).toEqual(['cluster_places']);
});

test('arrow keys wrap, Home and End jump, and Tab closes the menu', async () => {
  ipc([remembered('k-keys')], null);
  const user = userEvent.setup(); const { button } = switcher();
  button.focus(); await user.keyboard('{ArrowDown}');
  const menu = await screen.findByRole('menu');
  const items = [...menu.querySelectorAll<HTMLElement>('[role^="menuitem"]')];
  expect(document.activeElement).toBe(items[0]);
  await user.keyboard('{ArrowUp}'); expect(document.activeElement).toBe(items.at(-1));
  await user.keyboard('{ArrowDown}'); expect(document.activeElement).toBe(items[0]);
  await user.keyboard('{End}'); expect(document.activeElement).toBe(screen.getByRole('menuitem', { name: 'Manage clusters' }));
  await user.keyboard('{Home}{ArrowDown}'); expect(document.activeElement).toBe(screen.getByRole('menuitemradio', { name: 'k-keys' }));
  await user.keyboard('{Tab}');
  expect(screen.queryByRole('menu')).toBeNull();
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'After' }));
});

test('an outside click closes the menu without choosing', async () => {
  ipc([], null);
  const user = userEvent.setup(); const { onPlace, onClusters, onManage } = switcher();
  await user.click(screen.getByRole('button', { name: /^This computer/ })); await screen.findByRole('menu');
  await user.click(document.body);
  expect(screen.queryByRole('menu')).toBeNull();
  expect([onPlace, onClusters, onManage].every(fn => fn.mock.calls.length === 0)).toBe(true);
});

test('choosing this computer chooses it as the place and returns focus', async () => {
  ipc([remembered('k-local')], 'k-local');
  const user = userEvent.setup(); const { onPlace, button } = switcher({ kind: 'cluster', target, providers: [provider], providerId: provider.id });
  await user.click(button);
  expect((await screen.findByRole('menuitemradio', { name: 'k-local' })).getAttribute('aria-checked')).toBe('true');
  await user.keyboard('{Home}{Enter}');
  expect(onPlace).toHaveBeenCalledWith({ kind: 'thisComputer' });
  expect(document.activeElement).toBe(button);
});

test('a cluster connected in this run is selected through cluster_select, never reconnected', async () => {
  ipc([remembered('k-select', 'team-dev')], 'k-select');
  const user = userEvent.setup(); const { onPlace, onClusters } = switcher();
  await user.click(screen.getByRole('button', { name: /^This computer/ }));
  await user.click(await screen.findByRole('menuitemradio', { name: 'team-dev' }));
  await waitFor(() => expect(onPlace).toHaveBeenCalledWith({ kind: 'cluster', target, providers: [provider], providerId: provider.id }));
  expect(calls).toContainEqual({ command: 'cluster_select', args: { key: 'k-select' } });
  expect(calls.map(call => call.command)).not.toContain('cluster_reconnect');
  expect(onClusters).not.toHaveBeenCalled();
});

test('a remembered cluster not connected in this run opens in Clusters without any cluster call', async () => {
  const old = remembered('k-again', 'team-old');
  ipc([old], null);
  const user = userEvent.setup(); const { onPlace, onClusters } = switcher();
  await user.click(screen.getByRole('button', { name: /^This computer/ }));
  const row = await screen.findByRole('menuitemradio', { name: 'team-old' });
  expect(row.textContent).toContain('Connect again');
  await user.click(row);
  expect(onClusters).toHaveBeenCalledWith(old);
  expect(onPlace).not.toHaveBeenCalled();
  expect(calls.map(call => call.command)).toEqual(['cluster_places']);
});

test('a refused selection falls back to that cluster in Clusters', async () => {
  const lost = remembered('k-lost', 'team-lost');
  ipc([lost], 'k-lost', command => command === 'cluster_select' ? Promise.reject({ check: 'configuration', result: 'unknown' }) : undefined);
  const user = userEvent.setup(); const { onPlace, onClusters } = switcher({ kind: 'cluster' });
  await user.click(screen.getByRole('button', { name: /^This computer/ }));
  await user.click(await screen.findByRole('menuitemradio', { name: 'team-lost' }));
  await waitFor(() => expect(onClusters).toHaveBeenCalledWith(lost));
  expect(onPlace).toHaveBeenCalledWith({ kind: 'cluster' });
});

test('Add a cluster and Manage clusters go to Clusters', async () => {
  ipc([], null);
  const user = userEvent.setup(); const { onClusters, onManage } = switcher();
  await user.click(screen.getByRole('button', { name: /^This computer/ }));
  await user.click(await screen.findByRole('menuitem', { name: 'Add a cluster' }));
  expect(onClusters).toHaveBeenCalledWith();
  await user.click(screen.getByRole('button', { name: /^This computer/ }));
  await user.click(await screen.findByRole('menuitem', { name: 'Manage clusters' }));
  expect(onManage).toHaveBeenCalledOnce();
});

test('more than six places can be filtered by typing', async () => {
  ipc(Array.from({ length: 7 }, (_, index) => remembered(`k-filter-${index}`, `team-${index}`)), null);
  const user = userEvent.setup(); switcher();
  await user.click(screen.getByRole('button', { name: /^This computer/ }));
  const find = await screen.findByRole('searchbox', { name: 'Find a cluster' });
  expect(document.activeElement).toBe(find);
  await user.keyboard('team-3');
  expect(screen.getAllByRole('menuitemradio').map(item => item.querySelector('code')?.textContent)).toEqual(['team-3']);
  await user.keyboard('{ArrowDown}');
  expect(document.activeElement).toBe(screen.getByRole('menuitemradio', { name: 'team-3' }));
});

test('in the app, the footer keeps its words and a not-connected cluster opens its card in Clusters', async () => {
  localStorage.setItem('bridge.welcomeSeen.v1', '1');
  ipc([remembered('k-app', 'team-app')], null, command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'overview_files') return { state: 'complete', runActive: false, historyState: 'complete', runAgents: [], files: [] };
    if (command === 'overview_places') return { historyState: 'complete', unobserved: [], places: [] };
    if (command === 'overview_agents') return { state: 'unavailable', installation: null, agents: [], observation: null, observedAt: null, runActive: false, reason: 'stopped' };
    if (command === 'cluster_contexts') return [];
    return undefined;
  });
  const user = userEvent.setup(); render(<App />);
  const chip = () => document.querySelector<HTMLButtonElement>('.host-chip')!;
  await waitFor(() => expect(chip().textContent).toBe('This computer: The workspace is ready on this computer.'));
  expect(chip().tagName).toBe('BUTTON'); expect(chip().querySelector('.host-label')?.textContent).toBe('This computer');
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(chip());
  const row = await screen.findByRole('menuitemradio', { name: 'team-app' });
  expect(screen.getByRole('menuitemradio', { name: 'This computer' }).getAttribute('aria-checked')).toBe('true');
  await user.click(row);
  await screen.findByRole('heading', { name: 'team-app', level: 1 });
  expect(screen.getByRole('button', { name: 'Clusters' }).getAttribute('aria-current')).toBe('page');
  // The card takes over from here, exactly as the Clusters list's own button does.
  await act(async () => { await Promise.resolve(); });
  expect(calls.map(call => call.command)).not.toContain('cluster_select');
});

test('a click anywhere on the status card opens the menu, and one inside the menu is left alone', async () => {
  ipc([], null);
  render(<footer className="status-bar"><OrkaSwitcher place={{ kind: 'thisComputer' }} computerLine="Workspace ready" computerTone="ready" onPlace={vi.fn()} onClusters={vi.fn()} onManage={vi.fn()}>
    <span className="host-label">This computer</span>
  </OrkaSwitcher><span className="status-lines">Workspace ready</span></footer>);
  await userEvent.click(document.querySelector('.status-lines')!);
  const menu = await screen.findByRole('menu', { name: 'Orka in use' });
  await userEvent.click(within(menu).getByText('On this computer'));
  expect(screen.getByRole('menu', { name: 'Orka in use' })).toBeTruthy();
  await userEvent.click(document.querySelector('.status-lines')!);
  await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
});
