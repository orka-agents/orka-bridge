import { type ComponentProps, useCallback, useEffect, useRef, useState } from 'react';
import mark from './assets/orka-mark.svg';
import { type ClusterPlaceSnapshot, clusterPlaces, selectCluster, type SelectedPlace } from './clusters/api';
import { sendAgentFocus } from './clusters/send-api';
import { useSendSettlement } from './clusters/use-send-settlement';
import { Button } from './components/Button';
import { agentInfo, SAMPLE_DOCUMENT_ID, type SaveState, type SourceId, sameSource } from './designer/agent-api';
import { listDocuments, selectedDocument } from './designer/documents-api';
import Example, { type DocumentAction } from './designer/Example';
import { type RunSnapshot, runCall } from './designer/run-api';
import HistoryButtons from './chrome/HistoryButtons';
import type { ClustersView, DesignerView, Location } from './chrome/history';
import { useNavigationHistory } from './chrome/useNavigationHistory';
import ThemeButton from './chrome/ThemeButton';
import SidebarToggle from './chrome/SidebarToggle';
import { narrowWindow, readSidebarCollapsed, useNarrowWindow, writeSidebarCollapsed } from './chrome/sidebar';
import OrkaSwitcher from './chrome/OrkaSwitcher';
import WindowControls, { customTitleBar } from './chrome/WindowControls';
import { finishGuidedFirstRun, guidedFirstRun, guidedStep, resumeGuidedFirstRun } from './firstrun/guided';
import GuidedSteps from './firstrun/GuidedSteps';
import './firstrun/firstrun.css';
import './art.css';
import Home from './home/Home';
import Overview from './overview/Overview';
import { firstTestPending, retireFirstTest, useFirstTest } from './setup/first-test';
import type { ModelState } from './setup/Model';
import { preflightRecovery } from './setup/preflight-state';
import { setupView, workspaceView } from './setup/progress';
import { workspaceRecovery } from './setup/recovery-info';
import Setup, { type LocalClusterControl, type UpgradeNotice } from './setup/Setup';
import { hasSeenWelcome } from './setup/Welcome';
import type { ExamplePair, SetupStatus } from './setup/state';

type Page = 'Home' | 'Overview' | 'Designer' | 'Clusters';
// Home stands alone; Build holds the agents and their editor; Run holds the places they run.
const navGroups: { label?: string; pages: Page[] }[] = [{ pages: ['Home'] }, { label: 'Build', pages: ['Overview', 'Designer'] }, { label: 'Run', pages: ['Clusters'] }];

function PageIcon({ page }: { page: Page }) {
  return (
    <svg className="icon" viewBox="0 0 24 24" aria-hidden="true">
      {page === 'Home' && <><path d="m3.5 10.5 8.5-7 8.5 7" /><path d="M5.5 9v11h13V9" /><path d="M10 20v-5h4v5" /></>}
      {page === 'Overview' && <>
        <rect x="3" y="4" width="18" height="4.5" rx="1.5" />
        <rect x="3" y="10" width="18" height="4.5" rx="1.5" />
        <rect x="3" y="16" width="18" height="4.5" rx="1.5" />
      </>}
      {page === 'Designer' && <path d="M4 20l1-4L16 5a2.1 2.1 0 0 1 3 3L8 19l-4 1zM14 7l3 3" />}
      {page === 'Clusters' && <path d="M12 3 3 8l9 5 9-5-9-5zM3 13l9 5 9-5" />}
    </svg>
  );
}

function ComingButton({ label, reason }: { label: string; reason: string }) {
  // The wrapper makes the disabled control's reason reachable with the keyboard.
  return <span className="coming-control" tabIndex={0} role="group" aria-label={`${label}: ${reason}`} title={reason}>
    <Button type="button" kind="secondary" disabled title={reason}>{label}</Button>
    <span className="control-tip" aria-hidden="true">{reason}</span>
  </span>;
}

export default function App() {
  const [page, setPageState] = useState<Page>('Clusters');
  const [guided, setGuided] = useState(guidedFirstRun);
  const guidedLeft = useRef(false);
  const leaveGuided = useCallback(() => { guidedLeft.current = true; finishGuidedFirstRun(); setGuided(false); }, []);
  // Set up later, offered only where setup has stopped, ends guidance on the same page; focus stays on its heading.
  const setUpLater = useCallback(() => { leaveGuided(); requestAnimationFrame(() => document.getElementById('page-title')?.focus()); }, [leaveGuided]);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(readSidebarCollapsed);
  // Narrow windows always show the rail; there the toggle opens the full sidebar over the page until a page is chosen.
  const narrow = useNarrowWindow();
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const toggleSidebar = useCallback(() => {
    if (narrowWindow()) { setSidebarOpen(value => !value); return; }
    setSidebarCollapsed(value => { writeSidebarCollapsed(!value); return !value; });
  }, []);
  const overlay = !guided && narrow && sidebarOpen;
  useEffect(() => { if (!narrow) setSidebarOpen(false); }, [narrow]);
  useEffect(() => {
    if (!overlay) return;
    const key = (event: KeyboardEvent) => { if (event.key === 'Escape' && !event.defaultPrevented) setSidebarOpen(false); };
    const outside = (event: PointerEvent) => {
      const target = event.target as Element | null;
      if (!target?.closest('.page-nav, .status-bar, .sidebar-toggle-control')) setSidebarOpen(false);
    };
    window.addEventListener('keydown', key);
    document.addEventListener('pointerdown', outside);
    return () => { window.removeEventListener('keydown', key); document.removeEventListener('pointerdown', outside); };
  }, [overlay]);
  const overviewOpenRequest = useRef(0);
  const [overviewNotice, setOverviewNotice] = useState<string>();
  // Bridge opens on Clusters until Setup's first reads say whether anything is set up; a page the person picks first wins.
  const [launching, setLaunching] = useState(true);
  const launchOpen = useRef(true);
  const setPage = useCallback((next: Page) => { launchOpen.current = false; setLaunching(false); overviewOpenRequest.current++; setOverviewNotice(undefined); setPageState(next); }, []);
  // Moves the app makes itself replace the current Back/Forward entry instead of adding one.
  const appMove = useRef<() => void>(undefined);
  const observeLaunch = useCallback((home: boolean) => {
    if (launchOpen.current && home) { appMove.current?.(); setPageState('Home'); }
    launchOpen.current = false; setLaunching(false);
  }, []);
  const [place, setPlace] = useState<SelectedPlace>({ kind: 'thisComputer' });
  const remote = place.kind === 'cluster';
  // Cluster access lives in native memory and can stop authenticating when its token can no longer be renewed.
  // Watched passively (no cluster read) while a cluster is in use, so every surface says it needs reconnecting.
  const [lostKey, setLostKey] = useState<string>();
  const selectedTarget = place.kind === 'cluster' ? place.target?.id : undefined;
  useEffect(() => {
    setLostKey(undefined);
    if (!selectedTarget) return;
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    const observe = async () => {
      const snapshot = await clusterPlaces();
      if (!live) return;
      if (snapshot) setLostKey(snapshot.accessLost && snapshot.selectedKey ? snapshot.selectedKey : undefined);
      timer = setTimeout(() => { void observe(); }, 2000);
    };
    void observe();
    return () => { live = false; clearTimeout(timer); };
  }, [selectedTarget]);
  const accessLost = !!lostKey && !!selectedTarget;
  // The last cluster connected in this run with an AI connection, for Home to offer when this computer can't run the Navigator.
  const [lastCluster, setLastCluster] = useState<SelectedPlace>();
  useEffect(() => { if (place.kind === 'cluster' && place.target && place.providerId) setLastCluster(place); }, [place]);
  // The guided first run ends once the person moves on: another page, a cluster instead of this computer, or Set up later.
  // That includes Setup opening Designer when the automatic first test doesn't pass, so the person sees why.
  useEffect(() => { if (guided && (page !== 'Clusters' || remote)) leaveGuided(); }, [guided, page, remote, leaveGuided]);
  const content = useRef<HTMLElement>(null);
  useEffect(() => { if (content.current) content.current.scrollTop = 0; }, [page]);
  const [exampleOpen, setExampleOpen] = useState(false);
  const [overviewFocus, setOverviewFocus] = useState<ComponentProps<typeof Overview>['focus']>();
  const [overviewPending, setOverviewPending] = useState(false);
  const focusRequest = useRef(0);
  useEffect(() => { if (page !== 'Overview') { focusRequest.current++; setOverviewFocus(undefined); setOverviewPending(false); } }, [page]);
  // Navigation never changes the Send destination or the local VM; native derives the target from that Send's own record.
  const seeInOverview = useCallback(async (sendId: string) => {
    const id = ++focusRequest.current;
    setDocumentAction(undefined); setOverviewFocus({ id, sendId }); setOverviewPending(true); setPage('Overview');
    const target = await sendAgentFocus(sendId);
    if (id !== focusRequest.current) return;
    setOverviewFocus({ id, sendId, target }); setOverviewPending(false);
  }, [setPage]);
  const [documentId, setDocumentId] = useState(SAMPLE_DOCUMENT_ID);
  const [newAgentRequest, setNewAgentRequest] = useState(0);
  const [agentsRevision, setAgentsRevision] = useState(0);
  const [placesRevision, setPlacesRevision] = useState(0);
  const observeSendSettled = useCallback(() => setPlacesRevision(value => value + 1), []);
  const startSend = useSendSettlement(observeSendSettled);
  const [persistInitialSelection, setPersistInitialSelection] = useState(false);
  const [openDocumentRequest, setOpenDocumentRequest] = useState(0);
  const openRequest = useRef(0);
  const [overviewCounts, setOverviewCounts] = useState('Reading saved files…');
  const [savedAgents, setSavedAgents] = useState<number>();
  const [creatingAgent, setCreatingAgent] = useState(false);
  // Leaving New agent (Create or Cancel) replaces its Back/Forward entry with the agent shown next.
  const creatingShown = useRef(false);
  const observeCreating = useCallback((value: boolean) => { if (creatingShown.current && !value) appMove.current?.(); creatingShown.current = value; setCreatingAgent(value); }, []);
  const [documentAction, setDocumentAction] = useState<DocumentAction>();
  const actionSequence = useRef(0);
  const handleDocumentAction = useCallback((id: number) => setDocumentAction(current => current?.id === id ? undefined : current), []);
  const [exampleAttempt, setExampleAttempt] = useState(0);
  const [setup, setSetup] = useState<SetupStatus>();
  const [setupReadOnly, setSetupReadOnly] = useState(true);
  const workspaceAdmitted = useRef(false);
  const [editorOwned, setEditorOwned] = useState(false);
  const exampleReadOnly = setupReadOnly && !editorOwned && !(place.kind === 'cluster' && place.target);
  useEffect(() => {
    if (exampleOpen && (!setupReadOnly || place.kind === 'cluster' && place.target)) setEditorOwned(true);
  }, [exampleOpen, setupReadOnly, place]);
  const [checking, setChecking] = useState(true);
  const [welcome, setWelcome] = useState(false);
  // Welcome shows whenever nothing is set up, so the guided steps come back with it unless the person left them this run.
  useEffect(() => {
    if (welcome && page === 'Clusters' && !guided && !guidedLeft.current) { resumeGuidedFirstRun(); setGuided(true); }
  }, [welcome, page, guided]);
  const [model, setModel] = useState<ModelState>('idle');
  const [catalogEpoch, setCatalogEpoch] = useState(0);
  const [workspaceBusy, setWorkspaceBusy] = useState(false);
  const [returningWorkspace, setReturningWorkspace] = useState(false);
  const [workspaceStopping, setWorkspaceStopping] = useState(false);
  const [workspaceUpdating, setWorkspaceUpdating] = useState(false);
  const [upgradeNotice, setUpgradeNotice] = useState<UpgradeNotice>();
  const localCluster = useRef<LocalClusterControl>(null);
  const [localSetupView, setLocalSetupView] = useState(false);
  const catalogContext = useRef('');
  const observeModel = useCallback((value: ModelState) => { setModel(value); setCatalogEpoch(epoch => epoch + 1); }, []);
  const observeWorkspaceAction = useCallback((pending: boolean, stopping = false, updating = false, notice?: UpgradeNotice) => {
    setWorkspaceBusy(pending); setWorkspaceStopping(pending && stopping); setWorkspaceUpdating(pending && updating); setUpgradeNotice(notice);
    if (pending) setCatalogEpoch(epoch => epoch + 1);
  }, []);
  const choosePlace = useCallback((value: SelectedPlace) => { overviewOpenRequest.current++; setPlace(value); setCatalogEpoch(epoch => epoch + 1); }, []);
  const [agentName, setAgentName] = useState<string>();
  const [saveState, setSaveState] = useState<SaveState>('saved');
  const [saveNotice, setSaveNotice] = useState<{ id: string; edited: boolean }>({ id: '', edited: false });
  const [stale, setStale] = useState(false);
  const [exampleState, setExampleState] = useState<'loading' | 'loaded' | 'failed'>('loading');
  const [run, setRun] = useState<RunSnapshot>();
  const [runUnavailable, setRunUnavailable] = useState(false);
  const [overviewAiConnected, setOverviewAiConnected] = useState(false);
  // Setup keeps the sample's latest evidence even while Designer shows another document.
  const [sampleResult, setSampleResult] = useState<{ snapshot?: RunSnapshot }>();
  const [firstPass, setFirstPass] = useState<RunSnapshot>();
  const replacementObserved = useRef(false);
  const [sampleRetired, setSampleRetired] = useState(false);
  // null records an unsuccessful sample open; a retained run report cannot prove its files readable.
  const [sampleSource, setSampleSource] = useState<SourceId | null>();
  const [sampleStale, setSampleStale] = useState(false);
  const [sampleDirty, setSampleDirty] = useState(false);
  const [sampleRevision, setSampleRevision] = useState(0);
  const observeSampleRun = useCallback((snapshot: RunSnapshot | undefined) => {
    if (replacementObserved.current) return;
    // Observed sample completion consumes held setup intent; a later unreadable poll must not re-arm it.
    if (snapshot?.state === 'complete' && snapshot.report) retireFirstTest();
    setSampleResult({ snapshot });
  }, []);
  const sampleObserved = sampleResult !== undefined;
  const [sampleCheck, setSampleCheck] = useState(0);
  useEffect(() => {
    if (sampleRetired || !sampleObserved && sampleCheck === 0) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const observe = async () => {
      const result = await runCall('agent_run_status');
      if (cancelled) return;
      observeSampleRun(result.ok ? result.snapshot : undefined);
      timer = setTimeout(() => { void observe(); }, 500);
    };
    void observe();
    return () => { cancelled = true; clearTimeout(timer); };
  }, [sampleObserved, sampleCheck, observeSampleRun, sampleRetired]);
  const nativeConnection = useRef<boolean | undefined>(undefined);
  const observeRun = useCallback((snapshot: RunSnapshot | undefined, id: string) => {
    if (nativeConnection.current !== snapshot?.modelConnected) { nativeConnection.current = snapshot?.modelConnected; setCatalogEpoch(epoch => epoch + 1); }
    setRun(snapshot); setRunUnavailable(!snapshot);
    if (id === SAMPLE_DOCUMENT_ID) observeSampleRun(snapshot);
  }, [observeSampleRun]);
  const observeStale = useCallback((value: boolean, id: string) => {
    setStale(value);
    if (id === SAMPLE_DOCUMENT_ID) setSampleStale(value);
  }, []);
  const observeSave = useCallback((value: SaveState, id: string) => {
    setSaveState(value);
    // Opening reports saved too. Remember edits, including saves batched into one render.
    setSaveNotice(previous => ({ id, edited: value !== 'saved' || previous.id === id && previous.edited }));
    if (id === SAMPLE_DOCUMENT_ID) setSampleDirty(value !== 'saved');
  }, []);
  // A workspace chosen without Welcome ever being dismissed means browser storage was cleared, not a first launch.
  useEffect(() => { if (guided && setup?.selected === 'localVm' && !hasSeenWelcome()) leaveGuided(); }, [guided, setup?.selected, leaveGuided]);
  const observeSetup = useCallback((status: SetupStatus | undefined) => {
    if (status?.workspaceReplaced) {
      retireFirstTest();
      setFirstPass(undefined);
      if (!replacementObserved.current) { setSampleResult(undefined); setSampleStale(false); }
      replacementObserved.current = true;
      setSampleRetired(true);
    }
    if (status && replacementObserved.current) status = { ...status, workspaceReplaced: true };
    setSetup(status); setChecking(false);
    const context = `${status?.selected}/${status?.localVm}/${status?.model}`;
    if (context !== catalogContext.current) { catalogContext.current = context; setCatalogEpoch(epoch => epoch + 1); }
    // An unavailable poll must not admit discovery or Run during initial setup.
    if (status) {
      if (!status.selected) workspaceAdmitted.current = false;
      if (status.localVm === 'ready') workspaceAdmitted.current = true;
      setSetupReadOnly(status.selected === 'localVm' && !workspaceAdmitted.current);
    }
  }, []);
  const sampleFilesObserved = useRef(false);
  const observePair = useCallback((pair: (ExamplePair & { source?: SourceId }) | undefined, id: string) => {
    setExampleState(pair ? 'loaded' : 'failed');
    setAgentName(agentInfo(pair?.agent).name);
    if (id === SAMPLE_DOCUMENT_ID) {
      setSampleSource(pair ? pair.source : null);
      // Native first-open creates the sample files; refresh the hidden saved-only inventory once.
      if (pair && !sampleFilesObserved.current) { sampleFilesObserved.current = true; setAgentsRevision(value => value + 1); }
    }
  }, []);
  const cases = run?.report?.cases;
  const checksStale = stale || (run?.sourceCurrent === false && !!run.report);
  const localChecks = checksStale ? 'Tests not run since your edit' : runUnavailable ? 'Test status unavailable' : run?.cancellation === 'unconfirmed' ? 'Cancellation not confirmed' : run?.state === 'unknown' ? 'Test result unknown'
    : run?.cancellation === 'cancelling' ? 'Cancelling tests' : run?.state === 'running' ? 'Running tests…' : run?.report?.verdict === 'cancelled' ? 'Tests cancelled' : run?.report?.verdict === 'unknown' ? 'Test result unknown' : cases
    ? `${cases.filter(check => check.verdict === 'passed').length} of ${cases.length} passed`
    // Without a result the line still says where tests stand: nothing saved yet, or tests to run.
    : remote ? undefined : savedAgents === 0 && !agentName ? 'No saved agents yet' : savedAgents || agentName ? 'Tests need running' : undefined;
  const checks = remote && localChecks ? `This computer: ${localChecks}` : localChecks;
  const [designerView, setDesignerView] = useState<DesignerView>('Agent');
  const [clustersView, setClustersView] = useState<ClustersView>({ kind: 'places' });
  const placesRead = useRef<ClusterPlaceSnapshot>(undefined);
  const navigation = useNavigationHistory({
    enabled: !guided,
    location: page === 'Designer' ? creatingAgent ? { page, view: 'Agent', creating: true } : { page, documentId: exampleOpen ? documentId : undefined, view: exampleOpen ? designerView : 'Agent' } : page === 'Clusters' ? { page, view: clustersView } : { page },
    exists: async target => {
      if (target.page === 'Designer' && target.documentId && target.documentId !== SAMPLE_DOCUMENT_ID) {
        const agents = await listDocuments();
        return !agents || agents.some(agent => agent.id === target.documentId);
      }
      if (target.page === 'Clusters' && target.view.kind === 'cluster' && target.view.key) {
        const key = target.view.key;
        placesRead.current = await clusterPlaces();
        return !placesRead.current || placesRead.current.places.some(item => item.key === key);
      }
      return true;
    },
    restore: target => restorePlace(target),
  });
  useEffect(() => { appMove.current = navigation.automatic; }, [navigation.automatic]);
  const firstTest = useFirstTest({
    retired: !!setup?.workspaceReplaced,
    ready: !upgradeNotice && !setup?.workspaceReplaced && setup?.selected === 'localVm' && setup.localVm === 'ready' && model === 'connected' && !workspaceBusy && !remote,
    canOpen: page === 'Clusters' && localSetupView && setup?.localVm === 'ready' && !workspaceBusy && !remote,
    sampleDirty,
    onOpen: () => { if (!replacementObserved.current) { navigation.automatic(); openDesigner(SAMPLE_DOCUMENT_ID); } },
    onPrepared: () => setSampleRevision(value => value + 1),
    onSource: setSampleSource,
    onSnapshot: observeSampleRun,
    onPassed: setFirstPass,
    onUnavailable: () => setFirstPass(undefined),
  });
  const sample = sampleResult?.snapshot;
  const firstCheck = setup?.workspaceReplaced ? undefined : sampleSource === null ? 'unknown' : firstTest === 'running' ? 'running' : sampleResult ? sampleStale || sample?.sourceCurrent === false || (sameSource(sampleSource, sample?.source) === false && !!sample?.report) ? 'stale'
    : !sample || sample.state === 'unknown' ? 'unknown' : sample.state === 'running' ? 'running' : sample.state === 'complete' ? sample.report?.verdict === 'cancelled' ? 'unknown' : sample.report?.verdict : undefined
    : firstTest;
  // Tab, footer and Setup share the stage. The chip names the place, not AI or test health.
  const initialAutomatic = firstTest === 'running' || (model === 'connected' && firstTestPending() && (!sample || sample.state === 'idle'));
  const firstTestResult = !setup?.workspaceReplaced && sampleSource !== null && sample?.report ? { failed: sample.report.cases.filter(check => check.verdict === 'failed').length } : undefined;
  const ordinaryView = setupView(launching ? undefined : setup, { checking: checking || launching, stopping: workspaceStopping, starting: workspaceBusy && !workspaceStopping, workspace: returningWorkspace, model, check: firstCheck, initialAutomatic });
  const updating = workspaceUpdating && !!setup;
  const recovery = upgradeNotice === 'recovery' && !!setup;
  const required = upgradeNotice === 'required' && !!setup;
  const upgradeAttention = updating || !!upgradeNotice && !!setup && (upgradeNotice !== 'unknown' || !!setup.workspaceReplaced);
  const refreshing = upgradeNotice === 'checking';
  const occupied = upgradeNotice === 'busy';
  const upgradeLine = updating ? 'Updating workspace' : refreshing ? 'Checking workspace files' : occupied ? 'Workspace is busy' : recovery ? 'Workspace recovery needs attention' : 'Workspace update needs attention';
  const upgradeDetail = refreshing || occupied ? 'Showing last observed workspace information.' : recovery && !updating ? 'Finish recovery before using the local workspace.' : 'Your agent files are kept.';
  const view = upgradeAttention ? { ...ordinaryView, line: upgradeLine, tabLine: upgradeLine, footerLine: upgradeLine, detail: upgradeDetail, tone: updating || refreshing || occupied ? 'busy' as const : 'needs-you' as const, status: updating || refreshing || occupied ? 'running' as const : 'needs-you' as const, done: false } : ordinaryView;
  const { line: setupLine, detail } = view;
  const hostWorkspace = upgradeAttention ? { tone: view.tone, detail: `${upgradeLine}. ${upgradeDetail}` } : setup?.selected === 'localVm' && setup.localVm !== 'notIntegrated'
    ? workspaceView(setup, { stopping: workspaceStopping, starting: workspaceBusy && !workspaceStopping }) : undefined;
  const hostTone = hostWorkspace?.tone ?? view.tone;
  const workspaceReady = setup?.localVm === 'ready' && !recovery && !required && !(setup.workspaceReplaced && upgradeNotice === 'unknown');
  // undefined until the first read, so Home doesn't flash an AI warning before it knows.
  const [aiObserved, setAiObserved] = useState<boolean>();
  const [designerChat, setDesignerChat] = useState<{ documentId: string; reason?: string }>();
  const observeChatBlocked = useCallback((id: string, reason: string | undefined) => setDesignerChat({ documentId: id, reason }), []);
  useEffect(() => {
    setOverviewAiConnected(false); setAiObserved(undefined);
    if (page !== 'Overview' && page !== 'Home' || !workspaceReady || workspaceBusy || setup?.model !== 'available') return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    // Native connection state is workspace-wide; no Designer or saved-file selection is needed.
    const observe = async () => {
      const result = await runCall('agent_run_status');
      if (cancelled) return;
      const connected = result.ok && (result.snapshot.modelConnected ?? model === 'connected');
      setOverviewAiConnected(connected);
      // An unreadable poll proves nothing about the AI, so only a read snapshot can block Home.
      if (result.ok) setAiObserved(connected);
      timer = setTimeout(() => { void observe(); }, 500);
    };
    void observe();
    return () => { cancelled = true; clearTimeout(timer); };
  }, [page, workspaceReady, workspaceBusy, setup?.model, model]);
  const workspaceTransition = !!setup && ['preparing', 'installing', 'starting', 'closing', 'failed'].includes(setup.localVm);
  const overviewWorkspaceNote = !setup ? setupLine : upgradeAttention ? upgradeLine : workspaceStopping ? 'Stopping the workspace'
    : workspaceBusy && (!workspaceTransition || setup?.localVm === 'failed') ? 'Getting your workspace ready…'
      : workspaceTransition ? setupLine : undefined;
  const placeName = remote ? place.target?.context_name ?? place.pending?.name ?? 'Choose a cluster'
    : { local: 'This computer', none: 'This computer', checking: 'Checking…', unavailable: 'Status unavailable' }[view.place];
  const setupDetail = upgradeAttention ? `${upgradeLine}. ${upgradeDetail}` : setup?.localVm === 'failed' ? (setup.preflightFailure ? preflightRecovery(setup.preflightFailure) : workspaceRecovery(setup.workspaceFailure)).summary : detail;
  const hostDetail = setup?.localVm === 'failed' ? setupDetail : hostWorkspace?.detail ?? detail;
  const failedChecks = cases?.filter(check => check.verdict === 'failed').length ?? 0;
  const agentStatus = checksStale ? 'tests need running' : runUnavailable ? 'test status unavailable' : run?.cancellation === 'unconfirmed' ? 'cancellation not confirmed' : run?.state === 'unknown' ? 'test result unknown'
    : run?.cancellation === 'cancelling' ? 'cancelling tests'
    : run?.state === 'running' ? 'running tests'
    : run?.report?.verdict === 'cancelled' ? 'tests cancelled'
    : run?.report?.verdict === 'unknown' ? 'test result unknown'
    : failedChecks ? `${failedChecks} ${failedChecks === 1 ? 'test' : 'tests'} failing`
    : cases?.some(check => check.verdict === 'unknown') ? 'test result unknown'
    : cases?.some(check => check.verdict === 'cancelled') ? 'tests cancelled'
    : cases ? 'tests passing' : 'ready to edit';
  const agentStateWord = agentStatus.match(/\b(passing|failing|running|cancelled|cancelling|unknown|unavailable|unconfirmed|not confirmed|ready)\b/);
  const tabStatus: Record<Page, string> = {
    Home: 'Navigator',
    Overview: welcome ? '' : overviewCounts,
    Designer: creatingAgent ? 'No agent open' : agentName ? `${agentName}, ${agentStatus}` : !exampleOpen ? exampleReadOnly ? 'Ready after setup' : 'No agent open'
      : exampleState === 'failed' ? 'Example unavailable' : exampleState === 'loaded' ? 'Agent file' : 'Opening example…',
    Clusters: remote ? place.target ? `${place.target.context_name}, ${accessLost ? 'needs reconnecting' : 'connected'}`
      : place.pending ? `${place.pending.name}, ${place.pending.unreachable ? 'can’t reach' : 'needs reconnecting'}` : 'Choose a cluster' : welcome ? 'Start here' : returningWorkspace ? view.line : view.tabLine,
  };
  const tabTone = exampleReadOnly ? 'not-started' : stale || (run?.sourceCurrent === false && !!run.report) ? 'stale' : run?.state === 'running' ? 'running' : run?.cancellation === 'unconfirmed' ? 'needs-you' : run?.report?.verdict === 'failed' ? 'failed' : run?.report?.verdict === 'passed' ? 'passed' : 'not-started';
  const catalogConnected = run?.modelConnected ?? model === 'connected';
  // Discovery must not compete with preparation or consume the one-shot test's connection.
  const automaticTestPending = firstTest === 'running' || model === 'connected' && !sampleDirty && firstTestPending();
  const catalogReady = remote ? !!place.target && !!place.providerId : !workspaceBusy && setup?.selected === 'localVm' && workspaceReady && setup.model !== 'locked' && catalogConnected;
  const openDesigner = (id?: string, create = false) => {
    setDocumentAction(undefined);
    const request = ++openRequest.current;
    setPage('Designer');
    const open = (selected: string) => {
      if (request !== openRequest.current) return;
      const remount = exampleState === 'failed' && saveState === 'saved';
      if (remount) { setExampleAttempt(value => value + 1); setExampleState('loading'); }
      if (!exampleOpen || remount) setPersistInitialSelection(id !== undefined);
      else if (!create) setOpenDocumentRequest(value => value + 1);
      setDocumentId(selected); setExampleOpen(true);
      if (create) setNewAgentRequest(value => value + 1);
    };
    if (id || exampleOpen) open(id ?? documentId);
    else void selectedDocument().then(selected => open(selected ?? SAMPLE_DOCUMENT_ID));
  };
  const openOverviewDocument = async (id: string, action: 'open' | 'chat' | 'run', clusterKey?: string) => {
    if (clusterKey) {
      const request = ++overviewOpenRequest.current;
      let notice: string | undefined;
      try {
        const selected = await selectCluster(clusterKey);
        if (request !== overviewOpenRequest.current) return;
        const provider = selected.providers.find(p => p.ready && place.kind === 'cluster' && place.target?.id === selected.target.id && p.id === place.providerId)
          ?? selected.providers.find(p => p.ready);
        choosePlace({ kind: 'cluster', ...selected, providerId: provider?.id });
      } catch {
        if (request !== overviewOpenRequest.current) return;
        choosePlace({ kind: 'cluster' });
        notice = 'Connect this cluster again in Clusters to send or run its tests. Your file is still available here.';
      }
      localCluster.current?.showPlaces();
      openDesigner(id); setOverviewNotice(notice);
      setDocumentAction({ id: ++actionSequence.current, documentId: id, action: action === 'run' ? 'send' : 'open' });
      return;
    }
    // Local tests stay local; Try a task and Open keep the selected place.
    openDesigner(id);
    setDocumentAction({ id: ++actionSequence.current, documentId: id, action });
  };
  // Reconnect opens that cluster's page in Clusters, where Use confirms access again.
  const reconnectPlace = async (key?: string) => {
    setPage('Clusters');
    const snapshot = await clusterPlaces();
    const item = key ? snapshot?.places.find(candidate => candidate.key === key) : undefined;
    if (item) localCluster.current?.showCluster(item, snapshot?.selectedKey === key); else localCluster.current?.showPlaces();
  };
  // Back and Forward go where the sidebar, the agent picker and the Clusters list go, through the same calls.
  const restorePlace = (target: Location) => {
    setDocumentAction(undefined); setSidebarOpen(false);
    if (target.page === 'Designer') {
      if (target.creating) { openDesigner(undefined, true); return; }
      const id = target.documentId;
      if (!id) { if (exampleOpen) setPage('Designer'); else openDesigner(); return; }
      // Switching agents saves pending edits first, as the picker does; the view follows once the agent is open.
      if (exampleOpen && id === documentId) setPage('Designer'); else openDesigner(id);
      setDocumentAction({ id: ++actionSequence.current, documentId: id, action: target.view === 'Try' ? 'chat' : target.view === 'File' ? 'file' : 'open' });
      return;
    }
    setPage(target.page);
    if (target.page !== 'Clusters') return;
    const view = target.view;
    if (view.kind === 'places') localCluster.current?.showPlaces();
    else if (view.kind === 'local') localCluster.current?.showLocal();
    else {
      const item = view.key ? placesRead.current?.places.find(candidate => candidate.key === view.key) : undefined;
      if (view.key && !item) localCluster.current?.showPlaces();
      else localCluster.current?.showCluster(item, !!item && place.kind === 'cluster' && !!place.target && placesRead.current?.selectedKey === item.key);
    }
  };
  const localLine = welcome ? 'Nothing is set up yet' : setup?.localVm === 'failed' && !returningWorkspace || setup?.model === 'locked' && setup?.localVm === 'ready' ? setupDetail : view.footerLine;
  const footer = [
    remote ? place.target ? accessLost ? 'Needs reconnecting' : `Orka in ${place.target.service_namespace}/${place.target.service_name}`
      : place.pending ? place.pending.unreachable ? 'Can’t reach' : 'Needs reconnecting' : 'Not connected' : localLine,
    exampleState === 'loaded' && (saveState !== 'saved' || saveNotice.id === documentId && saveNotice.edited) ? { saved: 'Saved', dirty: 'Unsaved changes', saving: 'Saving…', error: 'Not saved' }[saveState] : undefined,
  ].filter(Boolean);
  const checkFirstTest = () => {
    setSampleCheck(value => value + 1);
    // Retry an unreadable source through Designer's existing guarded sample-open path.
    if (sampleSource === null) openDesigner(SAMPLE_DOCUMENT_ID);
  };
  return (
    // The guided first run has no sidebar toggle, so it always shows the full sidebar.
    <div className={guided ? 'app-shell guided' : 'app-shell'} data-sidebar={guided ? undefined : overlay ? 'overlay' : sidebarCollapsed ? 'collapsed' : undefined}>
      <header className="title-bar" data-tauri-drag-region={customTitleBar() ? 'deep' : undefined}>
        <div className="title-bar-start">
          {!guided && <SidebarToggle collapsed={narrow ? !overlay : sidebarCollapsed} onToggle={toggleSidebar} />}
          {!guided && <HistoryButtons canBack={navigation.canBack} canForward={navigation.canForward} onBack={navigation.back} onForward={navigation.forward} />}
          <span className="brand"><img src={mark} width="22" height="22" alt="Orka" /><b>Bridge</b></span>
        </div>
        <div className="title-bar-centre">{guided && <GuidedSteps current={guidedStep(view.step, view.done)} />}</div>
        <div className="title-bar-end">
          <ThemeButton />
          <WindowControls />
        </div>
      </header>
      <nav id="page-nav" className="page-nav" aria-label="Main">
        {navGroups.map(group => <div key={group.pages[0]} className="nav-group" role={group.label ? 'group' : undefined} aria-labelledby={group.label ? `nav-group-${group.label}` : undefined}>
          {group.label && <p className="nav-group-label" id={`nav-group-${group.label}`}>{group.label}</p>}
          {group.pages.map(name => (
            <button key={name} className="page-tab" type="button"
              aria-label={name} aria-describedby={`tab-status-${name}`}
              aria-current={page === name ? 'page' : undefined}
              onClick={() => {
                setDocumentAction(undefined); setSidebarOpen(false);
                if (name === 'Clusters' && page === 'Clusters') localCluster.current?.showPlaces();
                if (name === 'Designer' && !exampleOpen) openDesigner();
                else setPage(name);
              }}>
              <span className={`icon-tile tile-${name.toLowerCase()}`}><PageIcon page={name} /></span>
              <span className="tab-copy"><b>{name}</b><small id={`tab-status-${name}`} title={tabStatus[name]}>{name === 'Designer' && agentName ? <><span className="tab-agent-name">{agentName}, </span><span className="tab-agent-status">{agentStateWord ? <>{agentStatus.slice(0, agentStateWord.index)}<span className={`tab-status-word status-${tabTone}`}>{agentStateWord[0]}</span>{agentStatus.slice((agentStateWord.index ?? 0) + agentStateWord[0].length)}</> : agentStatus}</span></> : tabStatus[name]}</small></span>
            </button>
          ))}
        </div>)}
      </nav>
      <main ref={content} className="page-content" data-page={page} aria-labelledby="page-title">
        {page === 'Designer' && !exampleOpen && <div className="page-heading">
          <h1 id="page-title">{page}</h1>
          {page === 'Designer' && <div className="heading-actions">
            <Button type="button" kind="secondary" onClick={() => openDesigner(undefined, true)}>New agent</Button>
            <ComingButton label="Send to a cluster" reason="Open your agent before sending" />
          </div>}
        </div>}
        <div hidden={page !== 'Clusters'}>
          <Setup ref={localCluster} onView={setClustersView} place={place} accessLost={accessLost} onPlace={choosePlace} onLocalView={setLocalSetupView} onWorkspaceMode={setReturningWorkspace} onWorkspaceAction={observeWorkspaceAction} active={page === 'Clusters'} firstPass={firstPass} modelUnavailable={sample?.error === 'model'} firstCheck={firstCheck} firstTestResult={firstTestResult} initialAutomatic={initialAutomatic} onCheckFirstTest={checkFirstTest} onStarting={() => setSetupReadOnly(true)} onStatus={observeSetup} onModelState={observeModel} onWelcome={setWelcome} onLaunch={observeLaunch} guided={guided} onSetUpLater={guided ? setUpLater : undefined}
            onOpenExample={() => openDesigner(SAMPLE_DOCUMENT_ID)} onOpenAgent={() => openDesigner()} />
        </div>
        {page === 'Home' && <Home active revision={agentsRevision}
          // On a cluster the Navigator thinks with its chosen AI connection, once it is connected in this run.
          gate={remote ? !place.target ? place.pending?.unreachable ? { kind: 'unreachable', name: place.pending.name }
            : place.pending ? { kind: 'reconnect', name: place.pending.name, onReconnect: () => { void reconnectPlace(place.pending?.key); } } : { kind: 'choose' }
            : accessLost ? { kind: 'reconnect', name: place.target.context_name, onReconnect: () => { void reconnectPlace(lostKey); } }
            : !place.providerId ? { kind: 'noConnection', name: place.target.context_name }
            : { kind: 'ready', place: { name: place.target.context_name, targetId: place.target.id, providerId: place.providerId,
              conversation: JSON.stringify([place.target.context_name, place.target.service_namespace, place.target.service_name, place.target.watched_namespace, place.providers?.find(p => p.id === place.providerId)?.name ?? place.providerId]) } }
            : !workspaceReady || workspaceBusy ? { kind: 'waiting' } : setup?.model !== 'available' || aiObserved === false ? { kind: 'noModel' } : { kind: 'ready' }}
          onOpenPage={next => next === 'Designer' ? openDesigner() : setPage(next)}
          // The task card's results are the agent's Try a task results, so Designer opens on that view.
          onOpenAgent={id => { openDesigner(id); setDocumentAction({ id: ++actionSequence.current, documentId: id, action: 'chat' }); }}
          onAgentCreated={() => setAgentsRevision(value => value + 1)}
          onThisComputer={() => choosePlace({ kind: 'thisComputer' })}
          switchTo={lastCluster?.kind === 'cluster' && lastCluster.target ? { name: lastCluster.target.context_name, choose: () => choosePlace(lastCluster) } : undefined}
          onManageClusters={() => { setPage('Clusters'); localCluster.current?.showPlaces(); }}
          // Designer's own gate for the agent it has open (unsaved edits, missing instructions, pending reloads).
          blockedFor={id => designerChat?.documentId === id ? designerChat.reason : undefined} />}
        {page === 'Designer' && overviewNotice && <p className="muted" role="status">{overviewNotice}</p>}
        {exampleOpen && <div hidden={page !== 'Designer'}><Example key={exampleAttempt} sampleRevision={sampleRevision} actionRequest={documentAction} onActionHandled={handleDocumentAction} agentsRevision={agentsRevision} onRegistered={() => setAgentsRevision(value => value + 1)} documentId={documentId} persistInitialSelection={persistInitialSelection} openDocumentRequest={openDocumentRequest} newAgentRequest={newAgentRequest} onNewAgentHandled={() => setNewAgentRequest(0)} onDocument={setDocumentId} remote={remote} accessLost={accessLost} catalogPaused={!remote && automaticTestPending} catalogKey={catalogReady ? catalogEpoch : undefined} place={place} onChooseCluster={() => setPage('Clusters')} workspaceReady={workspaceReady && !workspaceBusy} setupPending={exampleReadOnly} active={page === 'Designer'} onPair={observePair} onRun={observeRun} onSaveState={observeSave} onStale={observeStale} onSeeInOverview={seeInOverview} startSend={startSend} onChatBlocked={observeChatBlocked} onView={setDesignerView} onCreating={observeCreating} /></div>}
        <Overview onNewAgent={() => openDesigner(undefined, true)} localAiConnected={overviewAiConnected} active={page === 'Overview'} runBusy={firstTest === 'running' || !!run?.busy} activeDocumentId={firstTest === 'running' ? SAMPLE_DOCUMENT_ID : run?.busy?.documentId} workspaceKey={`${setup?.localVm ?? 'unknown'}/${workspaceBusy}`} workspaceNote={overviewWorkspaceNote} filesRevision={agentsRevision} placesRevision={placesRevision} onCounts={setOverviewCounts} onSavedCount={setSavedAgents}
          onOpenDocument={(id, key) => { void openOverviewDocument(id, 'open', key); }}
          onChatDocument={id => openOverviewDocument(id, 'chat')}
          onRunDocument={(id, key) => { void openOverviewDocument(id, 'run', key); }}
          onOpenExample={() => openDesigner(SAMPLE_DOCUMENT_ID)}
          focus={overviewFocus} focusPending={overviewPending} onRetryFocus={seeInOverview}
          onChooseWorkspace={() => setPage('Clusters')} />
        {page === 'Designer' && !exampleOpen && <p role="status">Opening your agent…</p>}
      </main>
      <footer className="status-bar">
        <OrkaSwitcher title={remote ? placeName : hostDetail} place={place} accessLost={accessLost} computerLine={savedAgents ? `${localLine} · ${savedAgents} ${savedAgents === 1 ? 'agent' : 'agents'}` : localLine} computerTone={hostTone} onPlace={choosePlace}
          onClusters={item => { setPage('Clusters'); localCluster.current?.showCluster(item); }} onManage={() => { setPage('Clusters'); localCluster.current?.showPlaces(); }}>
          <span className={`live-dot ${remote ? place.target ? accessLost ? 'needs-you' : 'ready' : 'wait' : hostTone}`} aria-hidden="true" /><span className="host-label">{placeName}</span>{remote && place.target && <span className="chip">In use</span>}<span className="sr-only">: {remote ? place.target ? accessLost ? 'Needs reconnecting' : 'Access confirmed' : 'Not connected' : hostDetail}</span>
        </OrkaSwitcher>
        <span className="status-lines">{footer.map((part, index) => <span key={index}>{index > 0 && <span aria-hidden="true"> · </span>}<span aria-live={index > 0 ? 'polite' : undefined}>{part}</span></span>)}
</span>
        {checks && <span className="checks-status">{checks}</span>}
        <span className="sr-only" role="status" aria-atomic="true">{[
          page !== 'Clusters' || !localSetupView ? `Workspace: ${setupDetail === setupLine ? setupLine : `${setupLine} · ${setupDetail}`}` : undefined,
          page !== 'Designer' && checks ? `Checks: ${checks}` : undefined,
        ].filter(Boolean).join(' · ')}</span>
      </footer>
    </div>
  );
}
