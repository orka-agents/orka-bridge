import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from '../App';
import { SAMPLE_DOCUMENT_ID } from '../designer/agent-api';
import { noUpgrade } from '../setup/test-upgrade';
import { resetConversation } from './conversation-store';
import { NAVIGATOR_ID } from './navigator-api';

beforeEach(() => { localStorage.setItem('bridge.welcomeSeen.v1', '1'); resetConversation(); });
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

const proposal = '```navigator-card\n{"kind":"task","agent":"web-reader","task":"Read https://example.com"}\n```';
function native(options: { runStatus: 'connected' | 'disconnected' | 'unreadable'; agent?: string; navigator?: string }) {
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
      const connected = options.runStatus === 'connected';
      return { documentId, busy: null, state: 'idle', canRun: connected, modelConnected: connected, report: null, acknowledged: [], pending: null, error: null };
    }
    if (command === 'agent_chat_status') return { documentId, state: 'ready', busy: false, canSend: true, versions: [{ turns: [] }], error: null };
    if (command === 'navigator_status') return { chat: { documentId: NAVIGATOR_ID, state: 'ready', busy: false, canSend: true, error: null,
      versions: [{ turns: options.navigator ? [{ message: 'Read it', answer: options.navigator, phase: 'Succeeded', tools: [], contextRetained: true, terminationUnconfirmed: false, stopUnconfirmed: false, startedAt: 1_700_000_000_000, finishedAt: 1_700_000_003_000 }] : [] }] }, thinksWith: null };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
}

test.each([['connected', false], ['disconnected', true]] as const)('Home asks to connect the AI only when the workspace reports it unconnected (%s)', async (runStatus, warned) => {
  native({ runStatus });
  const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Home' }));
  await screen.findByRole('region', { name: 'What do you want done?' });
  await new Promise(resolve => setTimeout(resolve, 700));
  expect(!!screen.queryByRole('heading', { name: 'Connect the AI first' })).toBe(warned);
  expect(!!screen.queryByRole('textbox', { name: 'Message' })).toBe(!warned);
});

test('an unreadable status poll does not claim the AI is disconnected', async () => {
  native({ runStatus: 'unreadable' });
  const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Home' }));
  await screen.findByRole('textbox', { name: 'Message' });
  await new Promise(resolve => setTimeout(resolve, 700));
  expect(screen.queryByRole('heading', { name: 'Connect the AI first' })).toBeNull();
});

test('a proposed task honours Designer\'s gate for the agent Designer has open', async () => {
  native({ runStatus: 'connected', navigator: `A task for web-reader.\n\n${proposal}`, agent: '---\nname: web-reader\nmodel: gpt-5.6-terra\nkmx:\n  runtime: orka\n  schemaVersion: 1\n  tools: []\n---\n' });
  const user = userEvent.setup(); render(<App />);
  await user.click(screen.getByRole('button', { name: 'Designer' }));
  await screen.findByRole('textbox', { name: 'Instructions' });
  await user.click(screen.getByRole('button', { name: 'Home' }));
  const card = await screen.findByRole('article', { name: 'Proposed task' });
  expect(await within(card).findByText('Write instructions in Agent view before trying a task.')).toBeTruthy();
  expect(within(card).getByRole('button', { name: 'Create task' }).matches(':disabled')).toBe(true);
});

test('a finished workspace opens on Home with the Navigator', async () => {
  localStorage.setItem('bridge.firstTest.v1', 'attempted');
  native({ runStatus: 'connected' });
  render(<App />);
  expect(await screen.findByRole('textbox', { name: 'Message' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Home' }).getAttribute('aria-current')).toBe('page');
  expect(document.getElementById('tab-status-Home')?.textContent).toBe('Navigator');
});
