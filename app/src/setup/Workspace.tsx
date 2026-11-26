import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Button } from '../components/Button';
import { StatusPill } from '../components/DesignSystem';
import { Disclosure } from '../components/Disclosure';
import { type AgentEntry, creationRoot, listDocuments } from '../designer/documents-api';
import { infraProgress, workspaceView } from './progress';
import { helpDetails, workspaceRecovery } from './recovery-info';
import { preflightRecovery } from './preflight-state';
import type { SetupStatus } from './state';

// Returning workspace management is not another pass through first-run setup.
export default function Workspace({ status, busy, stopping, starting, error, active, onStart, onCancel, onCluster, upgrade, updating, updateBlocked, ordinaryRecovery = !upgrade }: {
  upgrade?: ReactNode; updating?: boolean; updateBlocked?: boolean; ordinaryRecovery?: boolean;
  status: SetupStatus; busy: boolean; stopping: boolean; starting: boolean; error?: string; active: boolean;
  onStart: () => void; onCancel?: () => void; onCluster?: () => void;
}) {
  const [agents, setAgents] = useState<AgentEntry[]>();
  const [path, setPath] = useState<string>();
  const [inside, setInside] = useState(false);
  // A failed start's details start open, as the board shows: the stage and what the workspace reported. A stop's stay closed.
  const [details, setDetails] = useState(() => status.workspaceFailure?.operation !== 'stop');
  const [quitError, setQuitError] = useState(false);
  const [quitting, setQuitting] = useState(false);
  const [helpError, setHelpError] = useState(false);
  const [progressDetails, setProgressDetails] = useState(false);
  const [notice, setNotice] = useState('');
  const helpGeneration = useRef(0);
  async function quit() {
    setQuitting(true); setQuitError(false);
    try { await invoke('quit_bridge'); } catch { setQuitError(true); setQuitting(false); }
  }
  useEffect(() => {
    if (!active) return;
    let current = true;
    setAgents(undefined); setPath(undefined);
    void listDocuments().then(value => { if (current) setAgents(value); });
    void creationRoot().then(value => { if (current) setPath(value); });
    return () => { current = false; };
  }, [active]);
  const view = workspaceView(status, { stopping, starting });
  const failed = status.localVm === 'failed' && !stopping && !starting;
  const closed = status.localVm === 'closed';
  const stopped = status.localVm === 'cancelled';
  const stopFailed = failed && status.workspaceFailure?.operation === 'stop';
  const forcedStop = stopFailed && status.workspaceFailure?.cause === 'forcedStop';
  const recovery = status.preflightFailure ? preflightRecovery(status.preflightFailure) : workspaceRecovery(status.workspaceFailure);
  useEffect(() => {
    helpGeneration.current += 1;
    setHelpError(false); setNotice('');
  }, [status.localVm, status.preflightFailure, status.workspaceFailure?.cause, status.workspaceFailure?.operation]);
  async function help() {
    const generation = helpGeneration.current;
    try { await invoke('setup_open_help', { topic: recovery.topic }); if (generation === helpGeneration.current) setHelpError(false); }
    catch { if (generation === helpGeneration.current) setHelpError(true); }
  }
  async function copy() {
    const info = status.preflightFailure ? { ...recovery, category: status.preflightFailure } : recovery;
    try { await navigator.clipboard.writeText(helpDetails(info, status.workspaceFailure?.codes)); setNotice('Details copied. Private diagnostic text is not included.'); }
    catch { setNotice("Couldn't copy. Open Show details to read the error."); }
  }
  // Starting shows the same bar, elapsed time and phases as first setup, and Cancel pauses it.
  const settingUp = !stopping && ['preparing', 'installing', 'starting'].includes(status.localVm);
  const infra = settingUp ? infraProgress(status) : null;
  const progress = status.workspaceProgress;
  const copying = status.localVm === 'installing' && progress?.phase === 'materializing';
  const statusShown = !upgrade || error || stopFailed || ordinaryRecovery && (failed || stopped);
  // A failure reads as a callout: what happened in the failed colour, then what to do.
  const statusCallout = statusShown && failed && !error;
  const statusLine = statusShown && <p role="status" className={statusCallout ? 'alert-title' : undefined}>{error ? 'Workspace status is unavailable. Check again before starting it.' : updating ? 'Updating the workspace. Your agent files are kept.' : stopFailed && !forcedStop ? "The workspace didn't stop." : failed ? recovery.summary : stopped ? 'The workspace stopped unexpectedly. Your agents and files are kept.' : view.detail}</p>;
  const forcedLines = forcedStop && <><p>{recovery.preservation}</p><p>{recovery.action}</p></>;
  const actionLine = failed && ordinaryRecovery && !stopFailed && recovery.action && <p>{recovery.action.replaceAll('Try again', 'Restart workspace').replaceAll('copy the details for help', 'open Show details for help')}</p>;
  // A failure names itself in the card title; the region keeps its name.
  const title = failed && ordinaryRecovery && !error && (!stopFailed || forcedStop) && recovery.heading || 'Workspace';
  const kept = failed && ordinaryRecovery && !error && (forcedStop ? 'Your workspace and agent files are kept.' : !stopFailed && recovery.preservation);
  return <section className="card returning-workspace" aria-label="Workspace">
    <div className="steps-head"><div className="workspace-title"><h2>{title}</h2><StatusPill state={error ? 'needs-you' : updating ? 'running' : updateBlocked ? 'needs-you' : view.status}>{error ? 'Status unavailable' : updating ? 'Updating' : updateBlocked ? 'Needs you' : view.word}</StatusPill></div>
    </div>
    {statusCallout ? <div className="alert alert-failed workspace-alert">{statusLine}{forcedLines}{actionLine}</div> : <>{statusLine}{forcedLines}</>}
    {kept && <p className="muted">{kept}</p>}
    {infra && <><div role="progressbar" aria-label="Workspace setup progress" aria-valuemin={0} aria-valuemax={100} className="infra-bar indeterminate"><span /></div>
      {infra.text && <p className="muted">{infra.text}</p>}
      <Disclosure label={progressDetails ? 'Hide details' : 'Show details'} open={progressDetails} controls="returning-workspace-progress" onOpenChange={setProgressDetails}>
        {progressDetails && <div id="returning-workspace-progress" className="workspace-progress-details"><dl>
          <div><dt>Checking cluster files</dt><dd>{status.localVm === 'preparing' || progress?.phase === 'verifying' ? 'In progress' : copying || status.localVm === 'starting' ? 'Checked' : 'Waiting'}</dd></div>
          <div><dt>Installing cluster files</dt><dd>{status.localVm === 'starting' ? 'Installed' : copying ? 'In progress' : 'Waiting'}</dd></div>
          <div><dt>Starting the workspace</dt><dd>{status.localVm === 'starting' ? 'In progress' : 'Waiting'}</dd></div></dl></div>}
      </Disclosure>
      {onCancel && <><div className="workspace-step-actions"><Button type="button" kind="secondary" disabled={busy || !!error} onClick={onCancel}>Cancel</Button></div>
        <p className="muted">Cancel pauses setup. Your downloaded files are kept.</p></>}</>}
    {upgrade}
    {(stopFailed || closed) && <div className="workspace-step-actions"><Button type="button" kind="primary" disabled={quitting} onClick={() => { void quit(); }}>Quit Bridge</Button>
      {stopFailed && ordinaryRecovery && <><Button type="button" kind="secondary" disabled title={recovery.action}>Restart workspace</Button>
        <Button type="button" kind="quiet" onClick={() => { void copy(); }}>Copy details for help</Button></>}</div>}
    {!statusCallout && actionLine}
    {ordinaryRecovery && (failed || stopped) && !stopFailed && <div className="workspace-step-actions"><Button type="button" kind="primary" size={stopped ? 'large' : 'default'} disabled={busy || !!error || failed && recovery.retryBlocked} title={failed && recovery.retryBlocked ? recovery.action : undefined} onClick={onStart}>Restart workspace</Button>
      {failed && recovery.topic && <Button type="button" kind="secondary" onClick={() => { void help(); }}>Show me how</Button>}
      {failed && !error && <Button type="button" kind="secondary" onClick={() => { void copy(); }}>Copy details for help</Button>}
      {failed && !error && onCluster && <Button type="button" kind="quiet" disabled={busy} onClick={onCluster}>Use a cluster you already have instead</Button>}</div>}
    {failed && helpError && <p role="alert">Couldn't open help. Open Show details to read the error.</p>}
    {failed && notice && <p role="status">{notice}</p>}
    {quitError && <p role="alert">Couldn't quit Bridge. Try again or use Quit Bridge from its menu.</p>}
    {failed && ordinaryRecovery && <Disclosure label={details ? 'Hide details' : 'Show details'} open={details} controls="returning-workspace-error" onOpenChange={setDetails}>
      {details && <div className="workspace-progress-details"><p>Stage: {recovery.stage}</p><pre id="returning-workspace-error">{status.localVmError || 'No further error details were reported.'}</pre></div>}
    </Disclosure>}
    {/* An update or recovery decision takes the card; the facts return once it is settled. */}
    {!updateBlocked && <>
      <dl className="workspace-facts">
        <div><dt>Agents</dt><dd>{agents ? agents.length ? `${agents[0].name}${agents.length > 1 ? ` and ${agents.length - 1} more` : ''}` : 'No saved agents yet' : 'Agent list unavailable'}</dd></div>
        <div><dt>Files</dt><dd>{path ? <code>{path}</code> : 'Folder unavailable'}</dd></div>
      </dl>
      <Disclosure label="What's inside" open={inside} controls="returning-workspace-inside" onOpenChange={setInside}>
        {inside && <p id="returning-workspace-inside">The workspace includes Orka and the tools your agents use. Uses about 4 GB of memory while Bridge is open. Quit Bridge to free it.</p>}
      </Disclosure>
    </>}
  </section>;
}
