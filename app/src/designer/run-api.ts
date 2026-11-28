import { invoke, isTauri } from '@tauri-apps/api/core';
import { unsupportedToolMessage } from './built-in-tools';
import { type DocumentRef, isDocumentId, isDocumentRef, isSourceId, SAMPLE_DOCUMENT_ID, type SourceId } from './agent-api';
export type Verdict = 'passed' | 'failed' | 'unknown' | 'cancelled';
type Assertion = { id: string; kind: 'contains' | 'toolCalled'; expected: string; verdict: Exclude<Verdict, 'cancelled'> };
type Case = { id: string; title?: string; question: string; phase: string; answer: string | null; completedTools: string[]; assertions: Assertion[]; verdict: Verdict };
type Receipt = { kind: 'Agent' | 'Task'; name: string; uid: string };
export const errors = {
  session: 'The running local VM is not available for Run.',
  unsupportedTool: unsupportedToolMessage,
  source: 'Check the format of your saved agent and check files.',
  model: 'Connect the local VM model in Clusters before running the example.',
  submit: 'Submission was not confirmed. A named resource may still exist; it was not retried.',
  observe: 'Observation did not finish. The acknowledged Task may still be running.',
} as const;
export type RunSnapshot = { attemptId?: string | null; documentId?: string; busy?: DocumentRef | null; state: 'idle' | 'running' | 'complete' | 'unknown'; canRun: boolean; modelConnected?: boolean; stage?: 'capture' | 'model' | 'submit' | 'observe' | 'cancel' | 'done' | null; canCancel?: boolean; canRetryCancellation?: boolean; canCheckCancellation?: boolean; cancellation?: 'none' | 'cancelling' | 'cancelled' | 'unconfirmed' | 'beforeStart'; model?: string | null; vendor?: 'openai' | 'anthropic' | null; source?: SourceId | null; sourceCurrent?: boolean; report: { cases: Case[]; verdict: Verdict } | null; acknowledged: Receipt[]; pending: Omit<Receipt, 'uid'> | null; error: keyof typeof errors | null };
function record(value: unknown, keys: string[]): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value) && Object.keys(value).sort().join(',') === keys.sort().join(',');
}
const text = (value: unknown, max = 4096): value is string => typeof value === 'string' && value.length > 0 && value.length <= max;
const verdict = (value: unknown): value is Verdict => value === 'passed' || value === 'failed' || value === 'unknown' || value === 'cancelled';
// A job run (job_run_start) reports through the same snapshot, with its stages, answer and review in `job`.
// Its stage reports grade the drafter's check and the reviewer's format, never the job, so native never sends them.
export type JobReview = { verdict: 'done' | 'partly' | 'notDone'; summary: string; gaps: string[] };
export type JobOutcome = { stage: 'draft' | 'run' | 'review'; instructions: string | null; tools: string[]; answer: string | null; used: string[]; review: JobReview | null; startedAt: number; finishedAt: number | null };
const jobErrors = ['draft', 'answer', 'review'] as const;
export type JobRunSnapshot = Omit<RunSnapshot, 'report' | 'error'> & { report: null; error: keyof typeof errors | typeof jobErrors[number] | null; job: JobOutcome };
const time = (value: unknown) => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
function isJobOutcome(value: unknown): value is JobOutcome {
  if (!record(value, ['stage', 'instructions', 'tools', 'answer', 'used', 'review', 'startedAt', 'finishedAt'])) return false;
  const review = value.review;
  return ['draft', 'run', 'review'].includes(value.stage as string) && (value.instructions === null || text(value.instructions)) && (value.answer === null || text(value.answer, 1048576))
    && Array.isArray(value.tools) && value.tools.every(t => t === 'web_fetch') && value.tools.length <= 1 && Array.isArray(value.used) && value.used.length <= 6000 && value.used.every(t => text(t))
    && time(value.startedAt) && (value.finishedAt === null || time(value.finishedAt))
    && (review === null || record(review, ['verdict', 'summary', 'gaps']) && ['done', 'partly', 'notDone'].includes(review.verdict as string) && text(review.summary, 500) && Array.isArray(review.gaps) && review.gaps.length <= 5 && review.gaps.every(g => text(g, 300)));
}
// While a job runs, every snapshot names it as the busy document, under native's fixed name rather than an agent's.
const isJobRef = (value: unknown) => record(value, ['documentId', 'name']) && isDocumentId(value.documentId) && value.name === 'Your job';
export const isRunSnapshot = (value: unknown): value is RunSnapshot => snapshot(value, false);
export const isJobRunSnapshot = (value: unknown): value is JobRunSnapshot => snapshot(value, true);
function snapshot(value: unknown, job: boolean) {
  const keys = ['state', 'canRun', 'report', 'acknowledged', 'pending', 'error', ...(job ? ['job'] : [])];
  if (typeof value === 'object' && value !== null) {
    if ('documentId' in value) keys.push('documentId');
    if ('attemptId' in value) keys.push('attemptId');
    if ('busy' in value) keys.push('busy');
    if ('source' in value) keys.push('source');
    if ('modelConnected' in value) keys.push('modelConnected');
    if ('stage' in value) keys.push('stage');
    if ('model' in value) keys.push('model');
    if ('vendor' in value) keys.push('vendor');
    for (const key of ['canCancel', 'canRetryCancellation', 'canCheckCancellation', 'cancellation']) if (key in value) keys.push(key);
  }
  if (!record(value, keys) || ('documentId' in value && !isDocumentId(value.documentId)) || ('busy' in value && value.busy !== null && !isDocumentRef(value.busy) && !isJobRef(value.busy)) || ('modelConnected' in value && typeof value.modelConnected !== 'boolean') || ('source' in value && value.source !== null && !isSourceId(value.source)) || typeof value.state !== 'string' || !['idle', 'running', 'complete', 'unknown'].includes(value.state) || typeof value.canRun !== 'boolean' || value.state === 'running' && value.canRun || value.error !== null && !(typeof value.error === 'string' && (Object.hasOwn(errors, value.error) || job && jobErrors.some(e => e === value.error)))) return false;
  if (job && !(isJobOutcome(value.job) && value.report === null)) return false;
  if ('attemptId' in value && value.attemptId !== null && !isDocumentId(value.attemptId)) return false;
  for (const key of ['canCancel', 'canRetryCancellation', 'canCheckCancellation']) if (key in value && typeof value[key] !== 'boolean') return false;
  if ('cancellation' in value && !['none', 'cancelling', 'cancelled', 'unconfirmed', 'beforeStart'].includes(value.cancellation as string)) return false;
  if ('stage' in value && !(value.state === 'idle' ? value.stage === null : value.state === 'running' ? ['capture', 'model', 'submit', 'observe', 'cancel'].includes(value.stage as string) : value.stage === 'done')) return false;
  if ('model' in value && value.model !== null && !text(value.model, 1048576)) return false;
  if ('vendor' in value && value.vendor !== null && value.vendor !== 'openai' && value.vendor !== 'anthropic') return false;
  if (!Array.isArray(value.acknowledged) || value.acknowledged.length > 33 || !value.acknowledged.every(r => record(r, ['kind', 'name', 'uid']) && (r.kind === 'Agent' || r.kind === 'Task') && text(r.name) && text(r.uid))) return false;
  if (value.pending !== null && !(record(value.pending, ['kind', 'name']) && (value.pending.kind === 'Agent' || value.pending.kind === 'Task') && text(value.pending.name))) return false;
  if (value.report === null) return job || value.state !== 'complete';
  const report = value.report;
  return record(report, ['cases', 'verdict']) && verdict(report.verdict) && Array.isArray(report.cases) && (report.cases.length > 0 || report.verdict === 'cancelled' && (value.cancellation === 'beforeStart' || value.state === 'running' && value.cancellation === 'cancelling') || report.verdict === 'unknown' && (value.cancellation === 'unconfirmed' || value.state === 'running' && value.cancellation === 'cancelling')) && report.cases.length <= 32 && report.cases.every(c =>
    record(c, ['id', 'question', 'phase', 'answer', 'completedTools', 'assertions', 'verdict', ...(typeof c === 'object' && c !== null && 'title' in c ? ['title'] : [])]) && (!('title' in c) || typeof c.title === 'string' && c.title.length <= 1048576) && text(c.id) && text(c.question, 1048576) && text(c.phase) && verdict(c.verdict) && (c.answer === null || typeof c.answer === 'string' && c.answer.length <= 1048576)
    && Array.isArray(c.completedTools) && c.completedTools.length <= 6000 && c.completedTools.every(t => text(t))
    && Array.isArray(c.assertions) && (c.verdict === 'cancelled' ? c.phase === 'Cancelled' && c.assertions.length === 0 : c.assertions.length > 0) && c.assertions.length <= 64 && c.assertions.every(a => record(a, ['id', 'kind', 'expected', 'verdict']) && text(a.id) && text(a.expected, 1048576) && (a.kind === 'contains' || a.kind === 'toolCalled') && verdict(a.verdict) && a.verdict !== 'cancelled'));
}
export function sampleRun(snapshot?: RunSnapshot): RunSnapshot | undefined {
  return snapshot?.documentId === SAMPLE_DOCUMENT_ID ? snapshot : undefined;
}
export async function runCall(command: 'agent_run_status' | 'agent_run' | 'agent_run_cancel' | 'agent_run_check_cancellation', documentId = SAMPLE_DOCUMENT_ID, bundledExample = false, attemptId?: string): Promise<{ ok: true; snapshot: RunSnapshot } | { ok: false; message: string }> {
  if (!isTauri()) return { ok: false, message: 'Run is available only in the native app.' };
  try { const value = await invoke<unknown>(command, { documentId, ...(bundledExample ? { bundledExample: true, ...(attemptId ? { attemptId } : {}) } : {}) }); if (isRunSnapshot(value) && value.documentId === documentId && 'busy' in value) return { ok: true, snapshot: value }; } catch { /* Never expose native errors or diagnostics. */ }
  return { ok: false, message: 'Run information is not supported by this build.' };
}
const jobRefusals = ['Connect the AI before running the job.', 'The job is invalid.'];
// Starts a saved job's run, or reads, stops or checks it by the job's id. No screen starts one yet.
export async function jobCall(command: 'job_run_start' | 'agent_run_status' | 'agent_run_cancel' | 'agent_run_check_cancellation', id: string, prompt = ''): Promise<{ ok: true; snapshot: JobRunSnapshot } | { ok: false; message: string }> {
  if (!isTauri()) return { ok: false, message: 'Run is available only in the native app.' };
  try {
    const value = await invoke<unknown>(command, command === 'job_run_start' ? { id, prompt } : { documentId: id });
    if (isJobRunSnapshot(value) && value.documentId === id && 'busy' in value) return { ok: true, snapshot: value };
  } catch (error) {
    // Only native's fixed refusals are shown; anything else stays generic.
    if (typeof error === 'string' && jobRefusals.includes(error)) return { ok: false, message: error };
  }
  return { ok: false, message: 'Run information is not supported by this build.' };
}
