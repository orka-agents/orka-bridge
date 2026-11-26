import { readFileSync } from 'node:fs';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import App from '../App';
import { noUpgrade } from '../setup/test-upgrade';
import { SetupProgress } from '../setup/Steps';
import { guidedStep } from './guided';

afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

const example = { agent: '---\nname: web-reader\nkmx:\n  runtime: orka\n  schemaVersion: 1\n  tools: [{ name: web_fetch, kind: builtin }]\n---\nRead the page and report its checksum.', tests: 'A test.' };
function nativeSetup(status: Record<string, unknown>, preflight?: Record<string, unknown>) {
  vi.stubGlobal('isTauri', true);
  mockIPC(command => {
    if (command === 'workspace_upgrade_status') return noUpgrade;
    if (command === 'setup_status') return status;
    if (command === 'setup_requirements') return { memoryBytes: 6 * 1024 ** 3, stateDiskBytes: 32e9, downloadBytes: null, setupDiskBytes: null, freeDiskBytes: null, readySeconds: null };
    if (command === 'setup_preflight' && preflight) return { platform: 'windows', virtualization: 'ready', memory: 'ready', installedMemoryBytes: 16 * 1024 ** 3, requiredMemoryBytes: 6 * 1024 ** 3, ...preflight };
    if (command === 'setup_example') return example;
    if (command === 'cluster_places') return { places: [], selectedKey: null, persistenceState: 'complete' };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
}
// A guided run already past Welcome, as after Get started on a first launch.
function guidedRun() {
  localStorage.setItem('bridge.welcomeSeen.v1', '1');
  localStorage.setItem('bridge.firstRun.v1', 'active');
}
const shell = () => document.querySelector('.app-shell')!;

test('a first launch is guided: the title bar shows the setup steps and the status card keeps announcing', async () => {
  nativeSetup({ selected: null, localVm: 'idle', model: 'locked' });
  const user = userEvent.setup();
  render(<App />);
  expect(shell().classList.contains('guided')).toBe(true);
  const steps = within(screen.getByRole('list', { name: 'Setup progress' })).getAllByRole('listitem');
  expect(steps.map(step => step.textContent)).toEqual(['1Set up', '2Connect AI', '3Try it']);
  expect(steps[0].getAttribute('aria-current')).toBe('step');
  expect(localStorage.getItem('bridge.firstRun.v1')).toBe('active');
  // The status card is off screen during guidance (firstrun.css), but its live region stays in the page.
  expect(screen.getByRole('contentinfo').querySelector('[role="status"]')).not.toBeNull();
  // The title bar has no way out of guidance: past Welcome it holds only the steps and the window's own controls.
  await user.click(await screen.findByRole('button', { name: 'Get started' }));
  expect(screen.queryByRole('button', { name: 'Show all of Bridge' })).toBeNull();
  await user.click(await screen.findByRole('button', { name: 'Set up' }));
  await screen.findByRole('heading', { name: 'Where should your agents run?' });
  expect(screen.queryByRole('button', { name: 'Show all of Bridge' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Set up later' })).toBeNull();
  expect(document.querySelector('.title-bar .guided-exit')).toBeNull();
  expect(within(document.querySelector<HTMLElement>('.title-bar-end')!).queryAllByRole('button').map(button => button.textContent?.trim() || button.getAttribute('aria-label'))).not.toContain('Show all of Bridge');
});

test('someone who already dismissed Welcome keeps the sidebar', () => {
  localStorage.setItem('bridge.welcomeSeen.v1', '1');
  nativeSetup({ selected: 'localVm', localVm: 'ready', model: 'available' });
  render(<App />);
  expect(shell().classList.contains('guided')).toBe(false);
  expect(screen.queryByRole('list', { name: 'Setup progress' })).toBeNull();
});

test('Set up later on a paused setup ends the guided run, keeps the page and its heading focused, and is remembered', async () => {
  guidedRun();
  nativeSetup({ selected: 'localVm', localVm: 'cancelled', model: 'locked' });
  const user = userEvent.setup();
  const { unmount } = render(<App />);
  expect(shell().classList.contains('guided')).toBe(true);
  const later = await screen.findByRole('button', { name: 'Set up later' });
  // Beside the paused setup's own action, reachable from the keyboard.
  expect(later.closest('.workspace-step-actions')?.textContent).toContain('Start again');
  expect(later.tabIndex).toBe(0);
  await user.click(later);
  expect(shell().classList.contains('guided')).toBe(false);
  expect(localStorage.getItem('bridge.firstRun.v1')).toBe('done');
  expect(screen.getByRole('button', { name: 'Clusters' }).getAttribute('aria-current')).toBe('page');
  expect(screen.queryByRole('button', { name: 'Set up later' })).toBeNull();
  await waitFor(() => expect(document.activeElement?.id).toBe('page-title'));
  unmount();
  render(<App />);
  expect(shell().classList.contains('guided')).toBe(false);
});

test('opening another page ends the guided run', async () => {
  localStorage.setItem('bridge.welcomeSeen.v1', '1');
  localStorage.setItem('bridge.firstRun.v1', 'active');
  nativeSetup({ selected: 'localVm', localVm: 'starting', model: 'locked' });
  const user = userEvent.setup();
  render(<App />);
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await waitFor(() => expect(shell().classList.contains('guided')).toBe(false));
  expect(localStorage.getItem('bridge.firstRun.v1')).toBe('done');
});

test('setup stages map onto three guided steps', () => {
  expect([0, 1, 2, 3, 4].map(step => guidedStep(step, false))).toEqual([0, 0, 0, 1, 2]);
  expect(guidedStep(4, true)).toBe(3);
});

test('an install whose automatic first test already ran is not a first launch', () => {
  localStorage.setItem('bridge.firstTest.v1', 'attempted');
  nativeSetup({ selected: null, localVm: 'idle', model: 'locked' });
  render(<App />);
  expect(shell().classList.contains('guided')).toBe(false);
});

test('a chosen workspace without a dismissed Welcome means cleared storage, so guidance ends', async () => {
  nativeSetup({ selected: 'localVm', localVm: 'ready', model: 'available' });
  render(<App />);
  await waitFor(() => expect(shell().classList.contains('guided')).toBe(false));
  expect(localStorage.getItem('bridge.firstRun.v1')).toBe('done');
});

test('guidance hides the sidebar but keeps the status live region; elsewhere the status card stays visible', () => {
  const guided = readFileSync('src/firstrun/firstrun.css', 'utf8');
  const rule = (selector: string) => guided.match(new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`))?.[1] ?? '';
  // The guided page spans the window: one column, no sidebar.
  expect(rule('.app-shell.guided')).toMatch(/grid-template-areas:\s*"title"\s*"main"/);
  expect(rule('.app-shell.guided > .page-nav')).toMatch(/display:\s*none/);
  // The status card is visually hidden, never removed, so assistive technology still hears its status.
  const status = rule('.app-shell.guided > .status-bar');
  expect(status).toMatch(/clip-path:\s*inset\(50%\)/);
  expect(status).not.toMatch(/display:\s*none|visibility:\s*hidden/);
  expect(guided).toMatch(/\.app-shell\.guided > \.status-bar > :not\(\.status-lines, \[role="status"\]\)\s*\{\s*display:\s*none;\s*\}/);
  expect(guided).not.toMatch(/\.status-lines[^,{]*\{[^}]*display:\s*none/);
  // Outside guidance nothing hides the status card or the sidebar.
  expect(guided.replace(/\.app-shell\.guided[^{]*\{[^}]*\}/g, '')).not.toMatch(/status-bar|page-nav/);
});

test('a computer with nothing set up is guided again alongside Welcome, and leaving is kept for the run', async () => {
  localStorage.setItem('bridge.welcomeSeen.v1', '1');
  localStorage.setItem('bridge.firstRun.v1', 'done');
  nativeSetup({ selected: null, localVm: 'idle', model: 'locked' }, { memory: 'insufficient', installedMemoryBytes: 4 * 1024 ** 3 });
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('button', { name: 'Get started' });
  await waitFor(() => expect(shell().classList.contains('guided')).toBe(true));
  expect(localStorage.getItem('bridge.firstRun.v1')).toBe('active');
  await user.click(screen.getByRole('button', { name: 'Get started' }));
  await user.click(await screen.findByRole('button', { name: 'Set up' }));
  // This computer can't run the workspace, so its checks offer the way out.
  await user.click(await screen.findByRole('button', { name: 'Set up later' }));
  expect(shell().classList.contains('guided')).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  // Clusters again shows the list, where This computer still offers Set up, without guidance coming back.
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  expect(await screen.findByRole('button', { name: 'Set up' })).toBeTruthy();
  expect(shell().classList.contains('guided')).toBe(false);
  expect(localStorage.getItem('bridge.firstRun.v1')).toBe('done');
});

test('Read its instructions opens them in place on the Meet card, and Open in Designer is the way out', async () => {
  guidedRun();
  nativeSetup({ selected: 'localVm', localVm: 'starting', model: 'locked' });
  const user = userEvent.setup();
  render(<App />);
  const meet = await screen.findByRole('region', { name: 'web-reader' });
  const read = await within(meet).findByRole('button', { name: 'Read its instructions' });
  expect(read.getAttribute('aria-expanded')).toBe('false');
  expect(within(meet).queryByRole('button', { name: 'Open in Designer' })).toBeNull();
  await user.click(read);
  expect(read.getAttribute('aria-expanded')).toBe('true');
  const text = within(meet).getByText('Read the page and report its checksum.');
  // Read-only and bounded: not a field, and focusable so a long text can scroll from the keyboard.
  expect(text.closest('textarea, input, [contenteditable="true"]')).toBeNull();
  expect(text.classList.contains('agent-instructions-text')).toBe(true);
  expect(text.tabIndex).toBe(0);
  expect(shell().classList.contains('guided')).toBe(true);
  expect(screen.getByRole('main').dataset.page).toBe('Clusters');
  await user.click(read);
  expect(within(meet).queryByText('Read the page and report its checksum.')).toBeNull();
  expect(shell().classList.contains('guided')).toBe(true);
  await user.click(read);
  await user.click(within(meet).getByRole('button', { name: 'Open in Designer' }));
  await waitFor(() => expect(screen.getByRole('main').dataset.page).toBe('Designer'));
  expect(shell().classList.contains('guided')).toBe(false);
  expect(localStorage.getItem('bridge.firstRun.v1')).toBe('done');
});

test('with the workspace ready, its instructions also open in place until the AI connects', async () => {
  guidedRun();
  const status: Record<string, unknown> = { selected: 'localVm', localVm: 'starting', model: 'locked' };
  nativeSetup(status);
  const user = userEvent.setup();
  render(<App />);
  const progress = await screen.findByRole('region', { name: 'Workspace setup' });
  Object.assign(status, { localVm: 'ready', model: 'available' });
  await within(progress).findByText('Your workspace is ready on this computer.');
  await user.click(await within(progress).findByRole('button', { name: 'Read its instructions' }));
  expect(await within(progress).findByText('Read the page and report its checksum.')).toBeTruthy();
  expect(shell().classList.contains('guided')).toBe(true);
  expect(screen.getByRole('main').dataset.page).toBe('Clusters');
  await user.click(within(progress).getByRole('button', { name: 'Open in Designer' }));
  await waitFor(() => expect(shell().classList.contains('guided')).toBe(false));
});

test('outside the guided run, Read its instructions still opens Designer', async () => {
  localStorage.setItem('bridge.welcomeSeen.v1', '1');
  localStorage.setItem('bridge.firstRun.v1', 'done');
  // Outside the guided run, Meet is beside a paused setup; a workspace getting ready is the two-card page.
  nativeSetup({ selected: 'localVm', localVm: 'cancelled', model: 'locked' });
  const user = userEvent.setup();
  render(<App />);
  const meet = await screen.findByRole('region', { name: 'web-reader' });
  const read = await within(meet).findByRole('button', { name: 'Read its instructions' });
  expect(read.tagName).toBe('BUTTON');
  await user.click(read);
  await waitFor(() => expect(screen.getByRole('main').dataset.page).toBe('Designer'));
});

// Ordinary guided screens have no way out but going forward; screens where setup has stopped offer Set up later.
test.each([
  ['the workspace starting', { localVm: 'starting', model: 'locked' }, false],
  ['the workspace preparing', { localVm: 'preparing', model: 'locked' }, false],
  ['the workspace ready for the AI', { localVm: 'ready', model: 'available' }, false],
  ['a paused setup', { localVm: 'cancelled', model: 'locked' }, true],
  ['a workspace that could not start', { localVm: 'failed', model: 'locked', localVmError: 'boot failed' }, true],
  ['a download that failed', { localVm: 'failed', model: 'locked', workspaceFailure: { operation: 'download', cause: 'network', codes: [] } }, true],
  ['a forced stop', { localVm: 'failed', model: 'locked', workspaceFailure: { operation: 'start', cause: 'forcedStop', codes: [] } }, true],
  ['a failed computer check at start', { localVm: 'failed', model: 'locked', preflightFailure: 'insufficientMemory' }, true],
  ['a build without the workspace', { localVm: 'notIntegrated', model: 'locked' }, true],
] as const)('guided: %s', async (_name, status, offered) => {
  guidedRun();
  // Setup opens on its progress card while the workspace gets ready, as a first run does, then reaches the state.
  const current: Record<string, unknown> = { selected: 'localVm', localVm: 'starting', model: 'locked' };
  nativeSetup(current);
  render(<App />);
  await screen.findByRole('region', { name: 'Workspace setup' });
  Object.assign(current, status);
  await waitFor(() => expect(document.querySelector('.setup-progress')?.textContent).toMatch(status.localVm === 'ready' ? /Your workspace is ready/ : /./));
  expect(screen.queryByRole('button', { name: 'Show all of Bridge' })).toBeNull();
  if (offered) {
    const later = await screen.findByRole('button', { name: 'Set up later' });
    expect(later.matches(':disabled')).toBe(false);
    if (status.localVm === 'failed') expect(later.closest('.workspace-failure')?.textContent).toContain('Use a cluster you already have instead');
  } else expect(screen.queryByRole('button', { name: 'Set up later' })).toBeNull();
  expect(shell().classList.contains('guided')).toBe(true);
});

test('guided: failed computer checks before setup offer Set up later beside the cluster alternative', async () => {
  localStorage.setItem('bridge.welcomeSeen.v1', '1');
  nativeSetup({ selected: null, localVm: 'idle', model: 'locked' }, { virtualization: 'disabled' });
  const user = userEvent.setup();
  render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Get started' }));
  await user.click(await screen.findByRole('button', { name: 'Set up' }));
  const checks = await screen.findByRole('region', { name: 'Computer requirements' });
  const later = await within(checks).findByRole('button', { name: 'Set up later' });
  expect(later.closest('.workspace-step-actions')?.textContent).toContain('Use a cluster instead');
  await user.click(later);
  expect(shell().classList.contains('guided')).toBe(false);
  expect(screen.getByRole('navigation', { name: 'Main' })).toBeTruthy();
});

// The AI and first-test screens where guided setup stops, and the Try it screens that move forward instead.
const ready = { selected: 'localVm', localVm: 'ready', model: 'available' } as const;
const progress = { status: ready, busy: false, stopping: false, guided: true, onCancel() {}, onRestart() {}, onOpenExample() {} };
test.each([
  ["the AI couldn't connect", { modelState: 'failed' }, true],
  ["the AI connection couldn't be reset", { modelState: 'resetFailed' }, true],
  ["the saved sign-in couldn't be removed", { modelState: 'removalFailed' }, true],
  ['the AI waiting to connect', { modelState: 'idle' }, false],
  ['the AI connecting', { modelState: 'connecting' }, false],
  ['the first test running', { modelState: 'connected', firstCheck: 'running' }, false],
  ['the first test failed', { modelState: 'connected', firstCheck: 'failed', firstTestResult: { failed: 1 } }, true],
  ['the first test result unknown', { modelState: 'connected', firstCheck: 'unknown', onCheckFirstTest() {} }, true],
  ['the first test not confirmed for want of the AI', { modelState: 'connected', firstCheck: 'unknown', modelUnavailable: true, onChooseProvider() {} }, true],
  ['the first test needing a new run', { modelState: 'connected', firstCheck: 'stale' }, false],
] as const)('guided: %s', async (_name, props, offered) => {
  const later = vi.fn();
  const user = userEvent.setup();
  render(<SetupProgress {...progress} {...props} onSetUpLater={later} />);
  const action = screen.queryByRole('button', { name: 'Set up later' });
  expect(!!action).toBe(offered);
  if (action) { await user.click(action); expect(later).toHaveBeenCalledOnce(); }
});
