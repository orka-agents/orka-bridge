import { useEffect, useRef, useState } from 'react';
import { Button } from '../components/Button';
import { StatusPill } from '../components/DesignSystem';
import { type FreshPlan, type FreshStatus, downloadLine, freshDetails, refusalCopy } from './fresh';

// One confirmation and one outcome for Start a fresh workspace, whichever card asked for it.
export default function FreshWorkspace({ asking, working, status, plan, onConfirm, onCancel, onDismiss }: {
  asking: boolean; working: boolean; status?: FreshStatus; plan?: FreshPlan;
  onConfirm: () => void; onCancel: () => void; onDismiss: () => void;
}) {
  const heading = useRef<HTMLHeadingElement>(null);
  const [notice, setNotice] = useState('');
  const replacing = working || status?.state === 'replacing';
  // An unfinished transaction stays in view until it is finished or sorted out.
  const view = replacing ? 'replacing' : status?.state === 'interrupted' || status?.state === 'conflict' ? status.state : asking ? 'confirm' : status?.state;
  useEffect(() => { if (view && view !== 'replacing' && view !== 'replaced') heading.current?.focus(); setNotice(''); }, [view]);
  if (!view) return null;
  async function copy() {
    try { await navigator.clipboard.writeText(['Bridge fresh workspace', ...freshDetails(status, plan)].join('\n')); setNotice('Details copied. No keys, sign-in tokens or folder paths are included.'); }
    catch { setNotice("Couldn't copy. Share the visible message for help."); }
  }
  const copyAction = <Button type="button" kind="quiet" onClick={() => { void copy(); }}>Copy details for help</Button>;
  const folders = (list: string[], intro: string) => <div className="workspace-fresh-reclaim">
    <p>{intro}</p>
    <ul>{list.map(path => <li key={path}><code>{path}</code></li>)}</ul>
  </div>;
  const reclaim = (list: string[]) => folders(list, `Bridge couldn't delete the previous workspace. It may still hold that workspace's copy of your sign-in. To remove it and free its space, quit Bridge and delete ${list.length > 1 ? 'these folders' : 'this folder'}:`);
  if (view === 'confirm') return <section className="card workspace-fresh" aria-labelledby="fresh-title" onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); onCancel(); } }}>
    <div className="steps-head"><h2 id="fresh-title" ref={heading} tabIndex={-1}>Start a fresh workspace?</h2></div>
    <ul className="workspace-fresh-list">
      <li>The workspace on this computer is replaced with a new one. Bridge stops it first.</li>
      <li>Your agent files are kept, and so are your test results and Send history.</li>
      <li>Bridge removes its saved AI sign-in or key, and you connect the AI again.</li>
      <li>Anything that exists only inside the workspace is lost: the agents on the local cluster, and the tasks run there with their answers, including Chat and Try a task conversations. Running your tests again puts your agents back on the local cluster.</li>
      <li>{downloadLine(plan)}</li>
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
  if (status?.state === 'refused') return <section className="card workspace-fresh" aria-labelledby="fresh-title">
    <div className="workspace-failure" role="alert"><div className="workspace-failure-explanation">
      <h2 id="fresh-title" ref={heading} tabIndex={-1} className="failure-summary">Bridge didn't start a fresh workspace</h2>
      <p>{refusalCopy(status)}</p>
      <p className="muted">{status.restarted ? 'The workspace on this computer is as it was. Nothing was moved or deleted, and Bridge started it again.' : 'The workspace on this computer is as it was. Nothing was moved or deleted.'}</p>
    </div>
    <div className="workspace-step-actions">
      <Button type="button" kind="secondary" onClick={onDismiss}>Done</Button>
      {copyAction}
    </div></div>
    {!!status.reclaimable?.length && reclaim(status.reclaimable)}
    {notice && <p role="status">{notice}</p>}
  </section>;
  // Set aside but not finished: say what is true and how it finishes.
  if (status?.state === 'interrupted') return <section className="card workspace-fresh" aria-labelledby="fresh-title">
    <div className="workspace-failure" role="alert"><div className="workspace-failure-explanation">
      <h2 id="fresh-title" ref={heading} tabIndex={-1} className="failure-summary">Bridge didn't finish starting a fresh workspace</h2>
      <p>Bridge set the workspace on this computer aside, but couldn't finish replacing it. Your agent files are kept.</p>
      <p className="muted">The workspace can't start until this is finished. Bridge finishes it when you try again, or the next time Bridge opens. If it keeps failing, quit Bridge and reopen it.</p>
    </div>
    <div className="workspace-step-actions">
      <Button type="button" kind="primary" onClick={onConfirm}>Finish starting a fresh workspace</Button>
      {copyAction}
    </div></div>
    {notice && <p role="status">{notice}</p>}
  </section>;
  if (status?.state === 'conflict') return <section className="card workspace-fresh" aria-labelledby="fresh-title">
    <div className="workspace-failure" role="alert"><div className="workspace-failure-explanation">
      <h2 id="fresh-title" ref={heading} tabIndex={-1} className="failure-summary">Bridge found workspace folders it didn't expect</h2>
      <p>While finishing a fresh workspace, Bridge found folders it didn't expect, so it left everything as it is. The workspace can't start until this is sorted out. Your agent files are kept.</p>
    </div>
    <div className="workspace-step-actions">{copyAction}</div></div>
    {folders(status.found, `To continue, quit Bridge and move ${status.found.length > 1 ? 'these' : 'this'} out of Bridge's folder, for example to your desktop, then reopen Bridge. Keep ${status.found.length > 1 ? 'them' : 'it'} until the new workspace works:`)}
    {notice && <p role="status">{notice}</p>}
  </section>;
  if (status?.state !== 'replaced') return null;
  // The new workspace is getting ready on the ordinary first-setup screen; this only says what changed.
  return <section className="card workspace-fresh workspace-fresh-done" aria-label="Fresh workspace">
    <p role="status"><b>Fresh workspace started.</b> Your agent files are kept. Connect the AI again when the workspace is ready.</p>
    {status.ai === 'kept' && <p>Bridge couldn't confirm its saved sign-in or key was removed. When you connect the AI, choose Reset AI connection to try again.</p>}
    {status.ai === 'unconfirmed' && <p>Bridge couldn't confirm its saved sign-in or key was removed, and can't offer to remove it from here. Copy the details for whoever helps you with Bridge.</p>}
    {status.cleaning ? <p className="muted">Deleting the previous workspace…</p> : status.reclaimable.length > 0 && reclaim(status.reclaimable)}
    <div className="workspace-step-actions">
      {status.ai === 'unconfirmed' && copyAction}
      <Button type="button" kind="quiet" onClick={onDismiss}>Done</Button>
    </div>
    {notice && <p role="status">{notice}</p>}
  </section>;
}
