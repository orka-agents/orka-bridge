import { cleanup, render, screen, within } from '@testing-library/react';
import { afterEach, expect, test } from 'vitest';
import { infraProgress, setupView, workspaceView } from './progress';
import { SetupProgress } from './Steps';
import type { SetupStatus } from './state';

const status = (localVm: SetupStatus['localVm'], extra: Partial<SetupStatus> = {}): SetupStatus => ({ selected: 'localVm', localVm, model: localVm === 'ready' ? 'available' : 'locked', ...extra });
const actions = { busy: false, stopping: false, onCancel() {}, onRestart() {}, onChange() {}, onOpenExample() {} };
const names = ['Where agents run', 'Get your workspace ready', 'Connect the AI', 'Run your first agent'];
afterEach(cleanup);

test('all infrastructure phases and failures share step two, and shell counters follow it', () => {
  for (const localVm of ['preparing', 'installing', 'starting'] as const) {
    expect(setupView(status(localVm))).toMatchObject({ step: 2, tabLine: 'Setting up, step 2 of 4', footerLine: 'Getting your workspace ready, step 2 of 4' });
  }
  for (const localVm of ['cancelled', 'closed', 'failed'] as const) expect(setupView(status(localVm))).toMatchObject({ step: 2 });
  for (const operation of ['download', 'prepare', 'start', 'services', 'stop'] as const) expect(setupView(status('failed', { workspaceFailure: { operation, cause: 'unknown', codes: [] } })).step).toBe(2);
  expect(setupView(status('failed', { preflightFailure: 'virtualizationDisabled' })).step).toBe(2);
  for (const model of ['idle', 'queued', 'connecting', 'failed'] as const) expect(setupView(status('ready'), { model })).toMatchObject({ step: 3, action: 'Connect the AI' });
  expect(setupView(status('ready'), { model: 'connected', initialAutomatic: true })).toMatchObject({ step: 4, tabLine: 'Setting up, step 4 of 4' });
  expect(setupView(status('ready'), { model: 'connected', check: 'unknown' })).toMatchObject({ step: 4, done: true, action: 'Manage' });
  for (const value of [setupView(undefined), setupView(status('cancelled')), setupView(status('failed')), setupView(status('ready'), { model: 'connected', check: 'running' }), setupView(status('ready'), { model: 'connected', check: 'unknown' })]) {
    expect(value.tabLine).not.toMatch(/step \d/);
    expect(value.footerLine).not.toMatch(/step \d/);
  }
});

test('startup is indeterminate with only observed startup time, never a transfer ETA or rough estimate', () => {
  for (const elapsedMs of [1000, 45000]) {
    const value = infraProgress(status('starting', { workspaceProgress: { phase: 'starting', bytes: null, totalBytes: null, elapsedMs, remainingMs: null } }));
    expect(value).toEqual({ value: null, text: `${elapsedMs / 1000} second${elapsedMs === 1000 ? '' : 's'} elapsed` });
  }
  expect(infraProgress(status('starting', { workspaceProgress: { phase: 'materializing', bytes: 100, totalBytes: 100, elapsedMs: 54000, remainingMs: 1000 } }))).toEqual({ value: null });
  // A full copy is read back before the workspace starts: no full bar that looks stuck, and no time left to promise.
  expect(infraProgress(status('installing', { workspaceProgress: { phase: 'materializing', bytes: 100, totalBytes: 100, elapsedMs: 1000, remainingMs: 1000 } }))).toEqual({ value: null, text: '0.0 GB copied. Bridge checks the copy before the workspace starts.' });
});

test('the same four rows stay in place across setup with progress inside the workspace row', () => {
  const view = render(<SetupProgress {...actions} status={status('installing')} />);
  for (const localVm of ['installing', 'starting', 'ready', 'failed', 'cancelled'] as const) {
    view.rerender(<SetupProgress {...actions} status={status(localVm)} />);
    const list = screen.getByRole('list', { name: 'Setup steps' });
    expect(within(list).getAllByRole('listitem').map(row => row.querySelector('b')?.textContent)).toEqual(names);
    expect(screen.queryByText('Completed setup steps')).toBeNull();
    const bar = screen.queryByRole('progressbar');
    if (localVm === 'installing' || localVm === 'starting') expect(bar?.closest('li')).toBe(within(list).getAllByRole('listitem')[1]);
    else expect(bar).toBeNull();
  }
});

test.each([['installing', 'Getting ready'], ['starting', 'Starting']] as const)('unreadable %s status retains its observed label, not Waiting', (phase, label) => {
  render(<SetupProgress {...actions} status={status(phase)} error="Unavailable" />);
  expect(screen.getByText(`Last known: ${label}`)).toBeTruthy();
  expect(screen.queryByRole('progressbar')).toBeNull();
  expect(document.querySelector('[aria-current="step"]')).toBeNull();
});

test('a saved selection on an unsupported build keeps its explanation visible', () => {
  render(<SetupProgress {...actions} status={status('notIntegrated')} />);
  const explanation = screen.getByRole('status');
  expect(explanation.textContent).toContain('Nothing will be downloaded or started.');
  expect(explanation.classList.contains('sr-only')).toBe(false);
});

test('unreadable status suppresses progress and current step without dropping rows', () => {
  render(<SetupProgress {...actions} status={status('starting')} error="Unavailable" />);
  expect(screen.queryByRole('progressbar')).toBeNull();
  expect(screen.getAllByRole('listitem')).toHaveLength(4);
  expect(document.querySelector('[aria-current="step"]')).toBeNull();
});

const phase = (name: 'verifying' | 'materializing', elapsedMs: number, bytes: number | null = null, totalBytes: number | null = null) => ({ phase: name, bytes, totalBytes, elapsedMs, remainingMs: null });
test('each step says what is happening, and times itself from when this view saw it begin', () => {
  const view = render(<SetupProgress {...actions} cards status={status('installing', { workspaceProgress: phase('verifying', 237000) })} />);
  const card = screen.getByRole('region', { name: 'Workspace setup' });
  expect(within(card).getByText('Getting ready', { selector: '.pill' })).toBeTruthy();
  expect(within(card).getByText('Checking cluster files…')).toBeTruthy();
  expect(within(card).getByText('237 seconds checking files.')).toBeTruthy();
  // Native's clock keeps running into the next step; this step counts from its own start.
  view.rerender(<SetupProgress {...actions} cards status={status('installing', { workspaceProgress: phase('materializing', 258000, 1e9, 3.8e9) })} />);
  expect(within(card).getByText('Installing the workspace…')).toBeTruthy();
  expect(within(card).getByText('Installing cluster files, 1.0 of 3.8 GB')).toBeTruthy();
  expect(within(card).getByText('21 seconds installing files.')).toBeTruthy();
  view.rerender(<SetupProgress {...actions} cards status={status('installing', { workspaceProgress: phase('materializing', 379000, 3.8e9, 3.8e9) })} />);
  expect(within(card).getByText('Checking the installed cluster files…')).toBeTruthy();
  expect(within(card).getByText('3.8 GB copied. Bridge checks the copy before the workspace starts.')).toBeTruthy();
  expect(within(card).getByRole('progressbar').getAttribute('aria-valuenow')).toBeNull();
  expect(within(card).getByText('142 seconds installing files.')).toBeTruthy();
  expect(within(card).queryByText(/Downloading/)).toBeNull();
});

test('a step already under way when the view opens says how long setup has run, not how long the step has', () => {
  render(<SetupProgress {...actions} cards status={status('installing', { workspaceProgress: phase('materializing', 30000, 1e9, 3.8e9) })} />);
  expect(screen.getByText('Setup has been running for 30 seconds.')).toBeTruthy();
});

test('the returning pill and line agree with the step setup is on', () => {
  expect(workspaceView(status('installing', { workspaceProgress: phase('verifying', 1000) }))).toMatchObject({ word: 'Getting ready', detail: 'Checking cluster files.' });
  expect(workspaceView(status('installing', { workspaceProgress: phase('materializing', 1000, 1, 2) }))).toMatchObject({ word: 'Getting ready', detail: 'Installing the workspace.' });
  expect(workspaceView(status('preparing'))).toMatchObject({ word: 'Getting ready', detail: 'Getting the cluster files ready.' });
  expect(workspaceView(status('starting'))).toMatchObject({ word: 'Starting', detail: 'Starting the workspace.' });
});
