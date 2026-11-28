import type { Status } from '../components/DesignSystem';
import type { ModelState } from './Model';
import { preflightRecovery } from './preflight-state';
import { workspaceRecovery } from './recovery-info';
import { formatRequirementGB } from './requirements';
import type { CheckStepState } from './Steps';
import type { SetupStatus } from './state';
import { unavailableCopy } from './availability';

// One derivation feeds the tab, footer and step list so their counters cannot disagree.
// Infrastructure substages share one workspace step; the file bar measures only file work.
export const setupSteps = 4;

// busy = the app is working (blue); needs-you = the creator's move (amber); ready = finished (green).
export type Tone = 'wait' | 'busy' | 'needs-you' | 'ready' | 'bad' | 'stale';
export type SetupView = { place: 'checking' | 'unavailable' | 'none' | 'local'; step: number; done: boolean; line: string; tabLine: string; footerLine: string; detail: string; tone: Tone; status: Status; action: string };

// A workspace that was running and stopped by itself: stopped, not a failed start. Try again starts it in place.
export const workspaceExited = (status: SetupStatus | undefined) => status?.localVm === 'failed' && status.workspaceFailure?.operation === 'serve' && status.workspaceFailure.cause === 'exited';
export const plural = (count: number, unit: string) => `${count} ${unit}${count === 1 ? '' : 's'}`;
// Native support must be affirmed: an unsupported platform or an older snapshot never offers the early choice.
export const canQueueModel = (status: SetupStatus | undefined) =>
  status?.modelSupported === true && status.selected === 'localVm' && ['preparing', 'installing', 'starting'].includes(status.localVm);
// What setup is doing now, from the phase native reports. Native reports no download progress: the install copies files
// already on this computer, and a full copy is then read back and checked before the workspace starts.
export function setupActivity(status: SetupStatus): string | undefined {
  const progress = status.workspaceProgress;
  if (status.localVm === 'starting') return 'Starting the workspace';
  if (status.localVm !== 'preparing' && status.localVm !== 'installing') return undefined;
  if (progress?.phase === 'verifying') return 'Checking cluster files';
  if (progress?.phase === 'materializing') return progress.bytes !== null && progress.bytes === progress.totalBytes ? 'Checking the installed cluster files' : 'Installing the workspace';
  return 'Getting the cluster files ready';
}
export function workspaceView(status: SetupStatus, { stopping = false, starting = false }: { stopping?: boolean; starting?: boolean } = {}): { word: string; detail: string; status: Status; tone: Tone } {
  if (stopping || status.localVm === 'closing') return { word: 'Stopping', detail: 'Stopping the workspace. Your agents and files are kept.', status: 'running', tone: 'busy' };
  // Before the workspace starts, the pill and line say what setup is doing, as its steps do.
  if (!starting && (status.localVm === 'preparing' || status.localVm === 'installing')) return { word: 'Getting ready', detail: `${setupActivity(status)}.`, status: 'running', tone: 'busy' };
  if (starting || status.selected === 'localVm' && status.localVm === 'idle' || status.localVm === 'starting') return { word: 'Starting', detail: 'Starting the workspace.', status: 'running', tone: 'busy' };
  if (status.localVm === 'failed' && status.workspaceFailure?.operation === 'stop' && status.workspaceFailure.cause === 'forcedStop') {
    const recovery = workspaceRecovery(status.workspaceFailure);
    return { word: 'Forcibly stopped', detail: `${recovery.summary} ${recovery.preservation} ${recovery.action}`, status: 'failed', tone: 'bad' };
  }
  if (workspaceExited(status)) return { word: 'Stopped', detail: 'The workspace on this computer stopped. Your agents and files are kept.', status: 'needs-you', tone: 'needs-you' };
  if (status.localVm === 'failed') return { word: status.workspaceFailure?.operation === 'stop' ? "Couldn't stop" : "Couldn't start", detail: status.workspaceFailure?.operation === 'stop' ? 'The workspace may still be stopping. Quit Bridge and reopen it before trying again.' : "The workspace couldn't start. Your agents and files are kept.", status: 'failed', tone: 'bad' };
  if (status.localVm === 'ready') return { word: 'Ready', detail: 'The workspace is ready on this computer.', status: 'passed', tone: 'ready' };
  // A stopped workspace waits on the creator to start it again.
  return { word: 'Stopped', detail: 'The workspace is stopped. Your agents and their files are kept.', status: 'needs-you', tone: 'needs-you' };
}
export function setupView(status: SetupStatus | undefined, { checking = false, stopping = false, starting = false, workspace = false, model = 'idle', check, initialAutomatic = false }: { checking?: boolean; stopping?: boolean; starting?: boolean; workspace?: boolean; model?: ModelState; check?: CheckStepState; initialAutomatic?: boolean } = {}): SetupView {
  // A Copilot choice checked on this computer waits for the workspace; say so beside the workspace's progress.
  const aiWaiting = model === 'checked' ? ', AI checked' : model === 'checking' ? ', checking the AI sign-in' : '';
  const view = (place: SetupView['place'], step: number, line: string, detail: string, tone: Tone, done = false): SetupView => ({
    place, step, done, line, detail, tone,
    tabLine: !done && step > 0 && (tone === 'busy' || tone === 'needs-you') ? `Setting up, step ${step} of ${setupSteps}` : line,
    footerLine: !done && step > 0 && (tone === 'busy' || tone === 'needs-you') ? `${step === 2 ? `Getting your workspace ready${aiWaiting}` : line}, step ${step} of ${setupSteps}` : line,
    status: ({ busy: 'running', 'needs-you': 'needs-you', wait: 'not-started', ready: 'passed', bad: 'failed', stale: 'stale' } as const)[tone],
    action: done ? 'Manage' : step === 3 ? 'Connect the AI' : place === 'local' ? 'Continue setup' : 'Set up',
  });
  if (!status) return checking ? view('checking', 0, 'Checking the workspace…', 'Checking the workspace…', 'busy') : view('unavailable', 0, 'Workspace unavailable', 'Workspace unavailable', 'needs-you');
  if (workspace) {
    const state = workspaceView(status, { stopping, starting });
    const result = view('local', 0, `Workspace ${state.word.toLowerCase()}`, state.detail, state.tone, status.localVm === 'ready');
    return { ...result, tabLine: `This computer, ${state.word.toLowerCase()}`, action: state.word === 'Stopped' && status.localVm === 'cancelled' ? 'Start' : 'Manage' };
  }
  const place = status.selected === 'localVm' ? 'local' : 'none';
  // Nothing on this computer waits on the creator when this build can't run it: not amber, and no setup to continue.
  if (status.localVm === 'notIntegrated') return { ...view(place, 0, unavailableCopy.line, unavailableCopy.detail, 'wait'), action: 'Details' };
  if (place === 'none') return view(place, 0, 'Choose where agents run', 'Choose where agents run', 'wait');
  if (stopping) return view(place, status.localVm === 'ready' ? 4 : 0, 'Stopping the workspace', 'Stopping the workspace', 'busy', status.localVm === 'ready');
  switch (status.localVm) {
    case 'starting': return view(place, 2, `Starting the workspace${aiWaiting}`, `Starting the workspace${aiWaiting}`, 'busy');
    case 'failed': {
      if (workspaceExited(status)) return view(place, 2, 'Workspace stopped', workspaceRecovery(status.workspaceFailure).summary, 'needs-you');
      const line = status.preflightFailure ? preflightRecovery(status.preflightFailure).summary : 'Setup failed';
      return view(place, 2, line, status.preflightFailure ? line : workspaceRecovery(status.workspaceFailure).summary, 'bad');
    }
    case 'closing': return view(place, 0, 'Stopping the workspace', 'Stopping the workspace', 'busy');
    case 'closed': return view(place, 2, 'Workspace stopped', 'Workspace stopped', 'wait');
    case 'cancelled': return view(place, 2, 'Setup paused', 'Downloaded files are kept.', 'wait');
    case 'idle': return view(place, 0, 'Not set up', 'Not set up', 'wait');
    case 'ready': {
      if (status.model === 'locked') return view(place, 3, 'AI not connected', 'AI not connected', 'wait');
      if (model === 'failed' || model === 'resetFailed' || model === 'removalFailed') return view(place, 3, 'AI not connected', 'The AI needs to be set up again', 'needs-you');
      if (model !== 'connected') {
        const detail = { resetting: 'Resetting the AI connection…', connecting: 'Connecting the AI…', removing: 'Removing the AI sign-in…', queued: 'AI queued', checking: 'Connecting the AI…', checked: 'AI queued', idle: 'Connect the AI' }[model];
        return view(place, 3, detail, detail, model === 'idle' ? 'needs-you' : model === 'queued' || model === 'checked' ? 'wait' : 'busy');
      }
      // Test evidence can need attention after setup; only the initial automatic run is a setup stage.
      const done = !initialAutomatic;
      const tone = check === 'failed' ? 'bad' : check === 'passed' ? 'ready' : check === 'running' ? 'busy' : check === 'stale' || check === 'unknown' ? 'stale' : initialAutomatic ? 'needs-you' : 'wait';
      const line = !done ? 'Run your first agent' : check ? { passed: 'Workspace ready', failed: 'Its test failed', running: 'Running test', stale: 'Tests need running', unknown: 'Test result unknown' }[check] : 'Workspace ready';
      return view(place, 4, line, 'AI connected', check || !done ? tone : 'ready', done);
    }
    default: return view(place, 2, `Getting the workspace ready${aiWaiting}`, `Getting the workspace ready${aiWaiting}`, 'busy');
  }
}

export type Infra = { value: number | null; text?: string; remaining?: string };
// Null means no unfinished infrastructure stage; value null means the transfer size is not known yet.
export function infraProgress(status: SetupStatus): Infra | null {
  const progress = status.workspaceProgress;
  if (status.localVm === 'starting') {
    return progress?.phase === 'starting'
      ? { value: null, text: `${plural(Math.floor(progress.elapsedMs / 1000), 'second')} elapsed` }
      : { value: null };
  }
  if (status.localVm !== 'preparing' && status.localVm !== 'installing') return null;
  const bytes = progress?.phase === 'materializing' ? progress.bytes : null;
  const total = progress?.phase === 'materializing' ? progress.totalBytes : null;
  const remaining = progress?.phase === 'materializing' ? progress.remainingMs : null;
  if (progress?.phase === 'verifying') return { value: null, text: "Bridge reads each file to make sure it's intact. This can take a few minutes." };
  if (bytes === null || total === null) return { value: null };
  // A full copy isn't finished: Bridge reads it back before the workspace starts. Say so rather than show a full bar.
  if (bytes >= total) return { value: null, text: `${formatRequirementGB(total)} copied. Bridge checks the copy before the workspace starts.` };
  return {
    value: bytes / total, text: `Installing cluster files, ${formatRequirementGB(bytes).replace(' GB', '')} of ${formatRequirementGB(total)}`,
    remaining: remaining !== null && remaining > 0 ? `${plural(Math.ceil(remaining / 1000), 'second')} left` : undefined,
  };
}
