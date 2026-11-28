import { readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

// jsdom does not lay out controls; native fixture screenshots exercise these values.
const css = readFileSync('src/components/controls.css', 'utf8');
describe('shared control styles', () => {
  it('pins the three control sizes and preserves keyboard focus', () => {
    expect(readFileSync('src/app.css', 'utf8')).toMatch(/--height-control-small: 28px;\s*--height-control: 34px;\s*--height-control-large: 44px;/);
    expect(css).toMatch(/\.button\s*\{[^}]*min-height: var\(--height-control\);[^}]*white-space: nowrap/s);
    expect(css).toMatch(/\.button-small\s*\{[^}]*min-height: var\(--height-control-small\)/s);
    expect(css).toMatch(/\.button-large\s*\{[^}]*min-height: var\(--height-control-large\)/s);
    expect(css).toMatch(/\.button-icon\s*\{[^}]*width: 28px/s);
    expect(readFileSync('src/app.css', 'utf8')).toContain(':focus-visible { outline: 2px solid var(--color-focus); outline-offset: 2px; }');
    expect(css).toMatch(/h1\[tabindex="-1"\]:focus\s*\{\s*outline: none;/);
  });
  it('shares theme-aware field chrome and draws a native select chevron', () => {
    expect(css).toMatch(/\.field\s*\{[^}]*background: var\(--color-card\)/s);
    expect(css).toMatch(/select\.field\s*\{[^}]*appearance: none/s);
    expect(css).toMatch(/input\.field, select\.field\s*\{\s*height: var\(--height-control\);/);
    expect(css).toMatch(/\.field::placeholder\s*\{[^}]*color: var\(--color-text-secondary\);[^}]*opacity: 1;/);
    expect(readFileSync('src/app.css', 'utf8')).toMatch(/\.tool-picker:popover-open\s*\{[^}]*height: fit-content;[^}]*align-content: start;/s);
    // The chevron is an SVG file per theme colour: the CSP allows images from Bridge only, never data: URIs.
    expect(css).toMatch(/select\.field\s*\{[^}]*background-image: var\(--chevron\);[^}]*background-size: 11px 11px/s);
    for (const file of ['ink', 'secondary', 'ink-dark', 'secondary-dark']) expect(css).toContain(`url('../assets/chevron/${file}.svg')`);
    expect(css).not.toContain('data:');
    expect(css).toContain('var(--color-text-secondary)');
    expect(css).not.toContain('.agent-editor');
    expect(readFileSync('src/app.css', 'utf8')).toMatch(/\.field\.model-select\s*\{\s*min-width: 12rem/);
  });
  it('shows focus on an ink edge instead of a second ring, and never removes a ring without its replacement', () => {
    const edge = 'outline: calc(var(--border-control) + 1px) solid var(--color-focus); outline-offset: calc(-1 * var(--border-control));';
    expect(css).toContain(`.field:focus-visible, .card:focus-visible { ${edge} }`);
    expect(css).toContain(`.status-bar:has(.host-chip:focus-visible) { ${edge} }`);
    expect(readFileSync('src/designer/designer.css', 'utf8')).toContain(`.agent-editor:focus-visible { ${edge} }`);
    expect(css).toContain(':root:not([data-focus="pointer"]) :is(.nav-composer, .try-composer):has(textarea:focus-visible) { outline: calc(var(--border-editing) + 1px) solid var(--color-focus); outline-offset: calc(-1 * var(--border-editing)); }');
    // A click into a text field keeps a quiet ink edge rather than no indicator.
    expect(css).toMatch(/:root\[data-focus="pointer"\] :is\(input, textarea\)\.field:focus-visible \{ outline: var\(--border-editing\) solid var\(--color-text\);/);
    // The detached ring with a page-coloured gap around the message box is gone.
    expect(readFileSync('src/home/home.css', 'utf8')).not.toMatch(/box-shadow:[^;]*--color-focus|outline: 0/);
    // Every rule that removes an outline, with the rule that draws its replacement or why none is drawn.
    const removals = readdirSync('src', { recursive: true }).filter(file => String(file).endsWith('.css')).sort()
      .flatMap(file => [...readFileSync(`src/${file}`, 'utf8').matchAll(/([^{}]+)\{[^}]*outline: (?:none|0)\b/g)].map(([, selector]) => `${file}: ${selector.replace(/\/\*[\s\S]*?\*\//g, '').trim()}`));
    expect(removals).toEqual([
      'chrome/switcher.css: .switcher-filter input:focus-visible', // the filter's underline turns ink, or the focus blue from the keyboard
      'components/controls.css: :root[data-focus="pointer"] select.field option:focus-visible', // the hover fill marks the row, in the same rule
      'components/controls.css: :is(.nav-composer, .try-composer) textarea:focus-visible', // the card's edge turns blue
      'components/controls.css: .status-bar .host-chip:focus-visible', // the status card's edge turns blue
      'components/controls.css: h1[tabindex="-1"]:focus', // a heading focused on navigation shows no ring (DESIGN.md)
      'setup/steps.css: .workspace-upgrade > p[role="status"]:first-child:focus', // a status line focused for announcement
    ]);
    const pairs = { 'chrome/switcher.css': ':root:not([data-focus="pointer"]) .switcher-filter:has(input:focus-visible) { border-bottom-color: var(--color-focus);' };
    for (const [file, replacement] of Object.entries(pairs)) expect(readFileSync(`src/${file}`, 'utf8')).toContain(replacement);
  });
  it('draws the select list as a Bridge popover where the engine allows it, and keeps the system list elsewhere', () => {
    const block = css.match(/@supports \(appearance: base-select\) \{([\s\S]*?)\n\}/)?.[1] ?? '';
    expect(block).toContain('select.field, select.field::picker(select) { appearance: base-select; }');
    expect(block).toMatch(/select\.field::picker\(select\) \{[^}]*border: 1px solid var\(--color-border-strong\);[^}]*border-radius: var\(--radius-overlay\);[^}]*box-shadow: var\(--shadow-popover\);/s);
    expect(block).toMatch(/max-width: calc\(100vw - 32px\);/);
    expect(block).toMatch(/select\.field option \{[^}]*min-height: 34px;[^}]*white-space: normal;/);
    expect(block).toContain('select.field option:checked { background: var(--color-well);');
    // High contrast keeps the system control, after the enhancement so it wins.
    expect(css.indexOf('@media (forced-colors: active) { select.field { appearance: auto;')).toBeGreaterThan(css.indexOf('@supports (appearance: base-select)'));
  });
  it('removes the six competing action style families', () => {
    for (const path of ['app.css', 'shell.css', 'setup/welcome.css', 'setup/steps.css', 'components/design-system.css']) {
      expect(readFileSync(`src/${path}`, 'utf8')).not.toMatch(/\.(?:shell-button|setup-button|part-button|workspace-primary|workspace-secondary|workspace-link)\b/);
    }
  });
});
