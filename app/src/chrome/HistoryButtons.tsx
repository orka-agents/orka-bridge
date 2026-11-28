import { type FocusEvent, useEffect, useRef } from 'react';
import { Button } from '../components/Button';
import { historyShortcutKeys, historyShortcutLabels } from './history';

// Back and Forward through the places visited in this run. Buttons opt out of the Windows title bar's drag region.
export default function HistoryButtons({ canBack, canForward, onBack, onForward }: { canBack: boolean; canForward: boolean; onBack: () => void; onForward: () => void }) {
  const labels = historyShortcutLabels();
  const keys = historyShortcutKeys();
  const backButton = useRef<HTMLButtonElement>(null);
  const forwardButton = useRef<HTMLButtonElement>(null);
  // Focus stays on the button, as on a sidebar item; a button left with nowhere to go hands focus to the other.
  useEffect(() => {
    const active = document.activeElement;
    const stranded = (button: HTMLButtonElement | null) => !!button && (active === button || active === document.body || active === null);
    if (!canBack && canForward && stranded(backButton.current) && pressed.current === 'back') forwardButton.current?.focus();
    if (!canForward && canBack && stranded(forwardButton.current) && pressed.current === 'forward') backButton.current?.focus();
  }, [canBack, canForward]);
  const pressed = useRef<'back' | 'forward'>(undefined);
  // Losing focus because the button turned disabled has no next target; moving elsewhere does.
  const leave = (event: FocusEvent<HTMLButtonElement>) => { if (event.relatedTarget) pressed.current = undefined; };
  return <>
    <span className="chrome-control history-control">
      <Button ref={backButton} kind="icon" className="history-back" aria-label="Back" aria-keyshortcuts={keys.back} disabled={!canBack} onClick={() => { pressed.current = 'back'; onBack(); }} onBlur={leave}>
        <svg className="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m14.5 6-6 6 6 6" /></svg>
      </Button>
      <span className="control-tip" aria-hidden="true">Back · {labels.back}</span>
    </span>
    <span className="chrome-control history-control">
      <Button ref={forwardButton} kind="icon" className="history-forward" aria-label="Forward" aria-keyshortcuts={keys.forward} disabled={!canForward} onClick={() => { pressed.current = 'forward'; onForward(); }} onBlur={leave}>
        <svg className="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m9.5 6 6 6-6 6" /></svg>
      </Button>
      <span className="control-tip" aria-hidden="true">Forward · {labels.forward}</span>
    </span>
  </>;
}
