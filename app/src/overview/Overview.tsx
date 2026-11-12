import { useEffect, useRef, useState } from 'react';
import { CardSection, Details, GeneratedValue, LongCard, StatusPill, type Status } from '../components/DesignSystem';
import { sameSource } from '../designer/agent-api';
import { readAgents, readFiles, type Agent, type LiveAgents, type Observation, type SavedFile, type SavedFiles } from './api';
import './overview.css';

type Props = { active: boolean; runBusy: boolean; workspaceKey: string; onOpenDocument: (id: string) => void; onOpenExample: () => void; onChooseWorkspace: () => void; onCounts?: (label: string) => void };
type CompleteAgents = LiveAgents & { state: 'complete'; installation: string; observedAt: number };
const complete = (v?: LiveAgents): v is CompleteAgents => v?.state === 'complete' && v.installation !== null && v.observedAt !== null;
const when = (ms: number) => new Date(ms).toLocaleString();
function declarations(file: SavedFile) {
  return file.summary ? <><p>Model declared in file: {file.summary.model}</p><p>Tools declared in file: {file.summary.tools.length ? file.summary.tools.join(', ') : 'None'}</p></> : <p>{file.reason ?? 'File declarations unavailable'}</p>;
}
function identity(agent: Agent, installation: string, observedAt: number) {
  return <dl className="overview-identity"><div><dt>Name</dt><dd>{agent.name}</dd></div><div><dt>Namespace</dt><dd>{agent.namespace}</dd></div><div><dt>UID</dt><dd>{agent.uid}</dd></div><div><dt>Installation</dt><dd>{installation}</dd></div><div><dt>Bridge observed</dt><dd>{when(observedAt)}</dd></div></dl>;
}
function associated(live: LiveAgents, agent: Agent): Observation | undefined {
  const o = live.observation;
  return o && o.installation === live.installation && o.namespace === agent.namespace && o.agentUid === agent.uid ? o : undefined;
}
export default function Overview({ active, runBusy, workspaceKey, onOpenDocument, onOpenExample, onChooseWorkspace, onCounts }: Props) {
  const [files, setFiles] = useState<SavedFiles>();
  const [live, setLive] = useState<LiveAgents>();
  const [last, setLast] = useState<CompleteAgents>();
  const [pending, setPending] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const epoch = useRef(0);
  const knownWorkspace = useRef(workspaceKey);
  useEffect(() => {
    const request = ++epoch.current;
    if (knownWorkspace.current !== workspaceKey) {
      knownWorkspace.current = workspaceKey; setLive(undefined); setLast(undefined);
    }
    if (!active) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    setPending(runBusy);
    async function list(retried = false) {
      const value = await readAgents();
      if (request !== epoch.current) return;
      setLive(value);
      if (complete(value)) { setLast(value); setPending(false); }
      else if (value.state === 'busy' && value.runActive) setPending(true);
      else if (value.state === 'busy' && !retried) timer = setTimeout(() => { void list(true); }, 250);
    }
    void readFiles().then(value => {
      if (request !== epoch.current) return;
      setFiles(value);
      if (runBusy || value.runActive) { setPending(true); return; }
      void list();
    });
    return () => { epoch.current++; clearTimeout(timer); };
  }, [active, runBusy, workspaceKey, refresh]);
  const observed = complete(live) ? live : last;
  const fileLabel = files?.state === 'complete' ? `${files.files.length} saved ${files.files.length === 1 ? 'file' : 'files'}` : 'Saved files unavailable';
  const agentLabel = live?.state === 'complete' && !pending ? `${live.agents.length} ${live.agents.length === 1 ? 'Agent' : 'Agents'}` : 'Agent count unavailable';
  const counts = files ? `${fileLabel} · ${agentLabel}` : 'Reading saved files…';
  useEffect(() => { if (active) onCounts?.(counts); }, [active, counts, onCounts]);
  const saved = files?.files ?? [];
  const visibleFiles = saved.filter(file => !observed?.agents.some(agent => {
    const o = associated(observed, agent);
    return o?.documentId === file.documentId && sameSource(file.source ?? undefined, o.source) === true;
  }));
  const allEmpty = files?.state === 'complete' && !saved.length && live?.state === 'complete' && !live.agents.length && !pending;
  const message = pending ? 'Refresh pending until checks finish' : live?.state === 'busy' ? 'Workspace busy'
    : live?.state === 'unavailable' ? live.reason === 'image' ? 'Agent list unavailable with this Mac image' : live.reason === 'stopped' ? 'Workspace stopped' : 'Could not read agents'
    : !live ? 'Reading agents…' : 'Workspace available';
  return <section className="overview" aria-label="Agents on this computer" hidden={!active}>
    <div className="page-heading"><h1 id={active ? 'page-title' : undefined}>Your agents</h1><button type="button" className="workspace-secondary" onClick={() => setRefresh(value => value + 1)}>Refresh</button></div>
    <div className="overview-place"><h2>This computer</h2><p role="status">{message}</p></div>
    <p className="muted overview-counts">{counts}</p>
    {pending && !observed && <p>Workspace busy running checks</p>}
    {observed && (live?.state !== 'complete' || pending) && <p className="muted">Last seen {when(observed.observedAt)}</p>}
    {live?.state === 'unavailable' && live.reason === 'stopped' && <button type="button" className="workspace-link" onClick={onChooseWorkspace}>Open Clusters</button>}
    {files?.state === 'unavailable' && <p role="alert">Could not read saved files</p>}
    {files?.state === 'complete' && !saved.length && !allEmpty && <p>No saved agent files</p>}
    {allEmpty && <div className="card overview-empty"><h3>No agents on this computer yet</h3><button type="button" className="workspace-primary" onClick={onOpenExample}>Open example</button></div>}
    <div className="overview-cards">
      {visibleFiles.map(file => <LongCard key={file.documentId} title={file.summary?.name || file.name} status={<span className="muted">Saved file</span>}
        action={file.state === 'readable' ? <button type="button" className="workspace-secondary" aria-label={`Open ${file.name}`} onClick={() => onOpenDocument(file.documentId)}>Open</button> : undefined}
        summary={file.summary?.description ?? (file.state === 'unreadable' ? 'Saved files unavailable' : 'Agent definition on this computer')}>
        <CardSection title="File declarations">{declarations(file)}</CardSection>
        <CardSection title="Checks"><p>Not run in this session for this saved version</p></CardSection>
        {file.source && <Details hint="Saved file hashes"><GeneratedValue label="Agent file hash" value={file.source.agent} /><GeneratedValue label="Check file hash" value={file.source.tests} /></Details>}
      </LongCard>)}
      {observed?.agents.map(agent => {
        const o = associated(observed, agent);
        const file = o && saved.find(f => f.documentId === o.documentId);
        const sourceUnavailable = !!o && !file?.source;
        const stale = !!o && (sourceUnavailable || sameSource(file?.source ?? undefined, o.source) !== true);
        const state: Status = stale ? 'stale' : o?.state === 'running' ? 'running' : o?.state === 'complete' && o.report?.verdict !== 'unknown' ? o.report?.verdict ?? 'not-started' : 'not-started';
        const check = sourceUnavailable ? 'Current saved version unavailable' : stale ? 'Checks not run since your change' : o?.state === 'running' ? 'Running checks' : o?.state === 'complete' && o.report && o.report.verdict !== 'unknown' ? `Checks ${o.report.verdict} in this session` : o ? 'Check result unknown' : 'Not tested by Bridge in this session';
        return <LongCard key={`${observed.installation}/${agent.namespace}/${agent.uid}`} title={file?.summary?.name || file?.name || agent.name}
          status={<StatusPill state={state}>Orka Agent</StatusPill>}
          action={file?.state === 'readable' ? <button type="button" className="workspace-secondary" aria-label={`Open ${file.name}`} onClick={() => onOpenDocument(file.documentId)}>Open</button> : undefined}
          summary={file?.summary?.description ?? 'Origin unavailable'}>
          <CardSection title="Checks"><p>{check}</p><p>{o ? `Bridge run observed: ${when(o.observedAt)}` : 'Last run: unavailable'}</p></CardSection>
          {file && <CardSection title="File declarations">{declarations(file)}</CardSection>}
          <CardSection title="Observed settings"><p>Type: unavailable · Model: unavailable · Tools: unavailable</p></CardSection>
          {file?.state === 'readable' ? <Details hint="Observed identity">{identity(agent, observed.installation, observed.observedAt)}</Details>
            : <details className="card-details"><summary>Open</summary><p>Read-only observation. No file is associated for editing.</p>{identity(agent, observed.installation, observed.observedAt)}</details>}
        </LongCard>;
      })}
    </div>
  </section>;
}
