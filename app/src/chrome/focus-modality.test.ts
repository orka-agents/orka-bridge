import { afterEach, expect, test } from 'vitest';
import { trackFocusModality } from './focus-modality';

let stop: (() => void) | undefined;
afterEach(() => { stop?.(); stop = undefined; document.body.replaceChildren(); });

const key = (target: Element, name: string) => target.dispatchEvent(new KeyboardEvent('keydown', { key: name, bubbles: true }));

test('a press marks pointer focus, typing in a text field keeps it, and a key that moves focus clears it', () => {
  stop = trackFocusModality();
  const root = document.documentElement;
  const field = document.body.appendChild(document.createElement('textarea'));
  const search = document.body.appendChild(Object.assign(document.createElement('input'), { type: 'search' }));
  const button = document.body.appendChild(document.createElement('button'));
  expect(root.dataset.focus).toBeUndefined();

  field.dispatchEvent(new Event('pointerdown', { bubbles: true }));
  expect(root.dataset.focus).toBe('pointer');
  for (const name of ['a', 'Enter', 'ArrowDown', 'Backspace']) key(field, name);
  key(search, 'x');
  expect(root.dataset.focus).toBe('pointer');

  key(field, 'Tab');
  expect(root.dataset.focus).toBeUndefined();

  button.dispatchEvent(new Event('pointerdown', { bubbles: true }));
  key(field, 'Escape');
  expect(root.dataset.focus).toBeUndefined();

  // Outside a text field any key is a keyboard move: Enter on a button that opens a picker, arrows in a menu.
  button.dispatchEvent(new Event('pointerdown', { bubbles: true }));
  key(button, 'Enter');
  expect(root.dataset.focus).toBeUndefined();
});

test('stopping removes the listeners and the mark', () => {
  stop = trackFocusModality();
  document.body.dispatchEvent(new Event('pointerdown', { bubbles: true }));
  expect(document.documentElement.dataset.focus).toBe('pointer');
  stop(); stop = undefined;
  expect(document.documentElement.dataset.focus).toBeUndefined();
  document.body.dispatchEvent(new Event('pointerdown', { bubbles: true }));
  expect(document.documentElement.dataset.focus).toBeUndefined();
});
