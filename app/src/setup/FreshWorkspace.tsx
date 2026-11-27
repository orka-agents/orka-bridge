import { useEffect, useRef, useState } from 'react';
import { Button } from '../components/Button';
import { StatusPill } from '../components/DesignSystem';
import { type FreshStatus, freshDetails, refusalCopy } from './fresh';

// One confirmation and one outcome for Start a fresh workspace, whichever card asked for it.
export default function FreshWorkspace({ asking, working, status, onConfirm, onCancel, onDismiss }: {
  asking: boolean; working: boolean; status?: FreshStatus;
  onConfirm: () => void; onCancel: () => void; onDismiss: () => void;
}) {
  const heading = useRef<HTMLHeadingElement>(null);
  const [notice, setNotice] = useState('');
  const replacing = working || status?.state === 'replacing';
  const view = replacing ? 'replacing' : asking ? 'confirm' : status?.state === 'refused' ? 'refused' : status?.state === 'replaced' ? 'replaced' : undefined;
  useEffect(() => { if (view === 'confirm' || view === 'refused') heading.current?.focus(); setNotice(''); }, [view]);
  if (!view) return null;
  async function copy() {
    try { await navigator.clipboard.writeText(['Bridge fresh workspace', ...freshDetails(status)].join('\n')); setNotice('Details copied. No keys, sign-in tokens or folder paths are included.'); }
    catch { setNotice("Couldn't copy. Share the visible message for help."); }
  }
  if (view === 'confirm') return <section className="card workspace-fresh" aria-labelledby="fresh-title" onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); onCancel(); } }}>
    <div className="steps-head"><h2 id="fresh-title" ref={heading} tabIndex={-1}>Start a fresh workspace?</h2></div>
    <ul className="workspace-fresh-list">
      <li>The workspace on this computer is replaced with a new one. Bridge stops it first.</li>
      <li>Your agent files are kept, and so are your test results and Send history.</li>
      <li>Bridge removes its saved AI sign-in or key, and you connect the AI again.</li>
      <li>Anything that exists only inside the workspace is lost: the agents on the local cluster, and the tasks run there with their answers, including Chat and Try a task conversations. Running your tests again puts your agents back on the local cluster.</li>
      <li>Bridge reuses the download already on this computer.</li>
    </ul>
    {/* The keeping choice leads, as in Bridge's other destructive confirmations. */}
    <div className="workspace-step-actions">
      <Button type="button" kind="primary" onClick={onCancel}>Keep this workspace</Button>
      <Button type="button" kind="secondary" className="button-danger" onClick={onConfirm}>Start a fresh workspace</Button>
    </div>
  </section>;
  if (view === 'replacing') return <section className="card workspace-fresh" aria-labelledby="fresh-title" aria-busy="true">
    <div className="steps-head"><h2 id="fresh-title">Starting a fresh workspace</h2><StatusPill state="running">Replacing</StatusPill></div>
    <p role="status">Stopping the workspace and setting it aside. Your agent files are kept.</p>
    <div className="infra-bar indeterminate" aria-hidden="true"><span /></div>
  </section>;
  if (view === 'refused' && status?.state === 'refused') return <section className="card workspace-fresh" aria-labelledby="fresh-title">
    <div className="workspace-failure" role="alert"><div className="workspace-failure-explanation">
      <h2 id="fresh-title" ref={heading} tabIndex={-1} className="failure-summary">Bridge didn't start a fresh workspace</h2>
      <p>{refusalCopy[status.reason]}</p>
      <p className="muted">The workspace on this computer is as it was. Nothing was moved or deleted.</p>
    </div>
    <div className="workspace-step-actions">
      <Button type="button" kind="secondary" onClick={onDismiss}>Done</Button>
      <Button type="button" kind="quiet" onClick={() => { void copy(); }}>Copy details for help</Button>
    </div></div>
    {notice && <p role="status">{notice}</p>}
  </section>;
  if (status?.state !== 'replaced') return null;
  // The new workspace is getting ready on the ordinary first-setup screen; this only says what changed.
  return <section className="card workspace-fresh workspace-fresh-done" aria-label="Fresh workspace">
    <p role="status"><b>Fresh workspace started.</b> Your agent files are kept. Connect the AI again when the workspace is ready.</p>
    {status.ai === 'kept' && <p>Bridge couldn't confirm its saved sign-in or key was removed. When you connect the AI, choose Reset AI connection to try again.</p>}
    {status.cleaning ? <p className="muted">Deleting the previous workspace…</p>
      : status.reclaimable.length > 0 && <div className="workspace-fresh-reclaim">
        <p>Bridge couldn't delete the previous workspace. To free its space, quit Bridge and delete {status.reclaimable.length > 1 ? 'these folders' : 'this folder'}:</p>
        <ul>{status.reclaimable.map(path => <li key={path}><code>{path}</code></li>)}</ul>
      </div>}
    <div className="workspace-step-actions"><Button type="button" kind="quiet" onClick={onDismiss}>Done</Button></div>
  </section>;
}
