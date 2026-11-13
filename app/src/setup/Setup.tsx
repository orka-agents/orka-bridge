import { useCallback, useEffect, useRef, useState } from 'react';
import Model, { type ModelState } from './Model';
import Preflight from './Preflight';
import { type LocalVmState, type SetupStatus, cancelLocal, changeLocal, selectLocal, setupStatus } from './state';
import { Meet, SetupProgress, SetupSteps, type CheckStepState } from './Steps';
import { canQueueModel } from './progress';
import { armFirstTest, retireFirstTest } from './first-test';
import Welcome, { hasSeenWelcome, welcomePreference } from './Welcome';
import { formatSize, workspaceRequirements, type WorkspaceRequirements } from './requirements';
import ClusterCard from '../clusters/ClusterCard';
import { clusterPlaces, type ClusterPlaceSnapshot, type SelectedPlace } from '../clusters/api';
import Places from '../clusters/Places';
import type { Place } from '../overview/api';
import './welcome.css';
import { workspaceRecovery } from './recovery-info';

const pending = (state: LocalVmState) => ['preparing', 'installing', 'starting', 'closing'].includes(state);
const runtimeCopy = {
  notIntegrated: 'Workspace setup is not included in this build. Nothing will be downloaded or started.',
};
export default function Setup({ onOpenExample, onOpenAgent, onStatus, onModelState, onWelcome, onExampleName, firstCheck, onStarting, onWorkspaceAction, onPlace, place, onLocalView, active = true }: {
  onOpenExample: () => void; onOpenAgent?: () => void; active?: boolean;
  onStatus?: (status: SetupStatus | undefined) => void;
  onModelState?: (state: ModelState) => void;
  onWelcome?: (visible: boolean) => void;
  onExampleName?: (name: string | undefined) => void;
  firstCheck?: CheckStepState;
  onStarting?: () => void;
  onWorkspaceAction?: (pending: boolean) => void;
  onPlace?: (place: SelectedPlace) => void;
  onLocalView?: (visible: boolean) => void;
  place?: SelectedPlace;
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
  // Navigation is not destination selection. Standalone callers retain the same contract.
  const [ownPlace, setOwnPlace] = useState<SelectedPlace>({ kind: 'thisComputer' });
  const currentPlace = place ?? ownPlace;
  const cluster = currentPlace.kind === 'cluster';
  const [panel, setPanel] = useState<'places' | 'local' | 'cluster'>();
  const view = panel ?? (status?.selected === 'localVm' && status.localVm !== 'ready' ? 'local' : 'places');
  useEffect(() => {
    if (!panel && status?.selected === 'localVm' && status.localVm !== 'ready') setPanel('local');
  }, [panel, status?.selected, status?.localVm]);
  useEffect(() => { onLocalView?.(view === 'local'); }, [view, onLocalView]);
  const [entry, setEntry] = useState<{ revision: number; current?: SelectedPlace; intent?: string }>();
  const [snapshot, setSnapshot] = useState<ClusterPlaceSnapshot>();
  const [snapshotUnavailable, setSnapshotUnavailable] = useState(false);
  const [requirements, setRequirements] = useState<WorkspaceRequirements | null>();
  const [preflightBlocked, setPreflightBlocked] = useState(true);
  const epoch = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const heading = useRef<HTMLHeadingElement>(null);
  const focusChoice = useRef(false);
  const changeInvoker = useRef<HTMLButtonElement>(null);
  const confirmation = useRef<HTMLElement>(null);
  useEffect(() => { if (confirmChange && active) confirmation.current?.focus(); }, [confirmChange, active]);
  const welcome = !seen && !!status && !error && status.selected !== 'localVm';
  useEffect(() => { onWelcome?.(welcome); }, [welcome, onWelcome]);
  const readSnapshot = active && !welcome && (seen || !!error || !!status);
  useEffect(() => {
    if (!readSnapshot) return;
    let live = true;
    setSnapshot(value => value && { ...value, selectedKey: null });
    void clusterPlaces().then(value => {
      if (!live) return;
      setSnapshotUnavailable(!value);
      setSnapshot(previous => value ?? (previous && { ...previous, selectedKey: null }));
    });
    return () => { live = false; };
  }, [readSnapshot, view, currentPlace]);
  useEffect(() => {
    if (focusChoice.current && active) heading.current?.focus();
    focusChoice.current = false;
  }, [seen, status?.selected, status?.localVm, active, view]);
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
  // Model stays mounted from preparing through ready, so it owns its queued state; no Model means no model state.
  const modelMounted = status?.selected === 'localVm' && (status.localVm === 'ready' || canQueueModel(status));
  useEffect(() => { if (!modelMounted) observeModel('idle'); }, [modelMounted, observeModel]);

  function start() {
    try { localStorage.setItem(welcomePreference, '1'); } catch { /* A presentation preference must not block setup. */ }
    focusChoice.current = true;
    setSeen(true);
  }
  function publishPlace(value: SelectedPlace) { setOwnPlace(value); onPlace?.(value); }
  function chooseCluster(item?: Place, selected = false) {
    retireFirstTest();
    start(); setPanel('cluster'); setConfirmChange(false);
    setEntry(previous => ({ revision: (previous?.revision ?? 0) + 1, current: selected ? currentPlace : undefined, intent: item?.context }));
  }
  function chooseComputer() { publishPlace({ kind: 'thisComputer' }); }
  function back() { focusChoice.current = true; setPanel('places'); }
  function openLocal() { focusChoice.current = true; setPanel('local'); }
  async function chooseLocal(action: 'start' | 'cancel' | 'change' = 'start') {
    if (mutation.current || !status || (action !== 'cancel' && busy)) return;
    if (action === 'start' && status.selected === 'localVm' && !['cancelled', 'closed', 'failed'].includes(status.localVm)) return;
    // Observation must not invalidate an explicit selection awaiting its reply.
    const firstStart = action === 'start' && status.selected !== 'localVm';
    if (action === 'change') retireFirstTest();
    clearTimeout(timer.current);
    ++epoch.current;
    mutation.current = true;
    onWorkspaceAction?.(true);
    if (action === 'start') { chooseComputer(); onStarting?.(); }
    setBusy(true); setSending(true); setPendingAction(action); setError(undefined);
    const result = await (action === 'cancel' ? cancelLocal() : action === 'change' ? changeLocal() : selectLocal());
    mutation.current = false;
    if (!mounted.current) return;
    onWorkspaceAction?.(false);
    setSending(false); setPendingAction(undefined); setConfirmChange(false);
    if (result.ok) { if (firstStart) armFirstTest(); focusChoice.current = true; setStatus(result.status); if (action === 'change') setPanel('places'); }
    else { setError(result.message); }
    onStatus?.(result.ok ? result.status : undefined);
    const waiting = result.ok && pending(result.status.localVm);
    setBusy(waiting);
    if (result.ok) setCheck(value => value + 1);
  }

  const titleId = active ? 'page-title' : undefined;
  if (!status && !error) return <h1 id={titleId}>Checking your workspace…</h1>;
  if (welcome && status) return <Welcome titleId={titleId} onStart={start} onCluster={() => chooseCluster()} unavailable={status.localVm === 'notIntegrated' ? runtimeCopy.notIntegrated : undefined} />;
  const selected = status?.selected === 'localVm';
  return <div className="places-workspace">
    {view !== 'places' && <button type="button" className="workspace-link places-back" onClick={back}>Back to places</button>}
    <div className="workspace-heading places-heading">
      <h1 ref={heading} tabIndex={-1} id={titleId}>{view === 'places' ? 'Places for your agents' : view === 'cluster' ? entry?.current ? 'Manage your cluster' : entry?.intent ? 'Connect a cluster' : 'Add a place' : selected ? status?.localVm === 'ready' ? modelState === 'connected' ? 'Run your first test' : 'Connect the AI' : status?.localVm === 'failed' ? workspaceRecovery(status.workspaceFailure).heading : 'Getting your workspace ready' : 'Set up this computer'}</h1>
      {view === 'places' && <button type="button" className="workspace-primary" onClick={() => chooseCluster()}>Add a place</button>}
      {view === 'local' && selected && !['ready', 'failed', 'notIntegrated'].includes(status?.localVm ?? '') && <p className="muted">Meanwhile, meet the agent you'll test first.</p>}
      {view === 'local' && selected && status?.localVm === 'ready' && !firstCheck && <p className="muted">On first setup, Bridge tests the bundled example after you connect the AI, then opens its result in Designer.</p>}
    </div>
    {view === 'places' && <Places snapshot={snapshot} unavailable={snapshotUnavailable} status={error ? undefined : status} model={modelState} place={currentPlace} onLocal={openLocal} onCluster={chooseCluster} />}
    <div hidden={view !== 'local'}>
      {cluster && <button type="button" className="workspace-secondary" onClick={chooseComputer}>Use this computer</button>}
      <div className={selected ? `setup workspace-setup-grid${status?.localVm === 'failed' ? ' workspace-setup-failed' : ''}` : 'workspace-choice local-preflight'}>
        {!selected && <>
          <section className="card">
            <h2>This computer</h2>
            <p>Your workspace is Orka and the tools your agents use, running quietly on this computer. It needs virtualization turned on.</p>
            <dl className="workspace-sizes">
              <div><dt>Memory</dt><dd>{requirements ? formatSize(requirements.memoryBytes) : requirements === undefined ? 'Reading…' : 'Memory requirements will be checked before setup'}</dd></div>
              <div><dt>Workspace image</dt><dd>{requirements ? formatSize(requirements.imageBytes) : requirements === undefined ? 'Reading…' : 'Workspace size will be shown before download'}</dd></div>
              <div><dt>Data disk, up to</dt><dd>{requirements ? formatSize(requirements.stateDiskBytes) : requirements === undefined ? 'Reading…' : 'Disk size will be shown before setup'}</dd></div>
            </dl>
            {view === 'local' && !error && status && requirements && status.localVm !== 'notIntegrated' && <Preflight requiredMemoryBytes={requirements.memoryBytes} onBlocked={setPreflightBlocked} />}
            <button className="workspace-disclosure" type="button" aria-expanded={inside} aria-controls="workspace-inside" onClick={() => setInside(value => !value)}>
              <svg className="icon" viewBox="0 0 24 24" aria-hidden="true"><path d={inside ? 'm6 15 6-6 6 6' : 'm6 9 6 6 6-6'} /></svg>What's inside
            </button>
            <p id="workspace-inside" hidden={!inside}>Orka, the open-source engine agents run on, in a separate workspace. Bridge checks its files before starting it.</p>
          </section>
          <div className="workspace-choice-footer"><span className="muted">Set up on this computer</span>
            <button type="button" className="workspace-primary" disabled={busy || !status || !!error || (status.localVm !== 'notIntegrated' && requirements !== null && preflightBlocked)} onClick={() => { void chooseLocal(); }}>Continue</button>
          </div>
          {status?.localVm === 'notIntegrated' && <p className="muted">{runtimeCopy.notIntegrated}</p>}
        </>}
        {selected && status && <SetupProgress status={status} firstCheck={firstCheck} busy={sending} stopping={sending && pendingAction !== 'start'} error={error} modelState={modelMounted ? modelState : 'idle'} onCancel={() => { void chooseLocal('cancel'); }} onRestart={() => { void chooseLocal(); }} onCluster={() => chooseCluster()} onChange={button => { changeInvoker.current = button; setConfirmChange(true); }} onOpenExample={onOpenExample}
          confirm={confirmChange && <section ref={confirmation} tabIndex={-1} className="workspace-change" aria-label="Change where agents run">
            <h2>Keep this workspace running?</h2><p>You can choose a cluster without stopping this workspace, or stop it and return to the places list. Your downloaded workspace files and agent files are kept.</p>
            <div className="workspace-step-actions"><button type="button" className="workspace-primary" disabled={sending} onClick={() => { setConfirmChange(false); changeInvoker.current?.focus(); }}>Keep workspace running</button><button type="button" className="workspace-secondary" disabled={sending} onClick={() => { void chooseLocal('change'); }}>Stop workspace and choose a place</button></div>
            <button type="button" className="workspace-link" disabled={sending} onClick={() => chooseCluster()}>Choose a cluster, keep this workspace running</button>
          </section>} />}
        {status && modelMounted && <div className="setup-model-slot"><Model enabled={status.model === 'available'} queueable={canQueueModel(status)} paused={!!error || sending} away={cluster} primary={!confirmChange} onState={observeModel} onSettled={() => setCheck(value => value + 1)} /></div>}
        {selected && status && <SetupSteps status={status} firstCheck={firstCheck} busy={sending} stopping={sending && pendingAction !== 'start'} error={error} modelState={modelMounted ? modelState : 'idle'} onRestart={() => { void chooseLocal(); }} onCluster={() => chooseCluster()} />}
        {selected && status && status.localVm !== 'ready' && status.localVm !== 'failed' && <Meet onRead={onOpenExample} onName={onExampleName} />}
      </div>
    </div>
    {entry && <ClusterCard active={active && view === 'cluster'} entry={entry} selection={currentPlace} onPlace={publishPlace} onOpenAgent={onOpenAgent ?? onOpenExample} />}
    {error && <div role="alert"><p>{error}</p>
      <button type="button" className="setup-button" disabled={busy}
        onClick={() => { setBusy(true); setError(undefined); setCheck(value => value + 1); }}>Check again</button>
    </div>}
  </div>;
}
