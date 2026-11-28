import { noUpgrade } from './test-upgrade';
import { openLocalSetup } from './test-navigation';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from '../App';

const preference = 'bridge.welcomeSeen.v1';
const unselected = { selected: null, localVm: 'idle', model: 'locked' };
const requirements = { memoryBytes: 6442450944, stateDiskBytes: 34359738368, downloadBytes: 2800000000, setupDiskBytes: null, freeDiskBytes: null, readySeconds: null };
function native(initial: object = unselected, sizing: unknown = requirements) {
  vi.stubGlobal('isTauri', true);
  let status = initial;
  const ipc = vi.fn((command: string) => {
    if (command === 'workspace_upgrade_status') return noUpgrade;
    if (command === 'setup_status') return status;
    if (command === 'setup_requirements') return sizing;
    if (command === 'setup_select_local') {
      status = { selected: 'localVm', localVm: 'starting', model: 'locked' };
      return status;
    }
    throw new Error('Unsupported command');
  });
  mockIPC(ipc);
  return ipc;
}
beforeEach(() => { localStorage.clear(); });
afterEach(() => { cleanup(); clearMocks(); vi.restoreAllMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

test('Get started remembers the welcome without starting a workspace, including on remount', async () => {
  const ipc = native(); const user = userEvent.setup(); const first = render(<App />);
  expect(await screen.findByRole('heading', { name: 'Your agents. Your models. Your keys. Your cluster.' })).toBeTruthy();
  expect(screen.getByText("Build and test an agent on this computer, then send the same agent to your team's Orka cluster.")).toBeTruthy();
  await waitFor(() => expect(document.querySelector('#tab-status-Overview')?.textContent).toBe(''));
  expect(screen.getByText('Write it')).toBeTruthy();
  expect(screen.getByText('Test it')).toBeTruthy();
  expect(screen.getByText('Share it')).toBeTruthy();
  expect(screen.getByText('Mostly a 2.8 GB download').parentElement).toBe(screen.getByRole('button', { name: 'Get started' }).parentElement);
  expect(screen.getByText("Send it to your team's cluster (preview).")).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Get started' }));
  const heading = await screen.findByRole('heading', { name: 'Clusters' });
  expect(document.activeElement).toBe(heading);
  expect(localStorage.getItem(preference)).toBe('1');
  first.unmount(); render(<App />);
  expect(await screen.findByRole('button', { name: 'Set up' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Get started' })).toBeNull();
  expect(ipc.mock.calls.some(([command]) => command === 'setup_select_local')).toBe(false);
});

test('welcome chrome advances only after Get started, including with denied preference storage', async () => {
  native(); vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new DOMException('Denied', 'SecurityError'); });
  const user = userEvent.setup(); render(<App />);
  expect(await within(screen.getByRole('navigation')).findByText('Start here')).toBeTruthy();
  expect(within(screen.getByRole('contentinfo')).getByText('Nothing is set up yet')).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Get started' }));
  expect(within(screen.getByRole('navigation')).getByText('Choose where agents run')).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  expect(document.querySelectorAll('#page-title')).toHaveLength(1);
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  expect(await screen.findByRole('button', { name: 'Set up' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Get started' })).toBeNull();
});

test('a saved native choice skips welcome even without a renderer preference', async () => {
  const ipc = native({ selected: 'localVm', localVm: 'starting', model: 'locked' }); render(<App />);
  expect(await screen.findByText('Starting the workspace…')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Get started' })).toBeNull();
  expect(ipc.mock.calls.some(([command]) => command === 'setup_select_local')).toBe(false);
});

test('someone who already runs Orka reaches their cluster from the screen after Welcome, without starting a local workspace', async () => {
  const ipc = native(); const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Get started' }));
  expect(screen.queryByRole('button', { name: 'I already run Orka on a cluster' })).toBeNull();
  await user.click(await screen.findByRole('button', { name: 'Add a cluster' }));
  expect(await screen.findByRole('button', { name: 'kubectl context' })).toBeTruthy();
  expect(ipc.mock.calls.some(([command]) => command === 'setup_select_local')).toBe(false);
});

test('keyboard disclosure and opening local setup are inert; only Continue starts once', async () => {
  localStorage.setItem(preference, '1'); const ipc = native(); const user = userEvent.setup(); render(<App />);
  const local = await screen.findByRole('button', { name: 'Set up' });
  local.focus(); await user.keyboard(' ');
  const disclosure = screen.getByRole('button', { name: "What's inside" });
  const details = document.getElementById(disclosure.getAttribute('aria-controls')!);
  expect(details).not.toBeNull();
  expect(details?.hasAttribute('hidden')).toBe(true);
  disclosure.focus(); await user.keyboard('{Enter}');
  expect(disclosure.getAttribute('aria-expanded')).toBe('true');
  expect(screen.getByText(/workspace includes Orka/)).toBeTruthy();
  expect(ipc.mock.calls.some(([command]) => command === 'setup_select_local')).toBe(false);
  const next = screen.getByRole('button', { name: 'Continue' });
  next.focus(); await user.keyboard('{Enter}{Enter}');
  expect(await screen.findByText('Starting the workspace…')).toBeTruthy();
  expect(document.activeElement).toBe(screen.getByRole('heading', { name: 'This computer' }));
  expect(ipc.mock.calls.filter(([command]) => command === 'setup_select_local')).toEqual([['setup_select_local', {}]]);
});

test('requirements render the native build values rather than fixed memory or disk sizes', async () => {
  localStorage.setItem(preference, '1'); native(unselected, { memoryBytes: 8589934592, stateDiskBytes: 68719476736, downloadBytes: 2380070756, setupDiskBytes: null, freeDiskBytes: null, readySeconds: null });
  render(<App />); await openLocalSetup();
  expect(screen.getByText('Downloads about 2.4 GB. Needs virtualization and 8.6 GB of memory. Data disk can grow to 68.7 GB.')).toBeTruthy();
  expect(screen.getByText('Use Orka on a Kubernetes cluster you already have.')).toBeTruthy();
  expect(screen.getAllByText(/step 1 of 4/i)).toHaveLength(1);
  expect(within(screen.getByRole('contentinfo')).queryByText(/step 1/)).toBeNull();
  await userEvent.setup().click(screen.getByRole('button', { name: "What's inside" }));
  expect(screen.getByRole('button', { name: 'Continue' }).matches(':disabled')).toBe(false);
  expect(screen.queryByText(/GiB|free disk|after installation/)).toBeNull();
});

test('debug requirements without a download retain RAM checks and disk facts without inventing a transfer', async () => {
  localStorage.setItem(preference, '1'); native(unselected, { ...requirements, downloadBytes: null });
  render(<App />); await openLocalSetup();
  expect(await screen.findByText('Needs virtualization and 6.4 GB of memory. Data disk can grow to 34.4 GB.')).toBeTruthy();
  expect(screen.queryByText(/Downloads about/)).toBeNull();
  expect(screen.getByRole('region', { name: 'Computer requirements' })).toBeTruthy();
  await waitFor(() => expect(screen.getByRole('button', { name: 'Continue' }).matches(':disabled')).toBe(false));
});

test('release welcome exposes unavailability before Get started without replacing the workflow copy', async () => {
  const ipc = native({ ...unselected, localVm: 'notIntegrated' }, null); render(<App />);
  expect(await screen.findByRole('button', { name: 'Get started' })).toBeTruthy();
  expect(screen.getByText("This build can't run agents here. Next, you connect a cluster you already have.")).toBeTruthy();
  // The lede doesn't promise tests on this computer either.
  expect(screen.getByText("Build an agent on this computer, then test it and run it on your team's Orka cluster.")).toBeTruthy();
  // Nothing is downloaded here, so no download line contradicts it.
  expect(screen.queryByText(/Mostly a/)).toBeNull();
  expect(screen.getByText('Share it')).toBeTruthy();
  expect(ipc.mock.calls.some(([command]) => command === 'setup_select_local')).toBe(false);
});

test('release-unavailable setup stays explicit and does not invent requirements', async () => {
  localStorage.setItem(preference, '1'); native({ ...unselected, localVm: 'notIntegrated' }, null); render(<App />);
  expect(await screen.findByText('Workspace setup is not included in this build. Nothing will be downloaded or started.')).toBeTruthy();
  expect(screen.queryByText(/GiB/)).toBeNull();
});

test.each(['getItem', 'setItem'] as const)('denied preference %s does not prevent setup', async method => {
  native(); vi.spyOn(Storage.prototype, method).mockImplementation(() => { throw new DOMException('Denied', 'SecurityError'); });
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Get started' }));
  expect(await screen.findByRole('button', { name: 'Set up' })).toBeTruthy();
});

test.each([
  { ...requirements, imageBytes: 3651994962 },
  { ...requirements, memoryBytes: Number.MAX_SAFE_INTEGER + 1 },
  { ...requirements, stateDiskBytes: 0 },
  { ...requirements, downloadBytes: 0 },
  { ...requirements, downloadBytes: 'SECRET_MARKER' },
  { memoryBytes: 'SECRET_MARKER', stateDiskBytes: 123 },
  { ...requirements, secret: 'SECRET_MARKER' },
  { ...requirements, readySeconds: -1 },
  { ...requirements, setupDiskBytes: 'SECRET_MARKER' },
  { memoryBytes: 6442450944, stateDiskBytes: 34359738368, downloadBytes: 2800000000 },
])('unsupported sizing cannot produce requirements copy or leak data: %j', async value => {
  localStorage.setItem(preference, '1'); native(unselected, value); render(<App />); await openLocalSetup();
  expect(await screen.findByText(/Requirements are not available yet for this build\./)).toBeTruthy();
  // What's inside still opens, and names no size.
  await userEvent.setup().click(screen.getByRole('button', { name: "What's inside" }));
  expect(screen.getByText(/Bridge checks the download before starting it\./)).toBeTruthy();
  expect(document.body.textContent).not.toMatch(/\d[.,]\d GB|GiB/);
  expect(document.body.textContent).not.toContain('SECRET_MARKER');
});

test('an unseen welcome cannot hide a failed status read or overwrite its preference', async () => {
  native(); let failed = true;
  const ipc = vi.fn(command => command === 'workspace_upgrade_status' ? noUpgrade : command === 'setup_status'
    ? failed ? Promise.reject('SECRET_MARKER') : { selected: 'localVm', localVm: 'starting', model: 'locked' }
    : command === 'setup_requirements' ? requirements : Promise.reject({ kind: 'unavailable' }));
  mockIPC(ipc); const user = userEvent.setup(); render(<App />);
  expect(await screen.findByRole('alert')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Get started' })).toBeNull();
  expect(localStorage.getItem(preference)).toBeNull();
  failed = false;
  await user.click(screen.getByRole('button', { name: 'Check again' }));
  expect(await screen.findByText('Starting the workspace…')).toBeTruthy();
  expect(screen.queryByRole('alert')).toBeNull();
  expect(localStorage.getItem(preference)).toBeNull();
  expect(ipc.mock.calls.some(([command]) => command === 'setup_select_local')).toBe(false);
});

test('a delayed initial status cannot flash welcome over an existing workspace', async () => {
  native(); let complete!: (value: unknown) => void;
  mockIPC(command => command === 'workspace_upgrade_status' ? noUpgrade : command === 'setup_status' ? new Promise(resolve => { complete = resolve; })
    : command === 'setup_requirements' ? requirements : Promise.reject({ kind: 'unavailable' }));
  render(<App />);
  expect(screen.queryByRole('button', { name: 'Get started' })).toBeNull();
  complete({ selected: 'localVm', localVm: 'starting', model: 'locked' });
  await waitFor(() => expect(screen.getByText('Starting the workspace…')).toBeTruthy());
  expect(screen.queryByRole('button', { name: 'Get started' })).toBeNull();
});

// Launch: Welcome whenever nothing is set up; Home once something is.
const team = { key: 'binding-primary', context: 'team-dev', namespace: 'agents', service: 'control / orka', stale: false, observedAt: 1700000000000, agents: [] };
function launch(status: object, places: object[] = [], upgrade: () => unknown = () => noUpgrade) {
  vi.stubGlobal('isTauri', true);
  const ipc = vi.fn((command: string) => {
    if (command === 'workspace_upgrade_status') return upgrade();
    if (command === 'setup_status') return status;
    if (command === 'setup_requirements') return requirements;
    if (command === 'cluster_places') return { places, selectedKey: null, persistenceState: 'complete' };
    if (command === 'overview_places') return { places, historyState: 'complete', unobserved: [] };
    return Promise.reject({ kind: 'unavailable' });
  });
  mockIPC(ipc);
  return ipc;
}
const current = () => document.querySelector('.page-tab[aria-current="page"]')?.getAttribute('aria-label');

test('a computer that dismissed Welcome before but set nothing up opens on Welcome again', async () => {
  localStorage.setItem(preference, '1'); launch(unselected); render(<App />);
  expect(await screen.findByRole('heading', { name: 'Your agents. Your models. Your keys. Your cluster.' })).toBeTruthy();
  expect(current()).toBe('Clusters');
  expect(await within(screen.getByRole('navigation')).findByText('Start here')).toBeTruthy();
  expect(within(screen.getByRole('contentinfo')).getByText('Nothing is set up yet')).toBeTruthy();
});

test.each(['Get started'])('%s dismisses Welcome for the rest of the run', async name => {
  localStorage.setItem(preference, '1'); launch(unselected); const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name }));
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  if (name !== 'Get started') await user.click(await screen.findByRole('button', { name: 'Back to clusters' }));
  expect(await screen.findByRole('button', { name: 'Set up' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Get started' })).toBeNull();
  expect(localStorage.getItem(preference)).toBe('1');
});

test('an unreadable cluster list falls back to the stored dismissal', async () => {
  localStorage.setItem(preference, '1'); native(); render(<App />);
  expect(await screen.findByRole('button', { name: 'Set up' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Get started' })).toBeNull();
  expect(current()).toBe('Clusters');
});

test('a finished workspace opens on Home without first showing the Clusters list', async () => {
  let finish!: (value: unknown) => void;
  const upgrade = new Promise(resolve => { finish = resolve; });
  localStorage.setItem('bridge.firstTest.v1', 'attempted');
  launch({ selected: 'localVm', localVm: 'ready', model: 'available' }, [], () => upgrade); render(<App />);
  await waitFor(() => expect(screen.getByRole('contentinfo').textContent).toContain('Checking the workspace…'));
  expect(screen.getByText('Checking the workspace…', { selector: 'main p' })).toBeTruthy();
  expect(screen.queryByRole('region', { name: 'This computer' })).toBeNull();
  finish(noUpgrade);
  expect(await screen.findByRole('region', { name: 'What do you want done?' })).toBeTruthy();
  expect(current()).toBe('Home');
  expect(screen.queryByRole('button', { name: 'Get started' })).toBeNull();
});

test('a remembered cluster without a local workspace opens on Home, even with fresh storage', async () => {
  launch(unselected, [team]); render(<App />);
  expect(await screen.findByRole('region', { name: 'What do you want done?' })).toBeTruthy();
  expect(current()).toBe('Home');
  expect(screen.queryByRole('button', { name: 'Get started' })).toBeNull();
});

test.each([
  ['setup in progress', { selected: 'localVm', localVm: 'starting', model: 'locked' }, 'attempted', noUpgrade],
  ['the first test still pending', { selected: 'localVm', localVm: 'ready', model: 'available' }, 'pending', noUpgrade],
  ['a workspace update', { selected: 'localVm', localVm: 'ready', model: 'available', workspaceReplaced: true }, 'attempted', { ...noUpgrade, state: 'incomplete' }],
])('%s still opens on Clusters', async (_, status, firstTest, upgrade) => {
  localStorage.setItem(preference, '1'); localStorage.setItem('bridge.firstTest.v1', firstTest);
  launch(status, [team], () => upgrade); render(<App />);
  await waitFor(() => expect(screen.queryByText('Checking the workspace…', { selector: 'main p' })).toBeNull());
  await new Promise(resolve => setTimeout(resolve, 50));
  expect(current()).toBe('Clusters');
  expect(screen.queryByRole('button', { name: 'Get started' })).toBeNull();
});

test('a page chosen before the launch reads finish is kept', async () => {
  let finish!: (value: unknown) => void;
  const upgrade = new Promise(resolve => { finish = resolve; });
  localStorage.setItem('bridge.firstTest.v1', 'attempted');
  launch({ selected: 'localVm', localVm: 'ready', model: 'available' }, [], () => upgrade); const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  finish(noUpgrade);
  await waitFor(() => expect(screen.getByRole('contentinfo').textContent).not.toContain('Checking the workspace…'));
  expect(current()).toBe('Overview');
});

const measuredMac = { memoryBytes: 6442450944, stateDiskBytes: 34359738368, downloadBytes: 2380070756, setupDiskBytes: 6997696512, freeDiskBytes: 24257590349, readySeconds: 134 };
test('a measured image shows its setup time on Welcome and its free and kept disk on the choice', async () => {
  native(unselected, measuredMac); render(<App />);
  expect(await screen.findByText('Mostly a 2.4 GB download, then about 2 minutes to start')).toBeTruthy();
  await userEvent.setup().click(screen.getByRole('button', { name: 'Get started' }));
  await openLocalSetup();
  expect(await screen.findByText('Downloads about 2.4 GB. Needs virtualization, 6.4 GB of memory and 24.3 GB of free disk; about 7.0 GB stays in use. Data disk can grow to 34.4 GB.')).toBeTruthy();
});

test('Welcome claims no time or size it was not given', async () => {
  native(unselected, null); render(<App />);
  expect(await screen.findByText('Mostly a download')).toBeTruthy();
  expect(document.body.textContent).not.toMatch(/minute|\d GB/);
});
