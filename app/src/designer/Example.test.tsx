import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import Example from './Example';
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); });
test('opening the fixed pair only observes Run, without authoring or credential action', async () => {
  vi.stubGlobal('isTauri', true);
  const ipc = vi.fn(command => command === 'setup_example' ? { agent: 'agent source', tests: 'test source' } : { state: 'idle', canRun: false, report: null, acknowledged: [], pending: null, error: null }); mockIPC(ipc);
  render(<Example />);
  expect(await screen.findByText('agent source')).toBeTruthy(); expect(screen.getByText('test source')).toBeTruthy();
  expect(screen.queryByRole('textbox')).toBeNull(); expect((await screen.findByRole('button', { name: 'Run test' }) as HTMLButtonElement).disabled).toBe(true);
  expect(ipc).toHaveBeenCalledWith('setup_example', {});
  expect(ipc.mock.calls.every(([cmd]) => ['setup_example', 'setup_run_status'].includes(cmd))).toBe(true);
});
test.each([null, {}, { agent: 'SECRET_MARKER', tests: '', extra: 'secret' }])('malformed fixed reader reply cannot render private data', async value => {
  vi.stubGlobal('isTauri', true); mockIPC(() => value); render(<Example />);
  expect(await screen.findByRole('alert')).toBeTruthy(); expect(document.body.textContent).not.toContain('SECRET_MARKER');
});
test('browser preview does not fabricate a native example read', async () => {
  vi.stubGlobal('isTauri', false); const ipc = vi.fn(); mockIPC(ipc); render(<Example />);
  expect(await screen.findByRole('alert')).toBeTruthy(); expect(ipc).not.toHaveBeenCalled();
});
