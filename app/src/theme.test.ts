import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const css = readFileSync('src/app.css', 'utf8');
const palettes = [...css.matchAll(/:root\s*\{([^}]+)\}/g)].map(([, block]) =>
  Object.fromEntries([...block.matchAll(/--([\w-]+):\s*([^;]+);/g)].map(([, name, value]) => [name, value.trim()])),
);

describe('Tuxedo theme contract', () => {
  it.each([
    ['light', 0, '#FCFCFD', '#0B0B0C', '#0369A1', '#15803D'],
    ['dark', 1, '#0E0E10', '#F4F4F5', '#4CB8F5', '#4ADE80'],
  ] as const)('%s keeps neutral chrome and Orka phase meanings', (_, index, page, ink, running, succeeded) => {
    expect(palettes[index]).toMatchObject({
      'color-page': page, 'color-text': ink, 'color-action': ink,
      'color-status-running': running, 'color-status-succeeded': succeeded,
    });
  });
  it.each([0, 1])('keeps secondary, faint and failure text readable in palette %s', index => {
    const palette = palettes[index];
    const luminance = (hex: string) => {
      const rgb = hex.slice(1).match(/../g)!.map(channel => parseInt(channel, 16) / 255)
        .map(channel => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4);
      return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
    };
    for (const ink of ['color-text-secondary', 'color-text-tertiary', 'color-status-failed']) {
      for (const surface of ['color-page', 'color-card', 'color-chrome', 'color-hover']) {
        const [a, b] = [luminance(palette[ink]), luminance(palette[surface])].sort((a, b) => b - a);
        expect((a + 0.05) / (b + 0.05)).toBeGreaterThanOrEqual(4.5);
      }
    }
  });
  it('uses the system dark preference and bundled Geist families', () => {
    expect(css).toContain('@media (prefers-color-scheme: dark)');
    expect(palettes[0]['font-ui']).toContain('Geist Variable');
    expect(palettes[0]['font-mono']).toContain('Geist Mono Variable');
    expect(css).not.toContain('Estuary');
  });
});
