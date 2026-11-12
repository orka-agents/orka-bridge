import { useEffect, useRef, useState } from 'react';
import { StatusPill } from '../components/DesignSystem';
import { connectModel, removeModel, type ModelFailure } from './modelState';
import { invoke } from '@tauri-apps/api/core';
import type { HelpTopic } from './recovery-info';

export type ModelState = 'idle' | 'queued' | 'connecting' | 'removing' | 'connected' | 'failed';
// `enabled` is native permission to touch credentials. `queueable` only permits remembering the
// creator's choice while the workspace is still arriving. `away` means another place was chosen.
export default function Model({ enabled, queueable = false, paused = false, away = false, onState, onSettled }: {
  enabled: boolean; queueable?: boolean; paused?: boolean; away?: boolean; onState?: (state: ModelState) => void; onSettled?: () => void;
}) {
  // In-memory consent only. It never carries a credential and is consumed once.
  const [queued, setQueued] = useState(false);
  const root = useRef<HTMLElement>(null);
  const status = useRef<HTMLDivElement>(null);
  const retry = useRef<HTMLButtonElement>(null);
  const removal = useRef<HTMLButtonElement>(null);
  // Which action the keyboard user started: focus follows its operation and returns to that same action.
  const refocus = useRef<'connect' | 'remove'>(undefined);
  const [busy, setBusy] = useState<'connect' | 'remove'>();
  const [removed, setRemoved] = useState(false);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<ModelFailure>();
  const [notice, setNotice] = useState('');
  const [failedAction, setFailedAction] = useState<'connect' | 'remove'>('connect');
  const epoch = useRef(0);
  const active = useRef(false);
  useEffect(() => {
    epoch.current++;
    active.current = false;
    if (!enabled) { setBusy(undefined); setReady(false); setRemoved(false); setError(undefined); }
    return () => { epoch.current++; };
  }, [enabled]);
  useEffect(() => {
    onState?.(!enabled ? queued ? 'queued' : 'idle' : busy === 'connect' ? 'connecting' : busy === 'remove' ? 'removing' : ready ? 'connected' : error ? 'failed' : queued ? 'queued' : 'idle');
  }, [enabled, busy, ready, error, queued, onState]);
  useEffect(() => { if (away) setQueued(false); }, [away]);
  // Runs after the enabled reset above. An unreadable observation or workspace mutation holds the intent.
  useEffect(() => {
    if (queued && enabled && !paused && !away) { setQueued(false); void perform('connect'); }
  }, [queued, enabled, paused, away]);
  // Busy and queued states replace the action with status; keep keyboard focus with the operation.
  // Focus is restored only while the operation still owns it: on the page (not hidden), and on nothing,
  // the status or its own action. Anywhere else the creator moved it, so drop the intent and leave focus alone.
  useEffect(() => {
    if (!refocus.current) return;
    const at = document.activeElement;
    const owns = !root.current?.closest('[hidden]') && (!at || at === document.body || at === status.current || at === retry.current || at === removal.current);
    if (!owns) { refocus.current = undefined; return; }
    if (busy || queued) { status.current?.focus(); return; }
    (refocus.current === 'remove' ? removal.current : retry.current)?.focus();
    if (document.activeElement === document.body) status.current?.focus();
    refocus.current = undefined;
  }, [busy, queued, ready, error]);
  async function perform(action: 'connect' | 'remove') {
    if (!enabled || paused || active.current || (action === 'connect' && ready)) return;
    active.current = true;
    const request = ++epoch.current;
    setBusy(action); setError(undefined); setRemoved(false); setNotice(''); setFailedAction(action);
    if (action === 'remove') setReady(false);
    const result = await (action === 'connect' ? connectModel() : removeModel());
    if (request !== epoch.current) return;
    active.current = false; setBusy(undefined);
    if (result.ok) { setReady(action === 'connect'); setRemoved(action === 'remove'); }
    else setError(result);
    onSettled?.();
  }
  async function help(topic: HelpTopic) {
    try { await invoke('setup_open_help', { topic }); setNotice('Help opened in your browser.'); }
    catch { setNotice("Couldn't open help. Open the GitHub CLI website for installation and sign-in instructions."); }
  }
  async function copy() {
    if (!error) return;
    try {
      await navigator.clipboard.writeText(['Bridge AI connection', `Operation: ${failedAction === 'remove' ? 'Removing sign-in' : 'Connecting AI'}`, `Category: ${error.category}`,
        ...error.codes.map(code => `OS code: ${code}`)].join('\n'));
      setNotice('Details copied. Private diagnostic text is not included.');
    } catch { setNotice("Couldn't copy. Share the visible error message for help."); }
  }
  function choose() {
    refocus.current = document.activeElement === retry.current ? 'connect' : undefined;
    if (enabled) void perform('connect'); else if (queueable && !paused) setQueued(true);
  }
  const cliUnavailable = error?.category === 'modelCaptureUnavailable';
  const signInCheck = error && ['modelCaptureFailed', 'modelCaptureInvalid', 'modelCaptureTimedOut'].includes(error.category);
  return <section ref={root} className="card setup-model" aria-labelledby="model-title" aria-busy={!!busy}>
    <h2 id="model-title">{enabled || queueable || queued ? 'Connect a model' : 'Model connection locked'}</h2>
    {!enabled && (queueable || queued) ? <p className="muted">You can choose now. Bridge connects it automatically when Orka is ready. No credentials are read or sent until then.</p>
      : !enabled ? <p className="muted">Start the local VM before connecting a model. No credentials are sent.</p>
      : <p className="muted">Use your existing GitHub CLI sign-in for Copilot in this VM. Bridge stores it in the native credential store and delivers it privately to the guest.</p>}
    <div className={error ? 'workspace-failure' : undefined} role={error ? 'alert' : undefined}>
      <div className={error ? 'workspace-failure-explanation' : undefined}>
        {error && <><p className="failure-summary">{error.message}</p>
        <p>{failedAction === 'remove' ? 'Press Remove sign-in again to retry cleanup. If it happens again, copy the details for help.'
          : cliUnavailable ? 'Install GitHub CLI if needed, then sign in to github.com in your terminal:'
          : signInCheck ? 'Check your GitHub CLI sign-in in your terminal:'
          : error.category === 'modelBusy' ? 'Wait for the other operation to finish, then press Check again.'
          : 'Press Check again to reconnect. If it happens again, copy the details for help.'}</p>
        {(cliUnavailable || signInCheck) && <pre>{cliUnavailable ? 'gh auth login --hostname github.com --web' : 'gh auth status --hostname github.com'}</pre>}
        <p>Your running workspace and agent files are kept. {failedAction === 'remove' ? 'Removal has not been confirmed.' : 'A sign-in may already have been saved.'}</p>
        {failedAction === 'connect' && <p className="muted">Check again reconnects the AI and may reuse the sign-in already saved for this workspace.</p>}</>}
        <div className="workspace-step-actions">
          {cliUnavailable && <button type="button" className="workspace-secondary" onClick={() => { void help('githubCliInstall'); }}>Install GitHub CLI</button>}
          {(cliUnavailable || signInCheck) && <button type="button" className="workspace-secondary" onClick={() => { void help('githubSignIn'); }}>Sign-in help</button>}
          {queued || busy || ready ? <div ref={status} tabIndex={-1} className="model-state">
            <StatusPill state={ready ? 'passed' : queued ? 'not-started' : 'running'}>{busy === 'remove' ? 'Removing sign-in…' : ready ? 'Connected' : queued ? 'Queued' : 'Connecting model…'}</StatusPill>
            {queued && <><span className="muted">Connects automatically when Orka is ready.</span>
              <button type="button" className="workspace-link" onClick={() => setQueued(false)}>Cancel queued connection</button></>}
          </div>
          : <button key="model-connect" ref={retry} type="button" className="workspace-primary" disabled={(!enabled && !queueable) || paused || !!busy} onClick={choose}>
            {!enabled && !queueable ? 'Connect model' : error && failedAction === 'connect' ? 'Check again' : 'Use GitHub sign-in'}
          </button>}
          {busy !== 'remove' && <button ref={removal} type="button" className="workspace-link" disabled={!enabled || paused || !!busy}
            onClick={() => { refocus.current = document.activeElement === removal.current ? 'remove' : undefined; void perform('remove'); }}>Remove sign-in</button>}
          {error && <button type="button" className="workspace-secondary" onClick={() => { void copy(); }}>Copy details for help</button>}
        </div>
      </div>
    </div>
    {busy === 'connect' && <p role="status">Connecting with Bridge’s saved sign-in, or reading your GitHub CLI sign-in if none is saved…</p>}
    {busy === 'remove' && <p role="status">Stopping the guest model and removing Bridge’s saved sign-in…</p>}
    {removed && <p role="status">Bridge no longer has a saved sign-in. GitHub CLI stays signed in.</p>}
    {ready && <><p role="status">Model connected: copilot / gpt-5.6-terra.</p>
      <p className="muted">The guest acknowledged Connect. This does not prove inference.</p></>}
    {notice && <p role="status">{notice}</p>}
  </section>;
}
