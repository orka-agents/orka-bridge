import { type RefObject, useLayoutEffect } from 'react';

const gap = 4;
const edge = 16;
const widest = 520;

// Where a picker opens: under its trigger, left edges aligned and kept inside the window, flipped above only when
// there is more room there and the list doesn't fit below. Plain numbers so it can be tested without a layout engine.
// `view` is the page area under the title bar (top to bottom), so a picker never covers the window's own controls.
export function placePopover(trigger: { top: number; bottom: number; left: number }, natural: number, view: { width: number; top?: number; bottom: number }) {
  const width = Math.min(widest, view.width - 2 * edge);
  const left = Math.min(Math.max(trigger.left, edge), view.width - width - edge);
  const below = view.bottom - trigger.bottom - gap - edge;
  const above = trigger.top - gap - (view.top ?? 0) - edge / 2;
  const flip = natural > below && above > below;
  const maxHeight = Math.max(0, flip ? above : below);
  const top = flip ? trigger.top - gap - Math.min(natural, maxHeight) : trigger.bottom + gap;
  return { top, left, width, maxHeight, flip };
}

// Positions an open popover next to its trigger through custom properties (CSP allows them; inline style attributes
// it does not), and follows the window as it resizes or the page scrolls.
export function useAnchoredPopover(open: boolean, trigger: RefObject<HTMLElement | null>, popover: RefObject<HTMLElement | null>) {
  useLayoutEffect(() => {
    const panel = popover.current;
    if (!open || !panel) return;
    const place = () => {
      const anchor = trigger.current?.getBoundingClientRect();
      if (!anchor) return;
      panel.classList.add('popover-anchored');
      // Measure the list at its natural height before limiting it.
      panel.style.removeProperty('--popover-max-height');
      const page = trigger.current?.closest('.page-content')?.getBoundingClientRect();
      const spot = placePopover(anchor, panel.scrollHeight, { width: window.innerWidth, top: page?.top ?? 0, bottom: Math.min(page?.bottom ?? window.innerHeight, window.innerHeight) });
      panel.style.setProperty('--popover-top', `${Math.round(spot.top)}px`);
      panel.style.setProperty('--popover-left', `${Math.round(spot.left)}px`);
      panel.style.setProperty('--popover-width', `${Math.round(spot.width)}px`);
      panel.style.setProperty('--popover-max-height', `${Math.round(spot.maxHeight)}px`);
      panel.classList.toggle('popover-above', spot.flip);
    };
    place();
    const frame = requestAnimationFrame(place);
    // A search that narrows the list changes its height, so a list shown above its trigger stays against it.
    const content = new MutationObserver(place);
    content.observe(panel, { childList: true, subtree: true });
    window.addEventListener('resize', place);
    // Capture scrolls of the page's own scroll area too, not only the window. Not the popover's own list: placing
    // measures the list unlimited, which resets its scroll, so the list could never scroll past its first choices.
    const scrolled = (event: Event) => { if (!(event.target instanceof Node && panel.contains(event.target))) place(); };
    document.addEventListener('scroll', scrolled, true);
    return () => { content.disconnect(); cancelAnimationFrame(frame); window.removeEventListener('resize', place); document.removeEventListener('scroll', scrolled, true); };
  }, [open, trigger, popover]);
}
