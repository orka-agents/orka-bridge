import { act, cleanup, render } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import { useNarrowWindow } from './sidebar';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); document.documentElement.style.removeProperty('--window-narrow'); });
function Probe() { return <p>{useNarrowWindow() ? 'narrow' : 'wide'}</p>; }

test('widening the window leaves the narrow layout even when the engine sends no media change event', () => {
  // A media query list that never fires change, as WebKitGTK was seen to do: only its matches value moves.
  let width = 540;
  vi.stubGlobal('matchMedia', () => ({ get matches() { return width <= 820; }, addEventListener: () => undefined, removeEventListener: () => undefined }));
  const view = render(<Probe />);
  expect(view.container.textContent).toBe('narrow');
  width = 1280;
  act(() => { window.dispatchEvent(new Event('resize')); });
  expect(view.container.textContent).toBe('wide');
  // Crossing the breakpoint restyles every element, so no rule of the narrow layout is left applied.
  expect(document.documentElement.style.getPropertyValue('--window-narrow')).toBe('0');
  width = 600;
  act(() => { window.dispatchEvent(new Event('resize')); });
  expect(view.container.textContent).toBe('narrow');
  expect(document.documentElement.style.getPropertyValue('--window-narrow')).toBe('1');
});
