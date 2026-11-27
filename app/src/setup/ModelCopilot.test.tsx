import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import Model from './Model';

const { invoke, isTauri } = vi.hoisted(() => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri }));
beforeEach(() => { invoke.mockReset(); isTauri.mockReturnValue(true); });
afterEach(() => { cleanup(); });

const models = [{ id: 'gpt-4.1' }, { id: 'gpt-6.1-terra', name: 'GPT-6.1 Terra' }, { id: 'claude-sonnet-5.5' }];
const choose = { kind: 'modelCopilotChooseModel', account: 'octocat', models, preselected: 'gpt-6.1-terra' };
const card = () => document.querySelector<HTMLElement>('.setup-model')!;
const commands = () => invoke.mock.calls.map(([command, args]) => args ? [command, args] : [command]);
const connected = (model: string) => ({ ready: true, connection: { kind: 'copilot', model, account: 'octocat' } });

test('Connect lists the account models with one preselected, and the person confirms the choice', async () => {
  invoke.mockImplementation((command: string, args?: { model?: string }) => {
    if (command !== 'setup_connect_model') throw new Error(command);
    if (!args?.model) return Promise.reject(choose);
    return Promise.resolve(connected(args.model));
  });
  render(<Model enabled />);
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  const picker = await screen.findByRole('combobox', { name: 'Model' }) as HTMLSelectElement;
  expect(picker.value).toBe('gpt-6.1-terra');
  expect([...picker.options].map(option => option.textContent)).toEqual(['gpt-4.1', 'gpt-6.1-terra · GPT-6.1 Terra', 'claude-sonnet-5.5']);
  expect(screen.queryByRole('alert')).toBeNull();
  // Nothing connected yet: the person still confirms.
  expect(commands()).toEqual([['setup_connect_model']]);
  fireEvent.change(picker, { target: { value: 'claude-sonnet-5.5' } });
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  await screen.findByText(/^Connected$/);
  expect(commands()).toEqual([['setup_connect_model'], ['setup_connect_model', { model: 'claude-sonnet-5.5' }]]);
  expect(screen.getByText('claude-sonnet-5.5', { selector: 'code' })).toBeTruthy();
});

test('the guided first run connects with the preselected model it names on the card', async () => {
  invoke.mockImplementation((_: string, args?: { model?: string }) => args?.model ? Promise.resolve(connected(args.model)) : Promise.reject(choose));
  render(<Model enabled setupCard />);
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  await screen.findByText(/^Connected$/);
  expect(commands()).toEqual([['setup_connect_model'], ['setup_connect_model', { model: 'gpt-6.1-terra' }]]);
  expect(card().textContent).toContain('gpt-6.1-terra');
});

test('a saved choice no longer offered asks for another pick without touching the workspace', async () => {
  invoke.mockImplementation((_: string, args?: { model?: string }) => args?.model === 'gpt-4.1' ? Promise.resolve(connected('gpt-4.1'))
    : Promise.reject({ kind: 'modelCopilotModelNotOffered', account: 'octocat', wanted: 'gpt-5.6-terra', models, preselected: 'gpt-6.1-terra' }));
  render(<Model enabled setupCard />);
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toContain("gpt-5.6-terra isn't offered to octocat any more.");
  expect(alert.textContent).toContain('Choose another model, then press Connect the AI.');
  expect(alert.textContent).toContain('Nothing was sent to your workspace.');
  const picker = within(alert).getByRole('combobox', { name: 'Model' }) as HTMLSelectElement;
  expect(picker.value).toBe('gpt-6.1-terra');
  fireEvent.change(picker, { target: { value: 'gpt-4.1' } });
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  await screen.findByText(/^Connected$/);
  expect(commands().at(-1)).toEqual(['setup_connect_model', { model: 'gpt-4.1' }]);
});

test.each([
  ['modelCopilotAccessDenied', "GitHub Copilot isn't available to octocat.", 'Check the account has Copilot, or that your organisation allows Copilot in the command line.', undefined],
  ['modelCopilotSignInRejected', "GitHub didn't accept your GitHub CLI sign-in.", 'Sign in again with GitHub CLI in your terminal.', 'gh auth login --hostname github.com --web'],
  ['modelCopilotTokenType', "This GitHub CLI sign-in uses a classic token, which GitHub Copilot doesn't accept here.", 'Sign in to GitHub CLI again with your browser', 'gh auth login --hostname github.com --web'],
  ['modelCopilotUnreachable', "This computer couldn't reach GitHub Copilot.", "Check this computer's internet connection, then press Check again.", undefined],
  ['modelCopilotMalformed', "GitHub Copilot sent a reply Bridge doesn't understand.", 'Press Check again.', undefined],
])('%s names its next step and that nothing reached the workspace', async (kind, summary, step, command) => {
  invoke.mockRejectedValue({ kind, account: 'octocat' });
  render(<Model enabled setupCard />);
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  const alert = await screen.findByRole('alert');
  expect(alert.querySelector('.failure-summary')?.textContent).toBe(summary);
  expect(alert.textContent).toContain(step);
  expect(alert.textContent).toContain('Nothing was sent to your workspace.');
  expect(alert.textContent).not.toContain('could not confirm');
  expect(alert.querySelector('pre')?.textContent).toBe(command);
  expect(screen.getByRole('button', { name: 'Check again' })).toBeTruthy();
  expect(!!screen.queryByRole('button', { name: 'Use your own API key' })).toBe(kind === 'modelCopilotAccessDenied');
});

test('access denied offers your own API key as the other route', async () => {
  invoke.mockRejectedValue({ kind: 'modelCopilotAccessDenied', account: 'octocat' });
  render(<Model enabled setupCard />);
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  fireEvent.click(await screen.findByRole('button', { name: 'Use your own API key' }));
  expect(screen.getByLabelText('API key')).toBeTruthy();
});

test('while the workspace starts, a checked choice waits with its account and model, then connects with it', async () => {
  invoke.mockImplementation((command: string, args?: { model?: string }) => command === 'setup_check_model'
    ? Promise.resolve({ account: 'octocat', models, preselected: 'gpt-6.1-terra' })
    : Promise.resolve(connected(args?.model ?? 'missing')));
  const onState = vi.fn();
  const view = render(<Model enabled={false} queueable setupCard onState={onState} />);
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  await screen.findByText('GitHub Copilot is ready: signed in as octocat, model gpt-6.1-terra. Connects when the workspace is ready.');
  expect(onState).toHaveBeenLastCalledWith('checked');
  expect(commands()).toEqual([['setup_check_model']]);
  view.rerender(<Model enabled queueable={false} setupCard onState={onState} />);
  await screen.findByText(/^Connected$/);
  expect(commands()).toEqual([['setup_check_model'], ['setup_connect_model', { model: 'gpt-6.1-terra' }]]);
});

test('a refused check while the workspace starts shows at once, is not queued, and Check again asks again', async () => {
  invoke.mockRejectedValue({ kind: 'modelCopilotAccessDenied', account: 'octocat' });
  const onState = vi.fn();
  const view = render(<Model enabled={false} queueable setupCard onState={onState} />);
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  expect((await screen.findByRole('alert')).textContent).toContain("GitHub Copilot isn't available to octocat.");
  expect(screen.queryByText('Queued')).toBeNull();
  expect(onState).toHaveBeenLastCalledWith('failed');
  fireEvent.click(screen.getByRole('button', { name: 'Check again' }));
  await waitFor(() => expect(commands()).toEqual([['setup_check_model'], ['setup_check_model']]));
  // Readiness never connects a refused choice by itself.
  view.rerender(<Model enabled queueable={false} setupCard onState={onState} />);
  await act(async () => {});
  expect(commands().filter(([command]) => command === 'setup_connect_model')).toEqual([]);
});

test('an account with no usable models says so and offers nothing to connect', async () => {
  invoke.mockResolvedValue({ account: 'octocat', models: [] });
  render(<Model enabled={false} queueable />);
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  await screen.findByText(/doesn't offer octocat a model Bridge can use for agents/);
  expect(screen.queryByText('Queued')).toBeNull();
  expect(screen.getByRole('button', { name: 'Connect the AI' }).hasAttribute('disabled')).toBe(true);
});

test('copied details name the preflight outcome and the workspace detail, never the account', async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  invoke.mockRejectedValue({ kind: 'modelCopilotAccessDenied', account: 'octocat' });
  render(<Model enabled setupCard diagnostic={'Guest Connect: {"stage":"model","code":"execution_unknown"}'} />);
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  fireEvent.click(await screen.findByRole('button', { name: 'Copy details for help' }));
  await waitFor(() => expect(writeText).toHaveBeenCalled());
  const text = writeText.mock.calls[0][0] as string;
  expect(text).toContain('Category: modelCopilotAccessDenied');
  expect(text).toContain('Copilot preflight: AccessDenied');
  expect(text).toContain('Workspace detail: Guest Connect: {"stage":"model","code":"execution_unknown"}');
  expect(text).not.toContain('octocat');
});

test('cleanup refused again explains that only a fresh workspace recovers; an older image keeps its own message', async () => {
  invoke.mockRejectedValue({ kind: 'modelReconnect', cleanup: ['disconnectRetainedModel', 'refused'] });
  render(<Model enabled />);
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  expect((await screen.findByRole('alert')).textContent).toContain("Bridge couldn't remove the previous AI connection from the workspace.");
  expect(screen.getByRole('alert').textContent).not.toContain('newer image');
  fireEvent.click(screen.getByRole('button', { name: 'Check again' }));
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('A fresh workspace is the sure way back'));
  cleanup();
  invoke.mockRejectedValue({ kind: 'modelReconnectImage', cleanup: ['modelStatusPreconditions', 'unsupported'] });
  render(<Model enabled />);
  fireEvent.click(screen.getByRole('button', { name: 'Connect the AI' }));
  expect((await screen.findByRole('alert')).textContent).toContain('The workspace may need a newer image.');
});
