import { useEffect, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow, type Window } from '@tauri-apps/api/window';
import './chrome.css';

// Bridge draws its own title bar only on Windows; other platforms keep the native frame.
export function customTitleBar(userAgent = navigator.userAgent) {
  return isTauri() && /\bWindows\b/.test(userAgent);
}

function currentWindow(): Window | undefined {
  try { return getCurrentWindow(); } catch { return undefined; }
}

export default function WindowControls() {
  const [win] = useState(() => customTitleBar() ? currentWindow() : undefined);
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    if (!win) return;
    let live = true;
    const sync = () => { void win.isMaximized().then(value => { if (live) setMaximized(value); }, () => undefined); };
    sync();
    window.addEventListener('resize', sync);
    return () => { live = false; window.removeEventListener('resize', sync); };
  }, [win]);
  if (!win) return null;
  // Close goes through the native close request, so the existing close-to-tray rules and notice apply.
  return <div className="window-controls">
    <button type="button" aria-label="Minimize" title="Minimize" onClick={() => { void win.minimize(); }}>
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5.5h10" /></svg>
    </button>
    <button type="button" aria-label={maximized ? 'Restore' : 'Maximize'} title={maximized ? 'Restore' : 'Maximize'} onClick={() => { void win.toggleMaximize(); }}>
      <svg viewBox="0 0 10 10" aria-hidden="true">{maximized
        ? <><path d="M2.5 2.5V.5h7v7h-2" /><rect x=".5" y="2.5" width="7" height="7" /></>
        : <rect x=".5" y=".5" width="9" height="9" />}</svg>
    </button>
    <button type="button" className="window-close" aria-label="Close" title="Close" onClick={() => { void win.close(); }}>
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="m.5.5 9 9m0-9-9 9" /></svg>
    </button>
  </div>;
}
