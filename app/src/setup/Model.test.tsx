import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import Model from './Model';
import userEvent from '@testing-library/user-event';
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
  expect(result).toMatchObject({ ok: false, message: 'The model connection could not be completed.' });
});
test('a full Windows credential store explains sign-in failure without exposing the native message', async () => {
  invoke.mockRejectedValue({ kind: 'modelStore', message: 'synthetic-private-token /path/raw-stderr',
    native: { call: 'CredWriteW', code: 8 } });
  render(<Model enabled />); fireEvent.click(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  expect((await screen.findByRole('alert')).textContent).toContain(
    "Windows couldn't save the sign-in. Windows Credential Manager is full (CredWriteW, error 8).");
  expect(document.body.textContent).not.toContain('synthetic-private-token');
});
test('native store details accept only known calls and unsigned Windows error codes', async () => {
  invoke.mockRejectedValue({ kind: 'modelStore', native: { call: 'CredReadW', code: 1168 } });
  expect(await connectModel()).toMatchObject({ ok: false,
    message: "Windows couldn't load the sign-in (CredReadW, error 1168)." });
  for (const native of [null, { call: 'synthetic-private-token', code: 8 },
    { call: 'CredWriteW', code: '8' }, { call: 'CredWriteW', code: -1 },
    { call: 'CredWriteW', code: 1.5 }, { call: 'CredWriteW', code: 4294967296 }]) {
    invoke.mockRejectedValue({ kind: 'modelStore', message: 'synthetic-private-token', native });
    expect(await connectModel()).toMatchObject({ ok: false,
      message: 'The native credential store could not save or load the sign-in.' });
  }
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
test('failed CLI recovery offers install and sign-in help and explicitly reconnects', async () => {
  invoke.mockRejectedValue({ kind: 'modelCaptureUnavailable', message: 'SECRET' });
  const observe = vi.fn(); render(<Model enabled onState={observe} />);
  fireEvent.click(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toContain('gh auth login --hostname github.com --web');
  expect(alert.textContent).toContain('workspace and agent files are kept');
  expect(screen.getByRole('button', { name: 'Install GitHub CLI' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Sign-in help' })).toBeTruthy();
  expect(alert.textContent).not.toContain('SECRET');
  await waitFor(() => expect(observe).toHaveBeenLastCalledWith('failed'));
  invoke.mockResolvedValue({ ready: true });
  const retry = screen.getByRole('button', { name: 'Check again' });
  fireEvent.click(retry); fireEvent.click(retry);
  await screen.findByText(/Model connected:/);
  expect(invoke.mock.calls.filter(([command]) => command === 'setup_connect_model')).toHaveLength(2);
});
test('reading sign-in errors show the status command without copying diagnostics', async () => {
  invoke.mockRejectedValue({ kind: 'modelCaptureFailed', message: 'SECRET', diagnostic: 'SECRET' });
  render(<Model enabled />); fireEvent.click(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  expect((await screen.findByRole('alert')).textContent).toContain('gh auth status --hostname github.com');
  expect(screen.getByRole('button', { name: 'Copy details for help' })).toBeTruthy();
  expect(document.body.textContent).not.toContain('SECRET');
});

test('keyboard connection and retry keep the same focused control across failure and busy states', async () => {
  let reject!: (error: unknown) => void;
  invoke.mockImplementation(() => new Promise((_resolve, fail) => { reject = fail; }));
  const user = userEvent.setup(); render(<Model enabled />);
  const button = screen.getByRole('button', { name: 'Use GitHub sign-in' });
  button.focus(); await user.keyboard('{Enter}');
  await act(async () => reject({ kind: 'modelConnect' }));
  const retry = await screen.findByRole('button', { name: 'Check again' });
  expect(retry).toBe(button); expect(document.activeElement).toBe(button);
  await user.keyboard('{Enter}');
  expect(screen.getByRole('button', { name: 'Connecting model…' })).toBe(button);
  await act(async () => reject({ kind: 'modelConnect' }));
  expect(document.activeElement).toBe(button);
});

test('help buttons select native topics and copying cannot include arbitrary native data', async () => {
  invoke.mockRejectedValue({ kind: 'modelCaptureUnavailable', message: 'SECRET /private/path', diagnostic: 'code: 5' });
  const user = userEvent.setup(); const copy = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue();
  render(<Model enabled />); await user.click(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  invoke.mockResolvedValue(null);
  await user.click(await screen.findByRole('button', { name: 'Install GitHub CLI' }));
  expect(invoke).toHaveBeenLastCalledWith('setup_open_help', { topic: 'githubCliInstall' });
  await user.click(screen.getByRole('button', { name: 'Sign-in help' }));
  expect(invoke).toHaveBeenLastCalledWith('setup_open_help', { topic: 'githubSignIn' });
  await user.click(screen.getByRole('button', { name: 'Copy details for help' }));
  expect(copy.mock.calls[0][0]).toBe('Bridge AI connection\nOperation: Connecting AI\nCategory: modelCaptureUnavailable');
  invoke.mockRejectedValue('SECRET'); copy.mockRejectedValue('SECRET');
  await user.click(screen.getByRole('button', { name: 'Copy details for help' }));
  expect(screen.getByRole('status').textContent).toContain("Couldn't copy");
  await user.click(screen.getByRole('button', { name: 'Sign-in help' }));
  expect(screen.getByRole('status').textContent).toContain("Couldn't open help");
  expect(document.body.textContent).not.toContain('SECRET');
});

test('preview and malformed responses cannot claim ready', async () => {
  isTauri.mockReturnValue(false);
  expect((await connectModel()).ok).toBe(false); expect(invoke).not.toHaveBeenCalled();
  isTauri.mockReturnValue(true);
  for (const response of [{ ready: false }, { ready: true, token: 'synthetic-private-token' }, null]) {
    invoke.mockResolvedValue(response); expect((await connectModel()).ok).toBe(false);
  }
});
