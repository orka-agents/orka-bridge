import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import App from './App';

afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); });

test('navigation_order', () => {
  render(<App />);
  const nav = screen.getByRole('navigation', { name: 'Main' });
  expect(within(nav).getAllByRole('button').map(button => button.textContent))
    .toEqual(['Overview', 'Designer', 'Clusters']);
  expect(screen.getByRole('img', { name: 'Orka' })).toBeTruthy();
  expect(screen.getByText('Bridge')).toBeTruthy();
});

test('selected_heading', async () => {
  const user = userEvent.setup();
  render(<App />);
  expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Clusters');
  for (const name of ['Designer', 'Clusters', 'Overview']) {
    await user.click(screen.getByRole('button', { name }));
    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
    expect(screen.getByRole('main', { name })).toBeTruthy();
    const selected = screen.getAllByRole('button').filter(button => button.getAttribute('aria-current') === 'page');
    expect(selected).toEqual([screen.getByRole('button', { name })]);
  }
});

test('keyboard_activation', async () => {
  const user = userEvent.setup();
  render(<App />);
  await user.tab();
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Overview' }));
  await user.tab();
  await user.keyboard(' ');
  expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Designer');
  await user.tab();
  await user.keyboard('{Enter}');
  expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Clusters');
});

test('honest_empty_states', async () => {
  const user = userEvent.setup();
  render(<App />);
  for (const name of ['Overview', 'Designer']) {
    await user.click(screen.getByRole('button', { name }));
    const main = screen.getByRole('main', { name });
    expect(within(main).getByText('Nothing is connected yet.')).toBeTruthy();
    expect(within(main).getAllByRole('heading', { level: 2 })).toHaveLength(name === 'Clusters' ? 2 : 1);
    expect(within(main).queryByRole('button')).toBeNull();
    expect(within(main).queryByRole('link')).toBeNull();
  }
});

test('Clusters reads setup and version without fetch or legacy IPC', async () => {
  const ipc = vi.fn(command => command === 'setup_status'
    ? { selected: null, localVm: 'notIntegrated', model: 'locked' }
    : { kind: 'version', version: 'v0.4.2', hasDiagnostics: false });
  mockIPC(ipc);
  vi.stubGlobal('isTauri', true);
  const fetch = vi.fn(); vi.stubGlobal('fetch', fetch);
  const user = userEvent.setup(); render(<App />);
  expect(await screen.findByText('Native setup available')).toBeTruthy();
  await user.click(screen.getByText('Engine diagnostics'));
  expect(await screen.findByRole('heading', { name: 'Engine available' })).toBeTruthy();
  expect(screen.getByText('Version v0.4.2. Cluster readiness has not been checked.')).toBeTruthy();
  expect(ipc.mock.calls.map(call => call[0])).toEqual(['setup_status', 'engine_call']);
  expect(ipc).toHaveBeenCalledWith('engine_call', { request: { action: { type: 'version' } } });
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  expect(screen.queryByRole('heading', { name: 'Engine available' })).toBeNull();
  expect(ipc).toHaveBeenCalledTimes(2);
  expect(fetch).not.toHaveBeenCalled();
});
