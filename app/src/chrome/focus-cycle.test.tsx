import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { useFocusCycle } from './focus-cycle';

// jsdom draws nothing, so every element reports one box: the stops are the ones the shell's DOM order gives.
beforeEach(() => { vi.spyOn(HTMLElement.prototype, 'getClientRects').mockReturnValue([{}] as unknown as DOMRectList); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); });
function Shell() {
  useFocusCycle(() => document.querySelector<HTMLElement>('.app-shell'));
  return <div className="app-shell">
    <header className="title-bar"><button type="button">Collapse sidebar</button></header>
    <nav><button type="button">Home</button></nav>
    <main><h1 tabIndex={-1} id="page-title">Clusters</h1><button type="button">Add a cluster</button></main>
    <footer className="status-bar"><button type="button">This computer</button></footer>
  </div>;
}

test('Tab runs title bar, sidebar, page, status card, and wraps to the title bar with no empty stop', async () => {
  const user = userEvent.setup(); render(<Shell />);
  screen.getByRole('heading', { name: 'Clusters' }).focus();
  const order: string[] = [];
  for (let i = 0; i < 5; i++) { await user.tab(); order.push(document.activeElement?.textContent ?? ''); }
  expect(order).toEqual(['Add a cluster', 'This computer', 'Collapse sidebar', 'Home', 'Add a cluster']);
  screen.getByRole('button', { name: 'Collapse sidebar' }).focus();
  await user.tab({ shift: true });
  expect(document.activeElement?.textContent).toBe('This computer');
  // The window itself takes no stop: Tab from the last control is handled, not left to the engine.
  expect(fireEvent.keyDown(screen.getByRole('button', { name: 'This computer' }), { key: 'Tab' })).toBe(false);
  expect(document.activeElement?.textContent).toBe('Collapse sidebar');
  expect(fireEvent.keyDown(screen.getByRole('button', { name: 'Home' }), { key: 'Tab' })).toBe(true);
});

test('Tab with focus on the document is left to the engine, which continues from a clicked button', () => {
  render(<Shell />);
  screen.getByRole('button', { name: 'Add a cluster' }).focus();
  // WebKit doesn't focus a clicked button: focus goes to the document and the engine starts the next Tab from the button.
  (document.activeElement as HTMLElement).blur();
  expect(document.activeElement).toBe(document.body);
  expect(fireEvent.keyDown(document.body, { key: 'Tab' })).toBe(true);
  expect(fireEvent.keyDown(document.body, { key: 'Tab', shiftKey: true })).toBe(true);
  expect(document.activeElement).toBe(document.body);
});
