import { useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { customTitleBar } from './WindowControls';

// The page draws Windows' title bar and hides the native menu bar, so its File menu lives here.
// Quit Bridge runs the same cleanup as the tray's Quit; Close still only hides to the tray.
export default function FileMenu() {
  const [shown] = useState(() => customTitleBar());
  const [open, setOpen] = useState(false);
  const [error, setError] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  if (!shown) return null;
  const close = (focus: boolean) => { setOpen(false); if (focus) button.current?.focus(); };
  return <div className="file-menu" onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false); }}
    onKeyDown={event => { if (event.key === 'Escape' && open) { event.stopPropagation(); close(true); } }}>
    <button ref={button} type="button" className="file-menu-button" aria-haspopup="menu" aria-expanded={open} aria-controls="file-menu" onClick={() => { setError(false); setOpen(value => !value); }}>File</button>
    {open && <div id="file-menu" role="menu" aria-label="File">
      <button type="button" role="menuitem" ref={item => item?.focus()} onClick={() => {
        close(false);
        void invoke('quit_bridge').catch(() => setError(true));
      }}>Quit Bridge</button>
    </div>}
    {error && <span className="file-menu-error" role="alert">Couldn’t quit Bridge. Use Quit Bridge from the tray icon’s menu.</span>}
  </div>;
}
