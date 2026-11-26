import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from '../App';
import { noUpgrade } from '../setup/test-upgrade';
import { isSidebarShortcut, sidebarPreference } from './sidebar';

const macAgent = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)';
const linuxAgent = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36';
const shell = () => document.querySelector('.app-shell')!;
const nav = () => screen.getByRole('navigation', { name: 'Main' });

beforeEach(() => {
  localStorage.setItem('bridge.welcomeSeen.v1', '1');
  localStorage.setItem('bridge.firstRun.v1', 'done');
  vi.stubGlobal('isTauri', true);
  mockIPC(command => {
    if (command === 'workspace_upgrade_status') return noUpgrade;
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    return { kind: 'version', version: 'v0.4.2', hasDiagnostics: false };
  });
});
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); vi.restoreAllMocks(); localStorage.clear(); });

test('the title bar button folds the sidebar to a rail and back, and the choice is remembered', async () => {
  const user = userEvent.setup();
  const { unmount } = render(<App />);
  const toggle = screen.getByRole('button', { name: 'Collapse sidebar' });
  expect(toggle.closest('.title-bar-start')).not.toBeNull();
  expect(toggle.getAttribute('aria-pressed')).toBe('false');
  expect(shell().hasAttribute('data-sidebar')).toBe(false);
  await user.click(toggle);
  expect(shell().getAttribute('data-sidebar')).toBe('collapsed');
  expect(screen.getByRole('button', { name: 'Expand sidebar' }).getAttribute('aria-pressed')).toBe('true');
  expect(localStorage.getItem(sidebarPreference)).toBe('collapsed');
  unmount();
  render(<App />);
  expect(shell().getAttribute('data-sidebar')).toBe('collapsed');
  await user.click(screen.getByRole('button', { name: 'Expand sidebar' }));
  expect(shell().hasAttribute('data-sidebar')).toBe(false);
  expect(localStorage.getItem(sidebarPreference)).toBe('expanded');
});

test('folded tabs keep their page names and status descriptions for assistive technology', () => {
  localStorage.setItem(sidebarPreference, 'collapsed');
  render(<App />);
  expect(shell().getAttribute('data-sidebar')).toBe('collapsed');
  const tabs = [...document.querySelectorAll<HTMLButtonElement>('button.page-tab[aria-label]')];
  expect(tabs.map(tab => tab.getAttribute('aria-label'))).toEqual(['Home', 'Overview', 'Designer', 'Clusters']);
  expect(within(nav()).getAllByRole('button').map(button => button.getAttribute('aria-label'))).toEqual(['Home', 'Overview', 'Designer', 'Clusters']);
  for (const tab of tabs) {
    const description = document.getElementById(tab.getAttribute('aria-describedby')!);
    expect(description?.closest('.page-tab')).toBe(tab);
    expect(description?.textContent).not.toBe('');
  }
  expect(screen.getByRole('button', { name: 'Home' }).getAttribute('aria-describedby')).toBe('tab-status-Home');
  expect(document.getElementById('tab-status-Home')?.textContent).toBe('Give an agent a task');
  // The status card keeps its words in the page; only its dot is drawn.
  const footer = screen.getByRole('contentinfo');
  expect(footer.querySelector('.host-label')?.textContent).not.toBe('');
  expect(footer.querySelector('.status-lines')).not.toBeNull();
});

test('Ctrl B toggles on Windows and Linux, Cmd B on macOS', () => {
  vi.stubGlobal('navigator', { ...navigator, userAgent: linuxAgent });
  render(<App />);
  fireEvent.keyDown(window, { key: 'b', ctrlKey: true });
  expect(shell().getAttribute('data-sidebar')).toBe('collapsed');
  fireEvent.keyDown(window, { key: 'b', metaKey: true });
  fireEvent.keyDown(window, { key: 'b', ctrlKey: true, shiftKey: true });
  expect(shell().getAttribute('data-sidebar')).toBe('collapsed');
  fireEvent.keyDown(window, { key: 'B', ctrlKey: true });
  expect(shell().hasAttribute('data-sidebar')).toBe(false);
  expect(screen.getByRole('button', { name: 'Collapse sidebar' }).getAttribute('aria-keyshortcuts')).toBe('Control+B');

  const press = (init: KeyboardEventInit) => isSidebarShortcut(new KeyboardEvent('keydown', init), macAgent);
  expect(press({ key: 'b', metaKey: true })).toBe(true);
  expect(press({ key: 'b', ctrlKey: true })).toBe(false);
  expect(press({ key: 'b', metaKey: true, altKey: true })).toBe(false);
});

test('narrow windows open the full sidebar over the rail, and choosing a page or Escape closes it', async () => {
  vi.stubGlobal('matchMedia', (query: string) => ({ matches: query === '(max-width: 820px)' }));
  vi.stubGlobal('navigator', { ...navigator, userAgent: linuxAgent });
  const user = userEvent.setup();
  render(<App />);
  // The rail is forced, so the toggle offers to expand it; the folded preference is never written.
  const toggle = screen.getByRole('button', { name: 'Expand sidebar' });
  expect(toggle.getAttribute('aria-pressed')).toBe('true');
  await user.click(toggle);
  expect(shell().getAttribute('data-sidebar')).toBe('overlay');
  expect(screen.getByRole('button', { name: 'Collapse sidebar' }).getAttribute('aria-pressed')).toBe('false');
  await user.click(screen.getByRole('button', { name: 'Overview' }));
  expect(shell().hasAttribute('data-sidebar')).toBe(false);
  fireEvent.keyDown(window, { key: 'b', ctrlKey: true });
  expect(shell().getAttribute('data-sidebar')).toBe('overlay');
  fireEvent.keyDown(window, { key: 'Escape' });
  expect(shell().hasAttribute('data-sidebar')).toBe(false);
  expect(localStorage.getItem(sidebarPreference)).toBeNull();
});

test('storage that throws keeps the full sidebar and still toggles', async () => {
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(key => { if (key === sidebarPreference) throw Error('denied'); return key === 'bridge.firstRun.v1' ? 'done' : key === 'bridge.welcomeSeen.v1' ? '1' : null; });
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw Error('denied'); });
  const user = userEvent.setup();
  render(<App />);
  expect(shell().hasAttribute('data-sidebar')).toBe(false);
  await user.click(screen.getByRole('button', { name: 'Collapse sidebar' }));
  expect(shell().getAttribute('data-sidebar')).toBe('collapsed');
});

test('the guided first run has no sidebar toggle and keeps the full sidebar', () => {
  localStorage.clear();
  localStorage.setItem(sidebarPreference, 'collapsed');
  render(<App />);
  expect(shell().classList.contains('guided')).toBe(true);
  expect(screen.queryByRole('button', { name: /sidebar/ })).toBeNull();
  expect(shell().hasAttribute('data-sidebar')).toBe(false);
  expect(screen.getByRole('button', { name: 'Theme: System. Click for Light' })).toBeTruthy();
});
