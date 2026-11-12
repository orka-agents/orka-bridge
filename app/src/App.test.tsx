import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { act, cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from './App';

beforeEach(() => { localStorage.setItem('bridge.welcomeSeen.v1', '1'); });
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

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
  expect(await screen.findByRole('heading', { name: 'Where should your agents run?' })).toBeTruthy();
  for (const name of ['Designer', 'Clusters', 'Overview']) {
    await user.click(screen.getByRole('button', { name }));
    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
    expect(await screen.findByRole('main', { name: name === 'Clusters' ? 'Where should your agents run?' : name === 'Overview' ? 'Your agents' : name })).toBeTruthy();
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
  expect(await screen.findByRole('heading', { name: 'Where should your agents run?' })).toBeTruthy();
});

test('navigation starts at the page heading after scrolling another page', async () => {
  const user = userEvent.setup(); render(<App />);
  const main = screen.getByRole('main');
  main.scrollTop = 300;
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  expect(main.scrollTop).toBe(0);
});

test('Overview reads saved documents without opening, setup or VM commands; Open uses document identity', async () => {
  vi.stubGlobal('isTauri', true);
  const id = '12345678-1234-1234-1234-123456789abc'; const calls: string[] = [];
  mockIPC((command, args) => {
    calls.push(command);
    if (command === 'setup_status') return { selected: null, localVm: 'idle', model: 'locked' };
    if (command === 'overview_files') return { state: 'complete', runActive: false, files: [{ documentId: id, name: 'second', state: 'readable', source: null, summary: null, reason: 'Declarations unavailable' }] };
    if (command === 'overview_agents') return { state: 'unavailable', installation: null, agents: [], observation: null, observedAt: null, runActive: false, reason: 'stopped' };
    if (command === 'agent_open') { expect(args).toEqual({ documentId: id }); return { documentId: id, name: 'second', agent: '---\nname: second\n---\nRead.', tests: 'Checks' }; }
    if (command === 'agent_run_status') return { documentId: id, busy: null, state: 'idle', canRun: false, report: null, acknowledged: [], pending: null, error: null };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await screen.findByRole('heading', { name: 'second' });
  expect(calls).not.toContain('agent_open');
  expect(calls.some(c => /setup_select|agent_run$|send_start/.test(c))).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Open second' }));
  expect(await screen.findByRole('heading', { name: 'second', level: 1 })).toBeTruthy();
  expect(document.querySelector('.host-chip')?.textContent).toContain('This computer');
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(await screen.findByRole('button', { name: 'Open second' }));
  expect(calls.filter(c => c === 'agent_open')).toHaveLength(1);
});

test('saved sample Open reads the actual file while setup still keeps Designer read-only', async () => {
  vi.stubGlobal('isTauri', true); const calls: string[] = [];
  const id = '9b2b93c4-375c-4815-9717-39852a96e411';
  mockIPC(command => {
    calls.push(command);
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'starting', model: 'locked' };
    if (command === 'overview_files') return { state: 'complete', runActive: false, files: [{ documentId: id, name: 'web-reader', state: 'readable', source: null, summary: null, reason: null }] };
    if (command === 'setup_example') return { agent: '---\nname: bundled-example\n---\nBundled content', tests: 'Checks' };
    if (command === 'agent_open') return { documentId: id, name: 'web-reader', agent: '---\nname: saved-reader\n---\nMy saved changes', tests: 'Checks' };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(await screen.findByRole('button', { name: 'Open web-reader' }));
  expect(await screen.findByRole('heading', { name: 'saved-reader', level: 1 })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'File' }));
  expect((screen.getByRole('textbox', { name: 'Agent file' }) as HTMLTextAreaElement).readOnly).toBe(true);
  expect(calls.some(c => /setup_select|agent_run$|send_start/.test(c))).toBe(false);
});

test('switching a readonly preview to a registered document never opens the unrelated sample', async () => {
  vi.stubGlobal('isTauri', true); const opened: string[] = [];
  const id = '12345678-1234-1234-1234-123456789abc';
  mockIPC((command, args) => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'starting', model: 'locked' };
    if (command === 'overview_files') return { state: 'complete', runActive: false, files: [{ documentId: id, name: 'other', state: 'readable', source: null, summary: null, reason: null }] };
    if (command === 'setup_example') return { agent: '---\nname: bundled-example\n---\nBundled content', tests: 'Checks' };
    if (command === 'agent_open') { const doc = (args as { documentId: string }).documentId; opened.push(doc); return { documentId: doc, name: 'other', agent: '---\nname: other\n---\nSaved content', tests: 'Checks' }; }
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  await screen.findByRole('heading', { name: 'bundled-example', level: 1 });
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(await screen.findByRole('button', { name: 'Open other' }));
  await screen.findByRole('heading', { name: 'other', level: 1 });
  expect(opened).toEqual([id]);
});

test('Designer opens the owned agent without workspace setup and shares its saved state', async () => {
  vi.stubGlobal('isTauri', true);
  mockIPC(command => command === 'setup_status' ? { selected: null, localVm: 'idle', model: 'locked' }
    : command === 'agent_open' ? { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', name: 'web-reader', agent: '---\nname: web-reader\ndescription: Reads a page.\n---\nRead.', tests: 'Checks' }
    : command === 'agent_run_status' ? { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', busy: null, state: 'idle', canRun: false, report: null, acknowledged: [], pending: null, error: null }
    : { kind: 'version', version: 'v0.4.2', hasDiagnostics: false });
  const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  expect(await screen.findByRole('heading', { name: 'web-reader', level: 1 })).toBeTruthy();
  expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
  expect(screen.getByRole('main', { name: 'web-reader' })).toBeTruthy();
  expect(within(screen.getByRole('contentinfo')).getByText('Saved').getAttribute('aria-live')).toBe('polite');
});

test.each([
  ['idle', null, 'Choose where agents run', 'Setting up, step 1 of 4'],
  ['preparing', 'localVm', 'Getting workspace ready', 'Setting up, step 2 of 4'],
  ['installing', 'localVm', 'Getting workspace ready', 'Setting up, step 2 of 4'],
  ['starting', 'localVm', 'Starting workspace', 'Setting up, step 2 of 4'],
  ['ready', 'localVm', 'AI connection unavailable', 'AI unavailable'],
  ['failed', 'localVm', "Bridge couldn't establish why the workspace failed.", 'Setup failed'],
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
  expect(screen.queryByRole('button', { name: /Search/ })).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  for (const name of ['New agent']) {
    const button = screen.getByRole('button', { name });
    expect(button.hasAttribute('disabled')).toBe(true);
    expect(button.getAttribute('title')).toMatch(/coming/i);
  }
  expect(screen.getByRole('button', { name: 'Send to a cluster' }).title).toMatch(/agent folder|Reopen your agent/);
  expect(document.querySelector('.host-chip')?.textContent).toContain('This computer');
});

test('setup refreshes while another page is open', async () => {
  let localVm = 'ready';
  mockIPC(command => command === 'setup_status' ? { selected: 'localVm', localVm, model: localVm === 'ready' ? 'available' : 'locked' } : { kind: 'version', version: 'v0.4.2', hasDiagnostics: false });
  vi.stubGlobal('isTauri', true);
  const user = userEvent.setup(); render(<App />);
  await within(screen.getByRole('contentinfo')).findByText('Choose the AI');
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  localVm = 'failed';
  expect(await within(screen.getByRole('navigation')).findByText('Setup failed', {}, { timeout: 2000 })).toBeTruthy();
  expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
});

test('model acknowledgment and observed check counts survive navigation', async () => {
  const pair = { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', name: 'web-reader', agent: '---\nname: page-reader\n---\nRead a page.', tests: 'A check' };
  const snapshot = { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', busy: null, state: 'complete', canRun: true, report: { verdict: 'failed', cases: [
    { id: 'a', phase: 'Succeeded', answer: 'ok', completedTools: [], assertions: [{ id: 'a1', kind: 'contains', verdict: 'passed' }], verdict: 'passed' },
    { id: 'b', phase: 'Succeeded', answer: 'no', completedTools: [], assertions: [{ id: 'b1', kind: 'contains', verdict: 'failed' }], verdict: 'failed' },
  ] }, acknowledged: [], pending: null, error: null };
  let modelConnected = false;
  mockIPC(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'setup_connect_model') { modelConnected = true; return { ready: true }; }
    if (command === 'agent_open') return pair;
    if (command === 'agent_run_status') return { ...snapshot, modelConnected };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  vi.stubGlobal('isTauri', true);
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Use GitHub sign-in' }));
  expect(await within(screen.getByRole('contentinfo')).findByText('AI connected')).toBeTruthy();
  expect(document.querySelector('.host-chip')?.getAttribute('title')).toBe('AI connected');
  expect(document.querySelector('.host-chip')?.textContent).toBe('This computer: AI connected');
  await user.click(screen.getByRole('button', { name: 'Open example' }));
  expect(await within(screen.getByRole('contentinfo')).findByText('1 of 2 checks passing')).toBeTruthy();
  expect(screen.queryByText(/Connect the model in Clusters first\./)).toBeNull();
  expect(within(screen.getByRole('navigation')).getByText((_, node) => node?.tagName === 'SMALL' && node.textContent === 'page-reader, 1 check failing')).toBeTruthy();
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
    if (command === 'agent_open') return { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', name: 'web-reader', agent: '---\nname: reader\n---\nRead.', tests: 'Check' };
    if (command === 'agent_run_status') return { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', busy: null, state: complete ? 'complete' : 'running', canRun: complete, report: complete ? report : null, acknowledged: [], pending: null, error: null };
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
  await user.click(await screen.findByRole('button', { name: 'Continue' }));
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 1200)); });
  expect(screen.getByRole('button', { name: 'Continue' }).matches(':disabled')).toBe(true);
  await act(async () => { rejectSelection(new Error('selection write failed')); });
  expect(await screen.findByText('The setup choice could not be read or saved.')).toBeTruthy();
});

test('an unavailable example is not reported as still opening', async () => {
  mockIPC(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'locked' };
    if (command === 'agent_open') return Promise.reject(new Error('file unavailable'));
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
    if (command === 'agent_open') return { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', name: 'web-reader', agent: '---\nname: reader\n---\nRead.', tests: 'Check' };
    if (command === 'agent_run_status') return state === 'unavailable' ? Promise.reject(new Error('IPC failed'))
      : { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', busy: null, state: 'unknown', canRun: false, report: null, acknowledged: [], pending: { kind: 'Task', name: 'maybe-created' }, error: 'observe' };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  vi.stubGlobal('isTauri', true);
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Open example' }));
  const expected = state === 'unknown' ? 'Check result unknown' : 'Check status unavailable';
  await within(screen.getByRole('contentinfo')).findByText(expected);
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  expect(within(screen.getByRole('contentinfo')).getByText(expected)).toBeTruthy();
  expect(within(screen.getByRole('navigation')).getByText((_, node) => node?.tagName === 'SMALL' && node.textContent === `reader, ${expected.toLowerCase()}`)).toBeTruthy();
  expect(within(screen.getByRole('contentinfo')).getByRole('status').textContent).toContain(expected);
});

test('a retained native report for older source is stale in the shared chrome', async () => {
  vi.stubGlobal('isTauri', true);
  mockIPC(command => command === 'setup_status' ? { selected: null, localVm: 'idle', model: 'locked' }
    : command === 'agent_open' ? { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', name: 'web-reader', agent: '---\nname: reader\n---\nEdited file.', tests: 'Checks', source: { agent: 'b'.repeat(64), tests: 'c'.repeat(64) } }
    : { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', busy: null, state: 'complete', canRun: false, source: { agent: 'a'.repeat(64), tests: 'c'.repeat(64) }, report: { verdict: 'passed', cases: [{ id: 'check', phase: 'Succeeded', answer: 'Earlier answer', completedTools: [], assertions: [{ id: 'contains-1', kind: 'contains', verdict: 'passed' }], verdict: 'passed' }] }, acknowledged: [], pending: null, error: null });
  const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  expect(await within(screen.getByRole('contentinfo')).findByText('Checks not run since your change')).toBeTruthy();
  expect(within(screen.getByRole('contentinfo')).queryByText('1 of 1 checks passing')).toBeNull();
});
test('Open example retries a failed read without discarding successful runs', async () => {
  let fail = true;
  mockIPC(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'agent_open') return fail ? Promise.reject(new Error('read failed')) : { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', name: 'web-reader', agent: '---\nname: recovered-reader\n---\nRead.', tests: 'Check' };
    if (command === 'agent_run_status') return { documentId: '9b2b93c4-375c-4815-9717-39852a96e411', busy: null, state: 'idle', canRun: false, report: null, acknowledged: [], pending: null, error: null };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  vi.stubGlobal('isTauri', true);
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Open example' }));
  await within(screen.getByRole('navigation')).findByText('Example unavailable');
  await user.click(screen.getByRole('button', { name: 'Clusters' })); fail = false;
  await user.click(screen.getByRole('button', { name: 'Open example' }));
  expect(await within(screen.getByRole('navigation')).findByText((_, node) => node?.tagName === 'SMALL' && node.textContent === 'recovered-reader, ready to edit')).toBeTruthy();
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

test.each(['saved file', 'cluster Agent'])('%s Open selects This computer; remembered cluster needs a fresh successful Use before preflight', async origin => {
  vi.stubGlobal('isTauri', true);
  const id = '12345678-1234-1234-1234-123456789abc';
  const context = { id: '00000000-0000-4000-8000-000000000001', name: 'team-dev' };
  const target = { id: '00000000-0000-4000-8000-000000000002', context_id: context.id, context_name: context.name, service_name: 'orka', service_namespace: 'control', watched_namespace: 'agents', image: 'ghcr.io/orka:0.9.4' };
  const provider = { id: '00000000-0000-4000-8000-000000000003', name: 'team-model', ready: true };
  const connected = { status: 'connected', providers: [provider], failure: null };
  const source = { agent: 'a'.repeat(64), tests: 'b'.repeat(64) };
  const calls: string[] = []; let uses = 0; let finish!: (value: unknown) => void;
  mockIPC(command => {
    calls.push(command);
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'locked' };
    if (command === 'cluster_contexts') return [context];
    if (command === 'cluster_discover') return [target];
    if (command === 'cluster_use') return ++uses === 1 ? connected : new Promise(resolve => { finish = resolve; });
    if (command === 'overview_places') return { historyState: 'complete', unobserved: [], places: origin === 'saved file' ? [] : [{ key: 'team-place', context: 'team-dev', service: 'control/orka', namespace: 'agents', stale: false, observedAt: 123, agents: [{ name: 'bridge-reader', namespace: 'agents', uid: 'remote-uid', agentType: null, model: null, tools: null, record: { id, source, targetDisplay: 'team-dev · agents', targetUids: { serviceNamespace: 'control', serviceUid: 'svc', deploymentNamespace: 'control', deploymentUid: 'dep', watchedNamespace: 'agents' }, provider: 'team-model', plannedAgentName: 'bridge-reader', agentUid: 'remote-uid', verdict: 'passed', time: 123 } }] }] };
    if (command === 'overview_files') return { state: 'complete', runActive: false, files: [{ documentId: id, name: 'reader', source, state: 'readable', summary: null, reason: null }] };
    if (command === 'overview_agents') return { state: 'unavailable', installation: null, agents: [], observation: null, observedAt: null, runActive: false, reason: 'stopped' };
    if (command === 'agent_open') return { documentId: id, name: 'reader', agent: '---\nname: reader\nmodel: literal-model\n---\nRead.', tests: 'Checks', source };
    if (command === 'agent_run_status') return { documentId: id, busy: null, state: 'idle', canRun: false, report: null, acknowledged: [], pending: null, error: null };
    if (command === 'send_status') return { documentId: id, view: null, busy: null };
    if (command === 'send_history') return [];
    if (command === 'engine_call') return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
    return null;
  });
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Change where agents run' }));
  await user.click(screen.getByRole('button', { name: 'Use a cluster you already have instead' }));
  await user.selectOptions(await screen.findByRole('combobox', { name: 'kubectl context' }), context.id);
  await user.click(await screen.findByRole('button', { name: 'Use this cluster' }));
  await screen.findByRole('button', { name: 'Open your agent' });
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  const originRegion = await screen.findByRole('region', { name: origin === 'saved file' ? 'This computer' : 'team-dev · agents' });
  await user.click(await within(originRegion).findByRole('button', { name: 'Open reader' }));
  await screen.findByRole('heading', { name: 'reader', level: 1 });
  expect(document.querySelector('.host-chip')?.textContent).toContain('This computer');
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  await user.click(screen.getByRole('button', { name: 'Change where agents run' }));
  await user.click(screen.getByRole('button', { name: 'Use a cluster you already have instead' }));
  expect((screen.getByRole('combobox', { name: 'kubectl context' }) as HTMLSelectElement).value).toBe(context.id);
  expect(uses).toBe(1); expect(calls).not.toContain('send_preflight'); expect(calls).not.toContain('send_start');
  await user.click(screen.getByRole('button', { name: 'Use this cluster' }));
  expect(calls).not.toContain('send_preflight');
  await act(async () => { finish(connected); });
  await screen.findByRole('button', { name: 'Open your agent' });
  await act(async () => {});
  expect(calls.filter(c => c === 'send_preflight')).toHaveLength(1);
  expect(calls).not.toContain('send_start');
  await user.click(screen.getByRole('button', { name: 'Open your agent' }));
  expect(await screen.findByRole('heading', { name: 'reader', level: 1 })).toBeTruthy();
});

test('setup Open example restores the sample after Overview opened another document', async () => {
  vi.stubGlobal('isTauri', true);
  const sample = '9b2b93c4-375c-4815-9717-39852a96e411'; const other = '12345678-1234-1234-1234-123456789abc';
  mockIPC((command, args) => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'locked' };
    if (command === 'overview_files') return { state: 'complete', runActive: false, files: [{ documentId: other, name: 'other', state: 'readable', source: null, summary: null, reason: null }] };
    if (command === 'agent_open') { const id = (args as { documentId: string }).documentId; const name = id === sample ? 'sample' : 'other'; return { documentId: id, name, agent: `---\nname: ${name}\n---\nRead.`, tests: 'Checks' }; }
    if (command === 'agent_run_status') return { documentId: (args as { documentId: string }).documentId, busy: null, state: 'idle', canRun: false, report: null, acknowledged: [], pending: null, error: null };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(await screen.findByRole('button', { name: 'Open other' }));
  await screen.findByRole('heading', { name: 'other', level: 1 });
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  await user.click(screen.getByRole('button', { name: 'Open example' }));
  expect(await screen.findByRole('heading', { name: 'sample', level: 1 })).toBeTruthy();
});

test('Overview Open retries a transient failure for the same document', async () => {
  vi.stubGlobal('isTauri', true); let opens = 0;
  const id = '9b2b93c4-375c-4815-9717-39852a96e411';
  mockIPC(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'locked' };
    if (command === 'overview_files') return { state: 'complete', runActive: false, files: [{ documentId: id, name: 'reader', source: null, state: 'readable', summary: null, reason: null }] };
    if (command === 'agent_open') { if (++opens === 1) throw Error('Temporary read failure'); return { documentId: id, name: 'reader', agent: '---\nname: reader\n---\nRead.', tests: 'Checks' }; }
    if (command === 'agent_run_status') return { documentId: id, busy: null, state: 'idle', canRun: false, report: null, acknowledged: [], pending: null, error: null };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(await screen.findByRole('button', { name: 'Open reader' }));
  await within(screen.getByRole('navigation')).findByText('Example unavailable');
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(await screen.findByRole('button', { name: 'Open reader' }));
  expect(await screen.findByRole('heading', { name: 'reader', level: 1 })).toBeTruthy(); expect(opens).toBe(2);
});

test('opening the already-local file does not acquire another VM catalog session', async () => {
  vi.stubGlobal('isTauri', true); let catalogs = 0;
  const id = '9b2b93c4-375c-4815-9717-39852a96e411';
  mockIPC(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'setup_connect_model') return { ready: true };
    if (command === 'workspace_catalog') { catalogs++; return null; }
    if (command === 'overview_files') return { state: 'complete', runActive: false, files: [{ documentId: id, name: 'reader', source: null, state: 'readable', summary: null, reason: null }] };
    if (command === 'overview_agents') return { state: 'unavailable', installation: null, agents: [], observation: null, observedAt: null, runActive: false, reason: 'stopped' };
    if (command === 'agent_open') return { documentId: id, name: 'reader', agent: '---\nname: reader\n---\nRead.', tests: 'Checks' };
    if (command === 'agent_run_status') return { documentId: id, busy: null, state: 'idle', canRun: false, report: null, acknowledged: [], pending: null, error: null };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Use GitHub sign-in' }));
  await user.click(await screen.findByRole('button', { name: 'Open example' }));
  await screen.findByRole('button', { name: 'Refresh catalog' });
  const before = catalogs;
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(await screen.findByRole('button', { name: 'Open reader' }));
  await screen.findByRole('heading', { name: 'reader', level: 1 });
  expect(catalogs).toBe(before);
});

test('Overview waits for native busy release even when the report is already complete', async () => {
  vi.stubGlobal('isTauri', true);
  const id = '9b2b93c4-375c-4815-9717-39852a96e411';
  let busy = true; let lists = 0;
  mockIPC(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'locked' };
    if (command === 'agent_open') return { documentId: id, name: 'reader', agent: '---\nname: reader\n---\nRead.', tests: 'Checks' };
    if (command === 'agent_run_status') return { documentId: id, busy: busy ? { documentId: id, name: 'reader' } : null, state: 'complete', canRun: false, report: { verdict: 'passed', cases: [{ id: 'a', phase: 'Succeeded', answer: 'ok', completedTools: [], assertions: [{ id: 'a1', kind: 'contains', verdict: 'passed' }], verdict: 'passed' }] }, acknowledged: [], pending: null, error: null };
    if (command === 'overview_files') return { state: 'complete', files: [], runActive: busy };
    if (command === 'overview_agents') { lists++; return { state: 'unavailable', installation: null, agents: [], observation: null, observedAt: null, runActive: false, reason: 'stopped' }; }
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Open example' }));
  await within(screen.getByRole('contentinfo')).findByText('1 of 1 checks passing');
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await screen.findByText('Refresh pending until checks finish');
  await user.click(screen.getByRole('button', { name: 'Refresh' }));
  expect(lists).toBe(0);
  busy = false;
  await within(screen.getByRole('region', { name: 'This computer' })).findByText('Workspace stopped', {}, { timeout: 2000 });
  expect(lists).toBe(1);
});

test('Clusters reads setup and version without fetch or legacy IPC', async () => {
  const ipc = vi.fn(command => command === 'setup_status'
    ? { selected: null, localVm: 'notIntegrated', model: 'locked' }
    : { kind: 'version', version: 'v0.4.2', hasDiagnostics: false });
  mockIPC(ipc);
  vi.stubGlobal('isTauri', true);
  const fetch = vi.fn(); vi.stubGlobal('fetch', fetch);
  const user = userEvent.setup(); render(<App />);
  expect(await screen.findByRole('button', { name: 'Continue' })).toBeTruthy();
  await user.click(screen.getByText('Engine diagnostics'));
  expect(await screen.findByRole('heading', { name: 'Engine available' })).toBeTruthy();
  expect(screen.getByText('Version v0.4.2. Cluster readiness has not been checked.')).toBeTruthy();
  expect(ipc.mock.calls.every(([command]) => ['setup_requirements', 'setup_status', 'engine_call'].includes(command))).toBe(true);
  expect(ipc.mock.calls.filter(([command]) => command === 'setup_requirements')).toEqual([['setup_requirements', {}]]);
  expect(ipc).toHaveBeenCalledWith('engine_call', { request: { action: { type: 'version' } } });
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  expect(screen.queryByRole('heading', { name: 'Engine available' })).toBeNull();
  expect(ipc.mock.calls.filter(call => call[0] === 'engine_call')).toHaveLength(1);
  expect(fetch).not.toHaveBeenCalled();
});
