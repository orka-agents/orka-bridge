import { readFileSync } from 'node:fs';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import App from '../App';
import { noUpgrade } from '../setup/test-upgrade';
import { guidedStep } from './guided';

afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

const example = { agent: '---\nname: web-reader\nkmx:\n  runtime: orka\n  schemaVersion: 1\n  tools: [{ name: web_fetch, kind: builtin }]\n---\nRead the page and report its checksum.', tests: 'A test.' };
function nativeSetup(status: Record<string, unknown>, preflight?: Record<string, unknown>) {
  vi.stubGlobal('isTauri', true);
  mockIPC(command => {
    if (command === 'workspace_upgrade_status') return noUpgrade;
    if (command === 'setup_status') return status;
    if (command === 'setup_requirements') return { memoryBytes: 6 * 1024 ** 3, imageBytes: 3e9, stateDiskBytes: 32e9, downloadBytes: null };
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
  // Welcome is the only guided screen without a way out; every later one offers Show all of Bridge to the keyboard.
  await user.click(await screen.findByRole('button', { name: 'Get started' }));
  const exit = await screen.findByRole('button', { name: 'Show all of Bridge' });
  expect(exit.closest('.title-bar')).not.toBeNull();
  expect(exit.matches(':disabled')).toBe(false);
  expect(exit.tabIndex).toBe(0);
});

test('someone who already dismissed Welcome keeps the sidebar', () => {
  localStorage.setItem('bridge.welcomeSeen.v1', '1');
  nativeSetup({ selected: 'localVm', localVm: 'ready', model: 'available' });
  render(<App />);
  expect(shell().classList.contains('guided')).toBe(false);
  expect(screen.queryByRole('list', { name: 'Setup progress' })).toBeNull();
});

test('leaving the guided run is remembered', async () => {
  localStorage.setItem('bridge.welcomeSeen.v1', '1');
  localStorage.setItem('bridge.firstRun.v1', 'active');
  nativeSetup({ selected: 'localVm', localVm: 'starting', model: 'locked' });
  const user = userEvent.setup();
  const { unmount } = render(<App />);
  expect(shell().classList.contains('guided')).toBe(true);
  await user.click(await screen.findByRole('button', { name: 'Show all of Bridge' }));
  expect(shell().classList.contains('guided')).toBe(false);
  expect(localStorage.getItem('bridge.firstRun.v1')).toBe('done');
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
  vi.stubGlobal('isTauri', true);
  mockIPC(command => {
    if (command === 'workspace_upgrade_status') return noUpgrade;
    if (command === 'setup_status') return { selected: null, localVm: 'idle', model: 'locked' };
    if (command === 'setup_requirements') return null;
    if (command === 'cluster_places') return { places: [], selectedKey: null, persistenceState: 'complete' };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('button', { name: 'Get started' });
  await waitFor(() => expect(shell().classList.contains('guided')).toBe(true));
  expect(localStorage.getItem('bridge.firstRun.v1')).toBe('active');
  await user.click(screen.getByRole('button', { name: 'Get started' }));
  await user.click(await screen.findByRole('button', { name: 'Show all of Bridge' }));
  expect(shell().classList.contains('guided')).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Overview' }));
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
  nativeSetup({ selected: 'localVm', localVm: 'starting', model: 'locked' });
  const user = userEvent.setup();
  render(<App />);
  const meet = await screen.findByRole('region', { name: 'web-reader' });
  const read = await within(meet).findByRole('button', { name: 'Read its instructions' });
  expect(read.tagName).toBe('BUTTON');
  await user.click(read);
  await waitFor(() => expect(screen.getByRole('main').dataset.page).toBe('Designer'));
});
