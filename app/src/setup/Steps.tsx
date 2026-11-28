import { type ReactNode, useEffect, useState } from 'react';
import { Button } from '../components/Button';
import { type Status, StatusIcon, StatusPill } from '../components/DesignSystem';
import { Disclosure } from '../components/Disclosure';
import { agentInfo } from '../designer/agent-api';
import type { ModelState } from './Model';
import { canQueueModel, infraProgress, plural, setupActivity, setupView, workspaceExited } from './progress';
import Recovery from './Recovery';
import { modelDetails } from './recovery-info';
import { type ExamplePair, loadExample, type SetupStatus, type WorkspaceProgress } from './state';
import './steps.css';
import { readForm } from '../designer/agent-form';
import { WorkspaceInside } from './Workspace';
import { builtInTools } from '../designer/built-in-tools';
import { testDefinitions } from '../designer/test-definitions';
import SetUpLater from '../firstrun/SetUpLater';

export type CheckStepState = 'running' | 'passed' | 'failed' | 'unknown' | 'stale';

type Props = {
  // Start a fresh workspace is replacing this workspace: its step says so instead of its last state.
  replacing?: boolean;
  upgrade?: ReactNode; updating?: boolean; updateBlocked?: boolean; ordinaryRecovery?: boolean;
  status: SetupStatus; busy: boolean; stopping: boolean; error?: string; modelState?: ModelState; firstCheck?: CheckStepState;
  modelUnavailable?: boolean;
  firstTestResult?: { failed: number }; initialAutomatic?: boolean; onCheckFirstTest?: () => void; onChooseProvider?: () => void;
  onCancel: () => void; onRestart: () => void; onCluster?: () => void; onOpenExample: () => void;
  // Start a fresh workspace, offered on a failure the ordinary recovery can't get past.
  onFresh?: () => void; helpLines?: string[];
  // Outside the guided first run, a ready workspace keeps What's inside and its deliberate Start a fresh workspace…,
  // whatever the AI and the first test: the setup card is this workspace's own card until setup finishes.
  onFreshWorkspace?: () => void; memoryBytes?: number | null;
  // During the guided first run: the agent's instructions open in place, and stuck screens offer Set up later.
  guided?: boolean; onSetUpLater?: () => void;
  // Outside the guided first run, a workspace that is getting ready is one card beside the AI card, without the steps.
  cards?: boolean;
};
// Native's elapsed time runs from the start of this run of setup (a resume starts it again), not from each step. The step
// a view sees begin is timed from there; a step already under way when the view opened can only say how long setup has run.
type StepClock = { phase: WorkspaceProgress['phase']; from: number; known: boolean; last: number };
function useStepTime(progress: WorkspaceProgress | null | undefined): string | undefined {
  const [clock, setClock] = useState<StepClock>();
  if (progress && (!clock || clock.phase !== progress.phase || progress.elapsedMs < clock.last)) {
    const continued = !!clock && progress.elapsedMs >= clock.last;
    // A restarted clock starts at this step; so does checking, setup's first step.
    const restarted = !!clock && progress.elapsedMs < clock.last;
    setClock({ phase: progress.phase, from: continued ? clock.last : 0, known: continued || restarted || progress.phase === 'verifying', last: progress.elapsedMs });
  } else if (progress && clock && clock.last !== progress.elapsedMs) setClock({ ...clock, last: progress.elapsedMs });
  if (!progress || progress.phase === 'starting' || !clock || clock.phase !== progress.phase) return undefined;
  const seconds = plural(Math.floor(Math.max(0, progress.elapsedMs - clock.from) / 1000), 'second');
  return clock.known ? `${seconds} ${progress.phase === 'verifying' ? 'checking files' : 'installing files'}.` : `Setup has been running for ${seconds}.`;
}
const rowClass: Record<Status, string> = { passed: 'step-done', failed: 'step-failed', running: 'step-current', 'needs-you': 'step-current', stale: 'step-waiting', 'not-started': 'step-waiting' };
// Completed rows stay in place; only the active row opens its progress.
function stepRows({ status, stopping, updating, updateBlocked, error, modelState = 'idle', firstCheck, initialAutomatic, replacing }: Pick<Props, 'status' | 'stopping' | 'updating' | 'updateBlocked' | 'error' | 'modelState' | 'firstCheck' | 'initialAutomatic' | 'replacing'>) {
  const view = setupView(status, { stopping, model: modelState, check: firstCheck, initialAutomatic });
  const failed = status.localVm === 'failed';
  const paused = status.localVm === 'cancelled' || status.localVm === 'closed';
  const connected = modelState === 'connected';
  const infra = stopping ? null : infraProgress(status);
  const workspace: [Status, string] = replacing ? ['running', 'Replacing'] : updating ? ['running', 'Updating'] : updateBlocked ? ['needs-you', 'Needs you'] : stopping || status.localVm === 'closing' ? ['running', 'Stopping']
    : status.localVm === 'ready' ? ['passed', 'Ready']
    : workspaceExited(status) ? ['needs-you', 'Stopped'] : failed ? ['failed', "Couldn't start"] : paused ? ['needs-you', 'Paused']
    : infra ? ['running', status.localVm === 'starting' ? 'Starting' : 'Getting ready'] : ['not-started', 'Waiting'];
  const model: [Status, string] = modelState === 'removalFailed' ? ['failed', "Couldn't remove"] : modelState === 'resetFailed' ? ['failed', "Couldn't reset"]
    : modelState === 'failed' ? ['needs-you', 'Needs you'] : connected ? ['passed', 'Connected']
    : modelState === 'resetting' ? ['running', 'Resetting'] : modelState === 'connecting' ? ['running', 'Connecting'] : modelState === 'removing' ? ['running', 'Removing sign-in']
    : modelState === 'queued' ? ['not-started', 'Queued']
    : modelState === 'checking' ? ['running', 'Checking sign-in'] : modelState === 'checked' ? ['not-started', 'Checked, waiting']
    : status.localVm === 'ready' ? status.model === 'locked' ? ['not-started', 'Unavailable'] : ['needs-you', 'Needs you']
    : canQueueModel(status) ? ['not-started', 'Pick any time'] : ['not-started', 'Waiting'];
  // A kept result belongs to an earlier workspace until this one is ready with its model connected.
  const test: [Status, string] = firstCheck && status.localVm === 'ready' && connected ? [{ running: 'running', passed: 'passed', failed: 'failed', unknown: 'stale', stale: 'stale' }[firstCheck] as Status,
    { running: 'Running', passed: 'Its test passed', failed: 'Its test failed', unknown: 'Result unknown', stale: 'Needs running' }[firstCheck]]
    : connected ? ['needs-you', 'Needs you'] : ['not-started', 'Waiting'];
  const names = ['Where agents run', 'Get your workspace ready', 'Connect the AI', 'Run your first agent'];
  // After an unreadable poll the snapshot is last known, not observed: keep the words but nothing runs or is current.
  const rows = [ ['passed', 'On this computer'] as [Status, string], workspace, model, test].map(([state, result], i) => ({ name: names[i], state: error ? 'not-started' as Status : state, result: error ? `Last known: ${result}` : result }));
  return { view, failed, paused, infra, rows };
}

export function SetupProgress(props: Props) {
  const { status, busy, stopping, error, onCancel, onRestart, onCluster, onOpenExample, guided, onSetUpLater, cards = false } = props;
  // On its own card the phases start open, as the board shows them.
  const [details, setDetails] = useState(cards);
  const { view, failed: failedStatus, paused, infra } = stepRows(props);
  const failed = failedStatus && !error;
  const ready = status.localVm === 'ready';
  const needsModel = props.modelState === 'connected' && props.modelUnavailable && props.firstCheck === 'unknown';
  const progress = status.workspaceProgress;
  const stepTime = useStepTime(progress);
  const copying = !stopping && status.localVm === 'installing' && progress?.phase === 'materializing';
  const stateLine = error ? 'Workspace status is unavailable.' : props.replacing ? 'Bridge is replacing this workspace with a fresh one. Your agent files are kept.' : props.updating ? 'Updating the workspace. Your agent files are kept.' : props.updateBlocked ? 'Workspace recovery needs attention.' : stopping ? 'Stopping the workspace…' : ready ? 'Your workspace is ready on this computer.'
    : paused ? 'Setup is paused. Choose Start again to resume.' : status.localVm === 'starting' ? 'Starting the workspace…'
    : status.localVm === 'closing' ? 'Stopping the workspace…'
    : status.localVm === 'notIntegrated' ? 'Workspace setup is not included in this build. Nothing will be downloaded or started.'
    : `${setupActivity(status) ?? 'Getting the cluster files ready'}…`;
  // A paused setup waits on the creator, as the board marks it.
  const pill: Status = error ? 'needs-you' : props.updating || props.replacing ? 'running' : props.updateBlocked ? 'needs-you' : paused ? 'needs-you' : view.status;
  const workspaceReady = ready && !error && !stopping && !props.updating && !props.updateBlocked && !props.replacing;
  const workspaceWord = error ? 'Status unavailable' : props.replacing ? 'Replacing' : props.updating ? 'Updating' : props.updateBlocked ? 'Needs you' : stopping || status.localVm === 'closing' ? 'Stopping' : workspaceReady ? 'Ready' : failed ? workspaceExited(status) ? 'Stopped' : "Couldn't start" : paused ? 'Needs you' : undefined;
  const settingUp = ['preparing', 'installing', 'starting'].includes(status.localVm);
  // While the workspace is getting ready the step says so; the head carries no second running word.
  const pillShown = cards || workspaceWord !== undefined || !(pill === 'running' && settingUp);
  const cardWord = cards && workspaceWord === undefined && settingUp ? status.localVm === 'starting' ? 'Starting' : 'Getting ready' : workspaceWord;
  const cancelRow = settingUp && <div className="workspace-step-actions step-cancel"><Button type="button" kind="secondary" disabled={busy || !!error} onClick={onCancel}>Cancel</Button><span className="muted">Cancel pauses setup. Your downloaded files are kept.</span></div>;
  // Paused, the step itself says what happened and offers Start again, beside what is kept.
  const pausedDetails = paused && !error && <>
    <p className="step-paused">Setup is paused. Choose Start again to resume.</p>
    {status.localVm === 'cancelled' && <p className="muted">{view.detail}</p>}
    {((props.ordinaryRecovery ?? !props.upgrade) || onSetUpLater) && <div className="workspace-step-actions">
      {(props.ordinaryRecovery ?? !props.upgrade) && <Button type="button" kind="primary" disabled={busy || !!error} onClick={onRestart}>Start again</Button>}
      {onSetUpLater && <SetUpLater onClick={onSetUpLater} />}
    </div>}
  </>;
  const workspaceDetails = props.upgrade ?? (pausedDetails || !failed && !error && <>
    {infra && <><div role="progressbar" aria-label="Workspace setup progress" aria-valuemin={0} aria-valuemax={100}
      aria-valuenow={infra.value === null ? undefined : Math.round(infra.value * 100)} className={`infra-bar${infra.value === null ? ' indeterminate' : ''}`}>
      <span style={infra.value === null ? undefined : { width: `${Math.round(infra.value * 100)}%` }} /></div>
      {(!cards || infra.text) && <p className="muted">{infra.text ?? (status.localVm === 'starting' ? 'Starting the workspace…' : stateLine)}</p>}</>}
    {/* Once ready, the phases are all done; their details stay only to show an error the workspace reported. */}
    {(!ready || !!status.localVmError) && <Disclosure label={details ? 'Hide details' : 'Show details'} open={details} controls="workspace-progress-details" onOpenChange={setDetails}>
      {details && <div id="workspace-progress-details" className="workspace-progress-details">
        <dl><div><dt>Checking cluster files</dt><dd>{status.localVm === 'preparing' || progress?.phase === 'verifying' ? 'In progress' : ready || copying || status.localVm === 'starting' ? 'Checked' : 'Waiting'}</dd></div>
          <div><dt>Installing cluster files</dt><dd>{ready || status.localVm === 'starting' ? 'Installed' : copying ? 'In progress' : 'Waiting'}</dd></div>
          <div><dt>Starting the workspace</dt><dd>{ready ? 'Ready' : status.localVm === 'starting' ? 'In progress' : 'Waiting'}</dd></div></dl>
        {stepTime && status.localVm !== 'starting' && <p className="muted">{stepTime}</p>}
        {status.localVmError && <pre tabIndex={0}>{ready ? modelDetails(status.localVmError) : status.localVmError}</pre>}
      </div>}</Disclosure>}
    {cancelRow}
  </>);
  return <section className="card workspace-steps setup-progress" aria-label="Workspace setup">
    <div className="steps-head"><h2>Workspace</h2>{pillShown && <StatusPill state={workspaceReady ? 'passed' : pill}>{cardWord}</StatusPill>}</div>
    {view.done && <span className="status-description">{view.line !== 'Workspace ready' ? view.line : undefined}</span>}
    <p role="status" className={error || status.localVm === 'notIntegrated' || cards ? undefined : 'sr-only'}>{paused && !error ? '' : cards && !error ? stateLine : view.done || paused || error || ready || stopping || status.localVm === 'notIntegrated' ? stateLine : failed ? 'Setup needs attention' : view.line}</p>
    {cards ? <div className="workspace-card-progress">{workspaceDetails}</div> : <SetupSteps {...props} workspaceDetails={workspaceDetails} />}
    {ready && !error && props.onChooseProvider && props.modelState === 'connected' && needsModel && <div className="first-test-recovery">
      <p>The first test could not be confirmed. If the AI could not be reached, check your connection or choose another provider.</p>
      <p className="muted">Changing provider first asks you to remove the current sign-in or key. Your agent files are kept.</p>
      <div className="workspace-step-actions"><Button type="button" kind="primary" disabled={busy} onClick={onOpenExample}>Try again in Designer</Button>
        {props.onChooseProvider && <Button type="button" kind="secondary" disabled={busy} onClick={props.onChooseProvider}>Choose another provider</Button>}
        {onSetUpLater && <SetUpLater onClick={onSetUpLater} />}</div>
    </div>}
    {!failed && <div className="workspace-step-actions">
      {/* Two cards leave no room for Meet; its introduction is one quiet click away, in Designer. */}
      {cards && settingUp && !error && <Button type="button" kind="quiet" onClick={onOpenExample}>See your first agent</Button>}
      {settingUp && (!!props.upgrade || !!error) && <><Button type="button" kind="secondary" disabled={busy || !!error} onClick={onCancel}>Cancel</Button><span className="muted">Cancel pauses setup. Your downloaded files are kept.</span></>}
      {status.localVm === 'notIntegrated' && onCluster && <Button type="button" kind="secondary" disabled={busy} onClick={onCluster}>Use a cluster you already have instead</Button>}
      {paused && (!!error || !!props.upgrade) && (props.ordinaryRecovery ?? !props.upgrade) && <Button type="button" kind="primary" disabled={busy || !!error} onClick={onRestart}>Start again</Button>}
      {onSetUpLater && (status.localVm === 'notIntegrated' || paused && (!!error || !!props.upgrade)) && <SetUpLater onClick={onSetUpLater} />}
      {ready && <>{props.modelState === 'connected' && !needsModel && <>
        {props.firstCheck === 'stale' ? <Button type="button" kind="primary" disabled={busy || !!error} onClick={onOpenExample}>Run it again</Button>
          : (props.firstCheck === 'unknown' || ((props.firstCheck === 'passed' || props.firstCheck === 'failed') && !props.firstTestResult)) ? <Button type="button" kind="primary" disabled={busy || !!error || !props.onCheckFirstTest} onClick={props.onCheckFirstTest}>Check again</Button>
          : !props.firstCheck && <Button type="button" kind="primary" disabled={busy || !!error} onClick={onOpenExample}>Open your first agent</Button>}
        {props.firstTestResult && <Button type="button" kind={props.firstCheck === 'stale' || props.firstCheck === 'unknown' || props.firstCheck === 'running' ? 'secondary' : 'primary'} disabled={busy || !!error} onClick={onOpenExample}>See what it answered</Button>}
        {onSetUpLater && (props.firstCheck === 'failed' || props.firstCheck === 'unknown') && <SetUpLater onClick={onSetUpLater} />}
      </>}{!needsModel && !guided && <Button type="button" kind="secondary" disabled={busy || !!error} onClick={onOpenExample}>Read its instructions</Button>}</>}
    </div>}
    {workspaceReady && !guided && <WorkspaceInside id="setup-workspace-inside" memoryBytes={props.memoryBytes} freshDisabled={busy}
      onFresh={(props.ordinaryRecovery ?? !props.upgrade) ? props.onFreshWorkspace : undefined} />}
    {/* Guided, the instructions open here instead of in Designer, so reading them doesn't leave the first run. */}
    {!failed && ready && !needsModel && guided && <GuidedInstructions disabled={busy || !!error} onOpen={onOpenExample} />}
  </section>;
}

export function SetupSteps(props: Pick<Props, 'status' | 'busy' | 'stopping' | 'error' | 'modelState' | 'firstCheck' | 'initialAutomatic' | 'onRestart' | 'onCluster' | 'upgrade' | 'updating' | 'updateBlocked' | 'ordinaryRecovery' | 'onSetUpLater' | 'onFresh' | 'helpLines'> & { workspaceDetails?: ReactNode }) {
  const { status, busy, error, onRestart, onCluster, onSetUpLater } = props;
  const { view, failed, rows } = stepRows(props);
  const aiFailed = ['failed', 'resetFailed', 'removalFailed'].includes(props.modelState ?? 'idle');
  return <ol className="steps-four" aria-label="Setup steps">
      {rows.map(({ name, state, result }, index) => {
        return <li key={name} className={rowClass[state]} aria-current={!view.done && index + 1 === view.step && ['running', 'needs-you'].includes(state) ? 'step' : undefined}>
        <StatusIcon state={state} /><b>{name}</b><span className="step-result">{result}</span>
        {index === 1 && props.workspaceDetails && <div className="step-more">{props.workspaceDetails}</div>}
        {index === 2 && onSetUpLater && aiFailed && <div className="step-more workspace-step-actions guided-later-row"><SetUpLater onClick={onSetUpLater} /></div>}
        {failed && (props.ordinaryRecovery ?? !props.upgrade) && index === 1 && <div className="step-more">
          <Recovery key={status.localVmError} detail={status.localVmError} preflightFailure={status.preflightFailure} failure={status.workspaceFailure} disabled={busy || !!error} onRetry={onRestart} onCluster={onCluster} onSetUpLater={onSetUpLater} onFresh={props.onFresh} helpLines={props.helpLines} /></div>}
      </li>; })}
    </ol>;
}

// The example pair, read once for the card that introduces it.
function useExample() {
  const [pair, setPair] = useState<ExamplePair>();
  const [failed, setFailed] = useState(false);
  useEffect(() => { let mounted = true; void loadExample().then(result => { if (!mounted) return; if (result.ok) setPair(result.pair); else setFailed(true); }); return () => { mounted = false; }; }, []);
  return { pair, failed };
}

// Read-only, in place: a long text scrolls inside its box, and Open in Designer is the deliberate way into the editor.
// The text is read only once the disclosure opens.
function Instructions({ id, disabled, onOpen, children }: { id: string; disabled?: boolean; onOpen: () => void; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  return <Disclosure className="agent-instructions" label="Read its instructions" open={open} controls={id} onOpenChange={setOpen}>
    {open && <div id={id} className="agent-instructions-body">
      {children}
      <Button type="button" kind="quiet" disabled={disabled} onClick={onOpen}>Open in Designer</Button>
    </div>}
  </Disclosure>;
}
function InstructionsText({ agent, failed }: { agent?: string; failed: boolean }) {
  const text = agent === undefined ? undefined : readForm(agent).instructions.trim() || agent;
  return text !== undefined ? <div className="agent-instructions-text reading" tabIndex={0}>{text}</div>
    : <p className="muted">{failed ? 'The example could not be opened. Try opening Designer.' : "Opening the example's introduction…"}</p>;
}
function ExampleInstructions() {
  const { pair, failed } = useExample();
  return <InstructionsText agent={pair?.agent} failed={failed} />;
}
function GuidedInstructions({ disabled, onOpen }: { disabled: boolean; onOpen: () => void }) {
  return <Instructions id="progress-instructions" disabled={disabled} onOpen={onOpen}><ExampleInstructions /></Instructions>;
}

export function Meet({ onRead, onName, guided = false }: { onRead: () => void; onName?: (name: string | undefined) => void; guided?: boolean }) {
  const { pair, failed } = useExample();
  const info = agentInfo(pair?.agent);
  useEffect(() => { if (pair) onName?.(info.name); }, [pair, info.name, onName]);
  const form = readForm(pair?.agent ?? '');
  const tools = form.tools.filter((tool): tool is typeof tool & { name: string } => !!tool.name);
  const tests = testDefinitions(pair?.tests ?? '');
  return <section className="card workspace-meet" aria-labelledby="meet-title">
    <span className="workspace-kicker">Meet your first agent</span><h2 id="meet-title">{info.name ?? 'Your first agent'}</h2>
    {pair ? <>{info.description && <p className="meet-description">{info.description}</p>}
      <dl><div><dt>It can use</dt><dd>{tools.length ? tools.map(tool => <span className="workspace-tool" key={tool.index}>{tool.builtin && Object.hasOwn(builtInTools, tool.name) ? builtInTools[tool.name] : 'Tool'} <code>{tool.name}</code></span>) : !form.toolsEditable || tools.length !== form.tools.length ? 'The tool list could not be read. Open its instructions to see the file.' : 'No tools are configured.'}</dd></div>
        {tests && <div><dt>How we'll test it</dt><dd>{tests.map(test => <p key={test.id}>{test.title ?? test.question}</p>)}</dd></div>}
        <div><dt>Whose is it</dt><dd>Yours. After setup, change its instructions and tests, or start a new agent.</dd></div></dl>
      {guided ? <Instructions id="meet-instructions" onOpen={onRead}><InstructionsText agent={pair.agent} failed={false} /></Instructions>
        : <Button type="button" kind="secondary" onClick={onRead}>Read its instructions</Button>}</>
      : <p className="muted">{failed ? 'The example could not be opened. Try opening Designer.' : "Opening the example's introduction…"}</p>}
  </section>;
}
