import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const css = readFileSync('src/app.css', 'utf8');
// Light, then dark when the system asks for it (unless Light is chosen), then dark when chosen outright.
const palettes = [...css.matchAll(/:root(?::not\(\[data-theme="light"\]\)|\[data-theme="dark"\])?\s*\{([^}]+)\}/g)].map(([, block]) =>
  Object.fromEntries([...block.matchAll(/--([\w-]+):\s*([^;]+);/g)].map(([, name, value]) => [name, value.trim()])),
);

const luminance = (hex: string) => {
  const rgb = hex.slice(1).match(/../g)!.map(channel => parseInt(channel, 16) / 255)
    .map(channel => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4);
  return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
};
const contrast = (foreground: string, background: string) => {
  const [lighter, darker] = [luminance(foreground), luminance(background)].sort((a, b) => b - a);
  return (lighter + 0.05) / (darker + 0.05);
};

describe('Orka ink and paper theme contract', () => {
  it.each([
    ['light', 0, '#ffffff', '#f6f6f7', '#f0f0f2', '#0b0b0c', '#a64b00', '#0369a1', '#047857', '#be123c'],
    ['dark', 1, '#09090a', '#1b1b1e', '#1b1b1e', '#f4f4f5', '#f5b544', '#4cb8f5', '#4ade80', '#f87171'],
  ] as const)('%s uses the Orka neutrals and state colours', (_, index, page, surface, soft, ink, pending, running, succeeded, failed) => {
    expect(palettes[index]).toMatchObject({
      'color-page': page, 'color-chrome': surface, 'color-hover': soft,
      'color-text': ink, 'color-action': ink, 'color-status-pending': pending,
      'color-status-running': running, 'color-status-succeeded': succeeded, 'color-status-failed': failed,
    });
  });
  it.each([0, 1])('keeps every text role readable on shared surfaces in palette %s', index => {
    const palette = palettes[index];
    for (const ink of ['color-text-secondary', 'color-text-tertiary', 'color-status-pending', 'color-status-running', 'color-status-succeeded', 'color-status-failed']) {
      for (const surface of ['color-page', 'color-card', 'color-chrome', 'color-hover']) {
        expect(contrast(palette[ink], palette[surface])).toBeGreaterThanOrEqual(4.5);
      }
    }
    expect(contrast(palette['color-action-text'], palette['color-action'])).toBeGreaterThanOrEqual(4.5);
    expect(contrast(palette['color-action-text'], palette['color-action-hover'])).toBeGreaterThanOrEqual(4.5);
    expect(contrast(palette['color-terminal-text'], palette['color-terminal'])).toBeGreaterThanOrEqual(4.5);
    // The focus ring is the running blue, so it shows against every surface a control sits on.
    expect(palette['color-focus']).toBe(palette['color-status-running']);
  });
  it.each([0, 1])('keeps every status tint readable in palette %s', index => {
    const palette = palettes[index];
    for (const [ink, surface] of [
      ['color-status-pending', 'color-tint-pending'],
      ['color-status-running', 'color-tint-running'],
      ['color-status-succeeded', 'color-tint-succeeded'],
      ['color-status-failed', 'color-tint-failed'],
      ['color-text-secondary', 'color-tint-stale'],
    ]) expect(contrast(palette[ink], palette[surface])).toBeGreaterThanOrEqual(4.5);
  });
  it('declares themed thin scrollbars and a sidebar of full-width page buttons', () => {
    const shell = readFileSync('src/shell.css', 'utf8');
    const controls = readFileSync('src/components/controls.css', 'utf8');
    expect(shell).toMatch(/scrollbar-width:\s*thin/);
    expect(shell).toMatch(/scrollbar-color:\s*var\(--color-text-tertiary\)\s+var\(--color-page\)/);
    expect(shell).toMatch(/\.app-shell\s*\{[^}]*grid-template-areas:\s*"title title"\s*"nav main"\s*"status main"/);
    expect(shell).toMatch(/\.page-tab\s*\{[^}]*width:\s*100%/);
    // Narrow windows keep a rail with page names, and the status card becomes a bar below the page, as main's footer was.
    const compact = shell.slice(shell.indexOf('@media (max-width: 820px)'));
    expect(compact).toMatch(/\.app-shell\s*\{[^}]*--sidebar-width:\s*76px;[^}]*grid-template-areas:\s*"title title"\s*"nav main"\s*"status status"/);
    expect(compact).toMatch(/\.brand b, \.nav-group-label, \.tab-copy small\s*\{\s*display:\s*none/);
    expect(compact).not.toMatch(/\.tab-copy\s*\{[^}]*display:\s*none|\.tab-copy b[^{]*\{[^}]*display:\s*none|clip-path|\.status-lines[^{]*\{[^}]*(display:\s*none|position:\s*absolute)|\.host-label[^{]*\{[^}]*(display:\s*none|position:\s*absolute)/);
    expect(controls).toMatch(/\.button-quiet\s*\{[^}]*padding-inline:\s*0/);
  });
  it('folds the wide sidebar to a 60px rail that keeps names and status for assistive technology', () => {
    const shell = readFileSync('src/shell.css', 'utf8');
    expect(shell).toMatch(/\.app-shell\s*\{[^}]*--sidebar-width:\s*248px;/);
    expect(shell).toMatch(/\.title-bar\s*\{[^}]*grid-template-columns:\s*minmax\(0, 1fr\) auto minmax\(0, 1fr\);[^}]*min-height:\s*44px/);
    // The folded rail applies to wide windows only, so the narrow rules after it keep their visible names and status bar.
    const folded = shell.slice(shell.indexOf('@media (min-width: 821px)'), shell.indexOf('@media (max-width: 820px)'));
    expect(folded).toMatch(/\.app-shell\[data-sidebar="collapsed"\]\s*\{\s*--sidebar-width:\s*60px;\s*\}/);
    expect(folded).toMatch(/\.app-shell\[data-sidebar="collapsed"\] :is\(\.nav-group-label, \.tab-copy, \.host-label, \.host-chip \.chip, \.status-lines, \.checks-status\)\s*\{[^}]*position: absolute;[^}]*clip-path: inset\(50%\)/);
    expect(folded).toMatch(/\.page-tab::after\s*\{\s*content: attr\(aria-label\)/);
    expect(folded).not.toMatch(/display:\s*none/);
    const compact = shell.slice(shell.indexOf('@media (max-width: 820px)'));
    expect(compact).toMatch(/\.sidebar-toggle-control\s*\{\s*display:\s*none/);
  });
  it('uses the system dark preference and bundled reading and code families', () => {
    expect(css).toContain('@media (prefers-color-scheme: dark)');
    expect(css).toMatch(/@media \(prefers-color-scheme: dark\) \{\s*:root:not\(\[data-theme="light"\]\) \{/);
    expect(palettes[0]['font-ui']).toContain('Geist Variable');
    expect(palettes[0]['font-mono']).toContain('Geist Mono Variable');
  });
  it('makes a chosen theme match the system one', () => {
    expect(palettes).toHaveLength(3);
    // Chosen Dark uses exactly the system dark values; chosen Light turns the system dark rule off, so it is the base palette.
    expect(palettes[2]).toEqual(palettes[1]);
    expect(css).toMatch(/:root\s*\{\s*color-scheme: light;/);
    expect(css).toMatch(/:root:not\(\[data-theme="light"\]\) \{\s*color-scheme: dark;/);
    expect(css).toMatch(/:root\[data-theme="dark"\] \{\s*color-scheme: dark;/);
    expect(css).not.toMatch(/:root\[data-theme="light"\]/);
  });
  it('uses one six-pixel radius and reserves colour for state', () => {
    expect(palettes[0]).toMatchObject({
      radius: '6px',
      'radius-control': 'var(--radius)',
      'radius-row': 'var(--radius)',
      'radius-card': 'var(--radius)',
      'radius-small': '4px',
      'height-control-small': '28px',
      'height-control': '34px',
      'height-control-large': '44px',
    });
    const designSystem = readFileSync('src/components/design-system.css', 'utf8');
    const shell = readFileSync('src/shell.css', 'utf8');
    const controls = readFileSync('src/components/controls.css', 'utf8');
    expect(designSystem).not.toContain('border-radius: 999px');
    expect(shell).toMatch(/\.tile-overview, \.tile-designer, \.tile-clusters\s*\{[^}]*color: var\(--color-text\);[^}]*background: var\(--color-icon-tile\)/s);
    expect(controls).toMatch(/\.button-danger\s*\{\s*color: var\(--color-status-failed\);/);
    expect(css).toMatch(/\.view-switch button\[aria-pressed="true"\]\s*\{[^}]*box-shadow: inset 0 0 0 1px var\(--color-border-strong\);/);
    expect(css).not.toMatch(/\.view-switch button\[aria-pressed="true"\]\s*\{[^}]*outline:/);
    expect(css).toMatch(/input:is\(\[type='radio'\], \[type='checkbox'\]\)\s*\{\s*accent-color: var\(--color-text\);/);
    expect(css).toMatch(/a\s*\{\s*color: var\(--color-text\);/);
    expect(css).not.toContain('color-violet');
  });
});
