import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import FileMenu from './FileMenu';

const windowsAgent = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36 Edg/142.0.0.0';
const macAgent = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)';
function desktop(userAgent: string, quit: () => unknown = () => null) {
  vi.stubGlobal('isTauri', true);
  vi.stubGlobal('navigator', { ...navigator, userAgent });
  const ipc = vi.fn((command: string) => { if (command === 'quit_bridge') return quit(); throw Error(command); });
  mockIPC(ipc);
  return ipc;
}
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); });

test('Windows keeps an in-window File > Quit Bridge that runs Quit cleanup', async () => {
  const ipc = desktop(windowsAgent); const user = userEvent.setup(); render(<FileMenu />);
  const file = screen.getByRole('button', { name: 'File' });
  await user.click(file);
  expect(file.getAttribute('aria-expanded')).toBe('true');
  const quit = screen.getByRole('menuitem', { name: 'Quit Bridge' });
  expect(document.activeElement).toBe(quit);
  await user.keyboard('{Enter}');
  expect(ipc.mock.calls.filter(([command]) => command === 'quit_bridge')).toHaveLength(1);
});

test('Escape closes the menu and returns focus without quitting', async () => {
  const ipc = desktop(windowsAgent); const user = userEvent.setup(); render(<FileMenu />);
  await user.click(screen.getByRole('button', { name: 'File' }));
  await user.keyboard('{Escape}');
  expect(screen.queryByRole('menu')).toBeNull();
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'File' }));
  expect(ipc).not.toHaveBeenCalled();
});

test('a failed quit says where else to quit', async () => {
  desktop(windowsAgent, () => { throw Error('busy'); }); const user = userEvent.setup(); render(<FileMenu />);
  await user.click(screen.getByRole('button', { name: 'File' }));
  await user.click(screen.getByRole('menuitem', { name: 'Quit Bridge' }));
  expect((await screen.findByRole('alert')).textContent).toContain('tray icon’s menu');
});

test('macOS keeps its native menus, so the page draws no File menu', () => {
  desktop(macAgent); const { container } = render(<FileMenu />);
  expect(container.innerHTML).toBe('');
});

test('the Windows title bar carries the File menu', async () => {
  desktop(windowsAgent);
  const { default: App } = await import('../App');
  render(<App />);
  const file = await screen.findByRole('button', { name: 'File' });
  expect(file.closest('header')?.classList.contains('title-bar')).toBe(true);
});
