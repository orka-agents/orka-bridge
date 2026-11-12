import type { ModelState } from './Model';
import { formatSize } from './requirements';
import type { SetupStatus } from './state';
import type { CheckStepState } from './Steps';

// One derivation feeds the tab, footer and step list so their counters cannot disagree.
// The stages are: install files, start, Orka ready, connect model, first test.
export const setupSteps = 5;
// A frontend estimate only: native reports elapsed startup time, never a measured typical.
export const startupEstimateMs = 30_000;
// The bar keeps room for startup and readiness, so it cannot finish before native says ready.
const transferShare = 0.7;
const startupShare = 0.28;

// busy = the app is working (blue); needs-you = the creator's move (amber); ready = finished (green).
export type Tone = 'wait' | 'busy' | 'needs-you' | 'ready' | 'bad';
export type SetupView = { place: 'checking' | 'unavailable' | 'none' | 'local'; step: number; done: boolean; line: string; detail: string; tone: Tone };

export const plural = (count: number, unit: string) => `${count} ${unit}${count === 1 ? '' : 's'}`;
export const canQueueModel = (status: SetupStatus | undefined) =>
  status?.selected === 'localVm' && ['preparing', 'installing', 'starting'].includes(status.localVm);
// Failures before the VM starts belong to stage 1; an unspecified failure matches the recovery default.
export function failedStep(status: SetupStatus): 1 | 2 {
  if (status.preflightFailure) return 1;
  const operation = status.workspaceFailure?.operation ?? 'start';
  return operation === 'start' || operation === 'services' || operation === 'stop' ? 2 : 1;
}

export function setupView(status: SetupStatus | undefined, { checking = false, model = 'idle', check }: { checking?: boolean; model?: ModelState; check?: CheckStepState } = {}): SetupView {
  const view = (place: SetupView['place'], step: number, line: string, detail: string, tone: Tone, done = false): SetupView => ({ place, step, done, line, detail, tone });
  if (!status) return checking ? view('checking', 0, 'Checking workspace…', 'Checking workspace…', 'wait') : view('unavailable', 0, 'Workspace unavailable', 'Workspace unavailable', 'wait');
  const place = status.selected === 'localVm' ? 'local' : 'none';
  if (status.localVm === 'notIntegrated') return view(place, 0, 'Workspace unavailable', 'Workspace unavailable', 'wait');
  if (place === 'none') return view(place, 0, 'Choose where agents run', 'Choose where agents run', 'wait');
  const at = (step: number) => `Setting up, step ${step} of ${setupSteps}`;
  switch (status.localVm) {
    case 'starting': return view(place, 2, at(2), 'Starting workspace', 'busy');
    case 'failed': return view(place, failedStep(status), 'Setup failed', 'Workspace needs attention', 'bad');
    case 'closing': return view(place, 0, 'Stopping workspace', 'Stopping workspace', 'busy');
    case 'closed': return view(place, 1, 'Workspace stopped', 'Workspace stopped', 'wait');
    case 'cancelled': return view(place, 1, 'Setup paused', 'What has arrived is kept', 'wait');
    case 'ready': {
      if (status.model === 'locked') return view(place, 4, 'AI unavailable', 'AI connection unavailable', 'wait');
      if (model === 'failed') return view(place, 4, 'AI needs attention', 'AI connection needs attention', 'bad');
      if (model !== 'connected') {
        const detail = { connecting: 'Connecting the AI…', removing: 'Removing the AI sign-in…', queued: 'AI queued', idle: 'Choose the AI' }[model];
        return view(place, 4, at(4), detail, model === 'idle' ? 'needs-you' : model === 'queued' ? 'wait' : 'busy');
      }
      // A stale, running or unknown first test is not evidence that setup finished.
      const done = check === 'passed' || check === 'failed';
      const tone = done ? 'ready' : check === 'running' ? 'busy' : check === 'unknown' ? 'wait' : 'needs-you';
      return view(place, 5, done ? 'This computer ready' : at(5), 'AI connected', tone, done);
    }
    default: return view(place, 1, at(1), 'Getting workspace ready', 'busy');
  }
}

export type Infra = { value: number | null; text?: string; remaining?: string; estimate?: string };
// Null means no unfinished infrastructure stage; value null means the transfer size is not known yet.
export function infraProgress(status: SetupStatus): Infra | null {
  const progress = status.workspaceProgress;
  if (status.localVm === 'starting') {
    const elapsed = progress?.phase === 'starting' ? progress.elapsedMs : 0;
    const left = Math.ceil((startupEstimateMs - elapsed) / 1000);
    return {
      value: transferShare + startupShare * Math.min(elapsed / startupEstimateMs, 1),
      text: progress?.phase === 'starting' ? `${plural(Math.floor(elapsed / 1000), 'second')} elapsed` : 'Getting the workspace ready…',
      estimate: left > 0 ? `Rough estimate, not a measurement: about ${plural(left, 'second')} left.` : 'Taking longer than the rough 30-second estimate. Still working.',
    };
  }
  if (status.localVm !== 'preparing' && status.localVm !== 'installing') return null;
  const bytes = progress?.phase === 'materializing' ? progress.bytes : null;
  const total = progress?.phase === 'materializing' ? progress.totalBytes : null;
  const remaining = progress?.phase === 'materializing' ? progress.remainingMs : null;
  if (bytes === null || total === null) return { value: null };
  return {
    value: transferShare * bytes / total, text: `Installing, ${formatSize(bytes)} of ${formatSize(total)}`,
    remaining: remaining !== null && remaining > 0 ? `${plural(Math.ceil(remaining / 1000), 'second')} left` : undefined,
  };
}
