/**
 * How focus last moved: by a pointer or from the keyboard. Browsers match `:focus-visible` on a text field even when it
 * is clicked, so on its own the keyboard indicator would show on every click into a field. The root carries
 * `data-focus="pointer"` after a press, and loses it on a key that moves focus (any key outside a text field, or Tab
 * and Escape inside one), so a clicked field keeps its quiet edge while typing and a field reached by Tab gets the full
 * indicator. Without the attribute every field gets the full indicator.
 */
const typing = (target: EventTarget | null) => target instanceof HTMLElement
  && (target.isContentEditable || target instanceof HTMLTextAreaElement
    || target instanceof HTMLInputElement && !['button', 'checkbox', 'color', 'file', 'image', 'radio', 'range', 'reset', 'submit'].includes(target.type));

export function trackFocusModality(root: HTMLElement = document.documentElement, events: Pick<Window, 'addEventListener' | 'removeEventListener'> = window) {
  const pointer = () => { root.dataset.focus = 'pointer'; };
  const key = (event: KeyboardEvent) => {
    if (event.key === 'Tab' || event.key === 'Escape' || !typing(event.target)) delete root.dataset.focus;
  };
  events.addEventListener('pointerdown', pointer, true);
  events.addEventListener('keydown', key, true);
  return () => {
    events.removeEventListener('pointerdown', pointer, true);
    events.removeEventListener('keydown', key, true);
    delete root.dataset.focus;
  };
}
