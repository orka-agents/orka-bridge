import { expect, test } from 'vitest';
import instructions from './navigator/instructions.md?raw';
import { digestSources, parseReply, validCard } from './cards';

const block = (value: unknown) => '```navigator-card\n' + JSON.stringify(value) + '\n```';
const task = { kind: 'task', agent: 'web-reader', task: 'Read https://example.com.' };

test('every card the instructions show the model is one Bridge draws', () => {
  const examples = [...instructions.matchAll(/```navigator-card\n([\s\S]*?)\n```/g)].map(match => JSON.parse(match[1]));
  expect(examples.map(example => example.kind)).toEqual(['task', 'agent', 'explain']);
  for (const example of examples) expect(validCard(example)).toBe(true);
});

test('one valid block becomes a card and leaves the prose', () => {
  const reply = parseReply(`That needs the live page.\n\n${block(task)}`);
  expect(reply).toEqual({ prose: 'That needs the live page.', card: task, sources: [] });
  expect(parseReply(`Draft:\n${block({ kind: 'explain', title: 'Terms', rows: [{ term: 'A', text: 'B' }, { term: 'C', text: 'D' }] })}`).card)
    .toEqual({ kind: 'explain', title: 'Terms', rows: [{ term: 'A', text: 'B' }, { term: 'C', text: 'D' }], actions: [] });
});

test('unknown kinds, unknown or missing fields and bad values stay plain text', () => {
  for (const value of [
    { ...task, kind: 'monitor' }, { ...task, extra: true }, { kind: 'task', agent: 'web-reader' }, { ...task, agent: 'Web Reader' },
    { ...task, task: ' ' }, { kind: 'agent', name: 'a', description: 'd', instructions: 'i', tools: ['request_approval'] },
    { kind: 'agent', name: 'a', description: 'two\nlines', instructions: 'i', tools: [] },
    { kind: 'agent', name: 'a', description: 'd', instructions: 'i', tools: ['web_fetch', 'web_fetch'] },
    { kind: 'explain', title: 'T', rows: [{ term: 'A', text: 'B' }] },
    { kind: 'explain', title: 'T', rows: [{ term: 'A', text: 'B' }, { term: 'C', text: 'D' }], actions: [{ label: 'Go', open: 'Settings' }] },
    { kind: 'explain', title: 'T', rows: [{ term: 'A', text: 'B' }, { term: 'C', text: 'D' }], actions: [{ label: 'Go', ask: 'x', open: 'Overview' }] },
    [task], 'task', null,
  ]) {
    const answer = `Here.\n\n${block(value)}`;
    expect(parseReply(answer)).toEqual({ prose: answer, card: undefined, sources: [] });
  }
  const broken = 'Here.\n\n```navigator-card\n{"kind":"task",\n```';
  expect(parseReply(broken).card).toBeUndefined();
});

test('two proposals in one reply are both shown as written', () => {
  const answer = `${block(task)}\n\n${block({ ...task, agent: 'other' })}`;
  expect(parseReply(answer)).toEqual({ prose: answer, card: undefined, sources: [] });
});

test('a Sources line is kept only when every page is in the bundled digest', () => {
  const [first, second] = [...digestSources];
  expect(parseReply(`An answer.\n\nSources: ${first}, \`${second}\``)).toEqual({ prose: 'An answer.', card: undefined, sources: [first, second] });
  const invented = `An answer.\n\nSources: ${first}, orka:secret.md`;
  expect(parseReply(invented)).toEqual({ prose: invented, card: undefined, sources: [] });
});
