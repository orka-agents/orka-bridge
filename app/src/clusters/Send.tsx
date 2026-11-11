import { useEffect, useRef, useState } from 'react';
import { type SourceId, sameSource } from '../designer/agent-api';
import type { RunSnapshot } from '../designer/run-api';
import type { SelectedPlace } from './api';
import { type SendCheck, type SendView, sendCall, sendErrors, sendPreflight } from './send-api';
import './clusters.css';
import { CardSection, Details, Elapsed, GeneratedValue, LongCard, StatusIcon, StatusPill, type Status } from '../components/DesignSystem';

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
export default function Send({ place, source, model, local, stale, blocked, visible, remote, onChooseCluster, onReloadSource }: {
  place: SelectedPlace; source?: SourceId; model?: string; local?: RunSnapshot; stale: boolean;
  blocked?: string; visible: boolean; remote: boolean; onChooseCluster?: () => void; onReloadSource?: () => Promise<void>;
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
  const [refresh, setRefresh] = useState(0);
  const [check, setCheck] = useState<{ key: string; value?: SendCheck; error?: string }>();
  const [reloadError, setReloadError] = useState<string>();
  const targetId = target?.id;
  const agentHash = source?.agent; const testsHash = source?.tests;
  const checkKey = JSON.stringify([targetId, providerId, agentHash, testsHash, refresh]);
  const checkingEnabled = (visible || remote) && !blocked && !!targetId && !!providerId && !!agentHash && !!testsHash;
  useEffect(() => {
    if (!checkingEnabled || !targetId || !providerId) return;
    let mounted = true;
    void sendPreflight(targetId, providerId).then(value => {
      if (mounted) setCheck({ key: checkKey, value });
    }, (error: Error) => { if (mounted) setCheck({ key: checkKey, error: error.message }); });
    return () => { mounted = false; };
  }, [checkingEnabled, checkKey, targetId, providerId]);
  const checked = checkingEnabled && check?.key === checkKey ? check : undefined;
  const matched = checked?.value && sameSource(source, checked.value.source) === true;
  const mismatched = checked?.value?.source != null && !matched;
  const reason = blocked ?? (!target || !provider ? 'Choose a cluster and Provider in Clusters first'
    : !checked ? 'Checking this destination…' : checked.error ?? (mismatched ? 'Your saved files changed. Reload them to check this destination.'
    : !checked.value?.canSend ? 'Resolve the destination checks before sending.' : undefined));
  async function mutate(command: 'send_start' | 'send_stop') {
    if (admitted.current) return;
    let args: Record<string, unknown>;
    if (command === 'send_start') {
      if (reason || !target || !providerId || !checked?.value?.source) return;
      args = { targetId: target.id, providerId, expectedSource: checked.value.source };
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
  const outdated = !!snapshot && (!current || !samePlace);
  const uncertain = snapshot?.state === 'unknown' || snapshot?.report?.verdict === 'unknown';
  const resultState: Status = outdated ? 'stale' : running || busy || (checkingEnabled && !checked) ? 'running' : uncertain ? 'not-started' : snapshot?.report?.verdict === 'passed' ? 'passed' : snapshot?.report?.verdict === 'failed' || snapshot?.error ? 'failed' : reason ? 'needs-you' : 'not-started';
  return <section tabIndex={-1} className="send-panel" aria-label="Send and cluster checks" hidden={!visible && !remote && !running}>
    <LongCard title={visible ? 'Send your agent' : 'Checks on your cluster'} status={<StatusPill state={resultState}>{resultState === 'not-started' && uncertain ? snapshot?.stage === 'stopped' ? 'Stopped watching' : 'Result unknown' : undefined}</StatusPill>} summary="The saved agent and check files go together. Each Send creates a new Agent, kept on the cluster even if a check fails."
      action={<div className="workspace-step-actions"><button type="button" className="setup-button" disabled={!!reason || busy || running || !!message} title={reason} onClick={() => { void mutate('send_start'); }}>{visible ? 'Send and run checks' : 'Run all checks'}</button>
      {running && <button type="button" className="workspace-secondary" disabled={busy} onClick={() => { void mutate('send_stop'); }}>Stop</button>}
    </div>}>
    <CardSection title="What you send">
    <dl className="cluster-details"><div><dt>Source</dt><dd>Your saved agent file{model && <> · <code>{model}</code></>}</dd></div>
      <div><dt>This computer</dt><dd>{local?.report ? localCurrent ? `Checks: ${words[local.report.verdict]}` : 'Changed since local checks, or revision not confirmed' : 'No local check result'}</dd></div>
      <div><dt>Destination</dt><dd>{target ? <><code>{target.context_name}</code> · <code>{target.watched_namespace}</code></> : 'Choose a cluster'}</dd></div>
      {provider && <div><dt>Provider</dt><dd><code>{provider.name}</code> · {provider.ready ? 'Ready' : 'Not ready'}</dd></div>}
    </dl>
    {(!target || !provider) && <button type="button" className="workspace-secondary" onClick={onChooseCluster} disabled={!onChooseCluster}>Choose a cluster</button>}
    {checkingEnabled && <section className="send-preflight" aria-label="Before Send" aria-busy={!checked}>
      <h4>Before Send</h4>
      {!mismatched && checked?.value && <ul>{checked.value.rows.map(row => <li key={`${row.label}-${row.message}`} data-state={row.status}>
        <span className="preflight-state"><StatusIcon state={row.status === 'allowed' ? 'passed' : row.status === 'blocked' ? 'needs-you' : row.status === 'cannotCheck' ? 'stale' : 'not-started'} />{{ allowed: 'Ready', blocked: 'Blocked', cannotCheck: 'Can’t check', unverified: 'Unverified' }[row.status]}</span>
        <div><b>{row.label}</b><p>{row.message}</p></div>
      </li>)}</ul>}
      {reason && <p role={checked ? 'alert' : 'status'}>{reason}</p>}
      {!checked && <Elapsed />}
      {checked && (mismatched ? <button type="button" className="workspace-secondary" disabled={busy || running || !onReloadSource} onClick={() => {
        setReloadError(undefined);
        void onReloadSource?.().catch(() => setReloadError('Your saved files could not be reloaded. Your draft is still here.'));
      }}>Reload saved files</button> : <button type="button" className="workspace-secondary" disabled={busy || running} onClick={() => setRefresh(v => v + 1)}>Check again</button>)}
      {reloadError && <p role="alert">{reloadError}</p>}
    </section>}
    </CardSection>
    <CardSection title="What happened">
    {message && <p role="alert">{message}</p>}
    {running && <><p role="status">Running checks on {targetName ?? 'the cluster'}…</p><Elapsed /></>}
    {snapshot?.stage === 'stopped' && <p role="status">Stopped watching. The cluster may still be running checks.</p>}
    {snapshot?.error && snapshot.stage !== 'stopped' && <p role="alert">{sendErrors[snapshot.error as keyof typeof sendErrors]}</p>}
    {snapshot && !samePlace && <p role="status">This result belongs to another cluster or Provider. It is not validation of your current selection.</p>}
    {snapshot && !current && <p className="muted">This result is for the captured file, not confirmed as your current revision.</p>}
    {snapshot?.state === 'unknown' && !snapshot.report && <p role="status">Checks: unknown</p>}
    {snapshot?.report && <div className={outdated ? 'result-stale' : undefined}><p role="status">Checks: {words[snapshot.report.verdict]}</p>
      {snapshot.report.cases.map(check => <div className="result-row" key={check.id}><StatusIcon state={outdated ? 'stale' : check.verdict === 'unknown' ? 'not-started' : check.verdict} /><h4><code>{check.id}</code>: {words[check.verdict]}</h4>
        {check.answer !== null ? <p className="reading">{check.answer}</p> : <p>No answer was observed.</p>}
        {!!check.completedTools.length && <p>Completed tool calls: {check.completedTools.join(', ')}</p>}
        <ul>{check.assertions.map(a => <li key={a.id}>{a.id}: {words[a.verdict]}</li>)}</ul>
      </div>)}
    </div>}
    </CardSection>
    {snapshot && <Details hint="Resources and file hashes"><dl className="cluster-details"><div><dt>Result destination</dt><dd>{targetName ?? 'Not confirmed'}</dd></div>
      <div><dt>Model sent</dt><dd><code>{snapshot.model ?? 'Not captured'}</code></dd></div>
    </dl>
      {snapshot.agentName && <GeneratedValue label="Planned Agent" value={snapshot.agentName} />}
      {snapshot.source && <><GeneratedValue label="Agent file hash" value={snapshot.source.agent} /><GeneratedValue label="Check file hash" value={snapshot.source.tests} /></>}
      {snapshot.acknowledged.map(r => <GeneratedValue key={`${r.kind}-${r.uid}`} label={r.kind} value={r.name} />)}
      {snapshot.pending && <GeneratedValue label={`Possibly created ${snapshot.pending.kind}`} value={snapshot.pending.name} />}
      <p>Results are kept only while Bridge is open. Stop ends watching and prevents further submissions; it does not cancel checks already sent.</p>
    </Details>}
    </LongCard>
  </section>;
}
