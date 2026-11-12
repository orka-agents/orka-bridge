import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import Model from './Model';
import userEvent from '@testing-library/user-event';
import { connectModel } from './modelState';

const { invoke, isTauri } = vi.hoisted(() => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri }));
beforeEach(() => { invoke.mockReset(); isTauri.mockReturnValue(true); });
afterEach(() => { cleanup(); });
test('remove is explicit, excludes duplicates and permits connecting again', async () => {
  const onState = vi.fn();
  invoke.mockResolvedValue({ ready: true });
  render(<Model enabled onState={onState} />);
  expect(invoke).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  await screen.findByText(/Model connected:/);
  let complete!: (value: unknown) => void;
  invoke.mockImplementation(() => new Promise(resolve => { complete = resolve; }));
  const remove = screen.getByRole('button', { name: 'Remove sign-in' });
  fireEvent.click(remove); fireEvent.click(remove);
  expect(invoke.mock.calls.filter(([cmd]) => cmd === 'setup_remove_model')).toEqual([['setup_remove_model']]);
  expect(screen.queryByText(/Model connected:/)).toBeNull();
  await act(async () => { complete({ ready: false }); });
  expect(screen.getByText(/GitHub CLI stays signed in/)).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Use GitHub sign-in' }).hasAttribute('disabled')).toBe(false);
  expect(onState).toHaveBeenLastCalledWith('idle');
});
test('remove is available before connecting and after connection failure, and never echoes raw failures', async () => {
  invoke.mockRejectedValue({ kind: 'modelConnect', message: 'synthetic-private-token' });
  const view = render(<Model enabled />);
  expect(screen.getByRole('button', { name: 'Remove sign-in' }).hasAttribute('disabled')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  await screen.findByRole('alert');
  invoke.mockRejectedValue({ kind: 'invented', message: 'synthetic-private-token' });
  fireEvent.click(screen.getByRole('button', { name: 'Remove sign-in' }));
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('could not confirm sign-in removal'));
  expect(document.body.textContent).not.toContain('synthetic-private-token');
  expect(screen.queryByText(/no longer has a saved sign-in/)).toBeNull();
  view.rerender(<Model enabled paused />);
  expect(screen.getByRole('button', { name: 'Remove sign-in' }).hasAttribute('disabled')).toBe(true);
});
test('failed removal retries cleanup rather than Connect and copies its own operation', async () => {
  const user = userEvent.setup(); const copy = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue();
  invoke.mockRejectedValue({ kind: 'modelStore', native: { call: 'CredDeleteW', code: 5 }, message: 'SECRET' });
  render(<Model enabled />);
  await user.click(screen.getByRole('button', { name: 'Remove sign-in' }));
  expect((await screen.findByRole('alert')).textContent).toContain('Remove sign-in again');
  await user.click(screen.getByRole('button', { name: 'Copy details for help' }));
  expect(copy.mock.calls[0][0]).toBe('Bridge AI connection\nOperation: Removing sign-in\nCategory: modelStore\nOS code: 5');
  invoke.mockResolvedValue({ ready: false });
  await user.click(screen.getByRole('button', { name: 'Remove sign-in' }));
  await screen.findByText(/no longer has a saved sign-in/);
  expect(invoke.mock.calls.filter(([cmd]) => cmd === 'setup_remove_model')).toHaveLength(2);
  expect(invoke.mock.calls.some(([cmd]) => cmd === 'setup_connect_model')).toBe(false);
  expect(document.body.textContent).not.toContain('SECRET');
});
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
  // Busy is status text, not a disabled action button.
  expect(screen.queryByRole('button', { name: /Connecting model/ })).toBeNull();
  expect(screen.getByText('Connecting model…').closest('button')).toBeNull();
  await act(async () => { complete({ ready: true }); });
  expect(screen.getByText('Model connected: copilot / gpt-5.6-terra.')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Model connected' })).toBeNull();
  expect(screen.getByText('Connected').closest('button')).toBeNull();
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

test('keyboard connection moves focus to its busy status and back to the retry action', async () => {
  let reject!: (error: unknown) => void;
  invoke.mockImplementation(() => new Promise((_resolve, fail) => { reject = fail; }));
  const user = userEvent.setup(); render(<Model enabled />);
  const button = screen.getByRole('button', { name: 'Use GitHub sign-in' });
  button.focus(); await user.keyboard('{Enter}');
  expect(screen.queryByRole('button', { name: /Connecting/ })).toBeNull();
  expect(document.activeElement).toBe(screen.getByText('Connecting model…').closest('[tabindex="-1"]'));
  await act(async () => reject({ kind: 'modelConnect' }));
  const retry = await screen.findByRole('button', { name: 'Check again' });
  expect(document.activeElement).toBe(retry);
  await user.keyboard('{Enter}');
  expect(screen.getByText('Connecting model…')).toBeTruthy();
  await act(async () => reject({ kind: 'modelConnect' }));
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Check again' }));
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
  expect(screen.getByRole('status').textContent).toContain("Couldn't copy. Share the visible error message for help.");
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

// Queued intent: in-memory consent to connect once Orka is ready. Credentials are untouched before then.
test('a choice while the workspace is starting is queued without any credential command', () => {
  const onState = vi.fn();
  render(<Model enabled={false} queueable onState={onState} />);
  expect(screen.getByRole('heading', { name: 'Connect a model' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Remove sign-in' }).hasAttribute('disabled')).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  expect(invoke).not.toHaveBeenCalled();
  expect(screen.getByText('Queued').closest('button')).toBeNull();
  expect(screen.getByText(/connects automatically when Orka is ready/i)).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Use GitHub sign-in' })).toBeNull();
  expect(onState).toHaveBeenLastCalledWith('queued');
});

test('queued intent connects exactly once when the model becomes available, without another click', async () => {
  invoke.mockResolvedValue({ ready: true });
  const view = render(<Model enabled={false} queueable />);
  fireEvent.click(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  view.rerender(<Model enabled paused={false} />);
  await screen.findByText(/Model connected:/);
  expect(invoke.mock.calls.filter(([cmd]) => cmd === 'setup_connect_model')).toHaveLength(1);
  view.rerender(<Model enabled={false} />); view.rerender(<Model enabled />);
  expect(invoke.mock.calls.filter(([cmd]) => cmd === 'setup_connect_model')).toHaveLength(1);
});

test('queued intent survives the readiness transition and fires only once unpaused and enabled', async () => {
  invoke.mockResolvedValue({ ready: true });
  const view = render(<Model enabled={false} queueable />);
  fireEvent.click(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  // Ready but locked (not yet available): hold, neither fire nor clear.
  view.rerender(<Model enabled={false} queueable={false} />);
  expect(invoke).not.toHaveBeenCalled();
  expect(screen.getByText('Queued')).toBeTruthy();
  // Available but an unreadable observation or mutation pauses: hold.
  view.rerender(<Model enabled paused />);
  expect(invoke).not.toHaveBeenCalled();
  view.rerender(<Model enabled paused={false} />);
  await screen.findByText(/Model connected:/);
  expect(invoke).toHaveBeenCalledExactlyOnceWith('setup_connect_model');
});

test('a locked model that is not queueable offers no queue', () => {
  render(<Model enabled={false} />);
  fireEvent.click(screen.getByRole('button', { name: 'Connect model' }));
  expect(screen.queryByText('Queued')).toBeNull();
  expect(invoke).not.toHaveBeenCalled();
});

test('cluster selection and cancelling clear intent so readiness needs fresh consent', () => {
  const view = render(<Model enabled={false} queueable />);
  fireEvent.click(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  view.rerender(<Model enabled={false} queueable away />);
  view.rerender(<Model enabled queueable={false} away={false} />);
  expect(invoke).not.toHaveBeenCalled();
  expect(screen.getByRole('button', { name: 'Use GitHub sign-in' })).toBeTruthy();
  view.rerender(<Model enabled={false} queueable />);
  fireEvent.click(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  fireEvent.click(screen.getByRole('button', { name: 'Cancel queued connection' }));
  expect(screen.queryByText('Queued')).toBeNull();
  view.rerender(<Model enabled />);
  expect(invoke).not.toHaveBeenCalled();
});

test('a failed queued connection is not retried automatically', async () => {
  invoke.mockRejectedValue({ kind: 'modelConnect' });
  const view = render(<Model enabled={false} queueable />);
  fireEvent.click(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  view.rerender(<Model enabled />);
  await screen.findByRole('alert');
  view.rerender(<Model enabled paused />); view.rerender(<Model enabled />);
  await act(async () => { await Promise.resolve(); });
  expect(invoke.mock.calls.filter(([cmd]) => cmd === 'setup_connect_model')).toHaveLength(1);
  expect(screen.getByRole('button', { name: 'Check again' })).toBeTruthy();
});

test('keyboard removal keeps focus with the operation and returns it to Remove sign-in on failure and success', async () => {
  let settle!: (value: { ok: boolean }) => void;
  invoke.mockImplementation(() => new Promise((resolve, reject) => { settle = ({ ok }) => ok ? resolve({ ready: false }) : reject({ kind: 'modelStore' }); }));
  const user = userEvent.setup(); render(<Model enabled />);
  const remove = screen.getByRole('button', { name: 'Remove sign-in' });
  remove.focus(); await user.keyboard('{Enter}');
  expect(screen.queryByRole('button', { name: 'Remove sign-in' })).toBeNull();
  expect(document.activeElement).not.toBe(document.body);
  expect(document.activeElement).toBe(screen.getByText('Removing sign-in…').closest('[tabindex="-1"]'));
  await act(async () => settle({ ok: false }));
  await screen.findByRole('alert');
  // Failed removal retries cleanup, so focus must land on Remove sign-in, never on Connect's retry.
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Remove sign-in' }));
  expect(document.activeElement).not.toBe(screen.getByRole('button', { name: 'Use GitHub sign-in' }));
  await user.keyboard('{Enter}');
  expect(invoke.mock.calls.filter(([cmd]) => cmd === 'setup_remove_model')).toHaveLength(2);
  expect(document.activeElement).not.toBe(document.body);
  await act(async () => settle({ ok: true }));
  await screen.findByText(/no longer has a saved sign-in/);
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Remove sign-in' }));
});

test('a mouse-started removal does not steal keyboard focus', async () => {
  invoke.mockResolvedValue({ ready: false });
  render(<Model enabled />);
  fireEvent.click(screen.getByRole('button', { name: 'Remove sign-in' }));
  await screen.findByText(/no longer has a saved sign-in/);
  expect(document.activeElement).toBe(document.body);
});

// Focus follows an operation only while the creator has not moved it. These tests put another control in the page.
function withNeighbour(props: { enabled: boolean; queueable?: boolean }, hidden = false) {
  return <><div hidden={hidden}><Model {...props} /></div><button type="button">Elsewhere</button></>;
}
const neighbour = () => screen.getByRole('button', { name: 'Elsewhere' });

test('a queued keyboard choice does not take focus back when Orka becomes ready after the creator moved on', async () => {
  invoke.mockImplementation(() => new Promise(() => {}));
  const user = userEvent.setup(); const view = render(withNeighbour({ enabled: false, queueable: true }));
  screen.getByRole('button', { name: 'Use GitHub sign-in' }).focus(); await user.keyboard('{Enter}');
  expect(document.activeElement).toBe(screen.getByText('Queued').closest('[tabindex="-1"]'));
  await user.tab(); await user.tab();
  neighbour().focus();
  view.rerender(withNeighbour({ enabled: true, queueable: true }));
  await screen.findByText('Connecting model…');
  expect(document.activeElement).toBe(neighbour());
});

test('a settled connection does not pull focus from another control, on success or failure', async () => {
  let settle!: (ok: boolean) => void;
  invoke.mockImplementation(() => new Promise((resolve, reject) => { settle = ok => ok ? resolve({ ready: true }) : reject({ kind: 'modelConnect' }); }));
  const user = userEvent.setup(); render(withNeighbour({ enabled: true }));
  screen.getByRole('button', { name: 'Use GitHub sign-in' }).focus(); await user.keyboard('{Enter}');
  neighbour().focus();
  await act(async () => settle(false));
  await screen.findByRole('alert');
  expect(document.activeElement).toBe(neighbour());
  // The failed attempt is over; a later keyboard retry owns focus again.
  const retry = screen.getByRole('button', { name: 'Check again' });
  retry.focus(); await user.keyboard('{Enter}');
  neighbour().focus();
  await act(async () => settle(true));
  await screen.findByText(/Model connected:/);
  expect(document.activeElement).toBe(neighbour());
});

test('a pending removal does not pull focus from another control once it settles', async () => {
  let settle!: (ok: boolean) => void;
  invoke.mockImplementation(() => new Promise((resolve, reject) => { settle = ok => ok ? resolve({ ready: false }) : reject({ kind: 'modelStore' }); }));
  const user = userEvent.setup(); render(withNeighbour({ enabled: true }));
  screen.getByRole('button', { name: 'Remove sign-in' }).focus(); await user.keyboard('{Enter}');
  neighbour().focus();
  await act(async () => settle(false));
  await screen.findByRole('alert');
  expect(document.activeElement).toBe(neighbour());
  screen.getByRole('button', { name: 'Remove sign-in' }).focus(); await user.keyboard('{Enter}');
  neighbour().focus();
  await act(async () => settle(true));
  await screen.findByText(/no longer has a saved sign-in/);
  expect(document.activeElement).toBe(neighbour());
});

test('a hidden Model never takes focus when its queued connection starts', async () => {
  invoke.mockImplementation(() => new Promise(() => {}));
  const user = userEvent.setup(); const view = render(withNeighbour({ enabled: false, queueable: true }));
  screen.getByRole('button', { name: 'Use GitHub sign-in' }).focus(); await user.keyboard('{Enter}');
  const status = screen.getByText('Queued').closest<HTMLElement>('[tabindex="-1"]')!;
  const focus = vi.spyOn(status, 'focus');
  // jsdom leaves focus on an element that a browser would drop when its page is hidden, so the Model must check visibility itself.
  view.rerender(withNeighbour({ enabled: false, queueable: true }, true));
  view.rerender(withNeighbour({ enabled: true, queueable: true }, true));
  await screen.findByText('Connecting model…');
  expect(focus).not.toHaveBeenCalled();
});
