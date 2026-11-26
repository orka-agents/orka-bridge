import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from '../App';
import { SAMPLE_DOCUMENT_ID } from '../designer/agent-api';
import { noUpgrade } from '../setup/test-upgrade';

beforeEach(() => { localStorage.setItem('bridge.welcomeSeen.v1', '1'); });
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

test.each([[false, true], [true, false]])('Home asks to connect the AI only when the workspace reports it unconnected (connected=%s)', async (modelConnected, warned) => {
  vi.stubGlobal('isTauri', true);
  mockIPC((command, args) => {
    if (command === 'workspace_upgrade_status') return noUpgrade;
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'agents_list') return [{ id: SAMPLE_DOCUMENT_ID, name: 'web-reader', path: null, available: true }];
    if (command === 'agent_run_status') return { documentId: (args as { documentId: string }).documentId, busy: null, state: 'idle', canRun: modelConnected, modelConnected, report: null, acknowledged: [], pending: null, error: null };
    if (command === 'agent_chat_status') return { documentId: SAMPLE_DOCUMENT_ID, state: 'ready', busy: false, canSend: modelConnected, versions: [{ turns: [] }], error: null };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
  const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Home' }));
  await screen.findByRole('textbox', { name: /one task$/ });
  await new Promise(resolve => setTimeout(resolve, 700));
  expect(screen.queryAllByText('Connect the AI in Clusters before trying a task.', { exact: false }).length > 0).toBe(warned);
});

function native(options: { runStatus: 'connected' | 'unreadable'; agent?: string }) {
  vi.stubGlobal('isTauri', true);
  mockIPC((command, args) => {
    const documentId = (args as { documentId?: string } | undefined)?.documentId ?? SAMPLE_DOCUMENT_ID;
    if (command === 'workspace_upgrade_status') return noUpgrade;
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'agents_list') return [{ id: SAMPLE_DOCUMENT_ID, name: 'web-reader', path: null, available: true }];
    if (command === 'agent_selected') return SAMPLE_DOCUMENT_ID;
    if (command === 'agent_open') return { documentId: SAMPLE_DOCUMENT_ID, name: 'web-reader', agent: options.agent ?? '---\nname: web-reader\nmodel: gpt-5.6-terra\nkmx:\n  runtime: orka\n  schemaVersion: 1\n  tools: []\n---\nRead pages.', tests: 'Checks' };
    if (command === 'agent_run_status') {
      if (options.runStatus === 'unreadable') throw Error('lost');
      return { documentId, busy: null, state: 'idle', canRun: true, modelConnected: true, report: null, acknowledged: [], pending: null, error: null };
    }
    if (command === 'agent_chat_status') return { documentId: SAMPLE_DOCUMENT_ID, state: 'ready', busy: false, canSend: true, versions: [{ turns: [] }], error: null };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
}

test('an unreadable status poll does not claim the AI is disconnected', async () => {
  native({ runStatus: 'unreadable' });
  const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Home' }));
  await screen.findByRole('textbox', { name: /one task$/ });
  await new Promise(resolve => setTimeout(resolve, 700));
  expect(screen.queryAllByText('Connect the AI in Clusters before trying a task.', { exact: false })).toHaveLength(0);
});

test('Home applies Designer\'s Try a task gate for the agent Designer has open', async () => {
  native({ runStatus: 'connected', agent: '---\nname: web-reader\nmodel: gpt-5.6-terra\nkmx:\n  runtime: orka\n  schemaVersion: 1\n  tools: []\n---\n' });
  const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  await screen.findByRole('textbox', { name: 'Instructions' });
  await user.click(screen.getByRole('button', { name: 'Home' }));
  // Designer's Try a task stays mounted but hidden, so look only inside Home.
  const home = await screen.findByRole('region', { name: 'What do you want done?' });
  expect((await within(home).findAllByText('Write instructions in Agent view before trying a task.', { exact: false })).length).toBeGreaterThan(0);
  expect(within(home).getByRole('button', { name: 'Run task' }).matches(':disabled')).toBe(true);
});

test('a finished workspace opens on Home', async () => {
  localStorage.setItem('bridge.firstTest.v1', 'attempted');
  native({ runStatus: 'connected' });
  render(<App />);
  expect(await screen.findByRole('textbox', { name: /one task$/ })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Home' }).getAttribute('aria-current')).toBe('page');
});
