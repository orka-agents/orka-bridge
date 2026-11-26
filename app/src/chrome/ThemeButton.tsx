import { useState } from 'react';
import { Button } from '../components/Button';
import { applyTheme, nextTheme, readTheme, type Theme, writeTheme } from './theme';

const names: Record<Theme, string> = { system: 'System', light: 'Light', dark: 'Dark' };

function ThemeIcon({ theme }: { theme: Theme }) {
  return <svg className="icon" viewBox="0 0 24 24" aria-hidden="true">
    {theme === 'system' && <><rect x="3" y="4" width="18" height="12" rx="2" /><path d="M8 20h8M12 16v4" /></>}
    {theme === 'light' && <><circle cx="12" cy="12" r="4" /><path d="M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4" /></>}
    {theme === 'dark' && <path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z" />}
  </svg>;
}

// One button cycles System, Light, Dark. The icon shows the current setting.
export default function ThemeButton() {
  const [theme, setTheme] = useState(readTheme);
  const label = `Theme: ${names[theme]}. Click for ${names[nextTheme(theme)]}`;
  return <span className="chrome-control">
    <Button kind="icon" className="theme-button" aria-label={label} onClick={() => {
      const next = nextTheme(theme);
      writeTheme(next); applyTheme(next); setTheme(next);
    }}><ThemeIcon theme={theme} /></Button>
    <span className="control-tip" aria-hidden="true">{label}</span>
  </span>;
}
