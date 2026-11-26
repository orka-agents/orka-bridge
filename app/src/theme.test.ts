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
const rgb = (hex: string) => hex.slice(1).match(/../g)!.map(channel => parseInt(channel, 16));
const hex = (channels: number[]) => `#${channels.map(channel => Math.round(channel).toString(16).padStart(2, '0')).join('')}`;
const over = (top: string, alpha: number, bottom: string) => hex(rgb(top).map((channel, index) => channel * alpha + rgb(bottom)[index] * (1 - alpha)));
// The page artwork is palette PNGs of one ink at several strengths. Its darkest point on the page is the palette entry
// with the most opacity laid over the paper, read from the PLTE and tRNS chunks.
const strongestInk = (file: string) => {
  const png = readFileSync(file);
  let palette = Buffer.alloc(0), opacity = Buffer.alloc(0);
  for (let at = 8; at < png.length;) {
    const length = png.readUInt32BE(at), type = png.toString('ascii', at + 4, at + 8), data = png.subarray(at + 8, at + 8 + length);
    if (type === 'PLTE') palette = data;
    if (type === 'tRNS') opacity = data;
    at += 12 + length;
  }
  let strongest = 0;
  for (let index = 1; index < opacity.length; index++) if (opacity[index] > opacity[strongest]) strongest = index;
  return { ink: hex([...palette.subarray(strongest * 3, strongest * 3 + 3)]), alpha: opacity[strongest] / 255 };
};
const art = (index: number) => ['chart-faint', 'pod-faint'].map(name => {
  const { ink, alpha } = strongestInk(`src/assets/art/${name}${index ? '-dark' : ''}.png`);
  return { name, alpha, darkest: over(ink, alpha, palettes[index]['color-page']) };
});

describe('Orka ink and paper theme contract', () => {
  it.each([
    // The boards' bg, bar, well, line and state colours. Dark red is a step lighter than the boards' #f87171 so it reads
    // at 4.5:1 over the page artwork.
    ['light', 0, '#ffffff', '#ffffff', '#fafafa', '#f3f3f5', '#f3f3f5', '#d4d4d8', '#0b0b0c', '#b45309', '#0369a1', '#15803d', '#b91c1c'],
    ['dark', 1, '#09090a', '#09090a', '#121214', '#18181b', '#1b1b1e', '#353539', '#f4f4f5', '#f5b544', '#4cb8f5', '#4ade80', '#f97b7b'],
  ] as const)('%s uses the Orka neutrals and state colours', (_, index, page, card, bar, well, soft, line, ink, pending, running, succeeded, failed) => {
    expect(palettes[index]).toMatchObject({
      'color-page': page, 'color-card': card, 'color-bar': bar, 'color-well': well, 'color-chrome': well, 'color-hover': soft,
      'color-border-strong': line, 'color-text': ink, 'color-action': ink, 'color-status-pending': pending,
      'color-status-running': running, 'color-status-succeeded': succeeded, 'color-status-failed': failed,
    });
  });
  it.each([0, 1])('keeps every text role readable on shared surfaces in palette %s', index => {
    const palette = palettes[index];
    for (const ink of ['color-text-secondary', 'color-text-tertiary', 'color-status-pending', 'color-status-running', 'color-status-succeeded', 'color-status-failed']) {
      for (const surface of ['color-page', 'color-card', 'color-bar', 'color-well', 'color-chrome', 'color-hover']) {
        expect(contrast(palette[ink], palette[surface])).toBeGreaterThanOrEqual(4.5);
      }
    }
    expect(contrast(palette['color-action-text'], palette['color-action'])).toBeGreaterThanOrEqual(4.5);
    expect(contrast(palette['color-action-text'], palette['color-action-hover'])).toBeGreaterThanOrEqual(4.5);
    expect(contrast(palette['color-terminal-text'], palette['color-terminal'])).toBeGreaterThanOrEqual(4.5);
    // The focus ring is the running blue, so it shows against every surface a control sits on.
    expect(palette['color-focus']).toBe(palette['color-status-running']);
  });
  it.each([0, 1])('bakes the artwork at the boards\' strength and keeps page text readable over its darkest line in palette %s', index => {
    const [chart, pod] = art(index);
    expect(chart.alpha).toBeCloseTo(index ? .20 : .16, 2);
    expect(pod.alpha).toBeCloseTo(index ? .14 : .10, 2);
    const palette = palettes[index];
    for (const { darkest } of [chart, pod]) {
      // Text set straight on the page: ink, secondary and tertiary words, and red errors and danger actions.
      for (const ink of ['color-text', 'color-text-secondary', 'color-text-tertiary', 'color-status-failed']) {
        expect(contrast(palette[ink], darkest)).toBeGreaterThanOrEqual(4.5);
      }
      // Every state colour can be a pill on the page; a pill's paper backing (design-system.css) keeps it readable.
      const backing = over(palette['color-page'], .8, darkest);
      for (const ink of ['color-text-secondary', 'color-status-pending', 'color-status-running', 'color-status-succeeded', 'color-status-failed']) {
        expect(contrast(palette[ink], backing)).toBeGreaterThanOrEqual(4.5);
      }
    }
    const designSystem = readFileSync('src/components/design-system.css', 'utf8');
    expect(designSystem).toMatch(/\.pill\s*\{[^}]*background: color-mix\(in srgb, var\(--color-page\) 80%, transparent\)/);
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
  it('declares themed thin scrollbars and a flat page beside a sidebar of full-width page buttons', () => {
    const shell = readFileSync('src/shell.css', 'utf8');
    const controls = readFileSync('src/components/controls.css', 'utf8');
    expect(shell).toMatch(/scrollbar-width:\s*thin/);
    expect(shell).toMatch(/scrollbar-color:\s*var\(--color-text-tertiary\)\s+var\(--color-page\)/);
    expect(shell).toMatch(/\.app-shell\s*\{[^}]*--sidebar-width:\s*232px;[^}]*grid-template-areas:\s*"title title"\s*"nav main"\s*"status main";[^}]*background: var\(--color-bar\)/);
    // The frame is the bar colour with hairlines under the title bar and beside the sidebar; the page is flat paper.
    expect(shell).toMatch(/\.title-bar\s*\{[^}]*grid-template-columns:\s*minmax\(0, 1fr\) auto minmax\(0, 1fr\);[^}]*min-height:\s*44px;[^}]*border-bottom: 1px solid var\(--color-border-strong\);[^}]*background: var\(--color-bar\)/);
    expect(shell).toMatch(/\.page-content\s*\{[^}]*border-left: 1px solid var\(--color-border-strong\);[^}]*background: var\(--color-page\)/);
    expect(shell).not.toMatch(/\.page-content\s*\{[^}]*(box-shadow|border-radius|margin:)/);
    expect(shell).toMatch(/\.page-tab\s*\{[^}]*width:\s*100%[^}]*min-height: 44px/);
    expect(shell).toMatch(/\.page-tab\[aria-current="page"\]\s*\{\s*background: var\(--color-well\);/);
    expect(shell).toMatch(/\.status-bar\s*\{[^}]*border: var\(--border-control\) solid var\(--color-text\)/);
    expect(controls).toMatch(/\.button-quiet\s*\{[^}]*padding-inline:\s*0/);
  });
  it('folds the sidebar to a 60px rail that keeps names and status for assistive technology, wide by choice and narrow always', () => {
    const shell = readFileSync('src/shell.css', 'utf8');
    // The folded rail in wide windows and the narrow rail are the same rules for two selectors.
    const folded = shell.slice(shell.indexOf('@media (min-width: 821px)'), shell.indexOf('@media (max-width: 820px)'));
    const compact = shell.slice(shell.indexOf('@media (max-width: 820px)'), shell.indexOf('@media (max-width: 600px)'));
    const rail = compact.split('\n').filter(line => line.includes('.app-shell:not(.guided')).join('\n');
    expect(folded).toMatch(/\.app-shell\[data-sidebar="collapsed"\]\s*\{\s*--sidebar-width:\s*60px;\s*\}/);
    expect(rail).toMatch(/\.app-shell:not\(\.guided\)\s*\{\s*--sidebar-width:\s*60px;\s*\}/);
    for (const [block, shellSelector] of [[folded, '.app-shell[data-sidebar="collapsed"]'], [rail, '.app-shell:not(.guided, [data-sidebar="overlay"])']] as const) {
      const at = (rule: string) => block.split('\n').find(line => line.trimStart().startsWith(`${shellSelector} ${rule}`)) ?? '';
      expect(at(':is(.nav-group-label, .tab-copy, .host-label, .host-chip .chip, .status-lines, .checks-status)')).toMatch(/position: absolute;[^}]*clip-path: inset\(50%\)/);
      expect(at('.page-tab::after')).toMatch(/content: attr\(aria-label\)/);
      expect(at('.status-bar')).toMatch(/place-items: center/);
      expect(block).not.toMatch(/display:\s*none/);
    }
    // Narrow windows keep Bridge and the toggle, which opens the full sidebar over the page.
    expect(compact).not.toMatch(/\.sidebar-toggle-control\s*\{[^}]*display:\s*none/);
    expect(rail).not.toMatch(/\.brand b/);
    expect(compact).toMatch(/\.app-shell\[data-sidebar="overlay"\]:not\(\.guided\)::before\s*\{[^}]*width: 232px/);
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
    // Sidebar tiles are outlined, not filled, so they show on the bar in both themes.
    expect(shell).toMatch(/\.tile-overview, \.tile-designer, \.tile-clusters\s*\{[^}]*color: var\(--color-text\);[^}]*background: transparent;[^}]*border: var\(--border-control\) solid var\(--color-border-strong\)/s);
    expect(controls).toMatch(/\.button-danger\s*\{\s*color: var\(--color-status-failed\);/);
    expect(css).toMatch(/\.view-switch button\[aria-pressed="true"\]\s*\{[^}]*box-shadow: inset 0 0 0 1px var\(--color-border-strong\);/);
    expect(css).not.toMatch(/\.view-switch button\[aria-pressed="true"\]\s*\{[^}]*outline:/);
    expect(css).toMatch(/input:is\(\[type='radio'\], \[type='checkbox'\]\)\s*\{\s*accent-color: var\(--color-text\);/);
    expect(css).toMatch(/a\s*\{\s*color: var\(--color-text\);/);
    expect(css).not.toContain('color-violet');
  });
});
