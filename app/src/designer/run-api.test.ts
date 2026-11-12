import { expect, test } from 'vitest';
import { isRunSnapshot } from './run-api';
const base = { state: 'complete', canRun: true, report: { verdict: 'passed', cases: [{ id: 'one', question: 'What is the answer?', phase: 'Succeeded', answer: '42', completedTools: [], verdict: 'passed', assertions: [{ id: 'answer', kind: 'contains', expected: '42', verdict: 'passed' }] }] }, acknowledged: [], pending: null, error: null };
test('frozen evidence contract accepts long valid source fields and optional local stages', () => {
  expect(isRunSnapshot(base)).toBe(true); // Send reuses this six-key adapter.
  expect(isRunSnapshot({ ...base, stage: 'done', model: 'model' })).toBe(true);
  expect(isRunSnapshot({ ...base, report: { ...base.report, cases: [{ ...base.report.cases[0], question: 'q'.repeat(1048576), assertions: [{ ...base.report.cases[0].assertions[0], expected: 'e'.repeat(1048576) }] }] } })).toBe(true);
});
test('missing or malformed evidence and impossible local stages fail closed', () => {
  for (const question of [undefined, null, 2, '', 'q'.repeat(1048577)]) {
    expect(isRunSnapshot({ ...base, report: { ...base.report, cases: [{ ...base.report.cases[0], question }] } })).toBe(false);
  }
  for (const expected of [undefined, null, 2, '', 'e'.repeat(1048577)]) {
    expect(isRunSnapshot({ ...base, report: { ...base.report, cases: [{ ...base.report.cases[0], assertions: [{ ...base.report.cases[0].assertions[0], expected }] }] } })).toBe(false);
  }
  expect(isRunSnapshot({ ...base, stage: 'observe' })).toBe(false);
  expect(isRunSnapshot({ ...base, stage: null })).toBe(false);
  expect(isRunSnapshot({ ...base, stage: 'private-stage' })).toBe(false);
  expect(isRunSnapshot({ ...base, model: 4 })).toBe(false);
});
