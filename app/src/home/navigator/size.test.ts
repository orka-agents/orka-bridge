import { expect, test } from 'vitest';
import instructions from './instructions.md?raw';
import digest from './digest.md?raw';

// The Navigator's Agent is one create request over the workspace transport, whose body may not exceed 64 KiB
// (runtime/protocol MAX_ORKA_BODY). navigator.rs allows 32 saved-agent lines of at most 240 bytes each.
const MAX_ORKA_BODY = 64 * 1024;
const encoded = (text: string) => new TextEncoder().encode(text).length;

test('the instructions, the digest and the largest saved-agent list fit one Agent create request', () => {
  const saved = Array.from({ length: 32 }, () => `- ${'a'.repeat(63)}: ${'"é '.repeat(58)}`.slice(0, 240)).join('\n');
  const prompt = `\n${instructions}\n\n# Saved agents on this computer\n\n${saved}\n\n${digest}`;
  const body = { name: `bridge-nav-agent-${'0'.repeat(32)}`, namespace: 'orka-system', spec: { providerRef: { name: 'p'.repeat(45) }, model: { name: 'm'.repeat(256) }, systemPrompt: { inline: prompt }, tools: [] } };
  const size = encoded(JSON.stringify(body));
  expect(size).toBeLessThanOrEqual(MAX_ORKA_BODY);
  // Leave room to grow the docs without a surprise: the digest alone stays under 40 KB.
  expect(encoded(digest)).toBeLessThan(40_000);
});

test('every digest section names where it came from', () => {
  const sections = digest.split(/^## /m).slice(1);
  expect(sections.length).toBeGreaterThan(10);
  for (const section of sections) expect(section.split('\n')[1]).toMatch(/^Source: (?:bridge|orka):[\w./-]+\.md(?:#[a-z0-9-]+)?$/);
});

test('the instructions describe exactly the card kinds Bridge draws', () => {
  const kinds = [...instructions.matchAll(/"kind":"(\w+)"/g)].map(match => match[1]);
  expect(kinds).toEqual(['task', 'agent', 'explain']);
  expect(instructions).toContain('```navigator-card');
});
