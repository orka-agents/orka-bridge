import { readFileSync } from 'node:fs';
import { expect, test } from 'vitest';
import { placePopover } from './anchored-popover';

const view = { width: 1200, top: 44, bottom: 820 };
test('a picker opens right under its trigger, its left edge on the trigger\'s', () => {
  expect(placePopover({ top: 370, bottom: 402, left: 405 }, 320, view)).toEqual({ top: 406, left: 405, width: 520, maxHeight: 398, flip: false });
});
test('it flips above only when the list does not fit below and there is more room above', () => {
  const low = placePopover({ top: 700, bottom: 732, left: 405 }, 320, view);
  expect(low.flip).toBe(true);
  expect(low.top + 320).toBe(696);
  expect(low.top).toBe(700 - 4 - 320);
  // Not enough room either way: it stays below and its list scrolls.
  const narrow = placePopover({ top: 323, bottom: 339, left: 100 }, 330, { width: 540, top: 44, bottom: 640 });
  expect(narrow).toMatchObject({ flip: false, top: 343, maxHeight: 281 });
});
test('it stays inside the window at any width and never covers the title bar', () => {
  const spot = placePopover({ top: 323, bottom: 339, left: 400 }, 200, { width: 540, top: 44, bottom: 640 });
  expect(spot.left).toBe(16); expect(spot.width).toBe(508);
  const above = placePopover({ top: 600, bottom: 632, left: 16 }, 2000, { width: 540, top: 44, bottom: 640 });
  expect(above.flip).toBe(true); expect(above.top).toBeGreaterThanOrEqual(44);
});
test('the list scrolls inside the popover, so its scrollbar is clear of the popover\'s edge', () => {
  const css = readFileSync('src/clusters/clusters.css', 'utf8');
  expect(css).toMatch(/\.context-picker\.popover-anchored \{ inset: auto; top: var\(--popover-top\); left: var\(--popover-left\);[^}]*overflow: hidden; \}/);
  expect(css).toMatch(/\.context-picker\.popover-anchored ul \{[^}]*overflow: auto;[^}]*scrollbar-gutter: stable; \}/);
});
