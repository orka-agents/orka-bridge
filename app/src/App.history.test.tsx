import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from './App';
import { noUpgrade } from './setup/test-upgrade';

vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ isMaximized: () => Promise.resolve(false), minimize: vi.fn(), toggleMaximize: vi.fn(), close: vi.fn() }) }));

const documentId = '4f0d8a62-1c3e-4b7a-9d55-2e6f71c0a9b3';
const pair = { documentId, name: 'reader', agent: '---\nname: reader\nmodel: literal-model\nkmx:\n  schemaVersion: 1\n  runtime: orka\n  tools: []\n---\nRead a page.', tests: 'Checks', path: '/owned/reader.agent.md' };
const inventory = [{ key: 'one', context: 'team-one', service: 'control/orka', namespace: 'agents', stale: false, observedAt: 123, checkedAt: 123, agents: [] }];
let agents: { id: string; name: string; path: string; available: boolean }[] = [];
let places = inventory;
const calls: string[] = [];
function ipc() {
  mockIPC(command => {
    calls.push(command);
    if (command === 'workspace_upgrade_status') return noUpgrade;
    if (command === 'setup_status') return { selected: null, localVm: 'idle', model: 'locked' };
    if (command === 'overview_files') return { state: 'complete', runActive: false, historyState: 'complete', runAgents: [], files: [] };
    if (command === 'overview_places') return { historyState: 'complete', unobserved: [], places: [] };
    if (command === 'overview_agents') return { state: 'unavailable', installation: null, agents: [], observation: null, observedAt: null, runActive: false, reason: 'stopped' };
    if (command === 'cluster_places') return { places, selectedKey: null, persistenceState: 'complete' };
    if (command === 'agent_open') return pair;
    if (command === 'agent_selected') return documentId;
    if (command === 'agents_list') return agents;
    if (command === 'agent_select') return null;
    if (command === 'agent_run_status') return { documentId, busy: null, state: 'idle', canRun: false, report: null, acknowledged: [], pending: null, error: null };
    if (command === 'send_status') return { documentId, view: null, busy: null };
    if (command === 'send_history') return [];
    return null;
  });
}
beforeEach(() => {
  vi.stubGlobal('isTauri', true); localStorage.setItem('bridge.welcomeSeen.v1', '1');
  agents = [{ id: documentId, name: 'reader', path: '/owned/reader.agent.md', available: true }]; places = inventory; calls.length = 0;
});
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

const backButton = () => screen.getByRole('button', { name: 'Back' }) as HTMLButtonElement;
const forwardButton = () => screen.getByRole('button', { name: 'Forward' }) as HTMLButtonElement;
const shown = () => document.querySelector('main')!.getAttribute('data-page');
const view = () => within(screen.getByRole('group', { name: 'Designer view' })).getAllByRole('button').find(button => button.getAttribute('aria-pressed') === 'true')?.textContent;
async function launched() {
  // A remembered cluster opens Bridge on Home. The app chose that page, so there is nothing to go back to.
  await waitFor(() => expect(shown()).toBe('Home'));
  expect(backButton().disabled).toBe(true); expect(forwardButton().disabled).toBe(true);
}
async function walk(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  expect(backButton().disabled).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  await screen.findByRole('heading', { name: 'reader', level: 1 });
  await user.click(screen.getByRole('button', { name: 'File' }));
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  const row = await screen.findByRole('region', { name: 'team-one' });
  await user.click(within(row).getByRole('button', { name: 'Use' }));
  await screen.findByRole('heading', { name: 'team-one', level: 1 });
}

test('Back and Forward retrace the places the person went, including Designer views and a cluster’s page', async () => {
  ipc(); const user = userEvent.setup(); render(<App />);
  await launched();
  await walk(user);
  await user.click(backButton());
  await screen.findByRole('heading', { name: 'Clusters', level: 1 });
  expect(shown()).toBe('Clusters'); expect(forwardButton().disabled).toBe(false);
  await user.click(backButton());
  await waitFor(() => expect(shown()).toBe('Designer'));
  await waitFor(() => expect(view()).toBe('File'));
  await user.click(backButton());
  await waitFor(() => expect(view()).toBe('Agent'));
  expect(shown()).toBe('Designer');
  await user.click(backButton());
  await waitFor(() => expect(shown()).toBe('Overview'));
  await user.click(forwardButton());
  await waitFor(() => expect(shown()).toBe('Designer'));
  await waitFor(() => expect(view()).toBe('Agent'));
  await user.click(forwardButton());
  await waitFor(() => expect(view()).toBe('File'));
  // Focus stays on the control used, as it does on a sidebar item.
  expect(document.activeElement).toBe(forwardButton());
  // Going somewhere new from the middle drops the forward entries.
  await user.click(screen.getByRole('button', { name: 'Home' }));
  await waitFor(() => expect(forwardButton().disabled).toBe(true));
  await user.click(backButton());
  await waitFor(() => expect(view()).toBe('File'));
  // Restoring ran nothing beyond what the walk ran by hand: the cluster's check ran once, when it was opened.
  expect(calls.filter(command => ['agent_save', 'cluster_use', 'cluster_reconnect', 'agent_run', 'send_start'].includes(command))).toEqual(['cluster_reconnect']);
});

test('an entry whose agent or cluster is gone is skipped', async () => {
  ipc(); const user = userEvent.setup(); render(<App />);
  await launched();
  await walk(user);
  await user.click(screen.getByRole('button', { name: 'Home' }));
  agents = []; places = [];
  await user.click(backButton());
  // The cluster's page and both Designer entries are gone; Back lands on the cluster list, then on Overview.
  await waitFor(() => expect(shown()).toBe('Clusters'));
  await screen.findByRole('heading', { name: 'Clusters', level: 1 });
  await user.click(backButton());
  await waitFor(() => expect(shown()).toBe('Overview'));
});

test('Alt+Left and Alt+Right move through history, but not from a text field', async () => {
  ipc(); const user = userEvent.setup(); render(<App />);
  await launched();
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  const instructions = await screen.findByRole('textbox', { name: 'Instructions' });
  fireEvent.keyDown(instructions, { key: 'ArrowLeft', altKey: true });
  expect(shown()).toBe('Designer');
  fireEvent.keyDown(document.body, { key: 'ArrowLeft', altKey: true });
  await waitFor(() => expect(shown()).toBe('Overview'));
  fireEvent.keyDown(document.body, { key: 'ArrowRight', altKey: true });
  await waitFor(() => expect(shown()).toBe('Designer'));
  // The mouse's back button too.
  fireEvent.mouseUp(document.body, { button: 3 });
  await waitFor(() => expect(shown()).toBe('Overview'));
});

test('the buttons follow the sidebar toggle in the tab order and name their shortcuts', async () => {
  ipc(); const user = userEvent.setup(); render(<App />);
  await launched();
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  screen.getByRole('button', { name: 'Collapse sidebar' }).focus();
  await user.tab();
  expect(document.activeElement).toBe(backButton());
  expect(backButton().getAttribute('aria-keyshortcuts')).toBe('Alt+ArrowLeft');
  expect(backButton().parentElement!.querySelector('.control-tip')!.textContent).toBe('Back · Alt+Left');
  expect(forwardButton().parentElement!.querySelector('.control-tip')!.textContent).toBe('Forward · Alt+Right');
  // Back to the start disables Back; focus moves to Forward rather than being lost.
  await user.click(backButton());
  await waitFor(() => expect(backButton().disabled).toBe(true));
  await waitFor(() => expect(document.activeElement).toBe(forwardButton()));
});

test('the Windows title bar has Back and Forward, no File menu, and keeps its buttons out of the drag region', async () => {
  vi.stubGlobal('navigator', { ...navigator, userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36 Edg/142.0.0.0' });
  ipc(); render(<App />);
  await launched();
  const bar = document.querySelector<HTMLElement>('header.title-bar')!;
  expect(bar.getAttribute('data-tauri-drag-region')).toBe('deep');
  expect(within(bar).queryByRole('button', { name: 'File' })).toBeNull();
  expect(bar.querySelector('.file-menu')).toBeNull();
  const order = within(bar).getAllByRole('button').map(button => button.getAttribute('aria-label') ?? button.textContent);
  expect(order.slice(0, 3)).toEqual(['Collapse sidebar', 'Back', 'Forward']);
  expect(order).toContain('Close');
  // Tauri's drag script lets clicks on buttons through unless they carry the drag attribute themselves.
  for (const button of within(bar).getAllByRole('button')) expect(button.closest('[data-tauri-drag-region]:not(header)')).toBeNull();
});

test('the guided first run has no Back or Forward; history starts when the full app appears', async () => {
  localStorage.setItem('bridge.firstRun.v1', 'active');
  vi.stubGlobal('isTauri', true);
  mockIPC(command => {
    if (command === 'workspace_upgrade_status') return noUpgrade;
    if (command === 'setup_status') return { selected: null, localVm: 'idle', model: 'locked' };
    if (command === 'setup_requirements') return { memoryBytes: 6 * 1024 ** 3, imageBytes: 3e9, stateDiskBytes: 32e9, downloadBytes: null };
    if (command === 'cluster_places') return { places: [], selectedKey: null, persistenceState: 'complete' };
    return null;
  });
  render(<App />);
  expect(document.querySelector('.app-shell')!.classList.contains('guided')).toBe(true);
  await screen.findByRole('heading', { level: 1 });
  expect(screen.queryByRole('button', { name: 'Back' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Forward' })).toBeNull();
});

test('New agent is a place: Back leaves it, Forward returns, and Cancel takes its entry’s place', async () => {
  ipc(); const user = userEvent.setup(); render(<App />);
  await launched();
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(await screen.findByRole('button', { name: 'New agent' }));
  await screen.findByRole('heading', { name: 'New agent', level: 1 });
  await user.click(backButton());
  await waitFor(() => expect(shown()).toBe('Overview'));
  await user.click(forwardButton());
  await screen.findByRole('heading', { name: 'New agent', level: 1 });
  expect(shown()).toBe('Designer');
  await user.click(screen.getByRole('button', { name: 'Cancel' }));
  await screen.findByRole('heading', { name: 'reader', level: 1 });
  // Back goes to Overview, not to the form that was cancelled.
  await user.click(backButton());
  await waitFor(() => expect(shown()).toBe('Overview'));
  await user.click(forwardButton());
  await screen.findByRole('heading', { name: 'reader', level: 1 });
  expect(forwardButton().disabled).toBe(true);
});
