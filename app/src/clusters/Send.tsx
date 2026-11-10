import { useEffect, useRef, useState } from 'react';
import { type SourceId, sameSource } from '../designer/agent-api';
import type { RunSnapshot } from '../designer/run-api';
import type { SelectedPlace } from './api';
import { type SendView, sendCall, sendErrors } from './send-api';
import './clusters.css';

export function SendEntry({ failingCheck, blocked, onProceed, onFix }: { failingCheck?: string; blocked?: string; onProceed: () => void; onFix: () => void }) {
  const [warning, setWarning] = useState(false);
  return <div className="send-entry">
    <button type="button" className="shell-button" disabled={!!blocked} title={blocked} onClick={() => { if (failingCheck) setWarning(true); else onProceed(); }}>Send to a cluster</button>
    {warning && failingCheck && <div className="send-warning" role="note"><p>Check {failingCheck} failed. You can still send this agent.</p>
      <button type="button" className="workspace-secondary" disabled={!!blocked} onClick={() => { setWarning(false); onProceed(); }}>Send anyway</button>
      <button type="button" className="workspace-disclosure" onClick={() => { setWarning(false); onFix(); }}>Fix it first</button>
    </div>}
  </div>;
}
const words = { passed: 'pass', failed: 'fail', unknown: 'unknown' } as const;
export default function Send({ place, source, model, local, stale, blocked, visible, remote, onChooseCluster }: {
  place: SelectedPlace; source?: SourceId; model?: string; local?: RunSnapshot; stale: boolean;
  blocked?: string; visible: boolean; remote: boolean; onChooseCluster?: () => void;
}) {
  const [snapshot, setSnapshot] = useState<SendView | null>(null);
  const [message, setMessage] = useState<string>();
  const [busy, setBusy] = useState(false);
  const admitted = useRef(false); const epoch = useRef(0);
  useEffect(() => {
    let mounted = true; let polling = false;
    async function observe() {
      if (polling || admitted.current) return;
      polling = true; const request = epoch.current;
      try { const result = await sendCall('send_status'); if (mounted && request === epoch.current) { setSnapshot(result); setMessage(undefined); } }
      catch (error) { if (mounted && request === epoch.current) setMessage((error as Error).message); }
      finally { polling = false; }
    }
    void observe(); const timer = window.setInterval(() => { void observe(); }, 500);
    return () => { mounted = false; epoch.current++; window.clearInterval(timer); };
  }, []);
  const target = place.kind === 'cluster' ? place.target : undefined;
  const providerId = place.kind === 'cluster' ? place.providerId : undefined;
  const provider = place.kind === 'cluster' ? place.providers?.find(p => p.id === providerId) : undefined;
  async function mutate(command: 'send_start' | 'send_stop') {
    if (admitted.current) return;
    let args: Record<string, string>;
    if (command === 'send_start') {
      if (blocked || !target || !providerId) return;
      args = { targetId: target.id, providerId };
    } else {
      if (!snapshot) return;
      args = { sendId: snapshot.id };
    }
    admitted.current = true; setBusy(true); const request = ++epoch.current;
    try {
      const result = await sendCall(command, args);
      if (request === epoch.current) { setSnapshot(result); setMessage(undefined); }
    } catch (error) { if (request === epoch.current) setMessage((error as Error).message); }
    finally { admitted.current = false; if (request === epoch.current) setBusy(false); }
  }
  const running = snapshot?.state === 'running';
  const samePlace = snapshot?.targetId === target?.id && snapshot?.providerId === providerId;
  const current = !stale && sameSource(source, snapshot?.source) === true;
  const localCurrent = !stale && sameSource(source, local?.source) === true;
  const targetName = snapshot?.targetName ?? (snapshot?.targetId === target?.id ? target?.context_name : undefined);
  const reason = blocked ?? ( !target || !provider ? 'Choose a cluster and Provider in Clusters first' : undefined);
  return <section tabIndex={-1} className="send-panel" aria-label="Send and cluster checks" hidden={!visible && !remote && !running}>
    <h3>{visible ? 'Send your agent' : 'Checks on your cluster'}</h3>
    <p>The saved agent and check files go together. Each Send creates a new Agent, kept on the cluster even if a check fails.</p>
    <dl className="cluster-details"><div><dt>Source</dt><dd>Your saved agent file{model ? ` · ${model}` : ''}</dd></div>
      <div><dt>This computer</dt><dd>{local?.report ? localCurrent ? `Checks: ${words[local.report.verdict]}` : 'Changed since local checks, or revision not confirmed' : 'No local check result'}</dd></div>
      <div><dt>Destination</dt><dd>{target ? `${target.context_name} · ${target.watched_namespace}` : 'Choose a cluster'}</dd></div>
      {provider && <div><dt>Provider</dt><dd>{provider.name} · {provider.ready ? 'Ready' : 'Not ready'}</dd></div>}
    </dl>
    {(!target || !provider) && <button type="button" className="workspace-secondary" onClick={onChooseCluster} disabled={!onChooseCluster}>Choose a cluster</button>}
    <div className="workspace-step-actions"><button type="button" className="setup-button" disabled={!!reason || busy || running || !!message} title={reason} onClick={() => { void mutate('send_start'); }}>{visible ? 'Send and run checks' : 'Run all checks'}</button>
      {running && <button type="button" className="workspace-secondary" disabled={busy} onClick={() => { void mutate('send_stop'); }}>Stop</button>}
    </div>
    {message && <p role="alert">{message}</p>}
    {running && <p role="status">Running checks on {targetName ?? 'the cluster'}…</p>}
    {snapshot?.stage === 'stopped' && <p role="status">Stopped watching. The cluster may still be running checks.</p>}
    {snapshot?.error && snapshot.stage !== 'stopped' && <p role="alert">{sendErrors[snapshot.error as keyof typeof sendErrors]}</p>}
    {snapshot && !samePlace && <p role="status">This result belongs to another cluster or Provider. It is not validation of your current selection.</p>}
    {snapshot && !current && <p className="muted">This result is for the captured file, not confirmed as your current revision.</p>}
    {snapshot?.state === 'unknown' && !snapshot.report && <p role="status">Checks: unknown</p>}
    {snapshot?.report && <><p role="status">Checks: {words[snapshot.report.verdict]}</p>
      {snapshot.report.cases.map(check => <div key={check.id}><h4>{check.id}: {words[check.verdict]}</h4>
        {check.answer !== null ? <pre className="example-source" tabIndex={0}>{check.answer}</pre> : <p>No answer was observed.</p>}
        {!!check.completedTools.length && <p>Completed tool calls: {check.completedTools.join(', ')}</p>}
        <ul>{check.assertions.map(a => <li key={a.id}>{a.id}: {words[a.verdict]}</li>)}</ul>
      </div>)}
    </>}
    {snapshot && <details><summary>Show details</summary><dl className="cluster-details"><div><dt>Result destination</dt><dd>{targetName ?? 'Not confirmed'}</dd></div>
      <div><dt>Model sent</dt><dd>{snapshot.model ?? 'Not captured'}</dd></div><div><dt>Planned Agent</dt><dd>{snapshot.agentName ?? 'Not prepared'}</dd></div>
      <div><dt>Agent file hash</dt><dd>{snapshot.source?.agent ?? 'Not captured'}</dd></div><div><dt>Check file hash</dt><dd>{snapshot.source?.tests ?? 'Not captured'}</dd></div>
    </dl><ul>{snapshot.acknowledged.map(r => <li key={`${r.kind}-${r.uid}`}>{r.kind}: {r.name}</li>)}</ul>
      {snapshot.pending && <p>Possibly created {snapshot.pending.kind}: {snapshot.pending.name}</p>}
      <p>Results are kept only while Bridge is open. Stop ends watching and prevents further submissions; it does not cancel checks already sent.</p>
    </details>}
  </section>;
}
