import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { helpDetails, workspaceRecovery } from './recovery-info';
import { preflightRecovery, type PreflightFailure } from './preflight-state';

export default function Recovery({ detail, preflightFailure, disabled, onRetry, onCluster }: { detail?: string | null; preflightFailure?: PreflightFailure; disabled: boolean; onRetry: () => void; onCluster?: () => void }) {
  const [expanded, setExpanded] = useState(false);
  const [notice, setNotice] = useState('');
  const info = preflightFailure ? preflightRecovery(preflightFailure) : workspaceRecovery(detail);
  async function help() {
    try { await invoke('setup_open_help', { topic: info.topic }); setNotice('Help opened in your browser.'); }
    catch { setNotice("Couldn't open help. Open Show details and ask for help with this error."); }
  }
  async function copy() {
    try { await navigator.clipboard.writeText(helpDetails(info, detail)); setNotice('Details copied. Private diagnostic text is not included.'); }
    catch { setNotice("Couldn't copy. Open Show details to read the error."); }
  }
  return <div className="workspace-failure" role="alert">
    <div className="workspace-failure-explanation"><p className="failure-summary">{info.summary}</p>
    <p>{info.action}</p><p>{preflightFailure ? 'No workspace files were changed by this check.' : 'What you downloaded is kept.'}</p>
    <div className="workspace-step-actions">
      {info.topic && <button type="button" className="workspace-primary" onClick={() => { void help(); }}>Show me how</button>}
      <button type="button" className={info.topic ? 'workspace-secondary' : 'workspace-primary'} disabled={disabled || info.retryBlocked}
        title={info.retryBlocked ? 'Quit Bridge and reopen it before trying again' : undefined} onClick={onRetry}>Try again</button>
      <button type="button" className="workspace-secondary" onClick={() => { void copy(); }}>Copy details for help</button>
    </div>
    <button type="button" className="workspace-disclosure" aria-expanded={expanded} aria-controls="workspace-failure-details" onClick={() => setExpanded(value => !value)}>{expanded ? 'Hide details' : 'Show details'}</button>
    {expanded && <div id="workspace-failure-details" className="workspace-progress-details"><p>Stage: {info.stage}</p><pre tabIndex={0}>{preflightFailure ? info.summary : detail || 'No further error details were reported.'}</pre></div>}
    </div>
    <button type="button" className="workspace-disclosure workspace-fallback" disabled={disabled || !onCluster} onClick={onCluster}>Use a cluster you already have instead</button>
    <p className="muted" role="status">{notice}</p>
  </div>;
}
