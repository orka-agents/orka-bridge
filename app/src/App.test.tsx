import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { act, cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import App from './App';

afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); });

test('navigation_order', () => {
  render(<App />);
  const nav = screen.getByRole('navigation', { name: 'Main' });
  expect(within(nav).getAllByRole('button').map(button => button.getAttribute('aria-label')))
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

test('navigation starts at the page heading after scrolling another page', async () => {
  const user = userEvent.setup(); render(<App />);
  const main = screen.getByRole('main');
  main.scrollTop = 300;
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  expect(main.scrollTop).toBe(0);
});

test('honest_empty_states', async () => {
  const user = userEvent.setup();
  render(<App />);
  for (const name of ['Overview', 'Designer']) {
    await user.click(screen.getByRole('button', { name }));
    const main = screen.getByRole('main', { name });
    expect(within(main).getByText('Nothing is connected yet.')).toBeTruthy();
    expect(within(main).getAllByRole('heading', { level: 2 })).toHaveLength(name === 'Clusters' ? 2 : 1);
    expect(within(main).queryByRole('button', { name: 'New agent' })?.hasAttribute('disabled')).toBe(name === 'Designer' ? true : undefined);
    expect(within(main).queryByRole('link')).toBeNull();
  }
});

test.each([
  ['idle', null, 'Choose where agents run', 'Setting up, step 1 of 4'],
  ['preparing', 'localVm', 'Getting workspace ready', 'Setting up, step 2 of 4'],
  ['installing', 'localVm', 'Getting workspace ready', 'Setting up, step 2 of 4'],
  ['starting', 'localVm', 'Starting workspace', 'Setting up, step 2 of 4'],
  ['ready', 'localVm', 'AI connection unavailable', 'AI unavailable'],
  ['failed', 'localVm', 'Workspace could not start', 'Setup paused'],
  ['closed', 'localVm', 'Workspace stopped', 'Workspace stopped'],
  ['notIntegrated', null, 'Workspace unavailable', 'Workspace unavailable'],
])('chrome reflects %s without invented readiness', async (localVm, selected, detail, tabLine) => {
  mockIPC(command => command === 'setup_status' ? { selected, localVm, model: 'locked' } : { kind: 'version', version: 'v0.4.2', hasDiagnostics: false });
  vi.stubGlobal('isTauri', true);
  render(<App />);
  const footer = screen.getByRole('contentinfo');
  expect(await within(footer).findByText(detail)).toBeTruthy();
  expect(within(screen.getByRole('navigation')).getByText(tabLine)).toBeTruthy();
  expect(within(footer).queryByText(/saved|GPT|checks passing/i)).toBeNull();
});

test('unbuilt controls are disabled with discoverable reasons', async () => {
  const user = userEvent.setup(); render(<App />);
  const search = screen.getByRole('button', { name: /Search/ });
  expect(search.hasAttribute('disabled')).toBe(true);
  expect(search.getAttribute('title')).toMatch(/Search.*coming/i);
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  for (const name of ['New agent', 'Send to a cluster']) {
    const button = screen.getByRole('button', { name });
    expect(button.hasAttribute('disabled')).toBe(true);
    expect(button.getAttribute('title')).toMatch(/coming/i);
  }
  expect(screen.getByText('This computer')).toBeTruthy();
});

test('setup refreshes while another page is open', async () => {
  let localVm = 'ready';
  mockIPC(command => command === 'setup_status' ? { selected: 'localVm', localVm, model: localVm === 'ready' ? 'available' : 'locked' } : { kind: 'version', version: 'v0.4.2', hasDiagnostics: false });
  vi.stubGlobal('isTauri', true);
  const user = userEvent.setup(); render(<App />);
  await within(screen.getByRole('contentinfo')).findByText('Choose the AI');
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  localVm = 'failed';
  expect(await within(screen.getByRole('navigation')).findByText('Setup paused', {}, { timeout: 2000 })).toBeTruthy();
  expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
});

test('model acknowledgment and observed check counts survive navigation', async () => {
  const pair = { agent: '---\nname: page-reader\n---\nRead a page.', tests: 'A check' };
  const snapshot = { state: 'complete', canRun: true, report: { verdict: 'failed', cases: [
    { id: 'a', phase: 'Succeeded', answer: 'ok', completedTools: [], assertions: [{ id: 'a1', kind: 'contains', verdict: 'passed' }], verdict: 'passed' },
    { id: 'b', phase: 'Succeeded', answer: 'no', completedTools: [], assertions: [{ id: 'b1', kind: 'contains', verdict: 'failed' }], verdict: 'failed' },
  ] }, acknowledged: [], pending: null, error: null };
  mockIPC(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'setup_connect_model') return { ready: true };
    if (command === 'setup_example') return pair;
    if (command === 'setup_run_status') return snapshot;
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  vi.stubGlobal('isTauri', true);
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Use GitHub sign-in' }));
  expect(await within(screen.getByRole('contentinfo')).findByText('AI connected')).toBeTruthy();
  expect(screen.getByText('This computer').getAttribute('title')).toBe('AI connected');
  expect(screen.getByText('This computer').textContent).toBe('This computer: AI connected');
  await user.click(screen.getByRole('button', { name: 'Open example' }));
  expect(await within(screen.getByRole('contentinfo')).findByText('1 of 2 checks passing')).toBeTruthy();
  expect(within(screen.getByRole('navigation')).getByText('page-reader, 1 check failing')).toBeTruthy();
  expect(within(screen.getByRole('navigation')).getByText('This computer ready')).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  expect(within(screen.getByRole('contentinfo')).getByText('AI connected')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Model connected' }).hasAttribute('disabled')).toBe(true);
});

test('a running check finishes in the footer after leaving Designer', async () => {
  let complete = false;
  const report = { verdict: 'passed', cases: [{ id: 'a', phase: 'Succeeded', answer: 'ok', completedTools: [], assertions: [{ id: 'a1', kind: 'contains', verdict: 'passed' }], verdict: 'passed' }] };
  mockIPC(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'locked' };
    if (command === 'setup_example') return { agent: '---\nname: reader\n---\nRead.', tests: 'Check' };
    if (command === 'setup_run_status') return { state: complete ? 'complete' : 'running', canRun: complete, report: complete ? report : null, acknowledged: [], pending: null, error: null };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  vi.stubGlobal('isTauri', true);
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Open example' }));
  await within(screen.getByRole('contentinfo')).findByText('Running checks…');
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  complete = true;
  expect(await within(screen.getByRole('contentinfo')).findByText('1 of 1 checks passing')).toBeTruthy();
  expect(document.querySelectorAll('#page-title')).toHaveLength(1);
});

test('the idle refresh does not erase a delayed selection failure', async () => {
  let rejectSelection!: (error: unknown) => void;
  mockIPC(command => {
    if (command === 'setup_status') return { selected: null, localVm: 'idle', model: 'locked' };
    if (command === 'setup_select_local') return new Promise((_, reject) => { rejectSelection = reject; });
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  vi.stubGlobal('isTauri', true);
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('radio', { name: /Local VM/ }));
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 1200)); });
  expect(screen.getByRole('radio', { name: /Local VM/ }).matches(':disabled')).toBe(true);
  await act(async () => { rejectSelection(new Error('selection write failed')); });
  expect(await screen.findByText('The setup choice could not be read or saved.')).toBeTruthy();
});

test('an unavailable example is not reported as still opening', async () => {
  mockIPC(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'locked' };
    if (command === 'setup_example') return Promise.reject(new Error('file unavailable'));
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  vi.stubGlobal('isTauri', true);
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Open example' }));
  expect(await within(screen.getByRole('navigation')).findByText('Example unavailable')).toBeTruthy();
  expect(within(screen.getByRole('navigation')).queryByText('Opening example…')).toBeNull();
});

test.each(['unknown', 'unavailable'])('background check status preserves %s outcomes', async state => {
  mockIPC(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'setup_example') return { agent: '---\nname: reader\n---\nRead.', tests: 'Check' };
    if (command === 'setup_run_status') return state === 'unavailable' ? Promise.reject(new Error('IPC failed'))
      : { state: 'unknown', canRun: false, report: null, acknowledged: [], pending: { kind: 'Task', name: 'maybe-created' }, error: 'observe' };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  vi.stubGlobal('isTauri', true);
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Open example' }));
  const expected = state === 'unknown' ? 'Check result unknown' : 'Check status unavailable';
  await within(screen.getByRole('contentinfo')).findByText(expected);
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  expect(within(screen.getByRole('contentinfo')).getByText(expected)).toBeTruthy();
  expect(within(screen.getByRole('navigation')).getByText(`reader, ${expected.toLowerCase()}`)).toBeTruthy();
  expect(within(screen.getByRole('contentinfo')).getByRole('status').textContent).toContain(expected);
});

test('Open example retries a failed read without discarding successful runs', async () => {
  let fail = true;
  mockIPC(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'setup_example') return fail ? Promise.reject(new Error('read failed')) : { agent: '---\nname: recovered-reader\n---\nRead.', tests: 'Check' };
    if (command === 'setup_run_status') return { state: 'idle', canRun: false, report: null, acknowledged: [], pending: null, error: null };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  vi.stubGlobal('isTauri', true);
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Open example' }));
  await within(screen.getByRole('navigation')).findByText('Example unavailable');
  await user.click(screen.getByRole('button', { name: 'Clusters' })); fail = false;
  await user.click(screen.getByRole('button', { name: 'Open example' }));
  expect(await within(screen.getByRole('navigation')).findByText('recovered-reader, read-only')).toBeTruthy();
});

test('a transient setup read failure recovers while off Clusters', async () => {
  let fail = true;
  mockIPC(command => command === 'setup_status' ? fail ? Promise.reject(new Error('read failed'))
    : { selected: 'localVm', localVm: 'starting', model: 'locked' }
    : { kind: 'version', version: 'v0.4.2', hasDiagnostics: false });
  vi.stubGlobal('isTauri', true);
  const user = userEvent.setup(); render(<App />);
  await within(screen.getByRole('navigation')).findByText('Workspace unavailable');
  await user.click(screen.getByRole('button', { name: 'Overview' })); fail = false;
  expect(await within(screen.getByRole('navigation')).findByText('Setting up, step 2 of 4', {}, { timeout: 2000 })).toBeTruthy();
});

test('a transient setup read failure does not invalidate an admitted model connection', async () => {
  let fail = false;
  let finishConnect!: (value: unknown) => void;
  mockIPC(command => {
    if (command === 'setup_status') return fail ? Promise.reject(new Error('read failed')) : { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'setup_connect_model') return new Promise(resolve => { finishConnect = resolve; });
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  vi.stubGlobal('isTauri', true);
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Use GitHub sign-in' }));
  fail = true;
  await within(screen.getByRole('navigation')).findByText('Workspace unavailable', {}, { timeout: 2000 });
  await act(async () => { finishConnect({ ready: true }); });
  expect(screen.getByRole('button', { name: 'Model connected' }).matches(':disabled')).toBe(true);
  fail = false;
  expect(await within(screen.getByRole('contentinfo')).findByText('AI connected', {}, { timeout: 2000 })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Model connected' }).matches(':disabled')).toBe(true);
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
  expect(ipc.mock.calls.filter(call => call[0] === 'engine_call')).toHaveLength(1);
  expect(fetch).not.toHaveBeenCalled();
});
