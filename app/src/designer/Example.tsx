import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { loadExample, type ExamplePair } from '../setup/state';
import { SAMPLE_DOCUMENT_ID, agentCall, agentInfo, type AgentPair, type SaveState } from './agent-api';
import Run from './Run';
import { readWorkspaceCatalog, type WorkspaceCatalogReply } from '../catalog';
import AgentParts from './AgentParts';
import { readForm } from './agent-form';
import type { RunSnapshot } from './run-api';
import type { SelectedPlace } from '../clusters/api';
import Send, { SendEntry, SendHistory } from '../clusters/Send';

export default function Example({ documentId = SAMPLE_DOCUMENT_ID, active = true, readOnly = false, preview = readOnly, remote = false, catalogKey, place = { kind: 'thisComputer' }, onChooseCluster, actions, onPair, onRun, onSaveState, onStale }: {
  documentId?: string; active?: boolean; readOnly?: boolean; preview?: boolean; remote?: boolean; catalogKey?: number; place?: SelectedPlace; onChooseCluster?: () => void; actions?: ReactNode;
  onPair?: (pair: ExamplePair | undefined) => void; onRun?: (snapshot: RunSnapshot | undefined) => void;
  onSaveState?: (state: SaveState) => void; onStale?: (stale: boolean) => void;
}) {
  const [boundDocument, setBoundDocument] = useState(documentId);
  const [catalogAttempt, setCatalogAttempt] = useState(0);
  const [catalog, setCatalog] = useState<{ key: number; attempt: number; reply?: WorkspaceCatalogReply }>();
  const catalogEnabled = catalogKey !== undefined && !readOnly && !remote;
  useEffect(() => {
    if (!catalogEnabled || catalogKey === undefined) return;
    const controller = new AbortController();
    void readWorkspaceCatalog(controller.signal).then(reply => {
      if (!controller.signal.aborted) setCatalog({ key: catalogKey, attempt: catalogAttempt, reply });
    });
    return () => controller.abort();
  }, [catalogEnabled, catalogKey, catalogAttempt]);
  const currentCatalog = catalogEnabled && catalog?.key === catalogKey && catalog.attempt === catalogAttempt ? catalog : undefined;
  const [pair, setPair] = useState<AgentPair>();
  const [draft, setDraft] = useState('');
  const [error, setError] = useState<string>();
  const [saveState, setSaveState] = useState<SaveState>('saved');
  const [pendingSoul, setPendingSoul] = useState(false);
  const pendingSoulCurrent = useRef(false);
  const [reloading, setReloading] = useState(false);
  const reloadAdmitted = useRef(false);
  const reloadEpoch = useRef(0);
  const [view, setView] = useState<'Agent' | 'File'>('Agent');
  const [stale, setStale] = useState(false);
  const observePendingSoul = useCallback((pending: boolean) => { pendingSoulCurrent.current = pending; setPendingSoul(pending); if (pending) setStale(true); }, []);
  const [sendOpen, setSendOpen] = useState(false);
  const [localRun, setLocalRun] = useState<RunSnapshot>();
  const observeRun = useCallback((value: RunSnapshot | undefined) => { setLocalRun(value); onRun?.(value); }, [onRun]);
  const openSend = () => {
    setSendOpen(true);
    requestAnimationFrame(() => { const panel = document.querySelector<HTMLElement>('.send-panel'); panel?.focus(); panel?.scrollIntoView?.({ block: 'start' }); });
  };
  const latest = useRef(''); const saved = useRef(''); const busy = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const lineEnding = useRef('\n'); const needsCommit = useRef(false);
  useEffect(() => {
    let mounted = true;
    const open = preview && boundDocument === SAMPLE_DOCUMENT_ID ? loadExample().then(result => { if (!result.ok) throw Error(result.message); return result.pair; }) : agentCall('agent_open', undefined, boundDocument);
    void open.then(value => {
      if (!mounted) return;
      setPair(value); setDraft(value.agent); latest.current = saved.current = value.agent;
      lineEnding.current = value.agent.includes('\r\n') ? '\r\n' : '\n'; onPair?.(value);
    }, () => { if (mounted) { setError('Your agent folder could not be opened. Open it again from Designer.'); onPair?.(undefined); } });
    return () => { mounted = false; };
  }, [onPair, preview, boundDocument]);
  useEffect(() => { onSaveState?.(pendingSoul ? 'dirty' : saveState); }, [saveState, pendingSoul, onSaveState]);
  useEffect(() => { onStale?.(stale); }, [stale, onStale]);

  const flush = useCallback(async (retry = false) => {
    clearTimeout(timer.current);
    if (readOnly || busy.current) return;
    if (!retry && !needsCommit.current && latest.current === saved.current) { setSaveState('saved'); return; }
    busy.current = true; setSaveState('saving'); setError(undefined);
    try {
      do {
        const content = latest.current;
        const value = await agentCall('agent_save', content, boundDocument);
        if (value.agent !== content) throw Error('Save did not confirm the current file.');
        saved.current = content; setPair(value); onPair?.(value);
        // Fingerprinted replies let Run distinguish older results from unsaved edits.
        if (value.source) setStale(latest.current !== content);
      } while (latest.current !== saved.current);
      needsCommit.current = false; setSaveState('saved');
    } catch { needsCommit.current = true; setSaveState('error'); setError('Your changes could not be saved. Your draft is still here.'); }
    finally { busy.current = false; }
  }, [onPair, readOnly, boundDocument]);
  useEffect(() => () => { clearTimeout(timer.current); reloadEpoch.current++; }, []);
  useEffect(() => {
    if (documentId === boundDocument || busy.current || pendingSoul || saveState === 'saving' || saveState === 'error') return;
    if (saveState === 'dirty') { void flush(); return; }
    reloadEpoch.current++; reloadAdmitted.current = false; setReloading(false);
    setPair(undefined); setDraft(''); latest.current = saved.current = '';
    setLocalRun(undefined); setStale(false); setError(undefined); setSendOpen(false);
    onPair?.(undefined); onRun?.(undefined); setBoundDocument(documentId);
  }, [documentId, boundDocument, pendingSoul, saveState, flush, onPair, onRun]);
  const edit = (value: string, rawFile = false) => {
    if (readOnly) return;
    const content = rawFile ? value.replace(/\r\n/g, '\n').replaceAll('\n', lineEnding.current) : value;
    if (content === latest.current) return;
    latest.current = content; setDraft(content); setStale(!pair?.source || content !== saved.current); setError(undefined);
    setSaveState('dirty'); clearTimeout(timer.current);
    timer.current = setTimeout(() => { void flush(); }, 500);
  };
  async function reloadSource() {
    if (readOnly || reloadAdmitted.current || busy.current || needsCommit.current || pendingSoulCurrent.current || latest.current !== saved.current) return;
    reloadAdmitted.current = true; setReloading(true);
    const request = reloadEpoch.current;
    const before = latest.current;
    try {
      const value = await agentCall('agent_open', undefined, boundDocument);
      // A file read must not replace edits made while it was in flight, including
      // the separate soul field whose draft is not yet in latest.current.
      if (request !== reloadEpoch.current || latest.current !== before || saved.current !== before || busy.current || needsCommit.current || pendingSoulCurrent.current) return;
      setPair(value); setDraft(value.agent); latest.current = saved.current = value.agent;
      lineEnding.current = value.agent.includes('\r\n') ? '\r\n' : '\n';
      setStale(false); onPair?.(value);
    } finally { if (request === reloadEpoch.current) { reloadAdmitted.current = false; setReloading(false); } }
  }
  const info = { ...agentInfo(draft), description: readForm(draft).description };
  const saveLabel = pendingSoul ? 'Unsaved changes' : { saved: '', dirty: 'Unsaved changes', saving: 'Saving…', error: 'Not saved' }[saveState];
  const sendBlocked = !pair ? error ? 'Reopen your agent before sending' : 'Wait for your agent folder to open'
    : reloading ? 'Wait for your saved files to reload' : readOnly ? 'Sending is unavailable while this example is read-only during setup'
    : saveState !== 'saved' || pendingSoul ? 'Save your changes first' : undefined;
  return <section className="designer" aria-labelledby={active ? 'page-title' : undefined}>
    <div className="page-heading designer-heading">
      <div className="agent-heading-copy"><h1 className="agent-name" id={active ? 'page-title' : undefined}>{info.name ?? (pair ? 'Agent file' : 'Designer')}</h1>{info.description && <p className="muted">{info.description}</p>}</div>
      <div className="heading-actions">
        <div className="view-switch" role="group" aria-label="Designer view">
          <button type="button" aria-pressed={view === 'Agent'} onClick={() => setView('Agent')}>Agent</button>
          <span className="coming-control" tabIndex={0} role="group" aria-label="Chat: Chat is coming" title="Chat is coming"><button type="button" disabled title="Chat is coming">Chat</button><span className="control-tip" aria-hidden="true">Chat is coming</span></span>
          <button type="button" aria-pressed={view === 'File'} disabled={!pair} title={!pair ? 'Wait for your agent folder to open' : undefined} onClick={() => setView('File')}>File</button>
        </div>
        {actions}
        <SendEntry blocked={sendBlocked} failingCheck={!stale && localRun?.sourceCurrent !== false ? localRun?.report?.cases.find(c => c.verdict === 'failed')?.id : undefined}
          onProceed={openSend} onFix={() => { setView('Agent'); requestAnimationFrame(() => document.querySelector<HTMLTextAreaElement>('.instructions-editor')?.focus()); }} />
      </div>
    </div>
    {!pair ? error ? <p role="alert">{error}</p> : <p role="status">Opening your agent…</p> : <>
      <div className="designer-file card">
        <div className="file-toolbar"><h2>{view === 'File' ? `${pair.name ?? 'web-reader'}.agent.md` : 'Instructions and settings'}</h2><span className="muted" role="status">{readOnly ? 'Read-only during setup' : saveLabel}</span></div>
        {view === 'File' ? <>
          <label className="sr-only" htmlFor="agent-file">Agent file</label>
          <p className="muted" id="file-hint">{readOnly ? 'You can read the example while your workspace gets ready. Editing opens after setup.' : 'Changes save automatically in your agent’s folder. Each save is kept in its Git history.'}</p>
          <textarea readOnly={readOnly} id="agent-file" aria-describedby="file-hint" className="agent-editor" spellCheck={false} value={draft} onChange={event => edit(event.target.value, true)} onBlur={() => { void flush(); }} onKeyDown={event => {
            if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') { event.preventDefault(); void flush(); }
          }} />
        </> : null}
        <div hidden={view !== 'Agent'}>
          <AgentParts source={draft} path={pair.path} readOnly={readOnly} catalog={currentCatalog?.reply} catalogLoading={catalogEnabled && !currentCatalog} onPendingEdit={observePendingSoul} onChange={edit} />
          {currentCatalog && (!currentCatalog.reply || currentCatalog.reply.models.state === 'unavailable' || currentCatalog.reply.tools.state === 'unavailable') && <button type="button" className="part-button" onClick={() => setCatalogAttempt(attempt => attempt + 1)}>Refresh catalog</button>}
        </div>
        {error && <div className="save-error"><p role="alert">{error}</p><button className="setup-button" type="button" onClick={() => { void flush(true); }}>Try saving again</button></div>}
      </div>
      <div className="designer-checks card">
        <Send documentId={boundDocument} place={place} source={pair.source} model={readForm(draft).model} local={localRun} stale={stale} blocked={sendBlocked} visible={sendOpen} remote={remote} onChooseCluster={onChooseCluster} onReloadSource={reloadSource} />
        {!remote && readOnly ? <div><h3>Checks</h3><span className="coming-control" tabIndex={0} role="group" aria-label="Run test: Wait for your workspace and AI to be ready" title="Wait for your workspace and AI to be ready"><button type="button" className="setup-button" disabled title="Wait for your workspace and AI to be ready">Run test</button><span className="control-tip" aria-hidden="true">Wait for your workspace and AI to be ready</span></span><p className="muted">Your workspace is getting ready. Connect the AI in Clusters before running a check.</p></div>
          : null}
        {!readOnly && <div hidden={remote}><Run documentId={boundDocument} source={pair.source} onSnapshot={observeRun} blocked={remote || pendingSoul || saveState !== 'saved'} stale={stale} onStart={() => { if (latest.current === draft) setStale(false); }} /></div>}
        <details className="check-source"><summary>Check file: {pair.name ?? 'web-reader'}.agent.test.md</summary><pre className="example-source" tabIndex={0} aria-label="Check file, read-only">{pair.tests}</pre></details>
      </div>
    </>}
    {!pair && <div className="designer-checks card"><SendHistory revision={boundDocument} /></div>}
  </section>;
}
