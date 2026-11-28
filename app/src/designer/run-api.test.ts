import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, expect, test, vi } from 'vitest';
import { SAMPLE_DOCUMENT_ID } from './agent-api';
import { isJobRunSnapshot, isRunSnapshot, jobCall, runCall } from './run-api';
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
test('optional native attempt correlation is validated without breaking older snapshots', () => {
  expect(isRunSnapshot({ ...base, attemptId: SAMPLE_DOCUMENT_ID })).toBe(true);
  expect(isRunSnapshot({ ...base, attemptId: null })).toBe(true);
  expect(isRunSnapshot({ ...base, attemptId: 'not-an-attempt' })).toBe(false);
  expect(isRunSnapshot(base)).toBe(true);
});
test('optional title metadata cannot hide valid run evidence and older reports remain readable', () => {
  const withTitle = (title: unknown) => ({ ...base, report: { ...base.report, cases: [{ ...base.report.cases[0], title }] } });
  expect(isRunSnapshot(withTitle('Says the answer'))).toBe(true);
  for (const title of ['', 'a\nline', '🙂'.repeat(161), 'gh' + 'p_' + 'A'.repeat(30)]) expect(isRunSnapshot(withTitle(title))).toBe(true);
  for (const title of [null, 42, 'a'.repeat(1048577)]) expect(isRunSnapshot(withTitle(title))).toBe(false);
  expect(isRunSnapshot(base)).toBe(true);
});
test('frozen evidence contract accepts long valid source fields and optional local stages', () => {
  expect(isRunSnapshot(base)).toBe(true); // Send reuses this six-key adapter.
  expect(isRunSnapshot({ ...base, stage: 'done', model: 'model' })).toBe(true);
  expect(isRunSnapshot({ ...base, stage: 'done', model: 'm'.repeat(1048576) })).toBe(true);
  expect(isRunSnapshot({ ...base, stage: 'done', model: 'm'.repeat(1048577) })).toBe(false);
  expect(isRunSnapshot({ ...base, report: { ...base.report, cases: [{ ...base.report.cases[0], question: 'q'.repeat(1048576), assertions: [{ ...base.report.cases[0].assertions[0], expected: 'e'.repeat(1048576) }] }] } })).toBe(true);
});
test('native cancellation metadata does not make ordinary Run snapshots unreadable', () => {
  const metadata = { canCancel: false, canRetryCancellation: false, canCheckCancellation: false, cancellation: 'none' };
  expect(isRunSnapshot({ ...base, ...metadata })).toBe(true);
  expect(isRunSnapshot({ ...base, ...metadata, state: 'running', canRun: false, stage: 'cancel', cancellation: 'cancelling' })).toBe(true);
  for (const key of ['canCancel', 'canRetryCancellation', 'canCheckCancellation']) {
    expect(isRunSnapshot({ ...base, ...metadata, [key]: 'yes' })).toBe(false);
  }
  expect(isRunSnapshot({ ...base, ...metadata, cancellation: 'untrusted' })).toBe(false);
});
test('shared cancelled results remain readable without fabricating failed assertions', () => {
  const cancelled = { ...base.report.cases[0], phase: 'Cancelled', verdict: 'cancelled', assertions: [] };
  expect(isRunSnapshot({ ...base, report: { verdict: 'cancelled', cases: [cancelled] } })).toBe(true);
  expect(isRunSnapshot({ ...base, report: { verdict: 'cancelled', cases: [{ ...cancelled, phase: 'Succeeded' }] } })).toBe(false);
  expect(isRunSnapshot({ ...base, report: { verdict: 'passed', cases: [{ ...base.report.cases[0], assertions: [] }] } })).toBe(false);
  expect(isRunSnapshot({ ...base, cancellation: 'beforeStart', report: { verdict: 'cancelled', cases: [] } })).toBe(true);
  expect(isRunSnapshot({ ...base, cancellation: 'unconfirmed', report: { verdict: 'unknown', cases: [] } })).toBe(true);
  expect(isRunSnapshot({ ...base, report: { verdict: 'passed', cases: [] } })).toBe(false);
});
test('an active cancellation recheck can retain an empty unknown report', () => {
  const recovering = { ...base, state: 'running', canRun: false, cancellation: 'cancelling', report: { verdict: 'unknown', cases: [] } };
  expect(isRunSnapshot(recovering)).toBe(true);
  expect(isRunSnapshot({ ...recovering, report: { verdict: 'cancelled', cases: [] } })).toBe(true);
  expect(isRunSnapshot({ ...recovering, state: 'complete', report: { verdict: 'cancelled', cases: [] } })).toBe(false);
  expect(isRunSnapshot({ ...recovering, state: 'complete' })).toBe(false);
  expect(isRunSnapshot({ ...recovering, cancellation: 'none' })).toBe(false);
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

// Shapes from the native job tests: the job's result is only in `job`, never a stage report.
const jobId = '5f0c2a1e-7b3d-4c8e-9a61-2d4f8b0e6c13';
const outcome = { stage: 'review', instructions: 'Fetch the page and reply with its title only.', tools: ['web_fetch'], answer: 'The title is Example Domain.', used: ['web_fetch'], review: { verdict: 'done', summary: 'It gives the page title.', gaps: [] }, startedAt: 1, finishedAt: 2 };
const job = { documentId: jobId, busy: null, state: 'complete', stage: 'done', canRun: true, report: null, acknowledged: [], pending: null, error: null, job: outcome };
test('a job snapshot carries its outcome, never a report, and only a job may end with a job error', () => {
  expect(isJobRunSnapshot(job)).toBe(true);
  for (const error of ['draft', 'answer', 'review', 'model']) expect(isJobRunSnapshot({ ...job, error, job: { ...outcome, review: null } })).toBe(true);
  expect(isRunSnapshot({ ...base, error: 'draft' })).toBe(false);
  expect(isRunSnapshot(job)).toBe(false);
  expect(isJobRunSnapshot({ ...job, report: base.report })).toBe(false);
  expect(isJobRunSnapshot({ ...job, job: undefined })).toBe(false);
  for (const change of [{ stage: 'check' }, { tools: ['code_exec'] }, { review: { verdict: 'passed', summary: 'x', gaps: [] } }, { review: { ...outcome.review, gaps: Array(6).fill('gap') } }, { extra: 1 }, { finishedAt: -1 }]) {
    expect(isJobRunSnapshot({ ...job, job: { ...outcome, ...change } })).toBe(false);
  }
});
test('an agent snapshot stays readable while a job holds the workspace under its fixed name', () => {
  expect(isRunSnapshot({ ...base, busy: { documentId: jobId, name: 'Your job' } })).toBe(true);
  expect(isRunSnapshot({ ...base, busy: { documentId: jobId, name: 'Someone else' } })).toBe(false);
});
test('a job starts with its id and sentence, and only native\'s fixed refusals are shown', async () => {
  vi.stubGlobal('isTauri', true);
  const ipc = vi.fn((command: string) => command === 'job_run_start' ? job : Promise.reject('diagnostic detail'));
  mockIPC(ipc);
  expect(await jobCall('job_run_start', jobId, 'Tell me the title')).toEqual({ ok: true, snapshot: job });
  expect(ipc.mock.calls[0]).toEqual(['job_run_start', { id: jobId, prompt: 'Tell me the title' }]);
  expect(await jobCall('agent_run_status', jobId)).toEqual({ ok: false, message: 'Run information is not supported by this build.' });
  mockIPC(() => Promise.reject('Connect the AI before running the job.'));
  expect(await jobCall('job_run_start', jobId, 'Tell me the title')).toEqual({ ok: false, message: 'Connect the AI before running the job.' });
});
