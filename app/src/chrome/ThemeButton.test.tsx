import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import { applyTheme, readTheme, themePreference } from './theme';
import ThemeButton from './ThemeButton';

const root = document.documentElement;
afterEach(() => { cleanup(); vi.restoreAllMocks(); localStorage.clear(); root.removeAttribute('data-theme'); });

test('cycles System, Light, Dark and back, naming the current and next setting', async () => {
  const user = userEvent.setup();
  render(<ThemeButton />);
  const labels = ['Theme: System. Click for Light', 'Theme: Light. Click for Dark', 'Theme: Dark. Click for System'];
  const attributes = [null, 'light', 'dark'];
  for (const [index, label] of [...labels, labels[0]].entries()) {
    const button = screen.getByRole('button', { name: label });
    expect(button.classList.contains('button-icon')).toBe(true);
    expect(root.getAttribute('data-theme')).toBe(attributes[index % 3]);
    expect(button.parentElement?.querySelector('.control-tip')?.textContent).toBe(label);
    if (index < labels.length) await user.click(button);
  }
});

test('remembers the choice and applies it on the next launch', async () => {
  const user = userEvent.setup();
  const { unmount } = render(<ThemeButton />);
  await user.click(screen.getByRole('button', { name: 'Theme: System. Click for Light' }));
  await user.click(screen.getByRole('button', { name: 'Theme: Light. Click for Dark' }));
  expect(localStorage.getItem(themePreference)).toBe('dark');
  unmount(); root.removeAttribute('data-theme');
  applyTheme(readTheme());
  expect(root.getAttribute('data-theme')).toBe('dark');
  render(<ThemeButton />);
  expect(screen.getByRole('button', { name: 'Theme: Dark. Click for System' })).toBeTruthy();
  await user.click(screen.getByRole('button', { name: 'Theme: Dark. Click for System' }));
  expect(localStorage.getItem(themePreference)).toBeNull();
  expect(root.hasAttribute('data-theme')).toBe(false);
});

test('an unknown stored value means System', () => {
  localStorage.setItem(themePreference, 'sepia');
  expect(readTheme()).toBe('system');
});

test('storage that throws keeps System and still switches the page', async () => {
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw Error('denied'); });
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw Error('denied'); });
  const user = userEvent.setup();
  render(<ThemeButton />);
  await user.click(screen.getByRole('button', { name: 'Theme: System. Click for Light' }));
  expect(root.getAttribute('data-theme')).toBe('light');
  expect(screen.getByRole('button', { name: 'Theme: Light. Click for Dark' })).toBeTruthy();
});
