import { useCallback, useEffect, useRef, useState } from 'react';
import { Chip } from '../components/DesignSystem';
import Model, { type ModelState } from './Model';
import Preflight from './Preflight';
import { type LocalVmState, type SetupStatus, cancelLocal, changeLocal, selectLocal, setupStatus } from './state';
import { Meet, SetupProgress, SetupSteps, type CheckStepState } from './Steps';
import { canQueueModel } from './progress';
import Welcome, { hasSeenWelcome, welcomePreference } from './Welcome';
import { formatSize, workspaceRequirements, type WorkspaceRequirements } from './requirements';
import ClusterCard from '../clusters/ClusterCard';
import type { SelectedPlace } from '../clusters/api';
import './welcome.css';
import { workspaceRecovery } from './recovery-info';

const pending = (state: LocalVmState) => ['preparing', 'installing', 'starting', 'closing'].includes(state);
const runtimeCopy = {
  notIntegrated: 'Workspace setup is not included in this build. Nothing will be downloaded or started.',
};
export default function Setup({ onOpenExample, onReady, onStatus, onModelState, onWelcome, onExampleName, firstCheck, onStarting, onWorkspaceAction, onPlace, active = true }: {
  onOpenExample: () => void; onReady: () => void; active?: boolean;
  onStatus?: (status: SetupStatus | undefined) => void;
  onModelState?: (state: ModelState) => void;
  onWelcome?: (visible: boolean) => void;
  onExampleName?: (name: string | undefined) => void;
  firstCheck?: CheckStepState;
  onStarting?: () => void;
  onWorkspaceAction?: (pending: boolean) => void;
  onPlace?: (place: SelectedPlace) => void;
}) {
  const [status, setStatus] = useState<SetupStatus>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(true);
  const [sending, setSending] = useState(false);
  const [pendingAction, setPendingAction] = useState<'start' | 'cancel' | 'change'>();
  const mutation = useRef(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const [confirmChange, setConfirmChange] = useState(false);
  const [modelState, setModelState] = useState<ModelState>('idle');
  const observeModel = useCallback((value: ModelState) => { setModelState(value); onModelState?.(value); }, [onModelState]);
  const [check, setCheck] = useState(0);
  const [seen, setSeen] = useState(hasSeenWelcome);
  const [inside, setInside] = useState(false);
  const [cluster, setCluster] = useState(false);
  const [requirements, setRequirements] = useState<WorkspaceRequirements | null>();
  const [preflightBlocked, setPreflightBlocked] = useState(true);
  const epoch = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const heading = useRef<HTMLHeadingElement>(null);
  const focusChoice = useRef(false);
  const changeInvoker = useRef<HTMLButtonElement>(null);
  const welcome = !seen && !!status && !error && status.selected !== 'localVm';
  useEffect(() => { onWelcome?.(welcome); }, [welcome, onWelcome]);
  useEffect(() => {
    if (focusChoice.current && active) heading.current?.focus();
    focusChoice.current = false;
  }, [seen, status?.selected, status?.localVm, active, cluster]);
  useEffect(() => {
    let active = true;
    void workspaceRequirements().then(value => { if (active) setRequirements(value); });
    return () => { active = false; };
  }, []);
  useEffect(() => {
    const observe = () => {
      if (!mutation.current && document.visibilityState !== 'hidden') setCheck(value => value + 1);
    };
    window.addEventListener('focus', observe);
    document.addEventListener('visibilitychange', observe);
    return () => {
      window.removeEventListener('focus', observe);
      document.removeEventListener('visibilitychange', observe);
    };
  }, []);
  useEffect(() => {
    if (mutation.current) return;
    const request = ++epoch.current;
    void setupStatus().then(result => {
      if (request !== epoch.current) return;
      if (result.ok) { setStatus(result.status); setError(undefined); }
      else {
        // An unreadable observation pauses actions, not an already admitted connection.
        setError(result.message);
      }
      const waiting = result.ok && pending(result.status.localVm);
      setBusy(waiting);
      timer.current = setTimeout(() => setCheck(value => value + 1), waiting ? 250 : 1000);
      onStatus?.(result.ok ? result.status : undefined);
    });
    return () => { epoch.current++; clearTimeout(timer.current); };
  }, [check, onStatus]);
  useEffect(() => { if (status?.localVm === 'ready') onReady(); }, [status?.localVm, onReady]);
  // Model stays mounted from preparing through ready, so it owns its queued state; no Model means no model state.
  const modelMounted = status?.selected === 'localVm' && (status.localVm === 'ready' || canQueueModel(status));
  useEffect(() => { if (!modelMounted) observeModel('idle'); }, [modelMounted, observeModel]);

  function start() {
    try { localStorage.setItem(welcomePreference, '1'); } catch { /* A presentation preference must not block setup. */ }
    focusChoice.current = true;
    setSeen(true);
  }
  function chooseCluster() {
    start(); setCluster(true); setConfirmChange(false); onPlace?.({ kind: 'cluster' });
  }
  function chooseComputer() {
    focusChoice.current = true; setCluster(false); onPlace?.({ kind: 'thisComputer' });
  }
  async function chooseLocal(action: 'start' | 'cancel' | 'change' = 'start') {
    if (mutation.current || !status || (action !== 'cancel' && busy)) return;
    if (action === 'start' && status.selected === 'localVm' && !['cancelled', 'closed', 'failed'].includes(status.localVm)) return;
    // Observation must not invalidate an explicit selection awaiting its reply.
    clearTimeout(timer.current);
    ++epoch.current;
    mutation.current = true;
    onWorkspaceAction?.(true);
    if (action === 'start') onStarting?.();
    setBusy(true); setSending(true); setPendingAction(action); setError(undefined);
    const result = await (action === 'cancel' ? cancelLocal() : action === 'change' ? changeLocal() : selectLocal());
    mutation.current = false;
    if (!mounted.current) return;
    onWorkspaceAction?.(false);
    setSending(false); setPendingAction(undefined); setConfirmChange(false);
    if (result.ok) { focusChoice.current = true; setStatus(result.status); }
    else { setError(result.message); }
    onStatus?.(result.ok ? result.status : undefined);
    const waiting = result.ok && pending(result.status.localVm);
    setBusy(waiting);
    if (result.ok) setCheck(value => value + 1);
  }

  const titleId = active ? 'page-title' : undefined;
  if (!status && !error) return <h1 id={titleId}>Checking your workspace…</h1>;
  if (welcome && status) return <Welcome titleId={titleId} onStart={start} onCluster={chooseCluster} unavailable={status.localVm === 'notIntegrated' ? runtimeCopy.notIntegrated : undefined} />;
  const selected = !cluster && status?.selected === 'localVm';
  return <>
    <div className="workspace-heading">
      <h1 ref={heading} tabIndex={-1} id={titleId}>{selected ? status?.localVm === 'ready' ? 'Choose the AI your agent uses' : status?.localVm === 'failed' ? workspaceRecovery(status.workspaceFailure).heading : 'Getting your workspace ready' : 'Where should your agents run?'}</h1>
      {selected && !['ready', 'failed', 'notIntegrated'].includes(status?.localVm ?? '') && <p className="muted">Meanwhile, meet the agent you'll test first.</p>}
      {!selected && <p className="muted">You can add more places later.</p>}
    </div>
    <div className={selected ? `setup workspace-setup-grid${status?.localVm === 'failed' ? ' workspace-setup-failed' : ''}` : 'workspace-choice'}>
      {!selected && <>
        <fieldset className="workspace-options" disabled={sending}>
          <legend className="workspace-sr-only">Where agents run</legend>
          <div className={`workspace-option${!cluster ? ' workspace-selected' : ''}`}>
            <input id="workspace-local" type="radio" name="workspace" aria-label="On this computer" aria-describedby="local-description" disabled={!cluster && (busy || !status || !!error)} checked={!cluster} onChange={chooseComputer} />
            <label htmlFor="workspace-local">On this computer</label><span className="pill">recommended</span>
            <div className="workspace-option-detail">
              <p id="local-description">Bridge runs a ready-made workspace quietly in the background. It needs virtualization turned on{requirements ? `, ${formatSize(requirements.memoryBytes)} of memory and a ${formatSize(requirements.imageBytes)} workspace image.` : '.'}</p>
              {requirements && <p>The data disk can grow to {formatSize(requirements.stateDiskBytes)}, in addition to the image.</p>}
              {requirements === undefined && <p>Reading this build's workspace size…</p>}
              {requirements === null && status?.localVm !== 'notIntegrated' && <p>Workspace size information is unavailable in this build.</p>}
              {!cluster && !error && status && requirements && status.localVm !== 'notIntegrated' && <Preflight requiredMemoryBytes={requirements.memoryBytes} onBlocked={setPreflightBlocked} />}
              <button className="workspace-disclosure" type="button" aria-expanded={inside} aria-controls="workspace-inside" onClick={() => setInside(value => !value)}>
                <svg className="icon" viewBox="0 0 24 24" aria-hidden="true"><path d={inside ? 'm6 15 6-6 6 6' : 'm6 9 6 6 6-6'} /></svg>What's inside
              </button>
              <p id="workspace-inside" hidden={!inside}>A small virtual machine with Orka, the open-source engine agents run on. Its files are checked against their published digests before it starts.</p>
            </div>
          </div>
          <div className={`workspace-option${cluster ? ' workspace-selected' : ''}`}>
            <input id="workspace-cluster" type="radio" name="workspace" aria-label="On a cluster you already have" checked={cluster} onChange={chooseCluster} />
            <label htmlFor="workspace-cluster">On a cluster you already have</label><Chip>for Kubernetes users</Chip>
            <p className="workspace-option-detail">Pick a kubectl context and Bridge uses the Orka on that cluster.</p>
            {cluster && <ClusterCard onPlace={place => onPlace?.(place)} onOpenAgent={onOpenExample} />}
          </div>
        </fieldset>
        {!cluster && <div className="workspace-choice-footer"><span className="muted">Choose where agents run</span>
          <button type="button" className="workspace-primary" disabled={busy || !status || !!error || (status.localVm !== 'notIntegrated' && requirements !== null && preflightBlocked)} onClick={() => { void chooseLocal(); }}>Continue</button>
        </div>}
        {!cluster && status?.localVm === 'notIntegrated' && <p className="muted">{runtimeCopy.notIntegrated}</p>}
      </>}
      {selected && status && <SetupProgress status={status} firstCheck={firstCheck} busy={sending} stopping={sending && pendingAction !== 'start'} error={error} modelState={modelMounted ? modelState : 'idle'} onCancel={() => { void chooseLocal('cancel'); }} onRestart={() => { void chooseLocal(); }} onCluster={chooseCluster} onChange={button => { changeInvoker.current = button; setConfirmChange(true); }} onOpenExample={onOpenExample}
        confirm={confirmChange && <section className="workspace-change" aria-label="Change where agents run">
          <button type="button" className="workspace-link" disabled={sending} onClick={chooseCluster}>Use a cluster you already have instead</button>
          <h2>Stop this workspace?</h2><p>Changing where agents run stops the workspace on this computer. Your files are kept.</p>
          <div className="workspace-step-actions"><button type="button" className="workspace-secondary" disabled={sending} onClick={() => { setConfirmChange(false); changeInvoker.current?.focus(); }}>Keep workspace running</button><button type="button" className="workspace-primary" disabled={sending} onClick={() => { void chooseLocal('change'); }}>Stop workspace and change</button></div>
        </section>} />}
      {status && modelMounted && <div className="setup-model-slot" hidden={cluster}><Model enabled={status.model === 'available'} queueable={canQueueModel(status)} paused={!!error || sending} away={cluster} onState={observeModel} onSettled={() => setCheck(value => value + 1)} /></div>}
      {selected && status && <SetupSteps status={status} firstCheck={firstCheck} busy={sending} stopping={sending && pendingAction !== 'start'} error={error} modelState={modelMounted ? modelState : 'idle'} onRestart={() => { void chooseLocal(); }} onCluster={chooseCluster} />}
      {selected && status && status.localVm !== 'ready' && status.localVm !== 'failed' && <Meet onRead={onOpenExample} onName={onExampleName} />}
      {!cluster && error && <div role="alert"><p>{error}</p>
        <button type="button" className="setup-button" disabled={busy}
          onClick={() => { setBusy(true); setError(undefined); setCheck(value => value + 1); }}>Check again</button>
      </div>}
    </div>
  </>;
}
