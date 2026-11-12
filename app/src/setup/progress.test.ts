import { expect, test } from 'vitest';
import { canQueueModel, infraProgress, setupView, startupEstimateMs } from './progress';
import type { CheckStepState } from './Steps';
import type { SetupStatus, WorkspaceProgress } from './state';

const status = (localVm: SetupStatus['localVm'], extra: Partial<SetupStatus> = {}): SetupStatus =>
  ({ selected: 'localVm', localVm, model: localVm === 'ready' ? 'available' : 'locked', ...extra });
const transfer = (bytes: number | null, totalBytes: number | null, remainingMs: number | null = null): WorkspaceProgress =>
  ({ phase: 'materializing', bytes, totalBytes, elapsedMs: 1000, remainingMs });
const starting = (elapsedMs: number): WorkspaceProgress => ({ phase: 'starting', bytes: null, totalBytes: null, elapsedMs, remainingMs: null });

test('one derivation maps native and model state to the five setup steps', () => {
  const step = (s: SetupStatus, model: Parameters<typeof setupView>[1] = {}) => setupView(s, model).line;
  expect(step(status('preparing'))).toBe('Setting up, step 1 of 5');
  expect(step(status('installing'))).toBe('Setting up, step 1 of 5');
  expect(step(status('starting'))).toBe('Setting up, step 2 of 5');
  expect(step(status('ready'), { model: 'idle' })).toBe('Setting up, step 4 of 5');
  expect(step(status('ready'), { model: 'queued' })).toBe('Setting up, step 4 of 5');
  expect(step(status('ready'), { model: 'connecting' })).toBe('Setting up, step 4 of 5');
  expect(step(status('ready'), { model: 'connected' })).toBe('Setting up, step 5 of 5');
  expect(step(status('ready'), { model: 'connected', check: 'running' })).toBe('Setting up, step 5 of 5');
  expect(setupView(status('ready'), { model: 'connected' }).step).toBe(5);
});

test('the place choice shows no step count', () => {
  for (const s of [status('idle', { selected: null }), status('failed', { selected: null })]) {
    const view = setupView(s);
    expect(view).toMatchObject({ place: 'none', step: 0, line: 'Choose where agents run', detail: 'Choose where agents run' });
    expect(view.line).not.toMatch(/step/i);
  }
});

test('host selection is native: local only when selected, never inferred while checking or unreadable', () => {
  expect(setupView(status('installing')).place).toBe('local');
  expect(setupView(status('idle', { selected: null })).place).toBe('none');
  expect(setupView(undefined, { checking: true })).toMatchObject({ place: 'checking', line: 'Checking workspace…' });
  expect(setupView(undefined, { checking: false })).toMatchObject({ place: 'unavailable', line: 'Workspace unavailable' });
});

test('completion needs a current first test result, never stale, running or unknown ones', () => {
  const done = (check?: CheckStepState) => setupView(status('ready'), { model: 'connected', check });
  expect(done('passed')).toMatchObject({ done: true, line: 'This computer ready' });
  expect(done('failed').done).toBe(true);
  for (const check of ['running', 'stale', 'unknown', undefined] as const) expect(done(check)).toMatchObject({ done: false, step: 5 });
});

test('model and workspace trouble keep their own words and tone', () => {
  expect(setupView(status('ready'), { model: 'failed' })).toMatchObject({ line: 'AI needs attention', tone: 'bad', step: 4 });
  expect(setupView(status('ready', { model: 'locked' }), { model: 'connected' })).toMatchObject({ line: 'AI unavailable' });
  expect(setupView(status('failed'))).toMatchObject({ line: 'Setup failed', tone: 'bad' });
  expect(setupView(status('failed', { workspaceFailure: { operation: 'start', cause: 'unknown', codes: [] } })).step).toBe(2);
  expect(setupView(status('failed', { workspaceFailure: { operation: 'download', cause: 'network', codes: [] } })).step).toBe(1);
  expect(setupView(status('cancelled')).line).toBe('Setup paused');
  expect(setupView(status('closed')).line).toBe('Workspace stopped');
  expect(setupView(status('notIntegrated', { selected: null })).line).toBe('Workspace unavailable');
});

test('only an unfinished local attempt can queue a model choice', () => {
  for (const state of ['preparing', 'installing', 'starting'] as const) expect(canQueueModel(status(state))).toBe(true);
  for (const state of ['idle', 'ready', 'failed', 'cancelled', 'closed', 'closing', 'notIntegrated'] as const) expect(canQueueModel(status(state))).toBe(false);
  expect(canQueueModel(status('installing', { selected: null }))).toBe(false);
  expect(canQueueModel(undefined)).toBe(false);
});

test('transfer bytes drive the bar but reserve room for startup and readiness', () => {
  const GiB = 1024 ** 3;
  const full = infraProgress(status('installing', { workspaceProgress: transfer(2 * GiB, 2 * GiB) }))!;
  expect(full.value).toBeLessThan(1);
  const half = infraProgress(status('installing', { workspaceProgress: transfer(GiB, 2 * GiB) }))!;
  expect(half.value).toBeGreaterThan(0); expect(half.value!).toBeLessThan(full.value!);
  expect(half.text).toBe('Installing, 1 GiB of 2 GiB');
  expect(infraProgress(status('installing', { workspaceProgress: transfer(null, null) }))).toEqual({ value: null });
  expect(infraProgress(status('installing', { workspaceProgress: null }))!.value).toBeNull();
});

test('remaining time is native, singular and never invented', () => {
  expect(infraProgress(status('installing', { workspaceProgress: transfer(1, 2, 1000) }))!.remaining).toBe('1 second left');
  expect(infraProgress(status('installing', { workspaceProgress: transfer(1, 2, 1001) }))!.remaining).toBe('2 seconds left');
  expect(infraProgress(status('installing', { workspaceProgress: transfer(1, 2, null) }))!.remaining).toBeUndefined();
  expect(infraProgress(status('installing', { workspaceProgress: transfer(1, 2, 0) }))!.remaining).toBeUndefined();
});

test('the bar is continuous from transfer into startup and never complete before ready', () => {
  const values = [
    infraProgress(status('installing', { workspaceProgress: transfer(100, 100) }))!.value!,
    infraProgress(status('starting', { workspaceProgress: null }))!.value!,
    infraProgress(status('starting', { workspaceProgress: starting(15000) }))!.value!,
    infraProgress(status('starting', { workspaceProgress: starting(startupEstimateMs) }))!.value!,
    infraProgress(status('starting', { workspaceProgress: starting(10 * startupEstimateMs) }))!.value!,
  ];
  for (let i = 1; i < values.length; i++) expect(values[i]).toBeGreaterThanOrEqual(values[i - 1]);
  expect(Math.max(...values)).toBeLessThan(1);
  expect(infraProgress(status('ready'))).toBeNull();
  for (const state of ['idle', 'cancelled', 'failed', 'closing'] as const) expect(infraProgress(status(state))).toBeNull();
});

test('startup is an explicit rough estimate, with elapsed time and honest overrun wording', () => {
  const early = infraProgress(status('starting', { workspaceProgress: starting(29200) }))!;
  expect(early.estimate).toMatch(/estimate/i);
  expect(early.estimate).toContain('1 second left');
  expect(early.text).toBe('29 seconds elapsed');
  const late = infraProgress(status('starting', { workspaceProgress: starting(45000) }))!;
  expect(late.estimate).toMatch(/taking longer/i);
  expect(late.estimate).not.toMatch(/left/);
  expect(late.text).toBe('45 seconds elapsed');
  expect(infraProgress(status('starting', { workspaceProgress: starting(1000) }))!.text).toBe('1 second elapsed');
});
