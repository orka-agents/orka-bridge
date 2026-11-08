import { useEffect, useRef, useState } from 'react';
import Model from './Model';
import { type LocalVmState, type SetupStatus, selectLocal, setupStatus } from './state';
import Welcome, { hasSeenWelcome, welcomePreference } from './Welcome';
import { formatSize, workspaceRequirements, type WorkspaceRequirements } from './requirements';
import './welcome.css';

const pending = (state: LocalVmState) => ['preparing', 'installing', 'starting', 'closing'].includes(state);
const runtimeCopy: Record<LocalVmState, string> = {
  notIntegrated: 'Workspace setup is not included in this build. Nothing will be downloaded or started.',
  idle: 'Continue to get your workspace ready.',
  preparing: 'Checking your workspace files…',
  installing: 'Getting your workspace ready…',
  starting: 'Starting your workspace…',
  ready: 'Your workspace is running on this computer.',
  closing: 'Stopping your workspace…',
  closed: 'Your workspace has stopped.',
  failed: 'Your workspace could not start or its shutdown could not be confirmed. Existing files were kept.',
};
export default function Setup({ onOpenExample, onReady }: { onOpenExample: () => void; onReady: () => void }) {
  const [status, setStatus] = useState<SetupStatus>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(true);
  const [check, setCheck] = useState(0);
  const [seen, setSeen] = useState(hasSeenWelcome);
  const [inside, setInside] = useState(false);
  const [requirements, setRequirements] = useState<WorkspaceRequirements | null>();
  const epoch = useRef(0);
  const heading = useRef<HTMLHeadingElement>(null);
  const focusChoice = useRef(false);
  useEffect(() => {
    if (focusChoice.current) { heading.current?.focus(); focusChoice.current = false; }
  }, [seen]);
  useEffect(() => {
    let active = true;
    void workspaceRequirements().then(value => { if (active) setRequirements(value); });
    return () => { active = false; };
  }, []);
  useEffect(() => {
    const request = ++epoch.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    void setupStatus().then(result => {
      if (request !== epoch.current) return;
      if (result.ok) { setStatus(result.status); setError(undefined); }
      else { setStatus(undefined); setError(result.message); }
      const waiting = result.ok && pending(result.status.localVm);
      setBusy(waiting);
      if (waiting) timer = setTimeout(() => setCheck(value => value + 1), 250);
    });
    return () => { epoch.current++; clearTimeout(timer); };
  }, [check]);
  useEffect(() => { if (status?.localVm === 'ready') onReady(); }, [status?.localVm, onReady]);

  function start() {
    try { localStorage.setItem(welcomePreference, '1'); } catch { /* A presentation preference must not block setup. */ }
    focusChoice.current = true;
    setSeen(true);
  }
  async function chooseLocal() {
    if (busy || !status || status.selected === 'localVm') return;
    const request = ++epoch.current;
    setBusy(true); setError(undefined);
    const result = await selectLocal();
    if (request !== epoch.current) return;
    if (result.ok) setStatus(result.status);
    else { setStatus(undefined); setError(result.message); }
    const waiting = result.ok && pending(result.status.localVm);
    setBusy(waiting);
    if (waiting) setCheck(value => value + 1);
  }

  if (!status && !error) return <h1 id="page-title">Checking your workspace…</h1>;
  if (!seen && status && status.selected !== 'localVm') return <Welcome titleId="page-title" onStart={start} />;
  const selected = status?.selected === 'localVm';
  return <>
    <div className="workspace-heading">
      <h1 ref={heading} tabIndex={-1} id="page-title">{selected ? 'Get your workspace ready' : 'Where should your agents run?'}</h1>
      {!selected && <p className="muted">You can add more places later.</p>}
    </div>
    <div className={selected ? 'setup' : 'workspace-choice'}>
      {!selected && <>
        <fieldset className="workspace-options" disabled={busy || !status}>
          <legend className="workspace-sr-only">Where agents run</legend>
          <div className="workspace-option workspace-selected">
            <input id="workspace-local" type="radio" name="workspace" aria-label="On this computer" aria-describedby="local-description" checked readOnly />
            <label htmlFor="workspace-local">On this computer</label><span className="workspace-badge">recommended</span>
            <div className="workspace-option-detail">
              <p id="local-description">Bridge runs a ready-made workspace quietly in the background. It needs virtualization turned on{requirements ? `, ${formatSize(requirements.memoryBytes)} of memory and a ${formatSize(requirements.imageBytes)} workspace image.` : '.'}</p>
              {requirements && <p>The data disk can grow to {formatSize(requirements.stateDiskBytes)}, in addition to the image.</p>}
              {requirements === undefined && <p>Reading this build's workspace size…</p>}
              {requirements === null && status?.localVm !== 'notIntegrated' && <p>Workspace size information is unavailable in this build.</p>}
              <button className="workspace-disclosure" type="button" aria-expanded={inside} aria-controls="workspace-inside" onClick={() => setInside(value => !value)}>
                <svg className="icon" viewBox="0 0 24 24" aria-hidden="true"><path d={inside ? 'm6 15 6-6 6 6' : 'm6 9 6 6 6-6'} /></svg>What's inside
              </button>
              <p id="workspace-inside" hidden={!inside}>A small virtual machine with Orka, the open-source engine agents run on. Its files are checked against their published digests before it starts.</p>
            </div>
          </div>
          <div className="workspace-coming workspace-option" title="Coming" tabIndex={0} role="group" aria-label="On a cluster you already have: Coming">
            <input id="workspace-cluster" type="radio" name="workspace" aria-label="On a cluster you already have" disabled checked={false} readOnly />
            <label htmlFor="workspace-cluster">On a cluster you already have</label><span className="muted workspace-expert">for Kubernetes users</span>
            <p className="workspace-option-detail">Pick a kubectl context and Bridge uses the Orka on that cluster.</p>
            <span className="workspace-tip" aria-hidden="true">Coming</span>
          </div>
        </fieldset>
        <div className="workspace-choice-footer"><span className="muted">Step 1 of 4</span>
          <button type="button" className="workspace-primary" disabled={busy || !status} onClick={() => { void chooseLocal(); }}>Continue</button>
        </div>
        {status?.localVm === 'notIntegrated' && <p className="muted">{runtimeCopy.notIntegrated}</p>}
      </>}
      {selected && <section className="card" aria-label="Workspace setup" aria-busy={busy}>
        <p role="status">{status && runtimeCopy[status.localVm]}</p>
        {status?.localVm === 'failed' && status.localVmError && <p role="alert">{status.localVmError}</p>}
        {status?.localVm === 'ready' && <button type="button" className="setup-button" onClick={onOpenExample}>Open example</button>}
      </section>}
      {error && <div role="alert"><p>{error}</p>
        <button type="button" className="setup-button" disabled={busy}
          onClick={() => { setBusy(true); setError(undefined); setCheck(value => value + 1); }}>Check again</button>
      </div>}
      {selected && <Model enabled={status?.localVm === 'ready' && status.model === 'available'} />}
    </div>
  </>;
}
