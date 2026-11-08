import { useCallback, useEffect, useRef, useState } from 'react';
import mark from './assets/orka-mark.svg';
import EngineStatus from './components/EngineStatus';
import Setup from './setup/Setup';
import Example from './designer/Example';
import type { SetupStatus, ExamplePair } from './setup/state';
import type { ModelState } from './setup/Model';
import type { RunSnapshot } from './designer/run-api';

const pages = ['Overview', 'Designer', 'Clusters'] as const;
type Page = (typeof pages)[number];

const emptyCopy: Record<Page, { title: string; detail: string }> = {
  Overview: {
    title: 'Your agents, wherever they run',
    detail: 'Agents on this machine and your connected clusters will appear here.',
  },
  Designer: {
    title: 'A place to shape your agent',
    detail: 'Your agent file, prompt, tools and tests will live here.',
  },
  Clusters: {
    title: 'A place for your agents to run',
    detail: 'Your local Orka and connected clusters will appear here.',
  },
};

function PageIcon({ page }: { page: Page }) {
  return (
    <svg className="icon" viewBox="0 0 24 24" aria-hidden="true">
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

const workspaceCopy = {
  idle: ['Setting up, step 1 of 4', 'Choose where agents run', 'wait'],
  preparing: ['Setting up, step 2 of 4', 'Getting workspace ready', 'busy'],
  installing: ['Setting up, step 2 of 4', 'Getting workspace ready', 'busy'],
  starting: ['Setting up, step 2 of 4', 'Starting workspace', 'busy'],
  ready: ['Setting up, step 3 of 4', 'Choose the AI', 'ready'],
  failed: ['Setup paused', 'Workspace could not start', 'bad'],
  closing: ['Stopping workspace', 'Stopping workspace', 'busy'],
  closed: ['Workspace stopped', 'Workspace stopped', 'wait'],
  notIntegrated: ['Workspace unavailable', 'Workspace unavailable', 'wait'],
} as const;

function ComingButton({ label, reason }: { label: string; reason: string }) {
  // The wrapper makes the disabled control's reason reachable with the keyboard.
  return <span className="coming-control" tabIndex={0} role="group" aria-label={`${label}: ${reason}`} title={reason}>
    <button type="button" className="shell-button" disabled title={reason}>{label}</button>
    <span className="control-tip" aria-hidden="true">{reason}</span>
  </span>;
}

export default function App() {
  const [page, setPage] = useState<Page>('Clusters');
  const content = useRef<HTMLElement>(null);
  useEffect(() => { if (content.current) content.current.scrollTop = 0; }, [page]);
  const [exampleOpen, setExampleOpen] = useState(false);
  const [engineReady, setEngineReady] = useState(false);
  const ready = useCallback(() => setEngineReady(true), []);
  const [setup, setSetup] = useState<SetupStatus>();
  const [checking, setChecking] = useState(true);
  const [model, setModel] = useState<ModelState>('idle');
  const [agentName, setAgentName] = useState<string>();
  const [exampleState, setExampleState] = useState<'loading' | 'loaded' | 'failed'>('loading');
  const [run, setRun] = useState<RunSnapshot>();
  const observeSetup = useCallback((status: SetupStatus | undefined) => { setSetup(status); setChecking(false); }, []);
  const observePair = useCallback((pair: ExamplePair | undefined) => {
    setExampleState(pair ? 'loaded' : 'failed');
    const frontMatter = pair?.agent.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/)?.[1];
    setAgentName(frontMatter?.match(/^name:\s*([a-z0-9][a-z0-9-]*)\s*$/m)?.[1]);
  }, []);
  const [setupLine, detail, tone] = setup ? workspaceCopy[setup.localVm]
    : checking ? ['Checking workspace…', 'Checking workspace…', 'wait'] : ['Workspace unavailable', 'Workspace unavailable', 'wait'];
  const workspaceReady = setup?.localVm === 'ready';
  const modelDetail = model === 'connecting' ? 'Connecting the AI…' : model === 'connected' ? 'AI connected'
    : model === 'failed' ? 'AI connection needs attention' : detail;
  const cases = run?.report?.cases;
  const checks = run?.state === 'running' ? 'Running checks…' : cases
    ? `${cases.filter(check => check.verdict === 'passed').length} of ${cases.length} checks passing` : undefined;
  const failedChecks = cases?.filter(check => check.verdict === 'failed').length ?? 0;
  const agentStatus = run?.state === 'running' ? 'running checks'
    : failedChecks ? `${failedChecks} ${failedChecks === 1 ? 'check' : 'checks'} failing`
    : cases?.some(check => check.verdict === 'unknown') ? 'check result unknown'
    : cases ? 'checks passing' : 'read-only';
  const tabStatus: Record<Page, string> = {
    Overview: checks ?? 'No agents yet',
    Designer: agentName ? `${agentName}, ${agentStatus}` : !exampleOpen ? 'No agent open'
      : exampleState === 'failed' ? 'Example unavailable' : exampleState === 'loaded' ? 'Read-only example' : 'Opening example…',
    Clusters: workspaceReady && run?.state === 'complete' ? 'This computer ready'
      : workspaceReady && model === 'connected' ? 'AI connected' : setupLine,
  };
  const copy = emptyCopy[page];
  return (
    <div className="app-shell">
      <header className="tab-bar">
        <span className="brand"><img src={mark} width="32" height="32" alt="Orka" /><b>Bridge</b></span>
        <nav className="page-nav" aria-label="Main">
          {pages.map(name => (
            <button key={name} className="page-tab" type="button"
              aria-label={name} aria-describedby={`tab-status-${name}`}
              aria-current={page === name ? 'page' : undefined}
              onClick={() => setPage(name)}>
              <span className="icon-tile"><PageIcon page={name} /></span>
              <span className="tab-copy"><b>{name}</b><small id={`tab-status-${name}`} title={tabStatus[name]}>{tabStatus[name]}</small></span>
            </button>
          ))}
        </nav>
        <div className="chrome-tools">
          <span className="coming-control search-control" tabIndex={0} role="group" aria-label="Search: Search is coming" title="Search is coming">
            <button type="button" className="shell-search" disabled title="Search is coming">
              <svg className="icon" viewBox="0 0 24 24" aria-hidden="true"><circle cx="10" cy="10" r="6" /><path d="m15 15 5 5" /></svg>
              <span>Search</span><kbd>{/Mac/i.test(navigator.platform) ? '⌘K' : 'Ctrl K'}</kbd>
            </button>
            <span className="control-tip" aria-hidden="true">Search is coming</span>
          </span>
          <span className="host-chip" title={detail}><span className={`live-dot ${tone}`} aria-hidden="true" />This computer<span className="sr-only">: {detail}</span></span>
        </div>
      </header>
      <main ref={content} className="page-content" aria-labelledby="page-title">
        {page !== 'Clusters' && <div className="page-heading">
          <h1 id="page-title">{page}</h1>
          {page === 'Designer' && <div className="heading-actions">
            <ComingButton label="New agent" reason="Creating agents is coming" />
            <ComingButton label="Send to a cluster" reason="Sending to a cluster is coming" />
          </div>}
        </div>}
        <div hidden={page !== 'Clusters'}>
          <Setup active={page === 'Clusters'} onReady={ready} onStatus={observeSetup} onModelState={setModel}
            onOpenExample={() => { setExampleOpen(true); setPage('Designer'); }} />
        </div>
        {exampleOpen && <div hidden={page !== 'Designer'}><Example onPair={observePair} onRun={setRun} /></div>}
        {page !== 'Clusters' && !(page === 'Designer' && exampleOpen) && <section className="card empty-state">
          <span className="icon-tile empty-icon"><PageIcon page={page} /></span>
          <div>
            <h2>{copy.title}</h2>
            <p className="empty-status">Nothing is connected yet.</p>
            <p className="muted">{copy.detail}</p>
          </div>
        </section>}
        {page === 'Clusters' && <details className="engine-diagnostics"><summary>Engine diagnostics</summary><EngineStatus key={String(engineReady)} /></details>}
      </main>
      <footer className="status-bar">
        <span className={`live-dot ${tone}`} aria-hidden="true" />
        <span>{workspaceReady ? 'This computer ready' : setupLine}</span>
        {detail !== setupLine || workspaceReady ? <><span aria-hidden="true">·</span><span>{workspaceReady ? modelDetail : detail}</span></> : null}
        {agentName && <><span aria-hidden="true">·</span><span>Read-only example</span></>}
        {checks && <span className="checks-status">{checks}</span>}
      </footer>
    </div>
  );
}
