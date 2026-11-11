import { useEffect, useRef, useState } from 'react';
import { CardSection, Details, Elapsed, GeneratedValue, LongCard, StatusIcon, StatusPill, type Status } from '../components/DesignSystem';
import { errors, runCall, type RunSnapshot } from './run-api';
import { sameSource, type SourceId } from './agent-api';
export default function Run({ onSnapshot, blocked = false, stale = false, onStart, source }: { onSnapshot?: (snapshot: RunSnapshot | undefined) => void; blocked?: boolean; stale?: boolean; onStart?: () => void; source?: SourceId }) {
  const [snapshot, setSnapshot] = useState<RunSnapshot>();
  const [message, setMessage] = useState<string>();
  const [sending, setSending] = useState(false);
  const active = useRef(false);
  const sourceCurrent = sameSource(source, snapshot?.source);
  useEffect(() => {
    active.current = true;
    const observe = async () => {
      const result = await runCall('setup_run_status');
      if (!active.current) return;
      if (result.ok) { setSnapshot(result.snapshot); setMessage(undefined); } else setMessage(result.message);
    };
    void observe(); const timer = window.setInterval(() => { void observe(); }, 500);
    return () => { active.current = false; window.clearInterval(timer); };
  }, []);
  useEffect(() => { if (snapshot || message) onSnapshot?.(message ? undefined : snapshot ? { ...snapshot, sourceCurrent } : undefined); }, [snapshot, message, onSnapshot, sourceCurrent]);
  const start = async () => {
    if (sending || blocked || !snapshot?.canRun) return;
    setSending(true);
    const result = await runCall('setup_run_example');
    if (!active.current) return;
    if (result.ok) { onStart?.(); setSnapshot(result.snapshot); setMessage(undefined); } else setMessage(result.message);
    setSending(false);
  };
  const running = sending || snapshot?.state === 'running';
  const outdated = stale || (sourceCurrent === false && !!snapshot?.report);
  const uncertain = snapshot?.state === 'unknown' || snapshot?.report?.verdict === 'unknown';
  const failed = snapshot?.error && !(snapshot.error === 'model' && snapshot.modelConnected);
  const state: Status = outdated ? 'stale' : running ? 'running' : uncertain ? 'not-started' : failed ? 'failed' : snapshot?.report?.verdict === 'passed' ? 'passed' : snapshot?.report?.verdict === 'failed' ? 'failed' : 'not-started';
  return <section className="example-run" aria-label="Test on the local VM">
    <LongCard title="Test on the local VM" status={<StatusPill state={state}>{state === 'not-started' && uncertain ? 'Result unknown' : undefined}</StatusPill>} summary={<>Run creates an Orka Agent and Task from these exact files.{snapshot?.modelConnected === false && ' Connect the model in Clusters first.'}</>}
      action={<button type="button" className="setup-button" disabled={blocked || running || !snapshot?.canRun} title={blocked ? 'Save your changes before running the check' : running ? 'Check is running' : !snapshot?.canRun ? 'Connect your workspace and AI in Clusters first' : undefined} onClick={() => { void start(); }}>Run test</button>}>
      <CardSection title="What happened">
        {outdated && <p role="status">Not run since your change. Any result below is from the earlier file.</p>}
        {running && <><p role="status">Running on the local VM…</p><Elapsed /></>}
        {message && <p role="status">{message}</p>}
        {snapshot?.error && !(snapshot.error === 'model' && snapshot.modelConnected) && <p role="alert">{errors[snapshot.error]}</p>}
        {snapshot?.report && <div className={outdated ? 'result-stale' : undefined}>
          <p role="status">{outdated ? 'Earlier test verdict' : 'Test verdict'}: {snapshot.report.verdict}</p>
          {snapshot.report.cases.map(c => <div className="result-row" key={c.id}>
            <StatusIcon state={outdated ? 'stale' : c.verdict === 'unknown' ? 'not-started' : c.verdict} /><h4><code>{c.id}</code></h4>
            <p>Task: {c.phase}</p>
            <h4>Answer</h4>
            {c.answer !== null ? <p className="reading">{c.answer}</p> : <p>No answer was observed.</p>}
            <h4>Completed tool calls</h4>
            {c.completedTools.length ? <ul>{c.completedTools.map(tool => <li key={tool}><code>{tool}</code></li>)}</ul> : <p>No completed tool calls were observed. Missing evidence is unknown.</p>}
            <h4>Assertions</h4>
            <ul>{c.assertions.map(a => <li key={a.id}><span>{a.kind}: {a.verdict}</span> <code className="muted">({a.id})</code></li>)}</ul>
          </div>)}
        </div>}
      </CardSection>
      {(!!snapshot?.acknowledged.length || snapshot?.pending || snapshot?.source) && <Details hint="Resources and file hashes">
        {snapshot.source && <><GeneratedValue label="Agent file hash" value={snapshot.source.agent} /><GeneratedValue label="Check file hash" value={snapshot.source.tests} /></>}
        {snapshot.acknowledged.map(r => <GeneratedValue key={r.uid} label={r.kind} value={r.name} />)}
        {snapshot.pending && <GeneratedValue label={`Possibly created ${snapshot.pending.kind}`} value={snapshot.pending.name} />}
      </Details>}
    </LongCard>
  </section>;
}
