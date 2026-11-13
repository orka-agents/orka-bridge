import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from '../App';

const sample = '9b2b93c4-375c-4815-9717-39852a96e411';
const pair = { agent: '---\nname: web-reader\n---\nRead a page.', tests: 'A checksum test.', source: { agent: 'a'.repeat(64), tests: 'b'.repeat(64) } };
const other = '12345678-1234-1234-1234-123456789abc';
const idle = { documentId: sample, busy: null, state: 'idle', canRun: true, modelConnected: true, report: null, acknowledged: [], pending: null, error: null };
function native(holdSave = false, lostReplies = false) {
  vi.stubGlobal('isTauri', true);
  let selected = false; let running = false; let finished = false;
  let unreadable = false; let verdict = 'passed'; let saved = pair;
  const status = () => ({ selected: selected ? 'localVm' : null, localVm: selected ? 'ready' : 'idle', model: selected ? 'available' : 'locked' });
  const ipc = vi.fn((command: string, payload?: unknown) => {
    const args = payload as Record<string, unknown> | undefined;
    if (command === 'setup_status') return status();
    if (command === 'setup_requirements') return null;
    if (command === 'setup_select_local') { selected = true; if (lostReplies) throw Error('Lost selection reply'); return status(); }
    if (command === 'setup_change_local') { selected = false; return status(); }
    if (command === 'setup_example') return { agent: pair.agent, tests: pair.tests };
    if (command === 'setup_connect_model') return { ready: true };
    if (command === 'agent_open') return args?.documentId === other ? { ...pair, documentId: other, name: 'other', agent: '---\nname: other\n---\nOther instructions.' } : { ...saved, documentId: sample, name: 'web-reader' };
    if (command === 'agent_save') { if (holdSave) return new Promise(() => {}); saved = { ...pair, agent: String(args?.content), source: { ...pair.source, agent: 'c'.repeat(64) } }; return { ...saved, documentId: sample, name: 'web-reader' }; }
    if (command === 'agent_run') { running = true; finished = false; if (lostReplies) throw Error('Lost run reply'); return { ...idle, state: 'running', canRun: false, busy: { documentId: sample, name: 'web-reader' } }; }
    if (command === 'agent_run_status') {
      if (args?.documentId === other) return { ...idle, documentId: other };
      if (unreadable) throw Error('Lost sample status');
      return finished ? { ...idle, source: pair.source, state: 'complete', report: { verdict, cases: [{ id: 'checksum', question: 'What checksum is on the page?', phase: 'Succeeded', answer: 'Correct checksum', completedTools: ['web_fetch'], assertions: [{ id: 'checksum', kind: 'contains', expected: 'Correct checksum', verdict }], verdict }] } } : running ? { ...idle, state: 'running', canRun: false, busy: { documentId: sample, name: 'web-reader' } } : idle;
    }
    if (command === 'send_history' || command === 'cluster_contexts') return [];
    if (command === 'overview_files') return { state: 'complete', files: [{ documentId: other, name: 'other', state: 'readable', source: null, summary: null, reason: null }], runActive: running && !finished };
    if (command === 'overview_agents') return { state: 'unavailable', installation: null, agents: [], observation: null, observedAt: null, runActive: false, reason: 'stopped' };
    throw Error('Unexpected command: ' + command);
  });
  mockIPC(ipc); return { ipc, finish: () => { finished = true; }, loseWorkspace: () => { selected = false; }, loseRunStatus: () => { unreadable = true; }, failNext: () => { verdict = 'failed'; } };
}
beforeEach(() => localStorage.setItem('bridge.welcomeSeen.v1', '1'));
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

async function completeSetup() {
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Continue' }));
  await user.click(await screen.findByRole('button', { name: 'Connect the AI' }, { timeout: 2000 }));
  return user;
}

test('first setup shows a running test, opens Designer on its result and never auto-runs on another visit', async () => {
  const { ipc, finish } = native(); const user = await completeSetup();
  await waitFor(() => expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toHaveLength(1));
  expect(screen.getByRole('button', { name: 'Clusters' }).getAttribute('aria-current')).toBe('page');
  expect(screen.getByRole('list', { name: 'Setup steps' }).children[4].textContent).toBe('Run your first testRunning');
  finish();
  await waitFor(() => expect(screen.getByRole('button', { name: 'Designer' }).getAttribute('aria-current')).toBe('page'));
  expect(await screen.findByText('1 of 1 passed')).toBeTruthy();
  await user.click(screen.getByText('checksum', { selector: 'code' }));
  expect(await screen.findByText('Correct checksum', { selector: 'p' })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  await user.click(screen.getByRole('button', { name: 'Open your test result' }));
  expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toEqual([['agent_run', { documentId: sample, bundledExample: true }]]);
});

test.each(['passed', 'failed', 'unreadable', 'edited'] as const)('Setup retains the latest sample evidence (%s), not the automatic result or another document', async state => {
  const { finish, loseRunStatus, failNext } = native(); const user = await completeSetup();
  await waitFor(() => expect(screen.getByRole('list', { name: 'Setup steps' }).children[4].textContent).toBe('Run your first testRunning')); finish();
  await screen.findByText('1 of 1 passed');
  if (state === 'failed') {
    failNext(); await user.click(screen.getByRole('button', { name: 'Run tests' })); finish();
    await screen.findByText('0 of 1 passed');
  } else if (state === 'unreadable') {
    loseRunStatus(); await screen.findByText('Run information is not supported by this build.');
  } else if (state === 'edited') {
    await user.click(screen.getByRole('button', { name: 'File' }));
    await user.type(screen.getByRole('textbox', { name: 'Agent file' }), ' Edited.');
    await screen.findByText('Saved', { selector: 'span' });
  }
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  const expected = { passed: 'Passed', failed: 'Failed', unreadable: 'Result unknown', edited: 'Needs running' }[state];
  expect(screen.getByRole('list', { name: 'Setup steps' }).children[4].textContent).toBe(`Run your first test${expected}`);
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(await screen.findByRole('button', { name: 'Open other' }));
  await screen.findByRole('heading', { name: 'other', level: 1 });
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  expect(screen.getByRole('list', { name: 'Setup steps' }).children[4].textContent).toBe(`Run your first test${expected}`);
});

test('a manual sample test stays observed when Designer switches to another agent before completion', async () => {
  const { finish, failNext } = native(); const user = await completeSetup();
  await waitFor(() => expect(screen.getByRole('list', { name: 'Setup steps' }).children[4].textContent).toBe('Run your first testRunning')); finish();
  await screen.findByText('1 of 1 passed');
  failNext(); await user.click(screen.getByRole('button', { name: 'Run tests' }));
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(await screen.findByRole('button', { name: 'Open other' }));
  await screen.findByRole('heading', { name: 'other', level: 1 });
  finish();
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  await waitFor(() => expect(screen.getByRole('list', { name: 'Setup steps' }).children[4].textContent).toBe('Run your first testFailed'));
});

test('a newer automatic test replaces retained sample evidence after Designer switches documents', async () => {
  const { ipc, finish } = native(); const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Continue' }));
  await user.click(await screen.findByRole('button', { name: 'Read its instructions' }));
  await screen.findByRole('button', { name: 'Run tests' });
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(await screen.findByRole('button', { name: 'Open other' }));
  await screen.findByRole('heading', { name: 'other', level: 1 });
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  await user.click(await screen.findByRole('button', { name: 'Connect the AI' }));
  await waitFor(() => expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toHaveLength(1));
  expect(screen.getByRole('list', { name: 'Setup steps' }).children[4].textContent).toBe('Run your first testRunning');
  await user.click(screen.getByRole('button', { name: 'Overview' })); finish();
  await screen.findByText('Workspace stopped', {}, { timeout: 2000 });
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  expect(screen.getByRole('list', { name: 'Setup steps' }).children[4].textContent).toBe('Run your first testPassed');
});

test('a requested document switch does not admit the pending test while the bound sample has unsaved edits', async () => {
  const { ipc } = native(true); const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Continue' }));
  await user.click(await screen.findByRole('button', { name: 'Read its instructions' }));
  await user.click(screen.getByRole('button', { name: 'File' }));
  await user.type(await screen.findByRole('textbox', { name: 'Agent file' }), ' Unsaved.');
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await user.click(await screen.findByRole('button', { name: 'Open other' }));
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  await user.click(await screen.findByRole('button', { name: 'Connect the AI' }));
  await screen.findByText('Connected with GitHub Copilot.');
  await new Promise(resolve => setTimeout(resolve, 550));
  expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toHaveLength(0);
  expect(localStorage.getItem('bridge.firstTest.v1')).toBe('pending');
});

test.each([false, true])('Overview refreshes after the automatic test releases native admission without mounting Designer (lost replies: %s)', async lostReplies => {
  const { ipc, finish } = native(false, lostReplies); const user = await completeSetup();
  await waitFor(() => expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toHaveLength(1));
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  await screen.findByText('Refresh pending until checks finish');
  finish();
  expect(await screen.findByText('Workspace stopped', {}, { timeout: 2000 })).toBeTruthy();
  expect(screen.queryByText('Refresh pending until checks finish')).toBeNull();
  expect(screen.getByRole('button', { name: 'Overview' }).getAttribute('aria-current')).toBe('page');
  expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toHaveLength(1);
});

test.each(['Overview', 'Designer', 'cluster', 'Stop', 'workspace lost'])('first-test completion does not interrupt %s', async destination => {
  const { ipc, finish, loseWorkspace } = native(); const user = await completeSetup();
  await waitFor(() => expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toHaveLength(1));
  if (destination === 'Overview' || destination === 'Designer') await user.click(screen.getByRole('button', { name: destination }));
  else if (destination === 'workspace lost') { loseWorkspace(); window.dispatchEvent(new Event('focus')); await screen.findByRole('button', { name: 'Continue' }); }
  else {
    await user.click(screen.getByRole('button', { name: 'Change where agents run' }));
    await user.click(screen.getByRole('button', { name: destination === 'cluster' ? 'Choose a cluster, keep this workspace running' : 'Stop workspace and choose a place' }));
  }
  finish();
  await new Promise(resolve => setTimeout(resolve, 750));
  const expected = destination === 'Overview' || destination === 'Designer' ? destination : 'Clusters';
  expect(within(screen.getByRole('navigation')).getByRole('button', { name: expected }).getAttribute('aria-current')).toBe('page');
  expect(ipc.mock.calls.filter(([c]) => c === 'agent_run')).toHaveLength(1);
});
