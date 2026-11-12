import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, within, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from '../App';
import { SetupSteps } from './Steps';

const pair = { agent: '---\nname: web-reader\ndescription: Answers from the web page named in the request\nkmx:\n  tools:\n    - kind: builtin\n      name: web_fetch\n---\nFetch before answering.', tests: 'The checksum check.' };
const progress = { phase: 'materializing', bytes: 1200000000, totalBytes: 3000000000, elapsedMs: 20000, remainingMs: 30000 };
function native(initial = 'installing', numbers: { phase: string; bytes: number | null; totalBytes: number | null; elapsedMs: number; remainingMs: number | null } | null = progress) {
  vi.stubGlobal('isTauri', true);
  let localVm = initial; let selected: 'localVm' | null = 'localVm'; let unavailable = false;
  const status = () => ({ selected, localVm, model: localVm === 'ready' ? 'available' : 'locked', workspaceProgress: ['installing', 'starting'].includes(localVm) ? numbers : null, localVmError: localVm === 'failed' ? 'PRIVATE_NATIVE_DETAIL' : null });
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_status') { if (unavailable) throw Error('unreadable'); return status(); }
    if (command === 'setup_cancel_local') { localVm = 'cancelled'; return status(); }
    if (command === 'setup_select_local') { selected = 'localVm'; localVm = 'installing'; return status(); }
    if (command === 'setup_change_local') { localVm = 'idle'; selected = null; return status(); }
    if (command === 'setup_example') return pair;
    if (command === 'agent_open') return { ...pair, documentId: '9b2b93c4-375c-4815-9717-39852a96e411', name: 'web-reader' };
    if (command === 'setup_connect_model') return { ready: true };
    if (command === 'agent_run_status') return { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', busy: null, state: 'idle', canRun: true, report: null, acknowledged: [], pending: null, error: null };
    throw new Error('Unexpected command: ' + command);
  }); mockIPC(ipc);
  return { ipc, ready: () => { localVm = 'ready'; }, fail: () => { localVm = 'failed'; }, unreadable: () => { unavailable = true; }, readable: () => { unavailable = false; } };
}
beforeEach(() => localStorage.setItem('bridge.welcomeSeen.v1', '1'));
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

test('five steps show native transfer bytes and ETA in one bar, with expandable details', async () => {
  native(); const user = userEvent.setup(); render(<App />);
  await screen.findByRole('heading', { name: 'Getting your workspace ready' });
  const steps = screen.getByRole('list', { name: 'Setup steps' });
  expect(within(steps).getAllByRole('listitem')).toHaveLength(5);
  for (const name of ['Install your workspace files', 'Start your workspace', 'Orka ready', 'Choose the AI your agent uses', 'Run your first test']) expect(within(steps).getByText(name)).toBeTruthy();
  expect(screen.getByText('Where agents run').nextElementSibling?.textContent).toBe('On this computer');
  expect(screen.getByText('Installing, 1.1 GiB of 2.8 GiB')).toBeTruthy();
  expect(screen.getByText('30 seconds left')).toBeTruthy();
  const bar = screen.getByRole('progressbar');
  expect(screen.getAllByRole('progressbar')).toHaveLength(1);
  expect(Number(bar.getAttribute('aria-valuenow'))).toBeGreaterThan(0);
  expect(Number(bar.getAttribute('aria-valuenow'))).toBeLessThan(70);
  expect(screen.queryByText(/Downloading/)).toBeNull();
  expect(screen.queryByRole('button', { name: 'Connect model' })).toBeNull();
  expect(screen.queryByText('Checking workspace files')).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Show details' }));
  expect(screen.getByText('Checking workspace files')).toBeTruthy();
  expect(screen.getAllByText('Starting workspace').length).toBeGreaterThan(0);
});

test('tab, footer and card agree on one step index and the model action is available during transfer', async () => {
  native(); render(<App />);
  expect(await within(screen.getByRole('navigation')).findByText('Setting up, step 1 of 5')).toBeTruthy();
  expect(within(screen.getByRole('contentinfo')).getByText('Setting up, step 1 of 5')).toBeTruthy();
  expect(screen.getByText('Step 1 of 5')).toBeTruthy();
  expect(document.body.textContent).not.toMatch(/of 4/);
  expect(screen.getByRole('button', { name: 'Use GitHub sign-in' }).hasAttribute('disabled')).toBe(false);
  expect(screen.getByText('Pick any time')).toBeTruthy();
});

test('compact order puts the model action before the long step history, with completed rows as single lines', async () => {
  native(); render(<App />);
  const model = (await screen.findByRole('heading', { name: 'Connect a model' })).closest('section')!;
  const history = screen.getByRole('list', { name: 'Setup steps' });
  const progress = screen.getByRole('region', { name: 'Workspace setup' });
  expect(progress.compareDocumentPosition(model) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(model.compareDocumentPosition(history) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(history.querySelectorAll('button, a')).toHaveLength(0);
});

test('startup uses the same bar, labels its time as a rough estimate and shows native elapsed time', async () => {
  native('starting', { phase: 'starting', bytes: null, totalBytes: null, elapsedMs: 12000, remainingMs: null }); render(<App />);
  const bar = await screen.findByRole('progressbar', { name: 'Workspace setup progress' });
  expect(Number(bar.getAttribute('aria-valuenow'))).toBeGreaterThanOrEqual(70);
  expect(Number(bar.getAttribute('aria-valuenow'))).toBeLessThan(100);
  expect(screen.getByText('12 seconds elapsed')).toBeTruthy();
  expect(screen.getByText(/Rough estimate, not a measurement: about 18 seconds left/)).toBeTruthy();
  expect(screen.getByText('Start your workspace').closest('li')?.getAttribute('aria-current')).toBe('step');
  expect(screen.getByText('Install your workspace files').closest('li')?.className).toBe('step-done');
  expect(bar.closest('main')).toBeTruthy();
});

test('startup past its estimate stays incomplete and says it is taking longer', async () => {
  native('starting', { phase: 'starting', bytes: null, totalBytes: null, elapsedMs: 95000, remainingMs: null }); render(<App />);
  const bar = await screen.findByRole('progressbar', { name: 'Workspace setup progress' });
  expect(Number(bar.getAttribute('aria-valuenow'))).toBeLessThan(100);
  expect(screen.getByText(/Taking longer than the rough 30-second estimate/)).toBeTruthy();
  expect(screen.queryByText(/seconds left/)).toBeNull();
  expect(screen.getByText('95 seconds elapsed')).toBeTruthy();
});

test('unavailable progress never fabricates bytes or time left, and the bar stays indeterminate', async () => {
  native('installing', null); render(<App />);
  await screen.findByRole('list', { name: 'Setup steps' });
  const bar = screen.getByRole('progressbar');
  expect(bar.hasAttribute('aria-valuenow')).toBe(false);
  expect(screen.queryByText(/seconds left|minute left|of .*GiB/)).toBeNull();
});

test('the one bar never completes before native ready and is gone once ready', async () => {
  const { ready } = native('starting', { phase: 'starting', bytes: null, totalBytes: null, elapsedMs: 31000, remainingMs: null }); render(<App />);
  await screen.findByRole('progressbar');
  ready();
  await waitFor(() => expect(screen.queryByRole('progressbar')).toBeNull(), { timeout: 2000 });
  const rows = within(screen.getByRole('list', { name: 'Setup steps' })).getAllByRole('listitem');
  expect(rows.slice(0, 3).map(row => row.className)).toEqual(['step-done', 'step-done', 'step-done']);
  expect(within(screen.getByRole('navigation')).getByText('Setting up, step 4 of 5')).toBeTruthy();
});

test('keyboard Cancel then start again calls native retry and keeps the Meet panel', async () => {
  const { ipc } = native(); const user = userEvent.setup(); render(<App />);
  const cancel = await screen.findByRole('button', { name: 'Cancel' }); cancel.focus();
  await user.keyboard('{Enter}');
  const retry = await screen.findByRole('button', { name: 'Start again' });
  expect(screen.getByText('What has arrived is kept.')).toBeTruthy();
  retry.focus(); await user.keyboard(' ');
  await screen.findByRole('button', { name: 'Cancel' });
  expect(ipc.mock.calls.filter(([c]) => c === 'setup_cancel_local')).toHaveLength(1);
  expect(ipc.mock.calls.filter(([c]) => c === 'setup_select_local')).toHaveLength(1);
});

test('Meet opens the actual example read-only without owned-file or Run commands', async () => {
  const { ipc } = native(); const user = userEvent.setup(); render(<App />);
  expect(await screen.findByRole('heading', { name: 'web-reader' })).toBeTruthy();
  expect(screen.getByText('Answers from the web page named in the request')).toBeTruthy();
  expect(await within(screen.getByRole('navigation')).findByText((_, node) => node?.tagName === 'SMALL' && node.textContent === 'web-reader, read-only during setup')).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Read its instructions' }));
  const main = screen.getByRole('main', { name: 'web-reader' });
  expect(within(main).getByText('Fetch before answering.')).toBeTruthy();
  const run = within(main).getByRole('button', { name: 'Run test' });
  expect(run.matches(':disabled')).toBe(true);
  expect(run.title).toBe('Wait for your workspace and AI to be ready');
  await user.click(within(main).getByRole('button', { name: 'File' }));
  const editor = screen.getByRole('textbox', { name: 'Agent file' }) as HTMLTextAreaElement;
  expect(editor.readOnly).toBe(true); editor.focus(); await user.keyboard('{Control>}s{/Control}');
  expect(ipc.mock.calls.some(([c]) => ['agent_open', 'agent_save', 'agent_run', 'agent_run_status'].includes(c))).toBe(false);
  expect(within(screen.getByRole('navigation')).getByText((_, node) => node?.tagName === 'SMALL' && node.textContent === 'web-reader, read-only during setup')).toBeTruthy();
});

test('unreadable status never upgrades the setup preview into an editable file', async () => {
  const { ipc, unreadable } = native(); const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Read its instructions' }));
  unreadable();
  await waitFor(() => expect(within(screen.getByRole('contentinfo')).getAllByText('Workspace unavailable').length).toBeGreaterThan(0));
  await user.click(screen.getByRole('button', { name: 'File' }));
  expect((screen.getByRole('textbox', { name: 'Agent file' }) as HTMLTextAreaElement).readOnly).toBe(true);
  expect(ipc.mock.calls.some(([c]) => ['agent_open', 'agent_save', 'agent_run_status'].includes(c))).toBe(false);
});

test('read-only view is freshly opened as editable only once setup is ready', async () => {
  const { ipc, ready } = native(); const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Read its instructions' }));
  expect(ipc.mock.calls.some(([c]) => c === 'agent_open')).toBe(false);
  ready();
  await waitFor(() => expect(ipc.mock.calls.filter(([c]) => c === 'agent_open')).toHaveLength(1));
  await user.click(screen.getByRole('button', { name: 'File' }));
  expect((screen.getByRole('textbox', { name: 'Agent file' }) as HTMLTextAreaElement).readOnly).toBe(false);
});

test('an established Designer stays editable when a running workspace later fails', async () => {
  const { ipc, fail } = native('ready'); const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Open example' }));
  await screen.findByRole('heading', { name: 'web-reader', level: 1 });
  await user.click(screen.getByRole('button', { name: 'File' }));
  const editor = screen.getByRole('textbox', { name: 'Agent file' }); fail();
  await within(screen.getByRole('contentinfo')).findByText('Setup failed', {}, { timeout: 2000 });
  expect(screen.getByRole('textbox', { name: 'Agent file' })).toBe(editor);
  expect((editor as HTMLTextAreaElement).readOnly).toBe(false);
  expect(ipc.mock.calls.filter(([c]) => c === 'agent_open')).toHaveLength(1);
});

test('failure is one plain red step, native details stay behind disclosure', async () => {
  native('failed'); const user = userEvent.setup(); render(<App />);
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toContain("Bridge couldn't establish why the workspace failed.");
  expect(alert.textContent).toContain('Your workspace and agent files are kept.');
  expect(alert.closest('li')?.classList.contains('step-failed')).toBe(true);
  expect(screen.queryByText('PRIVATE_NATIVE_DETAIL')).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Show details' }));
  expect(screen.getByText('PRIVATE_NATIVE_DETAIL')).toBeTruthy();
});

test('Designer stays read-only while the first native setup observation is pending', async () => {
  const { ipc } = native(); let observe!: (value: unknown) => void;
  mockIPC(command => command === 'setup_status' ? new Promise(resolve => { observe = resolve; }) : ipc(command));
  const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  await screen.findByRole('heading', { name: 'web-reader', level: 1 });
  expect(ipc.mock.calls.some(([command]) => command === 'agent_open')).toBe(false);
  observe({ selected: 'localVm', localVm: 'installing', model: 'locked' });
  await within(screen.getByRole('contentinfo')).findByText('Setting up, step 1 of 5');
  expect(screen.getByRole('button', { name: 'Run test' }).matches(':disabled')).toBe(true);
});

test('opening Designer while Continue awaits native admission cannot open an owned file', async () => {
  const { ipc } = native(); let admit!: (value: unknown) => void; let started = false;
  mockIPC(command => command === 'setup_status' ? { selected: started ? 'localVm' : null, localVm: started ? 'installing' : 'idle', model: 'locked' }
    : command === 'setup_select_local' ? new Promise(resolve => { admit = resolve; }) : ipc(command));
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Continue' }));
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  await screen.findByRole('heading', { name: 'web-reader', level: 1 });
  expect(ipc.mock.calls.some(([command]) => command === 'agent_open')).toBe(false);
  expect(screen.getByRole('button', { name: 'Run test' }).matches(':disabled')).toBe(true);
  started = true; admit({ selected: 'localVm', localVm: 'installing', model: 'locked' });
  await within(screen.getByRole('contentinfo')).findByText('Setting up, step 1 of 5');
});

test('a failed example read is not presented as an endless load', async () => {
  const { ipc } = native(); mockIPC(command => command === 'setup_example' ? Promise.reject('unavailable') : ipc(command)); render(<App />);
  expect(await screen.findByText('The example could not be opened. Try opening Designer.')).toBeTruthy();
  expect(screen.queryByText('Opening the example\'s introduction…')).toBeNull();
});

test.each(['running', 'passed', 'failed', 'unknown'] as const)('the last setup step reflects actual %s run evidence', async result => {
  const { ipc } = native('ready');
  const snapshot = { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', busy: null, state: result === 'running' ? 'running' : 'complete', canRun: result !== 'running', report: result === 'running' ? null : { verdict: result, cases: [{ id: 'checksum', phase: 'Succeeded', answer: 'checksum', completedTools: ['web_fetch'], verdict: result, assertions: [{ id: 'value', kind: 'contains', verdict: result }] }] }, acknowledged: [], pending: null, error: null };
  mockIPC(command => command === 'agent_run_status' ? snapshot : ipc(command));
  const user = userEvent.setup(); render(<App />);
  // A test result is shown only for a ready workspace whose AI is connected.
  await user.click(await screen.findByRole('button', { name: 'Use GitHub sign-in' }));
  await screen.findByText('Model connected: copilot / gpt-5.6-terra.');
  await user.click(await screen.findByRole('button', { name: 'Open example' }));
  await screen.findByRole('heading', { name: 'web-reader', level: 1 });
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  await waitFor(() => expect(screen.getByRole('list', { name: 'Setup steps' }).children[4].textContent).toContain({ running: 'Running', passed: 'Passed', failed: 'Failed', unknown: 'Result unknown' }[result]));
});

test('changing workspace and starting again preserves an owned draft after a failed save', async () => {
  const { ipc } = native('ready'); const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Open example' }));
  await screen.findByRole('heading', { name: 'web-reader', level: 1 });
  await user.click(screen.getByRole('button', { name: 'File' }));
  const editor = screen.getByRole('textbox', { name: 'Agent file' }) as HTMLTextAreaElement;
  await user.type(editor, 'Retained draft');
  await screen.findByText('Your changes could not be saved. Your draft is still here.');
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  await user.click(screen.getByRole('button', { name: 'Change where agents run' }));
  await user.click(screen.getByRole('button', { name: 'Stop workspace and change' }));
  await user.click(await screen.findByRole('button', { name: 'Continue' }));
  await screen.findByRole('button', { name: 'Cancel' });
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  expect(screen.getByRole('textbox', { name: 'Agent file' })).toBe(editor);
  expect(editor.value).toContain('Retained draft');
  expect(ipc.mock.calls.filter(([c]) => c === 'agent_open')).toHaveLength(1);
});

test('the AI step resets after a connected workspace leaves ready', async () => {
  const { fail } = native('ready'); const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Use GitHub sign-in' }));
  await screen.findByText('Model connected: copilot / gpt-5.6-terra.'); fail();
  await within(screen.getByRole('contentinfo')).findByText('Setup failed', {}, { timeout: 2000 });
  await waitFor(() => expect(screen.getByRole('list', { name: 'Setup steps' }).children[3].textContent).not.toContain('Connected'));
});

test('a delayed Cancel shows pending stop near the top instead of old byte progress', async () => {
  const { ipc } = native(); let done!: (value: unknown) => void;
  mockIPC(command => command === 'setup_cancel_local' ? new Promise(resolve => { done = resolve; }) : ipc(command));
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Cancel' }));
  expect(screen.getByText('Stopping your workspace…')).toBeTruthy();
  expect(screen.queryByRole('progressbar')).toBeNull();
  done({ selected: 'localVm', localVm: 'cancelled', model: 'locked' });
  await screen.findByRole('button', { name: 'Start again' });
});

test('model settling during Change cannot strand subsequent Continue actions', async () => {
  native('ready'); let connect!: (value: unknown) => void; let change!: (value: unknown) => void; let selected: 'localVm' | null = 'localVm';
  mockIPC(command => command === 'setup_status' ? { selected, localVm: selected ? 'ready' : 'cancelled', model: selected ? 'available' : 'locked' }
    : command === 'setup_connect_model' ? new Promise(resolve => { connect = resolve; })
    : command === 'setup_change_local' ? new Promise(resolve => { change = resolve; })
    : command === 'setup_select_local' ? { selected: 'localVm', localVm: 'installing', model: 'locked' }
    : Promise.reject('unavailable'));
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Use GitHub sign-in' }));
  await user.click(screen.getByRole('button', { name: 'Change where agents run' }));
  await user.click(screen.getByRole('button', { name: 'Stop workspace and change' }));
  connect({ ready: true }); await screen.findByText('Model connected: copilot / gpt-5.6-terra.');
  selected = null; change({ selected: null, localVm: 'cancelled', model: 'locked' });
  const next = await screen.findByRole('button', { name: 'Continue' });
  expect(next.matches(':disabled')).toBe(false);
});

test('Cancel and Start again restore keyboard focus after replacing the action', async () => {
  native(); const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Cancel' }));
  await screen.findByRole('button', { name: 'Start again' });
  expect(document.activeElement).toBe(screen.getByRole('heading', { name: 'Getting your workspace ready', level: 1 }));
  await user.click(screen.getByRole('button', { name: 'Start again' }));
  await screen.findByRole('button', { name: 'Cancel' });
  expect(document.activeElement).toBe(screen.getByRole('heading', { name: 'Getting your workspace ready', level: 1 }));
});

test('keeping the workspace running returns focus to the confirmation invoker', async () => {
  native('ready'); const user = userEvent.setup(); render(<App />);
  const invoker = await screen.findByRole('button', { name: 'Change where agents run' });
  await user.click(invoker); await user.click(screen.getByRole('button', { name: 'Keep workspace running' }));
  expect(document.activeElement).toBe(invoker);
});

test('Change where agents run does not stop until confirmed, and returns to chooser', async () => {
  const { ipc } = native('ready'); const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Change where agents run' }));
  expect(ipc.mock.calls.some(([c]) => c === 'setup_change_local')).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Keep workspace running' }));
  expect(ipc.mock.calls.some(([c]) => c === 'setup_change_local')).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Change where agents run' }));
  await user.click(screen.getByRole('button', { name: 'Stop workspace and change' }));
  expect(await screen.findByRole('heading', { name: 'Where should your agents run?' })).toBeTruthy();
  expect(ipc.mock.calls.filter(([c]) => c === 'setup_change_local')).toHaveLength(1);
});

test('an unreadable status makes the card unavailable instead of keeping the last running snapshot, and recovery keeps the queued choice', async () => {
  const { ipc, unreadable, readable } = native(); const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Use GitHub sign-in' }));
  expect(screen.getByRole('progressbar')).toBeTruthy();
  const card = () => within(screen.getByRole('region', { name: 'Workspace setup' }));
  expect(card().getByText('Step 1 of 5')).toBeTruthy();
  unreadable();
  await waitFor(() => expect(within(screen.getByRole('contentinfo')).getAllByText('Workspace unavailable').length).toBeGreaterThan(0), { timeout: 3000 });
  expect(card().getByText('Unavailable', { selector: '.pill' })).toBeTruthy();
  expect(card().queryByText('Step 1 of 5')).toBeNull();
  expect(screen.queryByRole('progressbar')).toBeNull();
  expect(screen.queryByText(/Installing, 1.1 GiB/)).toBeNull();
  expect(card().getByText('Workspace status is unavailable.')).toBeTruthy();
  const steps = screen.getByRole('list', { name: 'Setup steps' });
  // The list is a last-known snapshot, not a live observation: nothing runs and no row is current.
  expect(steps.querySelector('.step-current, [aria-current]')).toBeNull();
  expect(within(steps).getAllByText(/^Last known: /)).toHaveLength(5);
  expect(screen.getByText('Queued', { selector: '.pill' })).toBeTruthy();
  expect(ipc.mock.calls.filter(([command]) => command === 'setup_connect_model')).toHaveLength(0);
  readable();
  await waitFor(() => expect(screen.getByRole('progressbar')).toBeTruthy(), { timeout: 3000 });
  expect(card().getByText('Step 1 of 5')).toBeTruthy();
  expect(within(steps).queryByText(/^Last known: /)).toBeNull();
  expect(screen.getByText('Queued', { selector: '.pill' })).toBeTruthy();
  expect(ipc.mock.calls.filter(([command]) => command === 'setup_connect_model')).toHaveLength(0);
});

test.each([
  ['a workspace that is installing again', { selected: 'localVm', localVm: 'installing', model: 'locked' }, 'idle'],
  ['a disconnected workspace', { selected: 'localVm', localVm: 'ready', model: 'available' }, 'idle'],
  ['a failed AI connection', { selected: 'localVm', localVm: 'ready', model: 'available' }, 'failed'],
] as const)('a kept passed result is not shown for %s', (_name, status, modelState) => {
  render(<SetupSteps status={status} busy={false} stopping={false} modelState={modelState} firstCheck="passed" onRestart={() => {}} />);
  expect(screen.getByRole('list', { name: 'Setup steps' }).children[4].textContent).toBe('Run your first testWaiting');
});

test('a passed result is shown only once the workspace is ready and the AI is connected', () => {
  render(<SetupSteps status={{ selected: 'localVm', localVm: 'ready', model: 'available' }} busy={false} stopping={false} modelState="connected" firstCheck="passed" onRestart={() => {}} />);
  expect(screen.getByRole('list', { name: 'Setup steps' }).children[4].textContent).toBe('Run your first testPassed');
});
