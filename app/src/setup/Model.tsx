import { useEffect, useRef, useState } from 'react';
import { connectModel, removeModel, type ModelFailure } from './modelState';
import { invoke } from '@tauri-apps/api/core';
import type { HelpTopic } from './recovery-info';

export type ModelState = 'idle' | 'connecting' | 'removing' | 'connected' | 'failed';
export default function Model({ enabled, paused = false, onState, onSettled }: {
  enabled: boolean; paused?: boolean; onState?: (state: ModelState) => void; onSettled?: () => void;
}) {
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
    onState?.(!enabled ? 'idle' : busy === 'connect' ? 'connecting' : busy === 'remove' ? 'removing' : ready ? 'connected' : error ? 'failed' : 'idle');
  }, [enabled, busy, ready, error, onState]);
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
  const cliUnavailable = error?.category === 'modelCaptureUnavailable';
  const signInCheck = error && ['modelCaptureFailed', 'modelCaptureInvalid', 'modelCaptureTimedOut'].includes(error.category);
  return <section className="card setup-model" aria-labelledby="model-title" aria-busy={!!busy}>
    <h2 id="model-title">{enabled ? 'Connect a model' : 'Model connection locked'}</h2>
    {!enabled ? <p className="muted">Start the local VM before connecting a model. No credentials are sent.</p>
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
          <button key="model-connect" type="button" className="workspace-primary" disabled={!enabled || paused || !!busy || ready} onClick={() => { void perform('connect'); }}>
            {!enabled ? 'Connect model' : busy === 'connect' ? 'Connecting model…' : ready ? 'Model connected' : error && failedAction === 'connect' ? 'Check again' : 'Use GitHub sign-in'}
          </button>
          <button type="button" className="workspace-link" disabled={!enabled || paused || !!busy} onClick={() => { void perform('remove'); }}>
            {busy === 'remove' ? 'Removing sign-in…' : 'Remove sign-in'}
          </button>
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
