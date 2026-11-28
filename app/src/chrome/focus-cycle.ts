import { useEffect } from 'react';

// Tab follows the window's visual order because the DOM does: title bar, sidebar, page, status card. Focus usually
// starts on the page heading (it moves there on navigation), so Tab runs page, status card, then wraps. A desktop
// window has no browser around it to wrap through: without this the wrap would stop on the document itself, a stop
// with nothing to show. So Tab from the last control goes to the first (the title bar), and Shift+Tab back again.
const tabbable = 'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), summary, [tabindex]:not([tabindex="-1"])';

export function tabStops(root: ParentNode) {
  return [...root.querySelectorAll<HTMLElement>(tabbable)].filter(element => element.tabIndex >= 0
    && !element.closest('[hidden], [inert], [aria-hidden="true"]') && element.getClientRects().length > 0);
}

function openOverlay() {
  try { return !!document.querySelector(':popover-open, dialog[open]'); }
  catch { return !!document.querySelector('dialog[open]'); }
}

export function useFocusCycle(root: () => HTMLElement | null) {
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key !== 'Tab' || event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return;
      const shell = root();
      // An open popover or dialog keeps its own focus order.
      if (!shell || openOverlay()) return;
      const stops = tabStops(shell);
      if (!stops.length) return;
      const active = document.activeElement;
      // With focus on the document itself the engine already knows where Tab goes next: WebKit doesn't focus a clicked
      // button but starts the next Tab from it, so taking over here would send Tab from a clicked button to the title bar.
      if (!active || active === document.body) return;
      const outside = !shell.contains(active);
      if (event.shiftKey ? active === stops[0] || outside : active === stops.at(-1) || outside) {
        event.preventDefault();
        (event.shiftKey ? stops.at(-1) : stops[0])?.focus();
      }
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [root]);
}
