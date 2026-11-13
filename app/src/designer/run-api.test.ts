import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, expect, test, vi } from 'vitest';
import { SAMPLE_DOCUMENT_ID } from './agent-api';
import { isRunSnapshot, runCall } from './run-api';
afterEach(() => { clearMocks(); vi.unstubAllGlobals(); });
const base = { state: 'complete', canRun: true, report: { verdict: 'passed', cases: [{ id: 'one', question: 'What is the answer?', phase: 'Succeeded', answer: '42', completedTools: [], verdict: 'passed', assertions: [{ id: 'answer', kind: 'contains', expected: '42', verdict: 'passed' }] }] }, acknowledged: [], pending: null, error: null };
test('only automatic first-test admission opts into the bundled-source check', async () => {
  vi.stubGlobal('isTauri', true);
  const ipc = vi.fn(() => ({ ...base, documentId: SAMPLE_DOCUMENT_ID, busy: null }));
  mockIPC(ipc);
  expect((await runCall('agent_run')).ok).toBe(true);
  expect((await runCall('agent_run', SAMPLE_DOCUMENT_ID, true)).ok).toBe(true);
  expect(ipc.mock.calls).toEqual([
    ['agent_run', { documentId: SAMPLE_DOCUMENT_ID }],
    ['agent_run', { documentId: SAMPLE_DOCUMENT_ID, bundledExample: true }],
  ]);
});
test('frozen evidence contract accepts long valid source fields and optional local stages', () => {
  expect(isRunSnapshot(base)).toBe(true); // Send reuses this six-key adapter.
  expect(isRunSnapshot({ ...base, stage: 'done', model: 'model' })).toBe(true);
  expect(isRunSnapshot({ ...base, stage: 'done', model: 'm'.repeat(1048576) })).toBe(true);
  expect(isRunSnapshot({ ...base, stage: 'done', model: 'm'.repeat(1048577) })).toBe(false);
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
