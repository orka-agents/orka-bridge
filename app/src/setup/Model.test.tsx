import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import Model from './Model';
import { connectModel } from './modelState';

const { invoke, isTauri } = vi.hoisted(() => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri }));
beforeEach(() => { invoke.mockReset(); isTauri.mockReturnValue(true); });
afterEach(() => { cleanup(); });
test('locked model cannot invoke credentials', () => {
  render(<Model enabled={false} />);
  expect(screen.getByRole('button', { name: 'Connect model' }).hasAttribute('disabled')).toBe(true);
  expect(invoke).not.toHaveBeenCalled();
});
test('explicit sign-in uses no renderer arguments, excludes duplicate clicks and shows acknowledgment', async () => {
  let complete!: (value: unknown) => void;
  invoke.mockImplementation(() => new Promise(resolve => { complete = resolve; }));
  render(<Model enabled />);
  const button = screen.getByRole('button', { name: 'Use GitHub sign-in' });
  fireEvent.click(button); fireEvent.click(button);
  expect(invoke).toHaveBeenCalledExactlyOnceWith('setup_connect_model');
  expect(button.hasAttribute('disabled')).toBe(true);
  await act(async () => { complete({ ready: true }); });
  expect(screen.getByText('Model connected: copilot / gpt-5.6-terra.')).toBeTruthy();
  expect(screen.getByText(/does not prove inference/)).toBeTruthy();
  expect(screen.queryByText(/run the example/)).toBeNull();
});
test('known native stage error is shown without raw message; unknown errors never echo', async () => {
  invoke.mockRejectedValue({ kind: 'modelConnect', message: 'synthetic-private-token /path/raw-stderr' });
  render(<Model enabled />); fireEvent.click(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Guest Connect did not acknowledge a ready model'));
  expect(screen.queryByText(/synthetic-private-token/)).toBeNull();
  invoke.mockRejectedValue({ kind: 'invented', message: 'synthetic-private-token' });
  const result = await connectModel();
  expect(result).toEqual({ ok: false, message: 'The model connection could not be completed.' });
});
test('late success after runtime closes does not display ready', async () => {
  let complete!: (value: unknown) => void;
  invoke.mockImplementation(() => new Promise(resolve => { complete = resolve; }));
  const view = render(<Model enabled />);
  fireEvent.click(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  view.rerender(<Model enabled={false} />);
  await act(async () => { complete({ ready: true }); });
  expect(screen.queryByText(/Model connected:/)).toBeNull();
});
test('preview and malformed responses cannot claim ready', async () => {
  isTauri.mockReturnValue(false);
  expect((await connectModel()).ok).toBe(false); expect(invoke).not.toHaveBeenCalled();
  isTauri.mockReturnValue(true);
  for (const response of [{ ready: false }, { ready: true, token: 'synthetic-private-token' }, null]) {
    invoke.mockResolvedValue(response); expect((await connectModel()).ok).toBe(false);
  }
});
