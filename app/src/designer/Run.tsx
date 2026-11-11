import { useEffect, useRef, useState } from 'react';
import { errors, runCall, type RunSnapshot } from './run-api';
import { SAMPLE_DOCUMENT_ID, sameSource, type SourceId } from './agent-api';
export default function Run({ documentId = SAMPLE_DOCUMENT_ID, onSnapshot, blocked = false, stale = false, onStart, source }: { documentId?: string; onSnapshot?: (snapshot: RunSnapshot | undefined) => void; blocked?: boolean; stale?: boolean; onStart?: () => void; source?: SourceId }) {
  const [received, setSnapshot] = useState<RunSnapshot>();
  const snapshot = received?.documentId === documentId ? received : undefined;
  const [message, setMessage] = useState<string>();
  const [sending, setSending] = useState(false);
  const epoch = useRef(0);
  const sourceCurrent = sameSource(source, snapshot?.source);
  useEffect(() => {
    const request = ++epoch.current;
    setMessage(undefined); setSending(false);
    const observe = async () => {
      const result = await runCall('agent_run_status', documentId);
      if (request !== epoch.current) return;
      if (result.ok) { setSnapshot(result.snapshot); setMessage(undefined); } else setMessage(result.message);
    };
    void observe(); const timer = window.setInterval(() => { void observe(); }, 500);
    return () => { epoch.current++; window.clearInterval(timer); };
  }, [documentId]);
  useEffect(() => { if (snapshot || message) onSnapshot?.(message ? undefined : snapshot ? { ...snapshot, sourceCurrent } : undefined); }, [snapshot, message, onSnapshot, sourceCurrent]);
  const start = async () => {
    if (sending || blocked || !snapshot?.canRun) return;
    setSending(true);
    const request = epoch.current;
    const result = await runCall('agent_run', documentId);
    if (request !== epoch.current) return;
    if (result.ok) { onStart?.(); setSnapshot(result.snapshot); setMessage(undefined); } else setMessage(result.message);
    setSending(false);
  };
  const running = sending || snapshot?.state === 'running';
  const outdated = stale || (sourceCurrent === false && !!snapshot?.report);
  return <div className="example-run" aria-labelledby="run-title">
    <h3 id="run-title">Test on the local VM</h3>
    <p className="muted">Run creates an Orka Agent and Task from these exact files.{snapshot?.modelConnected === false && ' Connect the model in Clusters first.'}</p>
    <button type="button" className="setup-button" disabled={blocked || running || !snapshot?.canRun} title={blocked ? 'Save your changes before running the check' : running ? 'Check is running' : !snapshot?.canRun ? 'Connect your workspace and AI in Clusters first' : undefined} onClick={() => { void start(); }}>Run test</button>
    {snapshot?.busy && snapshot.busy.documentId !== documentId && <p role="status">{snapshot.busy.name} is running a test.</p>}
    {outdated && <p role="status">Not run since your change. Any result below is from the earlier file.</p>}
    {running && <p role="status">Running on the local VM…</p>}
    {message && <p role="status">{message}</p>}
    {snapshot?.error && !(snapshot.error === 'model' && snapshot.modelConnected) && <p role="alert">{errors[snapshot.error]}</p>}
    {snapshot?.pending && <p>Possibly created {snapshot.pending.kind}: {snapshot.pending.name}</p>}
    {snapshot?.report && <>
      <p role="status">{outdated ? 'Earlier test verdict' : 'Test verdict'}: {snapshot.report.verdict}</p>
      {snapshot.report.cases.map(c => <div key={c.id}>
        <h4>{c.id}</h4>
        <p>Task: {c.phase}</p>
        <h4>Answer</h4>
        {c.answer !== null ? <pre className="example-source" tabIndex={0}>{c.answer}</pre> : <p>No answer was observed.</p>}
        <h4>Completed tool calls</h4>
        {c.completedTools.length ? <ul>{c.completedTools.map(tool => <li key={tool}>{tool}</li>)}</ul> : <p>No completed tool calls were observed. Missing evidence is unknown.</p>}
        <h4>Assertions</h4>
        <ul>{c.assertions.map(a => <li key={a.id}><span>{a.kind}: {a.verdict}</span> <span className="muted">({a.id})</span></li>)}</ul>
      </div>)}
    </>}
    {!!snapshot?.acknowledged.length && <details><summary>Acknowledged resources</summary><ul>{snapshot.acknowledged.map(r => <li key={r.uid}>{r.kind}: {r.name}</li>)}</ul></details>}
  </div>;
}
