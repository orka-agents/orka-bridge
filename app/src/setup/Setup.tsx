import { type Ref, useCallback, useEffect, useImperativeHandle, useRef, useState } from 'react';
import type { ClustersView } from '../chrome/history';
import { type ClusterPlaceSnapshot, clusterPlaces, type SelectedPlace } from '../clusters/api';
import ClusterCard from '../clusters/ClusterCard';
import Places from '../clusters/Places';
import { Button } from '../components/Button';
import { Disclosure } from '../components/Disclosure';
import type { Place } from '../overview/api';
import { armFirstTest, firstTestAttempted, firstTestPending, rearmFirstTest, retireFirstTest } from './first-test';
import Model, { type ModelState } from './Model';
import Preflight from './Preflight';
import { canQueueModel, setupView } from './progress';
import { setupNeeds, type WorkspaceRequirements, workspaceRequirements } from './requirements';
import { type CheckStepState, Meet, SetupProgress } from './Steps';
import { cancelLocal, type LocalVmState, type SetupStatus, selectLocal, setupStatus } from './state';
import Welcome, { hasSeenWelcome, welcomePreference } from './Welcome';
import Workspace from './Workspace';
import { localUnavailable, unavailableCopy } from './availability';
import Upgrade, { UpgradeObservation, type UpgradeObservationState } from './Upgrade';
import { upgradeCall, type UpgradeStatus } from './upgrade-api';
import './welcome.css';
import type { RunSnapshot } from '../designer/run-api';
import FirstPass from './FirstPass';
import { workspaceRecovery } from './recovery-info';
import SetUpLater from '../firstrun/SetUpLater';
import FreshWorkspace from './FreshWorkspace';
import { type FreshStatus, freshDetails, startFresh } from './fresh';

const pending = (state: LocalVmState) => ['preparing', 'installing', 'starting', 'closing'].includes(state);
const failureKey = (status?: SetupStatus) => status?.workspaceFailure ? `${status.workspaceFailure.operation}/${status.workspaceFailure.cause}` : undefined;
export type UpgradeNotice = 'recovery' | 'required' | 'error' | 'unknown' | 'checking' | 'busy';
// showCluster opens a remembered cluster's card, or Add a cluster without one, as the Clusters list does; showLocal opens This computer.
export type LocalClusterControl = { showPlaces: () => void; showLocal: () => void; showCluster: (place?: Place, selected?: boolean) => void };
export default function Setup({ ref, onOpenExample, onOpenAgent, onStatus, onModelState, onWelcome, onLaunch, onExampleName, firstCheck, firstPass, modelUnavailable, firstTestResult, initialAutomatic, onCheckFirstTest, onStarting, onWorkspaceAction, onPlace, place, onLocalView, onWorkspaceMode, guided = false, onSetUpLater, active = true, onView }: {
  ref?: Ref<LocalClusterControl>;
  onOpenExample: () => void; onOpenAgent?: () => void; active?: boolean;
  onStatus?: (status: SetupStatus | undefined) => void;
  onModelState?: (state: ModelState) => void;
  onWelcome?: (visible: boolean) => void;
  // Reported once at launch: true when something is already set up, so Bridge can open on Home.
  onLaunch?: (home: boolean) => void;
  onExampleName?: (name: string | undefined) => void;
  firstCheck?: CheckStepState;
  firstPass?: RunSnapshot;
  modelUnavailable?: boolean;
  firstTestResult?: { failed: number }; initialAutomatic?: boolean; onCheckFirstTest?: () => void;
  onStarting?: () => void;
  onWorkspaceAction?: (pending: boolean, stopping?: boolean, updating?: boolean, notice?: UpgradeNotice) => void;
  onPlace?: (place: SelectedPlace) => void;
  onLocalView?: (visible: boolean) => void;
  /** The view shown, for Back and Forward: the list, This computer, or a cluster's page (no key: Add a cluster). */
  onView?: (view: ClustersView) => void;
  onWorkspaceMode?: (returning: boolean) => void;
  place?: SelectedPlace;
  // The guided first run: reading the example's instructions stays in place, and screens where setup has stopped offer Set up later.
  guided?: boolean; onSetUpLater?: () => void;
}) {
  const [status, setStatus] = useState<SetupStatus>();
  const [upgrade, setUpgrade] = useState<UpgradeStatus>();
  const [upgradeBusy, setUpgradeBusy] = useState(false);
  const [upgradeKnown, setUpgradeKnown] = useState(false);
  const [upgradeError, setUpgradeError] = useState<string>();
  const [upgradeCheck, setUpgradeCheck] = useState(0);
  const upgradeEpoch = useRef(0);
  const manualEpoch = useRef(0);
  const manualPending = useRef(false);
  const manualFailed = useRef(false);
  const manualIdentityless = useRef(false);
  const upgradeIdentity = useRef<string | null>(undefined);
  const [upgradeObservation, setUpgradeObservation] = useState<UpgradeObservationState>({ pending: false });
  const workspaceReplaced = useRef(false);
  const ordinaryStart = useRef<string | null>(undefined);
  const ordinaryRestartEpoch = useRef(0);
  const upgradeAction = useCallback((value: boolean) => {
    // A mutation invalidates both kinds of reads, including a lost mutation reply.
    ++epoch.current; ++upgradeEpoch.current; ++manualEpoch.current;
    manualPending.current = false; manualFailed.current = false; manualIdentityless.current = false; setUpgradeObservation({ pending: false });
    ordinaryStart.current = undefined;
    setUpgradeBusy(value); setUpgradeKnown(false);
    setCheck(check => check + 1);
  }, []);
  const upgrading = upgradeBusy || !!upgrade?.busy && !!upgrade.canCancel && !upgradeObservation.pending;
  const occupied = !!upgrade?.busy && !upgrading && !upgradeObservation.pending;
  const recoveryOnly = !!upgrade && ['sourceSelected', 'sourceRetained', 'incomplete', 'restoring', 'restored', 'conflict'].includes(upgrade.state);
  const notice: UpgradeNotice | undefined = upgradeObservation.pending ? 'checking' : occupied ? 'busy' : !upgradeKnown && (status?.workspaceReplaced || !!upgrade && upgrade.state !== 'none' || firstTestPending()) ? 'unknown' : recoveryOnly ? 'recovery' : upgrade?.state === 'eligible' ? 'required' : upgrade?.error ? 'error' : undefined;
  const reportedUpgrade = useRef(false);
  const [setupError, setError] = useState<string>();
  const retryUpgradeStatus = !setupError && !!status && !status.workspaceReplaced && (status.selected === null || !!upgrade);
  const error = setupError ?? (status?.workspaceReplaced ? upgradeError : undefined);
  const [busy, setBusy] = useState(true);
  const [sending, setSending] = useState(false);
  const [pendingAction, setPendingAction] = useState<'start' | 'cancel'>();
  // Start a fresh workspace: asked from the stuck AI card, a failure the ordinary recovery can't pass, or Manage.
  const [freshAsk, setFreshAsk] = useState(false);
  const [freshWorking, setFreshWorking] = useState(false);
  const [freshReply, setFreshReply] = useState<FreshStatus>();
  const [freshDismissed, setFreshDismissed] = useState<string>();
  // A new workspace gets a new AI card: nothing the old one learned applies to it.
  const [freshGeneration, setFreshGeneration] = useState(0);
  // The failure a Try again was pressed on; the same failure again means ordinary recovery can't pass it.
  const [retriedFailure, setRetriedFailure] = useState<string>();
  useEffect(() => {
    if (sending || upgrading || notice || reportedUpgrade.current) onWorkspaceAction?.(sending || upgrading || occupied || upgradeObservation.pending, sending && pendingAction !== 'start', upgrading, notice);
    reportedUpgrade.current = sending || upgrading || !!notice;
  }, [sending, pendingAction, upgrading, occupied, upgradeObservation.pending, notice, onWorkspaceAction]);
  const [workspace, setWorkspace] = useState(false);
  const initialStatus = useRef(true);
  const returningSession = useRef(firstTestAttempted());
  useEffect(() => { onWorkspaceMode?.(workspace); }, [workspace, onWorkspaceMode]);
  const mutation = useRef(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const [modelState, setModelState] = useState<ModelState>('idle');
  const [placeRecovery, setPlaceRecovery] = useState<HTMLDivElement | null>(null);
  const observeModel = useCallback((value: ModelState) => { setModelState(value); onModelState?.(value); }, [onModelState]);
  const [check, setCheck] = useState(0);
  const [setupCheck, setSetupCheck] = useState(0);
  const [seen] = useState(hasSeenWelcome);
  // Welcome's buttons dismiss it for the rest of this run, even while nothing is set up yet.
  const [dismissed, setDismissed] = useState(false);
  // Whether a cluster was remembered when Bridge opened; null when that couldn't be read.
  const [launchPlaces, setLaunchPlaces] = useState<boolean | null>();
  const [inside, setInside] = useState(false);
  // Navigation is not destination selection. Standalone callers retain the same contract.
  const [ownPlace, setOwnPlace] = useState<SelectedPlace>({ kind: 'thisComputer' });
  const currentPlace = place ?? ownPlace;
  const currentPlaceRef = useRef(currentPlace);
  useEffect(() => { currentPlaceRef.current = currentPlace; }, [currentPlace]);
  const cluster = currentPlace.kind === 'cluster';
  const [panel, setPanel] = useState<'places' | 'local' | 'cluster'>();
  const [payoffDismissed, setPayoffDismissed] = useState(false);
  const shownPayoff = useRef(false);
  const [providerChange, setProviderChange] = useState(0);
  const view = panel ?? (status?.selected === 'localVm' && status.localVm !== 'ready' ? 'local' : 'places');
  const displayError = error ?? (view === 'local' && retryUpgradeStatus ? upgradeError : undefined);
  useEffect(() => {
    if (!panel && status?.selected === 'localVm' && status.localVm !== 'ready') setPanel('local');
  }, [panel, status?.selected, status?.localVm]);
  useEffect(() => { onLocalView?.(view === 'local'); }, [view, onLocalView]);
  const eligiblePayoff = !!firstPass && active && view === 'local' && firstCheck === 'passed' && modelState === 'connected' && !error && !sending && status?.localVm === 'ready';
  useEffect(() => {
    if (!firstPass) return;
    if (!active || view !== 'local' || shownPayoff.current && !eligiblePayoff) setPayoffDismissed(true);
    if (eligiblePayoff) shownPayoff.current = true;
  }, [firstPass, active, view, eligiblePayoff]);
  const [entry, setEntry] = useState<{ revision: number; current?: SelectedPlace; remembered?: Place }>();
  const [clusterName, setClusterName] = useState<string>();
  const viewedCluster = view === 'cluster' ? entry?.remembered?.key ?? '' : undefined;
  useEffect(() => { onView?.(viewedCluster === undefined ? { kind: view === 'local' ? 'local' : 'places' } : { kind: 'cluster', key: viewedCluster || undefined }); }, [view, viewedCluster, onView]);
  const [snapshot, setSnapshot] = useState<ClusterPlaceSnapshot>();
  const [snapshotUnavailable, setSnapshotUnavailable] = useState(false);
  const snapshotEpoch = useRef(0);
  const [requirements, setRequirements] = useState<WorkspaceRequirements | null>();
  const [preflightBlocked, setPreflightBlocked] = useState(true);
  const [preflightFailed, setPreflightFailed] = useState(false);
  const [chosenLocation, setLocation] = useState<'local' | 'cluster'>('local');
  // A build without the local cluster can't choose this computer: the cluster is the only choice.
  const localOff = localUnavailable(status);
  const location = localOff ? 'cluster' : chosenLocation;
  const epoch = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const heading = useRef<HTMLHeadingElement>(null);
  const focusChoice = useRef(false);
  // Nothing set up: no local workspace and no cluster remembered at launch. An unreadable list falls back to the stored dismissal.
  const unchosen = !dismissed && !!status && !error && status.selected !== 'localVm';
  const welcome = unchosen && (launchPlaces === false || launchPlaces === null && !seen);
  useEffect(() => { onWelcome?.(welcome); }, [welcome, onWelcome]);
  const readSnapshot = active && (seen || !!error || !!status);
  useEffect(() => {
    if (!readSnapshot) return;
    const request = ++snapshotEpoch.current;
    setSnapshot(value => value && { ...value, selectedKey: null });
    void clusterPlaces().then(value => {
      if (request !== snapshotEpoch.current) return;
      setSnapshotUnavailable(!value);
      setLaunchPlaces(previous => previous ?? (value ? value.places.length > 0 : null));
      setSnapshot(previous => value ?? (previous && { ...previous, selectedKey: null }));
    });
    return () => { snapshotEpoch.current++; };
  }, [readSnapshot, view, currentPlace]);
  useEffect(() => {
    if (focusChoice.current && active) heading.current?.focus();
    focusChoice.current = false;
  }, [dismissed, status?.selected, status?.localVm, active, view]);
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
      const returning = returningSession.current;
      if (result.ok) {
        if (initialStatus.current) {
          // A saved choice also exists mid-setup. Only a retired first-test intent proves a returning session.
          setWorkspace(result.status.selected === 'localVm' && result.status.localVm !== 'notIntegrated' && returning); initialStatus.current = false;
        }
        workspaceReplaced.current ||= !!result.status.workspaceReplaced;
        if (workspaceReplaced.current) result.status.workspaceReplaced = true;
        setStatus(result.status); setError(undefined);
      }
      else {
        // An unreadable observation pauses actions, not an already admitted connection.
        setError(result.message);
      }
      const waiting = result.ok && pending(result.status.localVm);
      if (result.ok && !waiting) ordinaryStart.current = undefined;
      setBusy(waiting);
      timer.current = setTimeout(() => setSetupCheck(value => value + 1), waiting ? 250 : 1000);
      onStatus?.(result.ok ? result.status : undefined);
    });
    return () => { epoch.current++; clearTimeout(timer.current); };
  }, [check, setupCheck, onStatus]);
  useEffect(() => {
    const request = ++upgradeEpoch.current;
    let timer: ReturnType<typeof setTimeout>;
    void upgradeCall('status').then(update => {
      if (request !== upgradeEpoch.current) return;
      setUpgradeKnown(update.ok);
      setUpgradeError(update.ok ? undefined : update.message);
      if (update.ok) {
        observeUpgrade(update.status);
        if (update.status.state !== 'none') retireFirstTest();
      } else if (!manualPending.current && !manualFailed.current) setUpgrade(previous => previous && previous.state !== 'none'
        ? { ...previous, state: 'conflict', canReplace: false, canRestore: false, canCancel: false, busy: false, error: 'unknown' } : previous);
      timer = setTimeout(() => setUpgradeCheck(value => value + 1), 1000);
    });
    return () => { ++upgradeEpoch.current; clearTimeout(timer); };
  }, [check, upgradeCheck]);
  function observeUpgrade(value: UpgradeStatus) {
    if (value.transaction !== ordinaryStart.current || value.canCancel || value.state !== 'fresh') ordinaryStart.current = undefined;
    // Cached observations may discover another workspace, but never certify a manual recheck.
    if ((value.transaction !== null || !manualFailed.current && !manualPending.current) && upgradeIdentity.current !== undefined && upgradeIdentity.current !== value.transaction) {
      ++manualEpoch.current; manualPending.current = false; manualFailed.current = false; manualIdentityless.current = false;
      setUpgradeObservation({ pending: false });
    }
    if ((manualFailed.current || manualPending.current) && upgradeIdentity.current && (value.transaction === null || manualIdentityless.current)) {
      // Identity loss is not another workspace or proof of repair. Keep history, not authority.
      manualIdentityless.current = true;
      setUpgrade(previous => previous && { ...previous, state: 'conflict', canReplace: false, canRestore: false, canCancel: false, busy: false, error: value.error ?? 'unknown' });
      return;
    }
    upgradeIdentity.current = value.transaction;
    setUpgrade(value);
  }
  async function checkWorkspace() {
    if (manualPending.current) return;
    const request = ++manualEpoch.current;
    manualPending.current = true;
    setUpgradeObservation(previous => ({ ...previous, pending: true, checked: false }));
    // Setup snapshots and memory observations remain independent of the owner query.
    setCheck(value => value + 1);
    const result = await upgradeCall('refresh');
    if (!mounted.current || request !== manualEpoch.current) return;
    manualPending.current = false;
    if (result.ok) {
      manualFailed.current = false; manualIdentityless.current = false;
      ++upgradeEpoch.current;
      observeUpgrade(result.status);
      setUpgradeKnown(true); setUpgradeError(undefined);
      setUpgradeObservation({ pending: false, checked: true });
      if (result.status.state !== 'none') retireFirstTest();
      setUpgradeCheck(value => value + 1);
    } else {
      manualFailed.current = true;
      setUpgradeObservation({ pending: false, error: 'Could not check workspace files. They may still be in use or unavailable. Check again before continuing.' });
    }
  }
  const ordinaryRecovery = upgradeKnown
    ? !!upgrade && ['none', 'fresh', 'consumed'].includes(upgrade.state)
    : status?.selected === 'localVm' && !status.workspaceReplaced && !upgrade;
  // Home only for a finished local workspace with nothing pending, or a remembered cluster without one.
  // Setup in progress, a failure or a workspace update keeps Bridge on Clusters, as before.
  const launchHome = !status ? error ? false : undefined
    : status.selected === 'localVm'
      ? status.localVm !== 'ready' || !!status.workspaceReplaced || firstTestPending() ? false
        : upgradeKnown ? upgrade?.state === 'none' && !upgrade.busy : upgradeError !== undefined ? false : undefined
      : error ? false : launchPlaces === undefined ? undefined : launchPlaces === true;
  const [launched, setLaunched] = useState(false);
  useEffect(() => {
    if (launched || launchHome === undefined) return;
    setLaunched(true); onLaunch?.(launchHome);
  }, [launched, launchHome, onLaunch]);
  const launchPending = !launched && launchHome === undefined && !!status && !error;
  // Model stays mounted from preparing through ready, so it owns its queued state; no Model means no model state.
  const modelMounted = (!status?.workspaceReplaced || upgradeKnown && !!upgrade) && (!upgrade || ['none', 'fresh', 'consumed'].includes(upgrade.state)) && !upgradeBusy && !upgrade?.busy && upgrade?.credential !== 'attention' && upgrade?.credential !== 'eligible' && status?.selected === 'localVm' && (status.localVm === 'ready' || (!status.workspaceReplaced && canQueueModel(status)));
  useEffect(() => { if (!modelMounted) observeModel('idle'); }, [modelMounted, observeModel]);

  function start() {
    try { localStorage.setItem(welcomePreference, '1'); } catch { /* A presentation preference must not block setup. */ }
    focusChoice.current = true;
    setDismissed(true);
  }
  function publishPlace(value: SelectedPlace) { setOwnPlace(value); onPlace?.(value); }
  function chooseCluster(item?: Place, selected = false) {
    retireFirstTest();
    start(); setPanel('cluster'); setClusterName(item?.context ?? (selected && currentPlace.kind === 'cluster' ? currentPlace.target?.context_name : undefined));
    setEntry(previous => ({ revision: (previous?.revision ?? 0) + 1, current: selected ? currentPlace : undefined, remembered: item }));
  }
  function chooseComputer() { publishPlace({ kind: 'thisComputer' }); }
  function openExample() { if (firstPass) setPayoffDismissed(true); chooseComputer(); onOpenExample(); }
  function back() { focusChoice.current = true; setPanel('places'); }
  function openLocal() { focusChoice.current = true; if (status?.selected === 'localVm' && status.localVm !== 'notIntegrated' && firstCheck && !firstTestPending() && (!firstPass || payoffDismissed)) setWorkspace(true); setLocation('local'); setPreflightBlocked(true); setPanel('local'); }
  function removed(key: string, targetId: string | undefined, navigate: boolean) {
    const current = currentPlaceRef.current;
    if (targetId && current.kind === 'cluster' && current.target?.id === targetId) publishPlace({ kind: 'cluster' });
    snapshotEpoch.current++;
    setSnapshot(value => value && { ...value, places: value.places.filter(p => p.key !== key), selectedKey: value.selectedKey === key ? null : value.selectedKey });
    if (navigate) back();
  }
  // Only an explicit ordinary start of this Fresh transaction owns ordinary Cancel.
  // Observed Version startup/closing, and UI-owned upgrade mutations, stay fenced.
  const ordinaryStarting = ordinaryStart.current !== undefined && upgradeKnown && upgrade?.state === 'fresh'
    && upgrade.transaction === ordinaryStart.current && !upgrade.canCancel && !!status && ['preparing', 'installing', 'starting'].includes(status.localVm);
  async function chooseLocal(action: 'start' | 'cancel' = 'start') {
    if (mutation.current || upgradeBusy || upgrade?.busy && !(action === 'cancel' && ordinaryStarting) || !status || (action !== 'cancel' && busy)) return;
    if (action === 'start' && !ordinaryRecovery) return;
    if (action === 'start' && status.selected === 'localVm' && !['cancelled', 'closed', 'failed'].includes(status.localVm)) return;
    // Observation must not invalidate an explicit selection awaiting its reply.
    const firstStart = action === 'start' && status.selected !== 'localVm';
    const retrying = action === 'start' && status.localVm === 'failed' ? failureKey(status) : undefined;
    if (firstStart) armFirstTest();
    clearTimeout(timer.current);
    ++epoch.current;
    mutation.current = true;
    onWorkspaceAction?.(true, action !== 'start');
    if (action === 'start') { chooseComputer(); if (!workspace) onStarting?.(); }
    setBusy(true); setSending(true); setPendingAction(action); setError(undefined);
    const result = await (action === 'start' ? selectLocal() : cancelLocal());
    mutation.current = false;
    if (!mounted.current) return;
    onWorkspaceAction?.(false);
    setSending(false); setPendingAction(undefined);
    if (result.ok) {
      if (action === 'start') { ++ordinaryRestartEpoch.current; setRetriedFailure(retrying); }
      focusChoice.current = true; setStatus(result.status);
    }
    else { setError(result.message); }
    onStatus?.(result.ok ? result.status : undefined);
    const waiting = result.ok && pending(result.status.localVm);
    ordinaryStart.current = action === 'start' && waiting && upgrade?.state === 'fresh' ? upgrade.transaction : undefined;
    setBusy(waiting);
    if (result.ok) setCheck(value => value + 1);
    else timer.current = setTimeout(() => setCheck(value => value + 1), 1000);
  }

  const changing = sending || upgradeBusy || !!upgrade?.busy || upgradeObservation.pending || freshWorking;
  const freshStatus = status?.workspaceFresh ?? freshReply;
  const freshKey = !freshStatus ? undefined : freshStatus.state === 'replaced' ? `replaced/${freshStatus.transaction}/${!freshStatus.cleaning && freshStatus.reclaimable.length > 0}` : freshStatus.state === 'refused' ? `refused/${freshStatus.reason}/${freshStatus.reclaimable?.length ?? 0}` : freshStatus.state;
  const freshShown = freshStatus && freshKey !== freshDismissed ? freshStatus : undefined;
  // Native says when the action can't start (a workspace update's history); then it isn't offered.
  const freshPlan = status?.workspaceFreshPlan;
  const freshBlocked = freshPlan?.blocked;
  const freshLines = freshDetails(freshStatus, freshPlan);
  // An unfinished fresh workspace is the only way forward: the workspace card's own retry can't start it.
  const freshUnfinished = freshStatus?.state === 'interrupted' || freshStatus?.state === 'conflict';
  // Offered beside a failure the ordinary recovery can't get past: different cluster files, or the same failure after a retry.
  const failure = status?.localVm === 'failed' && !status.preflightFailure ? status.workspaceFailure : undefined;
  const freshOffered = !!failure && !['stopUnconfirmed', 'forcedStop', 'network'].includes(failure.cause) && !['download', 'requirements', 'stop'].includes(failure.operation)
    && (['workspaceVersionUnsupported', 'workspaceUpdateRequired'].includes(failure.cause) || retriedFailure === failureKey(status)) && !freshBlocked;
  function askFresh() {
    if (changing || !status || status.selected !== 'localVm' || freshBlocked) return;
    setFreshAsk(true); setFreshDismissed(undefined); setPanel('local');
  }
  async function confirmFresh() {
    if (mutation.current || freshWorking) return;
    setFreshAsk(false); setFreshWorking(true); setFreshDismissed(undefined); setFreshReply(undefined);
    clearTimeout(timer.current); ++epoch.current;
    mutation.current = true;
    onWorkspaceAction?.(true, true);
    const result = await startFresh();
    mutation.current = false;
    if (!mounted.current) return;
    onWorkspaceAction?.(false);
    setFreshWorking(false); setFreshReply(result);
    if (result.state === 'replaced') {
      // The new workspace sets up as on a new computer: the ordinary first-setup screen, its AI card ready to connect.
      setWorkspace(false); setRetriedFailure(undefined); setFreshGeneration(value => value + 1);
      // A new workspace runs its first test automatically again, once the AI is connected.
      rearmFirstTest();
    }
    setCheck(value => value + 1);
  }
  const upgradePanel = upgrade && upgrade.state !== 'none' ? <Upgrade status={upgrade} savedConnectionInAiCard={workspace} ordinaryRestartEpoch={ordinaryRestartEpoch.current} observation={upgradeObservation} canUseCredential={status?.localVm === 'ready' || upgrade.state === 'consumed'} disabled={sending || (!!status && pending(status.localVm)) || !!error || !upgradeKnown} onBusy={upgradeAction} onChange={value => { observeUpgrade(value); setCheck(check => check + 1); }} onCheck={() => { void checkWorkspace(); }} /> : undefined;
  const continueAction = <Button type="button" kind="primary" aria-describedby={location === 'local' && requirements ? 'preflight-reason' : undefined} disabled={busy || !status || !!error || (location === 'local' && (changing || !ordinaryRecovery || status.localVm !== 'notIntegrated' && requirements !== null && preflightBlocked))} onClick={() => { if (location === 'cluster') chooseCluster(); else void chooseLocal(); }}>Continue</Button>;
  useImperativeHandle(ref, () => ({ showPlaces() { if (view !== 'places') back(); }, showLocal() { openLocal(); }, showCluster(item?: Place, selected = false) { chooseCluster(item, selected); } }));
  const titleId = active ? 'page-title' : undefined;
  if (!status && !error || launchPending) return <><h1 id={titleId}>Clusters</h1><p role="status">Checking the workspace…</p></>;
  if (welcome && status) return <Welcome titleId={titleId} onStart={start} unavailable={localOff} requirements={requirements} />;
  const selected = status?.selected === 'localVm';
  const modelRecovery = ['failed', 'resetFailed', 'removalFailed', 'resetting'].includes(modelState);
  // A failed connection keeps the stage's name; the AI card beside the steps says what went wrong.
  const aiHeadings: Partial<Record<ModelState, string>> = {
    resetFailed: "The AI connection couldn't be reset",
    removalFailed: "The saved sign-in couldn't be removed",
    resetting: 'Resetting the AI connection',
    connected: 'Run your first agent',
  };
  const aiHeading = aiHeadings[modelState] ?? 'Connect the AI';
  const presentation = setupView(error ? undefined : status, { stopping: sending && pendingAction !== 'start', starting: sending && pendingAction === 'start', workspace, model: modelState, check: firstCheck, initialAutomatic });
  const upgradeAttention = !error && (upgrading || !!notice && (notice !== 'unknown' || status?.workspaceReplaced));
  const upgradeLine = upgrading ? 'Updating workspace' : upgradeObservation.pending ? 'Checking workspace files' : occupied ? 'Workspace is busy' : notice === 'recovery' ? 'Workspace recovery needs attention' : 'Workspace update needs attention';
  // The returning workspace's line repeats its detail ("Workspace ready"), so the list shows the detail once.
  const placesView = upgradeAttention ? { ...presentation, line: upgradeLine, detail: 'Your agent files are kept.', status: upgrading || occupied || upgradeObservation.pending ? 'running' as const : 'needs-you' as const, done: false, action: 'Manage' } : workspace ? { ...presentation, line: presentation.detail } : presentation;
  const updateBlocked = !!notice && (notice !== 'unknown' || !!status?.workspaceReplaced);
  // On This computer, an update's reusable connection is offered in the AI card, unusable until the workspace starts.
  const savedConnection = workspace && upgrade?.state === 'eligible' && upgrade.credential === 'eligible';
  // With no other clusters, Add a cluster is the page's next step unless This computer still needs setting up.
  const noClusters = !snapshotUnavailable && snapshot?.persistenceState === 'complete' && snapshot.places.length === 0;
  const localNext = !modelRecovery && !error && !!status && !localOff && !placesView.done && (selected || noClusters);
  const showPayoff = !workspace && !payoffDismissed && eligiblePayoff;
  // Outside the guided first run, a first workspace that is getting ready is two cards: the workspace and the AI, as the board shows.
  const twoCards = !guided && !workspace && selected && !!status && ['preparing', 'installing', 'starting'].includes(status.localVm) && !upgradePanel;
  const localStage = status?.localVm === 'ready' ? aiHeading : status?.localVm === 'failed'
    ? status.preflightFailure ? 'Check this computer' : workspaceRecovery(status.workspaceFailure).heading : 'Getting your workspace ready';
  return <div className="places-workspace">
    {(view === 'cluster' || view === 'local' && (workspace || twoCards) && !showPayoff) && <Button type="button" kind="quiet" className="places-back" onClick={back}>Back to clusters</Button>}
    <div className="workspace-heading places-heading" hidden={!!showPayoff}>
      <h1 ref={heading} tabIndex={-1} id={showPayoff ? undefined : titleId}>{view === 'places' ? 'Clusters' : view === 'cluster' ? clusterName ? <code>{clusterName}</code> : entry?.remembered ? 'Use a cluster' : 'Add a cluster' : workspace || selected ? 'This computer' : 'Where should your agents run?'}</h1>
      {view === 'local' && !workspace && !twoCards && selected && !presentation.done && <p className="muted">{localStage}</p>}
      {view === 'places' && <p className="muted">Choose where agents run</p>}
      {view === 'places' && <Button type="button" kind={noClusters && !localNext ? 'primary' : 'secondary'} onClick={() => chooseCluster()}>Add a cluster</Button>}
    </div>
    {view === 'places' && <Places modelRecovery={modelRecovery} recoveryRef={setPlaceRecovery} snapshot={snapshot} unavailable={snapshotUnavailable} status={error ? undefined : status} view={placesView} place={currentPlace} onLocal={openLocal} onCluster={chooseCluster} />}
    {showPayoff && firstPass && <FirstPass snapshot={firstPass} titleId={titleId} onOpen={openExample} />}
    <div hidden={view !== 'local' || !!showPayoff}>
      {cluster && !localOff && <Button type="button" kind="secondary" onClick={chooseComputer}>Use this computer</Button>}
      <div className={workspace ? `setup workspace-setup-grid returning-grid${updateBlocked || status?.localVm === 'failed' ? ' returning-grid-wide' : ''}` : selected ? `setup workspace-setup-grid${status?.localVm === 'failed' ? ' workspace-setup-failed' : ''}${twoCards ? ' returning-grid setup-cards' : ''}` : 'workspace-choice local-preflight'}>
        {!selected && !workspace && <>
          <section className="card">
            <p className="workspace-choice-step muted">Step 1 of 4 · Choose where agents run</p>
            <fieldset className="workspace-options">
              <legend className="workspace-sr-only">Where agents run</legend>
              <div className={`workspace-option${location === 'local' ? ' workspace-selected' : ''}${localOff ? ' workspace-option-off' : ''}`}>
                <input id="workspace-local" name="workspace-location" type="radio" value="local" checked={location === 'local'} disabled={localOff} aria-describedby={localOff ? 'workspace-local-off' : undefined} onChange={() => { setPreflightBlocked(true); setLocation('local'); }} />
                <label htmlFor="workspace-local">On this computer</label>{localOff ? <span className="chip">{unavailableCopy.line}</span> : <span className="chip">Recommended</span>}
                <div className="workspace-option-detail">
                  {localOff ? <p className="workspace-option-req" id="workspace-local-off">{unavailableCopy.reason}</p>
                    : <p className="workspace-option-req">{requirements ? setupNeeds(requirements) : requirements === undefined ? "Reading this build's requirements…" : 'Requirements are not available yet for this build.'}</p>}
                  {location === 'local' && view === 'local' && !error && status && requirements && status.localVm !== 'notIntegrated' && <Preflight requiredMemoryBytes={requirements.memoryBytes} onBlocked={setPreflightBlocked} onFailed={setPreflightFailed} onCluster={() => { setLocation('cluster'); document.getElementById('workspace-cluster')?.focus(); }} onSetUpLater={onSetUpLater} />}
                  {!localOff && !(location === 'local' && preflightFailed) && <Disclosure label="What's inside" open={inside} controls="workspace-inside" onOpenChange={setInside}>
                    <div id="workspace-inside" hidden={!inside}>
                      <p className="workspace-option-lede">The workspace includes Orka and the tools your agents use. Bridge checks the download before starting it.</p>
                    </div>
                  </Disclosure>}
                </div>
              </div>
              <div className={`workspace-option${location === 'cluster' ? ' workspace-selected' : ''}`}>
                <input id="workspace-cluster" name="workspace-location" type="radio" value="cluster" checked={location === 'cluster'} onChange={() => setLocation('cluster')} />
                <label htmlFor="workspace-cluster">On a cluster you already have</label>
                <div className="workspace-option-detail"><p>Use Orka on a Kubernetes cluster you already have.</p></div>
              </div>
            </fieldset>
          <div className="workspace-step-actions">{continueAction}
            {/* What Continue does next, beside it, as the boards show. */}
            {(location === 'cluster' || status?.localVm !== 'notIntegrated') && <span className="workspace-continue-hint" aria-hidden={location === 'local' && !preflightFailed ? true : undefined}>{location === 'cluster' ? 'Next, you pick the cluster. Setup on this computer is skipped.' : preflightFailed ? 'Continue is off until the check passes.' : 'These checks do not change this computer.'}</span>}</div>
          </section>
        </>}
        <div className="setup-column setup-main">
        <FreshWorkspace asking={freshAsk} working={freshWorking} status={freshShown} plan={freshPlan} onConfirm={() => { void confirmFresh(); }} onCancel={() => setFreshAsk(false)} onDismiss={() => setFreshDismissed(freshKey)} />
        {!workspace && !selected && upgradePanel}
        {workspace && status && !freshUnfinished && <Workspace ordinaryRecovery={ordinaryRecovery} updating={upgrading} updateBlocked={!!notice && (notice !== 'unknown' || !!status.workspaceReplaced)} upgrade={upgradePanel} status={status} busy={changing || recoveryOnly} stopping={sending && pendingAction !== 'start'} starting={sending && pendingAction === 'start'} error={error} onStart={() => { void chooseLocal(); }} onCancel={() => { void chooseLocal('cancel'); }} onCluster={() => chooseCluster()} active={active && view === 'local'} onFresh={freshBlocked ? undefined : askFresh} freshOffered={freshOffered} helpLines={freshLines} />}
        {!workspace && selected && status && !freshUnfinished && <SetupProgress ordinaryRecovery={ordinaryRecovery} updating={upgrading} updateBlocked={!!notice && (notice !== 'unknown' || !!status.workspaceReplaced)} upgrade={upgradePanel} modelUnavailable={modelUnavailable} onChooseProvider={() => setProviderChange(value => value + 1)} status={status} firstCheck={firstCheck} firstTestResult={firstTestResult} initialAutomatic={initialAutomatic} onCheckFirstTest={onCheckFirstTest} busy={sending || upgradeBusy || !!upgrade?.busy && !ordinaryStarting || upgradeObservation.pending || recoveryOnly} stopping={sending && pendingAction !== 'start'} error={error} modelState={modelMounted ? modelState : 'idle'} onCancel={() => { void chooseLocal('cancel'); }} onRestart={() => { void chooseLocal(); }} onCluster={() => chooseCluster()} onOpenExample={openExample} guided={guided} onSetUpLater={onSetUpLater} cards={twoCards} onFresh={freshOffered ? askFresh : undefined} helpLines={freshLines} />}
        </div>
        <div className="setup-column setup-side">
        {workspace && status?.localVm !== 'ready' && <section className="card workspace-ai-waiting"><h2>AI</h2>{savedConnection ? <>
          <p>A saved connection can be reused without connecting automatically.</p>
          <Button type="button" kind="secondary" disabled>Use saved connection</Button>
          <p className="muted">Start the workspace before changing its saved connection</p>
        </> : <p className="muted">{status && ['cancelled', 'closed'].includes(status.localVm) ? 'Start the workspace before connecting the AI.' : 'Waiting for the workspace to start.'}</p>}</section>}
        {status && modelMounted && <div className="setup-model-slot" hidden={workspace && status.localVm !== 'ready'}><Model key={`${upgrade?.credential === 'transferred' ? 'shared' : 'ordinary'}-${freshGeneration}`} diagnostic={status.localVmError} helpLines={freshLines} onFreshWorkspace={askFresh} freshBlocked={freshBlocked} sharedConnection={upgrade?.credential === 'transferred'} title={workspace ? 'AI' : undefined} inline={!workspace && !twoCards && !status.workspaceReplaced && canQueueModel(status)} setupCard={!workspace} changeRequest={providerChange} onPlaceCard={view === 'places'} recoveryTarget={view === 'places' ? placeRecovery : null} enabled={status.model === 'available'} queueable={!status.workspaceReplaced && canQueueModel(status)} paused={!!error || changing} away={cluster} primary={status.localVm === 'ready'} onConnectRequest={() => { chooseComputer(); if (view === 'places') setPanel('local'); }} onState={observeModel} onSettled={() => setCheck(value => value + 1)} /></div>}
        {!workspace && !twoCards && selected && status && status.localVm !== 'ready' && status.localVm !== 'failed' && <Meet guided={guided} onRead={openExample} onName={onExampleName} />}
        </div>
      </div>
    </div>
    {entry && <ClusterCard active={active && view === 'cluster'} entry={entry} selection={currentPlace} onPlace={publishPlace} onContextName={setClusterName} onOpenAgent={onOpenAgent ?? onOpenExample} onDone={back} onRemoved={removed} />}
    {!upgradePanel && <UpgradeObservation observation={upgradeObservation} />}
    {displayError && <div role="alert"><p>{displayError}</p>
      <Button type="button" kind="secondary" className="action-spaced" disabled={(retryUpgradeStatus ? sending : busy) || upgradeBusy || upgradeObservation.pending}
        onClick={() => { if (setupError) setSetupCheck(value => value + 1); else if (retryUpgradeStatus) setUpgradeCheck(value => value + 1); else void checkWorkspace(); }}>{retryUpgradeStatus ? 'Retry workspace status' : 'Check again'}</Button>
      {onSetUpLater && <SetUpLater onClick={onSetUpLater} />}
    </div>}
    {!error && !upgradePanel && upgradeObservation.error && <Button type="button" kind="secondary" disabled={upgradeObservation.pending} onClick={() => { void checkWorkspace(); }}>Check again</Button>}
  </div>;
}
