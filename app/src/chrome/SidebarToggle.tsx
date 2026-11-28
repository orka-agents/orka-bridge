import { useEffect } from 'react';
import { Button } from '../components/Button';
import { isSidebarShortcut, sidebarShortcutKeys, sidebarShortcutLabel } from './sidebar';

// Folds the sidebar to an icon rail and back. In narrow windows, where the rail is always shown, it opens the full
// sidebar over the page instead; App decides which.
export default function SidebarToggle({ collapsed, onToggle }: { collapsed: boolean; onToggle: () => void }) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.repeat || !isSidebarShortcut(event)) return;
      event.preventDefault();
      onToggle();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onToggle]);
  const label = collapsed ? 'Expand sidebar' : 'Collapse sidebar';
  return <span className="chrome-control sidebar-toggle-control">
    <Button kind="icon" className="sidebar-toggle" aria-label={label} aria-pressed={collapsed} aria-keyshortcuts={sidebarShortcutKeys()} aria-controls="page-nav" onClick={onToggle}>
      <svg className="icon" viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4.5" width="18" height="15" rx="2.5" /><path d="M9.5 4.5v15" /></svg>
    </Button>
    <span className="control-tip" aria-hidden="true">{label} · {sidebarShortcutLabel()}</span>
  </span>;
}
