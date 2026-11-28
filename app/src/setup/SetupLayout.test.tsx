import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import Setup from './Setup';
import Model from './Model';
import { SetupProgress } from './Steps';
const { invoke, isTauri } = vi.hoisted(() => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri }));
beforeEach(() => { invoke.mockReset(); isTauri.mockReturnValue(true); localStorage.setItem('bridge.welcomeSeen.v1', '1'); });
afterEach(() => { cleanup(); localStorage.clear(); });

test('a downloading workspace shows the AI card beside its one card, with one Cancel and no Meet outside the guided run', async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'installing', model: 'locked', modelSupported: true };
    if (command === 'setup_example') return { agent: '---\nname: web-reader\ndescription: Reads a page\n---\nFetch a page.', tests: '' };
    if (command === 'setup_requirements') return null;
    if (command === 'cluster_places') return { places: [], persistenceState: 'complete', accessLost: false, selectedKey: null };
    throw Error(command);
  });
  const openExample = vi.fn();
  render(<Setup onOpenExample={openExample} />);
  const workspace = await screen.findByRole('region', { name: 'Workspace setup' });
  expect(screen.queryByRole('list', { name: 'Setup steps' })).toBeNull();
  // Without Meet, the example's introduction is one quiet click away in the Workspace card.
  const see = within(workspace).getByRole('button', { name: 'See your first agent' });
  expect(see.className).toContain('button-quiet');
  fireEvent.click(see);
  expect(openExample).toHaveBeenCalledTimes(1);
  const ai = (await screen.findByRole('heading', { name: 'Connect the AI' })).closest('section')!;
  expect(ai.closest('.setup-side')).toBeTruthy();
  expect(workspace.closest('.setup-main')).toBeTruthy();
  expect(screen.queryByText('Meet your first agent')).toBeNull();
  expect(screen.getAllByRole('button', { name: /^Cancel$/ })).toHaveLength(1);
  expect(within(ai).getByRole('radio', { name: 'GitHub Copilot' })).toBeTruthy();
  fireEvent.click(within(ai).getByRole('button', { name: 'Connect the AI' }));
  expect(within(ai).queryByRole('radio')).toBeNull();
  expect(within(ai).getByText('Queued', { selector: '.pill' })).toBeTruthy();
  expect(within(ai).getByRole('button', { name: 'Cancel queued connection' })).toBeTruthy();
  expect(invoke.mock.calls.some(([cmd]) => cmd === 'setup_connect_model')).toBe(false);
});

test('in the guided first run, the download shows the compact AI card above Meet beside the steps, with one Cancel', async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'installing', model: 'locked', modelSupported: true };
    if (command === 'setup_example') return { agent: '---\nname: web-reader\ndescription: Reads a page\n---\nFetch a page.', tests: '' };
    if (command === 'setup_requirements') return null;
    if (command === 'cluster_places') return { places: [], persistenceState: 'complete', accessLost: false, selectedKey: null };
    throw Error(command);
  });
  render(<Setup guided onOpenExample={() => {}} />);
  const steps = await screen.findByRole('list', { name: 'Setup steps' });
  const change = await screen.findByRole('button', { name: 'Change' });
  const card = change.closest('.setup-model')!;
  expect(card.closest('.setup-side')).toBeTruthy();
  expect(steps.contains(change)).toBe(false);
  const meet = screen.getByText('Meet your first agent').closest('section')!;
  expect(card.compareDocumentPosition(meet) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(screen.getAllByRole('button', { name: /^Cancel$/ })).toHaveLength(1);
  expect(screen.queryByRole('radio')).toBeNull();
  fireEvent.click(change);
  expect(within(card as HTMLElement).getByRole('radio', { name: 'GitHub Copilot' })).toBeTruthy();
});

test('connected card exposes only Change, and Escape closes its panel without removing sign-in', async () => {
  invoke.mockResolvedValue({ ready: true }); render(<Model enabled />);
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  const change = await screen.findByRole('button', { name: 'Change' });
  expect(screen.getByText('Connected').closest('.steps-head')).toBeTruthy();
  expect(screen.queryByRole('button', { name: /Remove/ })).toBeNull();
  change.focus(); fireEvent.click(change);
  const panel = screen.getByRole('group', { name: 'Change the AI' });
  const remove = within(panel).getByRole('button', { name: 'Remove saved sign-in' });
  expect(remove.classList.contains('button-quiet')).toBe(true);
  fireEvent.keyDown(panel, { key: 'Escape' });
  expect(screen.queryByRole('group', { name: 'Change the AI' })).toBeNull();
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Change' }));
  expect(invoke.mock.calls.some(([cmd]) => cmd === 'setup_remove_model')).toBe(false);
});

test('readiness keeps keyboard focus with a queued choice when its card moves out of the step', async () => {
  const target = document.createElement('div'); document.body.append(target);
  const view = render(<Model enabled={false} queueable inline choiceTarget={target} />);
  fireEvent.click(screen.getByRole('button', { name: 'Change' }));
  const connect = screen.getByRole('button', { name: 'Connect the AI' });
  connect.focus(); fireEvent.click(connect);
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Change' }));
  let finish!: (value: unknown) => void;
  invoke.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  view.rerender(<Model enabled inline={false} choiceTarget={target} />);
  await act(async () => {});
  expect(document.activeElement?.closest('.setup-model')).toBeTruthy();
  expect(invoke.mock.calls.filter(([command]) => command !== 'setup_check_model')).toEqual([['setup_connect_model']]);
  await act(async () => finish({ ready: true }));
  expect(document.activeElement).toBe(screen.getByText('Connected').closest('[role="status"]'));
  view.unmount(); target.remove();
});

test('a queued inline choice can be revoked without stopping the workspace or connecting at readiness', async () => {
  const view = render(<Model enabled={false} queueable inline />);
  fireEvent.click(screen.getByRole('button', { name: 'Change' }));
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  fireEvent.click(screen.getByRole('button', { name: 'Change' }));
  const revoke = screen.getByRole('button', { name: "Don't connect automatically" });
  revoke.focus(); fireEvent.click(revoke);
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Change' }));
  view.rerender(<Model enabled />);
  await act(async () => {});
  expect(invoke.mock.calls.filter(([command]) => command !== 'setup_check_model')).toEqual([]);
});

test('the collapsed queued summary is a persistent live acknowledgement', () => {
  render(<Model enabled={false} queueable inline />);
  fireEvent.click(screen.getByRole('button', { name: 'Change' }));
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  const summary = screen.getByText((_, node) => node?.getAttribute('role') === 'status' && node.textContent === 'GitHub Copilot, connects when the workspace is ready');
  expect(summary.getAttribute('role')).toBe('status');
  expect(summary.closest('[hidden]')).toBeNull();
});

test('keyboard connection success retains focus inside the AI card', async () => {
  let finish!: (value: unknown) => void;
  invoke.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  render(<Model enabled />);
  const connect = screen.getByRole('button', { name: 'Connect the AI' });
  connect.focus(); fireEvent.click(connect);
  await act(async () => finish({ ready: true }));
  expect(document.activeElement).toBe(screen.getByText('Connected').closest('[role="status"]'));
});

test('a keyboard-started rejected key returns focus to the empty replacement field', async () => {
  invoke.mockRejectedValue({ kind: 'modelCredentialRejected' });
  render(<Model enabled />);
  fireEvent.click(screen.getByRole('radio', { name: /Your own API key/ }));
  const field = screen.getByLabelText('API key');
  fireEvent.change(field, { target: { value: 'sk-ant-synthetic-only-not-a-real-key' } });
  const connect = screen.getByRole('button', { name: 'Connect the AI' });
  connect.focus(); fireEvent.click(connect);
  await screen.findByRole('alert');
  expect(invoke).toHaveBeenCalledOnce();
  expect(screen.getByRole('button', { name: 'Check again' }).hasAttribute('disabled')).toBe(true);
  await waitFor(() => expect(document.activeElement).toBe(field));
});

test('Escape in ordinary key entry preserves input and cannot prime a later Change focus', async () => {
  let finish!: (value: unknown) => void;
  invoke.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  render(<><Model enabled /><input aria-label="Another field" /></>);
  fireEvent.click(screen.getByRole('radio', { name: /Your own API key/ }));
  const field = screen.getByLabelText('API key') as HTMLInputElement;
  fireEvent.change(field, { target: { value: 'sk-ant-synthetic-only-not-a-real-key' } }); field.focus();
  fireEvent.keyDown(field, { key: 'Escape' });
  expect(field.value).toBe('sk-ant-synthetic-only-not-a-real-key');
  const connect = screen.getByRole('button', { name: 'Connect the AI' });
  connect.focus(); fireEvent.click(connect);
  const other = screen.getByRole('textbox', { name: 'Another field' }); other.focus();
  await act(async () => finish({ ready: true }));
  expect(document.activeElement).toBe(other);
});

test('Escape from recovery removal confirmation restores its action without a stale Change focus', async () => {
  let finish!: (value: unknown) => void;
  invoke.mockRejectedValueOnce({ kind: 'modelConnect' }).mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  render(<><Model enabled /><input aria-label="Another field" /></>);
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  await screen.findByRole('alert');
  fireEvent.click(screen.getByRole('button', { name: 'Remove sign-in' }));
  const cancel = screen.getByRole('button', { name: 'Cancel' }); cancel.focus();
  fireEvent.keyDown(cancel, { key: 'Escape' });
  expect(screen.queryByRole('group', { name: 'Remove sign-in confirmation' })).toBeNull();
  expect(invoke).toHaveBeenCalledTimes(1);
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Remove sign-in' })));
  const retry = screen.getByRole('button', { name: 'Check again' }); retry.focus(); fireEvent.click(retry);
  const other = screen.getByRole('textbox', { name: 'Another field' }); other.focus();
  await act(async () => finish({ ready: true }));
  expect(document.activeElement).toBe(other);
});

test.each(['unavailable', 'stopping'] as const)('a retained ready snapshot cannot paint Ready while %s', mode => {
  render(<SetupProgress status={{ selected: 'localVm', localVm: 'ready', model: 'available' }} modelState="connected" busy={mode === 'stopping'} stopping={mode === 'stopping'} error={mode === 'unavailable' ? 'Observation unavailable' : undefined} onCancel={() => {}} onRestart={() => {}} onOpenExample={() => {}} />);
  const pill = document.querySelector('.steps-head .pill')!;
  expect(pill.classList.contains('status-passed')).toBe(false);
  expect(pill.textContent).toBe(mode === 'stopping' ? 'Stopping' : 'Status unavailable');
});
