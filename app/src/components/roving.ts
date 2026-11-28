import type { KeyboardEvent } from 'react';

// A segmented control (Agent / Try a task / File; All / This computer / a cluster) is one Tab stop, like a set of
// radio buttons: the chosen button is in the tab order and the others are not; Left/Right (and Up/Down) move to the
// next enabled choice and choose it; Home and End go to the first and last. The buttons keep their button role and
// aria-pressed, which tests, walks and assistive technology already read.
export const rovingIndex = (pressed: boolean, anyPressed: boolean, first: boolean) => pressed || !anyPressed && first ? 0 : -1;

export function rovingKeyDown(event: KeyboardEvent<HTMLElement>) {
  const keys: Record<string, number> = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 };
  if (!(event.key in keys) && event.key !== 'Home' && event.key !== 'End') return;
  if (event.altKey || event.ctrlKey || event.metaKey) return;
  const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>(':scope > button')].filter(button => !button.disabled);
  if (!buttons.length) return;
  const current = buttons.indexOf(event.target as HTMLButtonElement);
  const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1
    : (Math.max(current, 0) + keys[event.key] + buttons.length) % buttons.length;
  event.preventDefault();
  buttons[next].focus();
  if (buttons[next].getAttribute('aria-pressed') !== 'true') buttons[next].click();
}
