import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import Setup from './Setup';
import Model from './Model';
import { SetupProgress } from './Steps';
const { invoke, isTauri } = vi.hoisted(() => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri }));
beforeEach(() => { invoke.mockReset(); isTauri.mockReturnValue(true); localStorage.setItem('bridge.welcomeSeen.v1', '1'); });
afterEach(() => { cleanup(); localStorage.clear(); });

test('download shows Meet beside progress and the AI choice inside step 3, with one Cancel', async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'installing', model: 'locked', modelSupported: true };
    if (command === 'setup_example') return { agent: '---\nname: web-reader\ndescription: Reads a page\n---\nFetch a page.', tests: '' };
    if (command === 'setup_requirements') return null;
    if (command === 'cluster_places') return { places: [], persistenceState: 'complete', selectedKey: null };
    throw Error(command);
  });
  render(<Setup onOpenExample={() => {}} />);
  const steps = await screen.findByRole('list', { name: 'Setup steps' });
  const ai = within(steps).getByRole('button', { name: 'Change' });
  expect(ai.closest('li')?.textContent).toContain('Connect the AI');
  expect(screen.getByText('Meet your first agent').closest('.setup-side')?.querySelectorAll('section.card')).toHaveLength(1);
  expect(screen.getAllByRole('button', { name: /^Cancel$/ })).toHaveLength(1);
  expect(screen.queryByRole('radio')).toBeNull();
  fireEvent.click(ai);
  expect(within(steps).getByRole('radio', { name: 'GitHub Copilot' })).toBeTruthy();
  fireEvent.click(within(steps).getByRole('button', { name: 'Connect the AI' }));
  expect(screen.queryByRole('radio')).toBeNull();
  expect(screen.getAllByRole('button', { name: /^Cancel$/ })).toHaveLength(1);
  expect(invoke.mock.calls.some(([cmd]) => cmd === 'setup_connect_model')).toBe(false);
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
  invoke.mockImplementation(() => new Promise(() => {}));
  view.rerender(<Model enabled inline={false} choiceTarget={target} />);
  await act(async () => {});
  expect(document.activeElement?.closest('.setup-model')).toBeTruthy();
  expect(invoke).toHaveBeenCalledExactlyOnceWith('setup_connect_model');
  view.unmount(); target.remove();
});

test('the collapsed queued summary is a persistent live acknowledgement', () => {
  render(<Model enabled={false} queueable inline />);
  fireEvent.click(screen.getByRole('button', { name: 'Change' }));
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  const summary = screen.getByText('GitHub Copilot, connects when the workspace is ready');
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

test.each(['unavailable', 'stopping'] as const)('a retained ready snapshot cannot paint Ready while %s', mode => {
  render(<SetupProgress status={{ selected: 'localVm', localVm: 'ready', model: 'available' }} modelState="connected" busy={mode === 'stopping'} stopping={mode === 'stopping'} error={mode === 'unavailable' ? 'Observation unavailable' : undefined} onCancel={() => {}} onRestart={() => {}} onChange={() => {}} onOpenExample={() => {}} />);
  const pill = document.querySelector('.steps-head .pill')!;
  expect(pill.classList.contains('status-passed')).toBe(false);
  expect(pill.textContent).toBe(mode === 'stopping' ? 'Stopping' : 'Status unavailable');
});

test('an empty or whitespace key disables Connect, without keeping the credential in React state', async () => {
  render(<Model enabled />);
  fireEvent.click(screen.getByRole('radio', { name: /Your own API key/ }));
  const connect = screen.getByRole('button', { name: 'Connect the AI' });
  expect(connect.matches(':disabled')).toBe(true);
  fireEvent.change(screen.getByLabelText('API key'), { target: { value: '   ' } });
  expect(connect.matches(':disabled')).toBe(true);
  fireEvent.change(screen.getByLabelText('API key'), { target: { value: 'sk-ant-synthetic-only' } });
  expect(connect.matches(':disabled')).toBe(false);
  await act(async () => {});
  expect(document.body.textContent).not.toContain('sk-ant-synthetic-only');
});
