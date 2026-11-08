import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import Example from './Example';
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); });
test('native example is a fixed read-only pair with no authoring or credential action', async () => {
  vi.stubGlobal('isTauri', true);
  const ipc = vi.fn(() => ({ agent: 'agent source', tests: 'test source' })); mockIPC(ipc);
  render(<Example />);
  expect(await screen.findByText('agent source')).toBeTruthy(); expect(screen.getByText('test source')).toBeTruthy();
  expect(screen.queryByRole('textbox')).toBeNull(); expect(screen.queryByRole('button', { name: 'Run' })).toBeNull();
  expect(ipc).toHaveBeenCalledExactlyOnceWith('setup_example', {});
});
test.each([null, {}, { agent: 'SECRET_MARKER', tests: '', extra: 'secret' }])('malformed fixed reader reply cannot render private data', async value => {
  vi.stubGlobal('isTauri', true); mockIPC(() => value); render(<Example />);
  expect(await screen.findByRole('alert')).toBeTruthy(); expect(document.body.textContent).not.toContain('SECRET_MARKER');
});
test('browser preview does not fabricate a native example read', async () => {
  vi.stubGlobal('isTauri', false); const ipc = vi.fn(); mockIPC(ipc); render(<Example />);
  expect(await screen.findByRole('alert')).toBeTruthy(); expect(ipc).not.toHaveBeenCalled();
});
